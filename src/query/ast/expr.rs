use crate::document::Value;

/// Binary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    // Comparison
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    // Logical
    And,
    Or,
    // Coalesce
    Coalesce, // ??
}

/// Unary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,       // -expr
    Not,       // NOT expr
    IsNull,    // expr IS NULL
    IsNotNull, // expr IS NOT NULL
    IsNone,    // expr IS NONE (field missing)
    IsNotNone, // expr IS NOT NONE (field exists)
}

/// Argument to an aggregate function
#[derive(Debug, Clone, PartialEq)]
pub enum AggregateArg {
    /// COUNT(*)
    Wildcard,
    /// SUM(field), COUNT(field), etc.
    Field(String),
}

/// An aggregate function call in projection
#[derive(Debug, Clone, PartialEq)]
pub struct AggregateCall {
    pub namespace: Option<String>,
    pub function: String,
    pub arg: AggregateArg,
    pub distinct: bool,
}

/// FTS operator type
#[derive(Debug, Clone, PartialEq)]
pub enum FtsOperator {
    /// Simple @@ operator
    Simple,
    /// Named @:name@ operator
    Named(String),
}

/// Field with optional boost for MATCH clause
#[derive(Debug, Clone, PartialEq)]
pub struct MatchField {
    pub name: String,
    pub boost: Option<f64>,
}

/// MATCH mode for combining scores
#[derive(Debug, Clone, PartialEq, Default)]
pub enum MatchMode {
    /// Sum of all field scores (default)
    #[default]
    Sum,
    /// Max score plus tie_breaker * rest
    Max { tie_breaker: Option<f64> },
}

/// FTS match target - single field or MATCH clause
#[derive(Debug, Clone, PartialEq)]
pub enum FtsTarget {
    /// Single field: field @@ "query"
    Field(String),
    /// Multi-field: MATCH(title^2, body) @@ "query"
    Match {
        fields: Vec<MatchField>,
        mode: MatchMode,
    },
}

/// Reference to parent context in correlated subquery.
/// Supports nested parents: $parent.field, $parent.parent.field, etc.
#[derive(Debug, Clone, PartialEq)]
pub struct ParentRef {
    /// Number of levels up: 0 = $parent, 1 = $parent.parent, ...
    pub depth: usize,
    /// Field name to access (supports nested: "address.city")
    pub field: String,
}

/// A block of statements that returns the value of the last expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub statements: Vec<super::stmt::Statement>,
}

/// Unified expression node
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Literal value: 42, "hello", true, null
    Literal(Value),
    /// Field reference: name, score, address.city
    Field(String),
    /// Binary operation: expr + expr, expr AND expr, expr = expr
    BinaryOp {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
    },
    /// Unary operation: -expr, NOT expr
    UnaryOp { op: UnaryOp, expr: Box<Expr> },
    /// Scalar function call: UPPER(name), CONCAT(a, b)
    FunctionCall {
        name: String,
        namespace: Option<String>,
        args: Vec<Expr>,
    },
    /// Aggregate function call (only in projections)
    Aggregate(AggregateCall),
    /// IN with literal list: status IN ["active", "pending"]
    InList {
        expr: Box<Expr>,
        list: Vec<Expr>,
        negated: bool,
    },
    /// IN with subquery: id IN (SELECT ...)
    InSubquery {
        expr: Box<Expr>,
        subquery: Box<super::stmt::SelectAst>,
        negated: bool,
    },
    /// IN with expression (variable or field): status IN $statuses
    InExpr {
        expr: Box<Expr>,
        target: Box<Expr>,
        negated: bool,
    },
    /// Subquery in projection: (SELECT ...)
    Subquery(Box<super::stmt::SelectAst>),
    /// Parent reference: $parent.field or $parent.parent.field
    ParentRef(ParentRef),
    /// Element reference: $value or $value.name
    ValueRef(Option<String>),
    /// Array expression: [expr, expr, ...]
    Array(Vec<Expr>),
    /// Object expression: {key: expr, key: expr, ...}
    Object(Vec<(String, Expr)>),
    /// Array/subquery indexing: `expr[index]`
    Index { base: Box<Expr>, index: Box<Expr> },
    /// Array/subquery slicing: expr[start..end]
    Slice {
        base: Box<Expr>,
        start: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
    },
    /// Postfix field access: expr.field
    FieldAccess { base: Box<Expr>, field: String },
    /// Constant reference: math::pi, math::e (no parentheses)
    Constant { namespace: String, name: String },
    /// Variable reference: $x, $tax_rate
    Variable(String),
    /// Range expression: start..end (always inclusive)
    Range { start: Box<Expr>, end: Box<Expr> },
    /// Full-text search expression
    Fts {
        target: FtsTarget,
        operator: FtsOperator,
        query: String,
    },
    /// KNN vector search: embedding <|10|> [0.1, 0.2, ...]
    KnnSearch {
        /// Field containing the vector
        field: String,
        /// Query vector expression
        vector: Box<Expr>,
        /// Number of neighbors to return
        k: usize,
        /// Optional ef_search parameter for accuracy/speed tradeoff
        effort: Option<usize>,
    },
    /// Graph traversal: ->follows->user, <-likes{1..3}<-post
    Traversal(TraversalExpr),
    /// IF cond THEN block [ELSE IF cond THEN block]* [ELSE block] END
    If {
        branches: Vec<(Box<Expr>, Block)>,
        else_block: Option<Block>,
    },
    /// FOR $var IN iterable DO block END
    For {
        binding: String,
        iterable: Box<Expr>,
        body: Block,
    },
    /// BREAK — exit innermost FOR loop
    Break,
    /// CONTINUE — skip to next iteration
    Continue,
}

