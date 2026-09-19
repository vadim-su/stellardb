//! Graph traversal operator using BFS.

use std::collections::{HashSet, VecDeque};

use crate::document::{Document, Value};
use crate::edge::Edge;
use crate::query::ast::{
    TraversalDepth, TraversalDirection, TraversalExpr, TraversalMode, TraversalStep,
    TraversalTarget,
};
use crate::query::error::ExecuteError;
use crate::query::execute::context::ExecutionContext;
use crate::query::execute::eval::eval_filter;

use super::operator::{Operator, Row};

const DEFAULT_MAX_DEPTH: usize = 10;

/// Minimum number of nodes to trigger batch loading
const BATCH_THRESHOLD: usize = 64;
/// Maximum number of nodes per batch chunk
const BATCH_CHUNK_SIZE: usize = 256;
/// Default limit for edges per node
const DEFAULT_EDGE_LIMIT: usize = 1000;

pub struct TraversalOp<'a, Ctx: ExecutionContext> {
    ctx: &'a Ctx,
    expr: TraversalExpr,
    start_nodes: Vec<String>,

    // Depth buckets for level-based BFS
    depth_buckets: Vec<Vec<String>>,
    current_depth: usize,

    // Pending results from batch processing
    pending_results: VecDeque<(Edge, String)>, // (edge, next_node)

    visited: HashSet<String>,
    opened: bool,
}

impl<'a, Ctx: ExecutionContext> TraversalOp<'a, Ctx> {
    pub fn new(ctx: &'a Ctx, expr: TraversalExpr, start_nodes: Vec<String>) -> Self {
        let expr = Self::normalize_target(ctx, expr);
        let max_depth = Self::calculate_max_depth(&expr);

        Self {
            ctx,
            expr,
            start_nodes,
            depth_buckets: vec![Vec::new(); max_depth + 1],
            current_depth: 0,
            pending_results: VecDeque::new(),
            visited: HashSet::new(),
            opened: false,
        }
    }

    fn calculate_max_depth(expr: &TraversalExpr) -> usize {
        let mut max = 0;
        for step in &expr.steps {
            max += match &step.depth {
                TraversalDepth::Single => 1,
                TraversalDepth::Exact(n) => *n,
                TraversalDepth::Range { max: Some(m), .. } => *m,
                TraversalDepth::Range { max: None, .. } => DEFAULT_MAX_DEPTH,
            };
        }
        max
    }