/// Result type for `Expr::transform()`.
pub enum TransformResult {
    /// Replace this node (no further recursion into its children).
    Replace(Expr),
    /// Recurse into children of the given expression.
    Recurse(Expr),
}

impl Expr {
    /// Visit every `Expr` node in the tree (pre-order).
    /// The callback receives each node and returns `ControlFlow::Continue(())` to keep
    /// walking or `ControlFlow::Break(T)` to stop immediately.
    pub fn walk<T>(
        &self,
        f: &mut impl FnMut(&Expr) -> std::ops::ControlFlow<T>,
    ) -> std::ops::ControlFlow<T> {
        use std::ops::ControlFlow;

        f(self)?;

        match self {
            // Leaf nodes — no children
            Expr::Literal(_)
            | Expr::Field(_)
            | Expr::ParentRef(_)
            | Expr::ValueRef(_)
            | Expr::Variable(_)
            | Expr::Constant { .. }
            | Expr::Aggregate(_)
            | Expr::Fts { .. }
            | Expr::Break
            | Expr::Continue => ControlFlow::Continue(()),

            Expr::BinaryOp { left, right, .. } => {
                left.walk(f)?;
                right.walk(f)
            }
            Expr::UnaryOp { expr, .. } => expr.walk(f),
            Expr::FunctionCall { args, .. } => {
                for arg in args {
                    arg.walk(f)?;
                }
                ControlFlow::Continue(())
            }
            Expr::InList { expr, list, .. } => {
                expr.walk(f)?;
                for item in list {
                    item.walk(f)?;
                }
                ControlFlow::Continue(())
            }
            Expr::InSubquery { expr, .. } => {
                // Only walk the outer expression, not the inner subquery.
                // Callers that need to descend into InSubquery's SelectAst
                // should handle it in their callback.
                expr.walk(f)?;
                ControlFlow::Continue(())
            }
            Expr::InExpr { expr, target, .. } => {
                expr.walk(f)?;
                target.walk(f)
            }
            // Subquery — do NOT walk into the inner SelectAst by default.
            // Callers that need deep subquery traversal should handle Subquery in their callback.
            Expr::Subquery(_) => ControlFlow::Continue(()),
            Expr::Array(items) => {
                for item in items {
                    item.walk(f)?;
                }
                ControlFlow::Continue(())
            }
            Expr::Object(pairs) => {
                for (_, v) in pairs {
                    v.walk(f)?;
                }
                ControlFlow::Continue(())
            }
            Expr::Index { base, index } => {
                base.walk(f)?;
                index.walk(f)
            }
            Expr::Slice { base, start, end } => {
                base.walk(f)?;
                if let Some(s) = start {
                    s.walk(f)?;
                }
                if let Some(e) = end {
                    e.walk(f)?;
                }
                ControlFlow::Continue(())
            }
            Expr::FieldAccess { base, .. } => base.walk(f),
            Expr::Range { start, end } => {
                start.walk(f)?;
                end.walk(f)
            }
            Expr::KnnSearch { vector, .. } => vector.walk(f),
            Expr::Traversal(t) => {
                for step in &t.steps {
                    if let Some(ref filter) = step.edge_filter {
                        filter.walk(f)?;
                    }
                }
                ControlFlow::Continue(())
            }
            Expr::If {
                branches,
                else_block,
            } => {
                for (cond, block) in branches {
                    cond.walk(f)?;
                    for stmt in &block.statements {
                        if let super::stmt::Statement::Expr(e) = stmt {
                            e.walk(f)?;
                        }
                    }
                }
                if let Some(block) = else_block {
                    for stmt in &block.statements {
                        if let super::stmt::Statement::Expr(e) = stmt {
                            e.walk(f)?;
                        }
                    }
                }
                ControlFlow::Continue(())
            }
            Expr::For { iterable, body, .. } => {
                iterable.walk(f)?;
                for stmt in &body.statements {
                    if let super::stmt::Statement::Expr(e) = stmt {
                        e.walk(f)?;
                    }
                }
                ControlFlow::Continue(())
            }
        }
    }