    /// Normalize traversal expression:
    /// 1. If target is Node/NodeAll/NodeField/NodeFields with a name that's not a collection,
    ///    treat it as an edge label and add it as a step.
    /// 2. If intermediate step labels are collection names (not edge labels),
    ///    convert them to node_filter on the following step.
    fn normalize_target(ctx: &Ctx, mut expr: TraversalExpr) -> TraversalExpr {
        // Step 1: Normalize target (if target name is not a collection, it's an edge label)
        let (collection_name, new_target) = match &expr.target {
            TraversalTarget::Node(name) => (Some(name.clone()), Some(TraversalTarget::EdgeAll)),
            TraversalTarget::NodeAll(name) => (Some(name.clone()), Some(TraversalTarget::EdgeAll)),
            TraversalTarget::NodeField(name, field) => (
                Some(name.clone()),
                Some(TraversalTarget::EdgeField(field.clone())),
            ),
            TraversalTarget::NodeFields(name, fields) => (
                Some(name.clone()),
                Some(TraversalTarget::EdgeFields(fields.clone())),
            ),
            _ => (None, None),
        };

        if let Some(name) = collection_name {
            // If collection doesn't exist, treat target as edge label
            if !ctx.collection_exists(&name) {
                // Use target_direction from parser (captures -> or <- before target)
                // Fall back to Outgoing if not specified
                let direction = expr
                    .target_direction
                    .unwrap_or(TraversalDirection::Outgoing);

                // Add new step with this label
                expr.steps.push(TraversalStep {
                    direction,
                    label: Some(name),
                    depth: TraversalDepth::Single,
                    mode: TraversalMode::Deduplicate,
                    edge_filter: None,
                    node_filter: None,
                });

                // Use the preserved target type
                if let Some(target) = new_target {
                    expr.target = target;
                }
            }
        }

        // Step 2: Handle intermediate collection names as node filters
        // If a step's label is an existing collection name, it means "filter to nodes in this collection"
        // not "traverse edges with this label"
        // Example: ->viewed->product->bought_together
        //   - "product" is a collection, so it's a node filter between viewed and bought_together edges
        //   - Result: [viewed, bought_together{node_filter: product}]
        if expr.steps.len() >= 2 {
            let mut new_steps = Vec::new();
            let mut pending_node_filter: Option<String> = None;

            for step in expr.steps.drain(..) {
                if let Some(ref label) = step.label
                    && ctx.collection_exists(label)
                {
                    // This step's label is a collection name, not an edge label
                    // Save it as node_filter for the next step
                    pending_node_filter = Some(label.clone());
                    continue; // Skip adding this step - it's a filter, not a traversal
                }

                // Add this step, possibly with a pending node_filter
                let mut step_with_filter = step;
                if pending_node_filter.is_some() {
                    step_with_filter.node_filter = pending_node_filter.take();
                }
                new_steps.push(step_with_filter);
            }

            // Apply a trailing collection filter to the last traversal step.
            if let Some(filter) = pending_node_filter
                && let Some(last_step) = new_steps.last_mut()
            {
                last_step.node_filter = Some(filter);
            }

            expr.steps = new_steps;
        }

        expr
    }

    fn get_max_depth(&self) -> usize {
        let mut max = 0;
        for step in &self.expr.steps {
            max += match &step.depth {
                TraversalDepth::Single => 1,
                TraversalDepth::Exact(n) => *n,
                TraversalDepth::Range { max: Some(m), .. } => *m,
                TraversalDepth::Range { max: None, .. } => DEFAULT_MAX_DEPTH,
            };
        }
        max
    }

    /// Create a Row from an Edge for use in edge filter evaluation.
    fn edge_to_row(edge: &Edge) -> Row {
        let mut fields = edge.fields.clone();
        fields.insert("id".into(), Value::String(edge.id.clone()));
        fields.insert("from".into(), Value::Reference(edge.from.clone()));
        fields.insert("to".into(), Value::Reference(edge.to.clone()));
        Row::from_doc(Document {
            id: edge.id.clone(),
            fields,
        })
    }

    fn traverse_step(
        &self,
        from: &str,
        step: &TraversalStep,
    ) -> Result<Vec<(Edge, String)>, ExecuteError> {
        // Check node_filter: if set, current node must belong to this collection
        // Node ID format: collection:key
        if let Some(ref collection) = step.node_filter {
            let expected_prefix = format!("{}:", collection);
            if !from.starts_with(&expected_prefix) {
                return Ok(Vec::new()); // Skip - node doesn't match collection filter
            }
        }

        let mut results = Vec::new();

        let edges = match step.direction {
            TraversalDirection::Outgoing => {
                let label = step.label.as_deref().unwrap_or("");
                self.ctx
                    .list_edges_out(from, label, None, 1000)
                    .map_err(ExecuteError::Storage)?
                    .0
            }
            TraversalDirection::Incoming => {
                let label = step.label.as_deref().unwrap_or("");
                self.ctx
                    .list_edges_in(from, label, None, 1000)
                    .map_err(ExecuteError::Storage)?
                    .0
            }
            TraversalDirection::Bidirectional => {
                let label = step.label.as_deref().unwrap_or("");
                let mut out = self
                    .ctx
                    .list_edges_out(from, label, None, 1000)
                    .map_err(ExecuteError::Storage)?
                    .0;
                let in_edges = self
                    .ctx
                    .list_edges_in(from, label, None, 1000)
                    .map_err(ExecuteError::Storage)?
                    .0;
                out.extend(in_edges);
                out
            }
        };

        for edge in edges {
            // Apply edge filter if present
            if let Some(ref filter) = step.edge_filter {
                let edge_row = Self::edge_to_row(&edge);
                if !eval_filter(filter, &edge_row, self.ctx)? {
                    continue;
                }
            }

            let next_node = match step.direction {
                TraversalDirection::Outgoing => edge.to.clone(),
                TraversalDirection::Incoming => edge.from.clone(),
                TraversalDirection::Bidirectional => {
                    if edge.from == from {
                        edge.to.clone()
                    } else {
                        edge.from.clone()
                    }
                }
            };

            results.push((edge, next_node));
        }

        Ok(results)
    }

    fn format_result(&self, edge: &Edge, node_id: &str) -> Result<Row, ExecuteError> {
        match &self.expr.target {
            TraversalTarget::Edge => {
                let doc = Document {
                    id: edge.id.clone(),
                    fields: [("id".to_string(), Value::String(edge.id.clone()))].into(),
                };
                Ok(Row::from_doc(doc))
            }
            TraversalTarget::EdgeAll => {
                let mut fields = edge.fields.clone();
                fields.insert("id".to_string(), Value::String(edge.id.clone()));
                fields.insert("from".to_string(), Value::Reference(edge.from.clone()));
                fields.insert("to".to_string(), Value::Reference(edge.to.clone()));
                let doc = Document {
                    id: edge.id.clone(),
                    fields,
                };
                Ok(Row::from_doc(doc))
            }
            TraversalTarget::EdgeField(field) => {
                let value = edge.fields.get(field).cloned().unwrap_or(Value::Null);
                let doc = Document {
                    id: edge.id.clone(),
                    fields: [(field.clone(), value)].into(),
                };
                Ok(Row::from_doc(doc))
            }
            TraversalTarget::Node(_collection) => {
                let doc = Document {
                    id: node_id.to_string(),
                    fields: [("id".to_string(), Value::String(node_id.to_string()))].into(),
                };
                Ok(Row::from_doc(doc))
            }
            TraversalTarget::NodeAll(_collection) => {
                let parts: Vec<&str> = node_id.splitn(2, ':').collect();
                if parts.len() == 2
                    && let Ok(Some(doc)) = self.ctx.get_document(parts[0], parts[1])
                {
                    return Ok(Row::from_doc(doc));
                }
                let doc = Document {
                    id: node_id.to_string(),
                    fields: [("id".to_string(), Value::String(node_id.to_string()))].into(),
                };
                Ok(Row::from_doc(doc))
            }
            TraversalTarget::NodeField(_collection, field) => {
                let parts: Vec<&str> = node_id.splitn(2, ':').collect();
                if parts.len() == 2
                    && let Ok(Some(doc)) = self.ctx.get_document(parts[0], parts[1])
                {
                    let value = doc.fields.get(field).cloned().unwrap_or(Value::Null);
                    let result_doc = Document {
                        id: node_id.to_string(),
                        fields: [(field.clone(), value)].into(),
                    };
                    return Ok(Row::from_doc(result_doc));
                }
                Ok(Row::from_doc(Document {
                    id: node_id.to_string(),
                    fields: [(field.clone(), Value::Null)].into(),
                }))
            }
            TraversalTarget::EdgeFields(selections) => {
                let mut fields = std::collections::HashMap::new();

                // Build edge object with from/to as Reference
                let mut edge_obj = edge.fields.clone();
                edge_obj.insert("id".to_string(), Value::String(edge.id.clone()));
                edge_obj.insert("from".to_string(), Value::Reference(edge.from.clone()));
                edge_obj.insert("to".to_string(), Value::Reference(edge.to.clone()));

                for sel in selections {
                    let value = self.resolve_field_path(&edge_obj, &sel.path)?;
                    let key = sel.alias.clone().unwrap_or_else(|| {
                        sel.path.rsplit('.').next().unwrap_or(&sel.path).to_string()
                    });
                    fields.insert(key, value);
                }

                Ok(Row::from_doc(Document {
                    id: edge.id.clone(),
                    fields,
                }))
            }
            TraversalTarget::NodeFields(_, selections) => {
                let node_doc = self.load_node_document(node_id)?;
                let mut fields = std::collections::HashMap::new();

                for sel in selections {
                    let value = self.resolve_field_path(&node_doc.fields, &sel.path)?;
                    let key = sel.alias.clone().unwrap_or_else(|| {
                        sel.path.rsplit('.').next().unwrap_or(&sel.path).to_string()
                    });
                    fields.insert(key, value);
                }

                Ok(Row::from_doc(Document {
                    id: node_id.to_string(),
                    fields,
                }))
            }
        }
    }