    /// Returns true if any node in the expression tree satisfies the predicate.
    pub fn any(&self, mut f: impl FnMut(&Expr) -> bool) -> bool {
        self.walk(&mut |expr| {
            if f(expr) {
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            }
        })
        .is_break()
    }

    /// Transform the expression tree (pre-order).
    /// The closure receives each node and returns:
    /// - `Ok(TransformResult::Replace(expr))` — replace this node (no further recursion)
    /// - `Ok(TransformResult::Recurse(expr))` — recurse into children of the returned expression
    /// - `Err(e)` — propagate an error
    ///
    /// Does NOT recurse into `Expr::Subquery` inner SelectAst by default.
    /// Callers that need to transform inside subqueries should handle `Expr::Subquery`
    /// in the closure and return `Replace(...)`.
    pub fn transform<E>(
        self,
        f: &mut impl FnMut(Expr) -> Result<TransformResult, E>,
    ) -> Result<Expr, E> {
        // First check if the closure wants to handle this node directly
        let node = match f(self)? {
            TransformResult::Replace(replaced) => return Ok(replaced),
            TransformResult::Recurse(node) => node,
        };

        // Closure returned Recurse — recurse into children
        match node {
            // Leaf nodes — no children to transform
            Expr::Literal(_)
            | Expr::Field(_)
            | Expr::ParentRef(_)
            | Expr::ValueRef(_)
            | Expr::Variable(_)
            | Expr::Constant { .. }
            | Expr::Aggregate(_)
            | Expr::Fts { .. }
            | Expr::Break
            | Expr::Continue => Ok(node),

            Expr::BinaryOp { left, op, right } => Ok(Expr::BinaryOp {
                left: Box::new(left.transform(f)?),
                op,
                right: Box::new(right.transform(f)?),
            }),
            Expr::UnaryOp { op, expr } => Ok(Expr::UnaryOp {
                op,
                expr: Box::new(expr.transform(f)?),
            }),
            Expr::FunctionCall {
                name,
                namespace,
                args,
            } => Ok(Expr::FunctionCall {
                name,
                namespace,
                args: args
                    .into_iter()
                    .map(|a| a.transform(f))
                    .collect::<Result<Vec<_>, _>>()?,
            }),
            Expr::InList {
                expr,
                list,
                negated,
            } => Ok(Expr::InList {
                expr: Box::new(expr.transform(f)?),
                list: list
                    .into_iter()
                    .map(|e| e.transform(f))
                    .collect::<Result<Vec<_>, _>>()?,
                negated,
            }),
            Expr::InSubquery {
                expr,
                subquery,
                negated,
            } => Ok(Expr::InSubquery {
                expr: Box::new(expr.transform(f)?),
                subquery,
                negated,
            }),
            Expr::InExpr {
                expr,
                target,
                negated,
            } => Ok(Expr::InExpr {
                expr: Box::new(expr.transform(f)?),
                target: Box::new(target.transform(f)?),
                negated,
            }),
            // Subquery — do NOT recurse into the inner SelectAst by default
            Expr::Subquery(_) => Ok(node),
            Expr::Array(items) => Ok(Expr::Array(
                items
                    .into_iter()
                    .map(|e| e.transform(f))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            Expr::Object(pairs) => Ok(Expr::Object(
                pairs
                    .into_iter()
                    .map(|(k, v)| Ok((k, v.transform(f)?)))
                    .collect::<Result<Vec<_>, E>>()?,
            )),
            Expr::Index { base, index } => Ok(Expr::Index {
                base: Box::new(base.transform(f)?),
                index: Box::new(index.transform(f)?),
            }),
            Expr::Slice { base, start, end } => Ok(Expr::Slice {
                base: Box::new(base.transform(f)?),
                start: start.map(|s| s.transform(f)).transpose()?.map(Box::new),
                end: end.map(|e| e.transform(f)).transpose()?.map(Box::new),
            }),
            Expr::FieldAccess { base, field } => Ok(Expr::FieldAccess {
                base: Box::new(base.transform(f)?),
                field,
            }),
            Expr::Range { start, end } => Ok(Expr::Range {
                start: Box::new(start.transform(f)?),
                end: Box::new(end.transform(f)?),
            }),
            Expr::KnnSearch {
                field,
                vector,
                k,
                effort,
            } => Ok(Expr::KnnSearch {
                field,
                vector: Box::new(vector.transform(f)?),
                k,
                effort,
            }),
            Expr::Traversal(t) => {
                let steps = t
                    .steps
                    .into_iter()
                    .map(|mut step| {
                        if let Some(filter) = step.edge_filter {
                            step.edge_filter = Some(Box::new(filter.transform(f)?));
                        }
                        Ok(step)
                    })
                    .collect::<Result<Vec<_>, E>>()?;
                Ok(Expr::Traversal(TraversalExpr {
                    steps,
                    target: t.target,
                    target_direction: t.target_direction,
                }))
            }
            Expr::If {
                branches,
                else_block,
            } => {
                let branches = branches
                    .into_iter()
                    .map(|(cond, block)| {
                        let cond = Box::new(cond.transform(f)?);
                        let block = transform_block(block, f)?;
                        Ok((cond, block))
                    })
                    .collect::<Result<Vec<_>, E>>()?;
                let else_block = else_block.map(|b| transform_block(b, f)).transpose()?;
                Ok(Expr::If {
                    branches,
                    else_block,
                })
            }
            Expr::For {
                binding,
                iterable,
                body,
            } => Ok(Expr::For {
                binding,
                iterable: Box::new(iterable.transform(f)?),
                body: transform_block(body, f)?,
            }),
        }
    }

    /// Try to convert a constant expression (Literal, Object, Array) to a Value.
    /// Returns None if the expression contains non-constant parts (e.g., field references, functions).
    pub fn try_to_value(&self) -> Option<Value> {
        match self {
            Expr::Literal(v) => Some(v.clone()),
            Expr::Object(pairs) => {
                let mut fields = std::collections::HashMap::new();
                for (key, val_expr) in pairs {
                    let val = val_expr.try_to_value()?;
                    fields.insert(key.clone(), val);
                }
                Some(Value::Object(fields))
            }
            Expr::Array(items) => {
                let mut values = Vec::new();
                for item in items {
                    let val = item.try_to_value()?;
                    values.push(val);
                }
                Some(Value::Array(values))
            }
            // Unary negation of a constant (e.g., -0.5 in arrays)
            Expr::UnaryOp {
                op: UnaryOp::Neg,
                expr: inner,
            } => {
                let val = inner.try_to_value()?;
                match val {
                    Value::Int(i) => Some(Value::Int(-i)),
                    Value::Float(f) => Some(Value::Float(-f)),
                    _ => None,
                }
            }
            // Non-constant expressions (fields, functions, etc.)
            _ => None,
        }
    }

    /// Returns true if this expression tree contains any `Aggregate` node.
    pub fn contains_aggregate(&self) -> bool {
        self.any(|e| matches!(e, Expr::Aggregate(_)))
    }
}

/// Transform all `Expr` nodes inside a `Block`'s statements.
pub fn transform_block<E>(
    block: Block,
    f: &mut impl FnMut(Expr) -> Result<TransformResult, E>,
) -> Result<Block, E> {
    let statements = block
        .statements
        .into_iter()
        .map(|stmt| match stmt {
            super::stmt::Statement::Expr(expr) => {
                Ok(super::stmt::Statement::Expr(expr.transform(f)?))
            }
            other => Ok(other),
        })
        .collect::<Result<Vec<_>, E>>()?;
    Ok(Block { statements })
}

/// A single item in a projection list
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionItem {
    pub expr: Expr,
    pub alias: Option<String>,
}

/// Sort direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OrderDirection {
    #[default]
    Asc,
    Desc,
}

/// Single sort key: expression + direction
#[derive(Debug, Clone, PartialEq)]
pub struct OrderItem {
    pub expr: Expr,
    pub direction: OrderDirection,
}

/// Direction of graph traversal
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraversalDirection {
    /// -> outgoing edges
    Outgoing,
    /// <- incoming edges
    Incoming,
    /// <-> both directions
    Bidirectional,
}

/// Depth specification for traversal
#[derive(Debug, Clone, PartialEq, Default)]
pub enum TraversalDepth {
    /// Single hop (default)
    #[default]
    Single,
    /// Exact number of hops: {3}
    Exact(usize),
    /// Range of hops: {1..5}, {2..}, {..5}, {..}
    Range {
        min: Option<usize>,
        max: Option<usize>,
    },
}

/// Mode for handling cycles during traversal
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TraversalMode {
    /// Deduplicate nodes (default) - each node appears once
    #[default]
    Deduplicate,
    /// Return all paths including cycles
    All,
}

/// A single step in a traversal path
#[derive(Debug, Clone, PartialEq)]
pub struct TraversalStep {
    /// Direction: ->, <-, <->
    pub direction: TraversalDirection,
    /// Edge label (None = wildcard *)
    pub label: Option<String>,
    /// Filter on edge: (follows WHERE since > 2024)
    pub edge_filter: Option<Box<Expr>>,
    /// Depth: {1..5}
    pub depth: TraversalDepth,
    /// Cycle handling mode
    pub mode: TraversalMode,
    /// Node collection filter: only traverse from nodes in this collection
    /// Set during normalization when an intermediate step is a collection name, not edge label
    pub node_filter: Option<String>,
}

/// What to return at the end of traversal
#[derive(Debug, Clone, PartialEq)]
pub enum TraversalTarget {
    /// Return edge IDs: ->follows
    Edge,
    /// Return all edge fields: ->follows.*
    EdgeAll,
    /// Return specific edge field: ->follows.since
    EdgeField(String),
    /// Return node IDs: ->follows->user
    Node(String),
    /// Return all node fields: ->follows->user.*
    NodeAll(String),
    /// Return specific node field: ->follows->user.name
    NodeField(String, String),
    /// Multiple edge fields: ->contains.{quantity, to.name}
    EdgeFields(Vec<FieldSelection>),
    /// Multiple node fields: ->follows->user.{name, email}
    NodeFields(String, Vec<FieldSelection>),
}

/// Complete traversal expression
#[derive(Debug, Clone, PartialEq)]
pub struct TraversalExpr {
    /// Steps in the traversal path
    pub steps: Vec<TraversalStep>,
    /// What to return
    pub target: TraversalTarget,
    /// Direction for the final target (used when target is a non-collection edge label)
    pub target_direction: Option<TraversalDirection>,
}

/// A single field in multi-field selection
#[derive(Debug, Clone, PartialEq)]
pub struct FieldSelection {
    /// Output alias (None = use last segment of path as name)
    pub alias: Option<String>,
    /// Field path: "quantity" or "to.name"
    pub path: String,
}