    /// Load a node document by its full ID (collection:key)
    fn load_node_document(&self, node_id: &str) -> Result<Document, ExecuteError> {
        let parts: Vec<&str> = node_id.splitn(2, ':').collect();
        if parts.len() == 2
            && let Ok(Some(doc)) = self.ctx.get_document(parts[0], parts[1])
        {
            return Ok(doc);
        }
        // Return empty doc if not found
        Ok(Document {
            id: node_id.to_string(),
            fields: std::collections::HashMap::new(),
        })
    }

    /// Resolve a dot-separated field path, dereferencing References as needed
    fn resolve_field_path(
        &self,
        obj: &std::collections::HashMap<String, Value>,
        path: &str,
    ) -> Result<Value, ExecuteError> {
        let segments: Vec<&str> = path.split('.').collect();
        if segments.is_empty() {
            return Ok(Value::Null);
        }

        let mut current = obj.get(segments[0]).cloned().unwrap_or(Value::Null);

        for segment in &segments[1..] {
            current = match current {
                Value::Object(ref map) => map.get(*segment).cloned().unwrap_or(Value::Null),
                Value::Reference(ref ref_id) => {
                    // Dereference and get field
                    let doc = self.load_node_document(ref_id)?;
                    doc.fields.get(*segment).cloned().unwrap_or(Value::Null)
                }
                _ => Value::Null,
            };
        }

        Ok(current)
    }

    /// Check if we should use batch loading for current depth
    fn should_use_batch(&self) -> bool {
        self.depth_buckets
            .get(self.current_depth)
            .map(|bucket| bucket.len() >= BATCH_THRESHOLD)
            .unwrap_or(false)
    }

    /// Process a batch of nodes at current depth
    fn process_batch(&mut self, step: &TraversalStep) -> Result<(), ExecuteError> {
        let bucket = &mut self.depth_buckets[self.current_depth];
        let chunk_size = std::cmp::min(BATCH_CHUNK_SIZE, bucket.len());
        let chunk: Vec<String> = bucket.drain(..chunk_size).collect();
        let refs: Vec<&str> = chunk.iter().map(|s| s.as_str()).collect();

        let label = step.label.as_deref().unwrap_or("");

        let edges_map = match step.direction {
            TraversalDirection::Outgoing => self
                .ctx
                .batch_list_edges_out(&refs, label, DEFAULT_EDGE_LIMIT)
                .map_err(ExecuteError::Storage)?,
            TraversalDirection::Incoming => self
                .ctx
                .batch_list_edges_in(&refs, label, DEFAULT_EDGE_LIMIT)
                .map_err(ExecuteError::Storage)?,
            TraversalDirection::Bidirectional => {
                let mut out = self
                    .ctx
                    .batch_list_edges_out(&refs, label, DEFAULT_EDGE_LIMIT)
                    .map_err(ExecuteError::Storage)?;
                let in_edges = self
                    .ctx
                    .batch_list_edges_in(&refs, label, DEFAULT_EDGE_LIMIT)
                    .map_err(ExecuteError::Storage)?;
                // Merge incoming edges
                for (node, edges) in in_edges {
                    out.entry(node).or_default().extend(edges);
                }
                out
            }
        };

        // Process results - check node filter for each source node
        for (from, edges) in edges_map {
            // Check node_filter: if set, source node must belong to this collection
            if let Some(ref collection) = step.node_filter {
                let expected_prefix = format!("{}:", collection);
                if !from.starts_with(&expected_prefix) {
                    continue; // Skip - node doesn't match collection filter
                }
            }

            for edge in edges {
                // Apply edge filter if present
                if let Some(ref filter) = step.edge_filter {
                    let edge_row = Self::edge_to_row(&edge);
                    if !eval_filter(filter, &edge_row, self.ctx)? {
                        continue;
                    }
                }

                let next_node = match step.direction {
                    TraversalDirection::Outgoing => edge.to.clone(),
                    TraversalDirection::Incoming => edge.from.clone(),
                    TraversalDirection::Bidirectional => {
                        if edge.from == from {
                            edge.to.clone()
                        } else {
                            edge.from.clone()
                        }
                    }
                };
                self.pending_results.push_back((edge, next_node));
            }
        }

        Ok(())
    }

    /// Process nodes sequentially (for small batches)
    fn process_sequential(&mut self, step: &TraversalStep) -> Result<(), ExecuteError> {
        let bucket = &mut self.depth_buckets[self.current_depth];
        if let Some(current) = bucket.pop() {
            let neighbors = self.traverse_step(&current, step)?;
            for (edge, next_node) in neighbors {
                self.pending_results.push_back((edge, next_node));
            }
        }
        Ok(())
    }

    /// Map an absolute depth to (step_index, relative_depth_within_step).
    /// Each step occupies a number of depth levels based on its depth spec.
    fn depth_to_step(&self, depth: usize) -> Option<(usize, usize)> {
        let mut offset = 0;
        for (i, step) in self.expr.steps.iter().enumerate() {
            let step_levels = match &step.depth {
                TraversalDepth::Single => 1,
                TraversalDepth::Exact(n) => *n,
                TraversalDepth::Range { max: Some(m), .. } => *m,
                TraversalDepth::Range { max: None, .. } => DEFAULT_MAX_DEPTH,
            };
            if depth < offset + step_levels {
                return Some((i, depth - offset));
            }
            offset += step_levels;
        }
        None
    }

    /// Get the min relative depth for a step (0-indexed).
    /// Single: 0 (emit at depth 0). Exact(n): n-1 (emit only at last level).
    /// Range: min-1 (emit starting from min level).
    fn step_min_depth(step: &TraversalStep) -> usize {
        match &step.depth {
            TraversalDepth::Single => 0,
            TraversalDepth::Exact(n) => n.saturating_sub(1),
            TraversalDepth::Range { min: Some(m), .. } => m.saturating_sub(1),
            TraversalDepth::Range { min: None, .. } => 0,
        }
    }

    /// Check if an absolute depth is the last level for its step.
    fn is_last_depth_for_step(&self, depth: usize) -> bool {
        if let Some((step_idx, rel_depth)) = self.depth_to_step(depth) {
            let step = &self.expr.steps[step_idx];
            let step_levels = match &step.depth {
                TraversalDepth::Single => 1,
                TraversalDepth::Exact(n) => *n,
                TraversalDepth::Range { max: Some(m), .. } => *m,
                TraversalDepth::Range { max: None, .. } => DEFAULT_MAX_DEPTH,
            };
            rel_depth + 1 >= step_levels
        } else {
            true
        }
    }
}

impl<Ctx: ExecutionContext> Operator for TraversalOp<'_, Ctx> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        self.opened = true;

        // Initialize depth bucket 0 with start nodes
        for node in &self.start_nodes {
            self.depth_buckets[0].push(node.clone());
            if self.expr.steps.first().map(|s| s.mode) != Some(TraversalMode::All) {
                self.visited.insert(node.clone());
            }
        }

        Ok(())
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        let max_depth = self.get_max_depth();

        loop {
            // 1. First, process any pending results
            while let Some((edge, next_node)) = self.pending_results.pop_front() {
                let Some((step_idx, rel_depth)) = self.depth_to_step(self.current_depth) else {
                    continue;
                };
                let step = &self.expr.steps[step_idx];
                let should_dedupe = step.mode == TraversalMode::Deduplicate;

                let is_last_step = step_idx + 1 >= self.expr.steps.len();
                let is_last_depth_in_step = self.is_last_depth_for_step(self.current_depth);
                let min_depth = Self::step_min_depth(step);

                // Should we emit this result? Only on the last step, at or above min depth
                let should_emit = is_last_step && rel_depth >= min_depth;
                // Should we continue BFS to next depth bucket?
                // Yes if: still within this step's range, OR moving to next step
                let should_continue = !is_last_depth_in_step || !is_last_step;

                if should_emit {
                    let dedupe_key = match &self.expr.target {
                        TraversalTarget::Edge
                        | TraversalTarget::EdgeAll
                        | TraversalTarget::EdgeField(_)
                        | TraversalTarget::EdgeFields(_) => edge.id.clone(),
                        _ => next_node.clone(),
                    };

                    let is_dup = should_dedupe && self.visited.contains(&dedupe_key);

                    if !is_last_depth_in_step && !is_dup {
                        // Emit AND continue within same step (range depth): add to next bucket
                        if self.current_depth + 1 < self.depth_buckets.len() {
                            self.depth_buckets[self.current_depth + 1].push(next_node.clone());
                        }
                    }

                    if is_dup {
                        continue;
                    }

                    if should_dedupe {
                        self.visited.insert(dedupe_key);
                    }

                    return Ok(Some(self.format_result(&edge, &next_node)?));
                } else if should_continue {
                    // Not emitting: either intermediate step or below min depth
                    if should_dedupe && self.visited.contains(&next_node) {
                        continue;
                    }
                    if should_dedupe {
                        self.visited.insert(next_node.clone());
                    }
                    if self.current_depth + 1 < self.depth_buckets.len() {
                        self.depth_buckets[self.current_depth + 1].push(next_node);
                    }
                }
                // else: last depth of last step, below min — skip
            }

            // 2. Check if current depth has nodes to process
            let has_nodes = self
                .depth_buckets
                .get(self.current_depth)
                .map(|b| !b.is_empty())
                .unwrap_or(false);

            if !has_nodes {
                // Move to next depth
                self.current_depth += 1;
                if self.current_depth >= max_depth {
                    return Ok(None); // BFS complete
                }
                // Check if depth maps to a valid step
                if self.depth_to_step(self.current_depth).is_none() {
                    return Ok(None);
                }
                continue;
            }

            // 3. Process nodes at current depth
            let Some((step_idx, _)) = self.depth_to_step(self.current_depth) else {
                return Ok(None);
            };
            let step = self.expr.steps[step_idx].clone();

            // Check node_filter before processing (for sequential path)
            if let Some(filter_collection) = &step.node_filter {
                // Filter nodes in bucket
                let bucket = &mut self.depth_buckets[self.current_depth];
                let expected_prefix = format!("{}:", filter_collection);
                bucket.retain(|node| node.starts_with(&expected_prefix));
            }

            if self.should_use_batch() {
                self.process_batch(&step)?;
            } else {
                self.process_sequential(&step)?;
            }
        }
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        self.depth_buckets.iter_mut().for_each(|b| b.clear());
        self.pending_results.clear();
        self.visited.clear();
        Ok(())
    }
}
