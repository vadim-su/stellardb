use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use crate::document::{Document, Value};
use crate::query::ast::{DataSource, Expr, Projection, Statement};
use crate::query::error::ExecuteError;
use crate::query::execute::builder::OperatorBuilder;
use crate::query::execute::context::ExecutionContext;
use crate::query::execute::operators::dml::InsertOp;
use crate::query::execute::operators::operator::{
    Operator, QueryDeadline, Row, check_deadline, collect_all_with_deadline,
};
use crate::query::execute::subquery::{
    materialize_subqueries_in_plan, post_process_correlated_subqueries_rows,
    post_process_traversals_rows, projection_has_non_correlated_subquery, projection_has_subquery,
    projection_has_traversal, resolve_subqueries_in_expr, resolve_subqueries_in_projection,
};
use crate::query::optimize::Optimizer;
use crate::query::plan::{ExplainNode, LogicalPlan, OperatorStats, PhysicalOp};

/// Execute a logical plan and return rows directly (no JSON conversion).
/// This is the main entry point for query execution.
pub fn execute<Ctx: ExecutionContext>(
    plan: LogicalPlan,
    ctx: &Ctx,
    deadline: QueryDeadline,
) -> Result<Vec<Row>, ExecuteError> {
    check_deadline(deadline)?;

    // Handle EXPLAIN specially - returns text as single row
    if let LogicalPlan::Explain {
        analyze,
        plan: inner,
        materialize_time_micros,
    } = plan
    {
        return execute_explain_to_rows(*inner, analyze, materialize_time_micros, ctx, deadline);
    }

    // Materialize any IN-subqueries into IN-lists
    let plan = materialize_subqueries_in_plan(plan, ctx)?;
    check_deadline(deadline)?;

    // Optimize: LogicalPlan -> PhysicalOp
    let optimizer = Optimizer::new(ctx);
    let physical = optimizer.optimize(plan);

    // Handle DML - they return modified documents
    match physical {
        PhysicalOp::Insert {
            collection,
            documents,
        } => {
            let mut op = InsertOp::new(ctx, collection, documents);
            collect_all_with_deadline(&mut op, deadline)
        }

        PhysicalOp::Create {
            collection,
            key,
            assignments,
        } => {
            let row = Row::from_doc(Document {
                id: String::new(),
                fields: Default::default(),
            });
            let mut fields = std::collections::HashMap::with_capacity(assignments.len() + 1);
            for assignment in assignments {
                let expr = resolve_subqueries_in_expr(&assignment.expr, ctx)?;
                let value = super::eval::eval_expr(&expr, &row, None, None, ctx)?;
                super::eval::set_nested_field(&mut fields, &assignment.path, value)?;
            }
            if let Some(key) = key {
                fields.insert("id".into(), Value::String(key));
            }
            let documents = vec![crate::query::ast::ObjectLiteral {
                fields: fields.into_iter().collect(),
            }];
            let mut op = InsertOp::new(ctx, collection, documents);
            collect_all_with_deadline(&mut op, deadline)
        }

        PhysicalOp::Update {
            assignments, input, ..
        } => execute_update_to_rows(assignments, *input, ctx, deadline),

        PhysicalOp::Delete { collection, input } => {
            execute_delete_to_rows(collection, *input, ctx, deadline)
        }

        PhysicalOp::Upsert {
            collection,
            key,
            assignments,
            replace,
        } => execute_upsert_to_rows(collection, key, assignments, replace, ctx, deadline),

        PhysicalOp::DeleteEdge { from, label, to } => {
            execute_delete_edge(&from, &label, to.as_deref(), ctx, deadline)
        }

        // SELECT queries use the builder
        _ => {
            let builder = OperatorBuilder::new(ctx);
            let mut root = builder.build(physical);
            collect_all_with_deadline(&mut *root, deadline)
        }
    }
}

fn execute_update_to_rows<Ctx: ExecutionContext>(
    assignments: Vec<crate::query::ast::Assignment>,
    input: PhysicalOp,
    ctx: &Ctx,
    deadline: QueryDeadline,
) -> Result<Vec<Row>, ExecuteError> {
    use super::subquery::{expr_has_subquery, resolve_subqueries_in_assignment_expr};

    let builder = OperatorBuilder::new(ctx);
    let mut input_op = builder.build(input);

    let has_subqueries = assignments.iter().any(|a| expr_has_subquery(&a.expr));
    let mut results = Vec::new();

    input_op.open()?;
    while let Some(row) = input_op.next()? {
        check_deadline(deadline)?;
        let mut fields = row.doc.fields.clone();

        for assignment in &assignments {
            let resolved_expr = if has_subqueries && expr_has_subquery(&assignment.expr) {
                resolve_subqueries_in_assignment_expr(&assignment.expr, &row.doc, ctx)?
            } else {
                assignment.expr.clone()
            };

            let value = super::eval::eval_expr(&resolved_expr, &row, None, None, ctx)?;
            super::eval::set_nested_field(&mut fields, &assignment.path, value)?;
        }

        let updated = Document {
            id: row.doc.id.clone(),
            fields,
        };
        ctx.set_document(&updated).map_err(ExecuteError::Storage)?;
        results.push(Row::from_doc(updated));
    }
    input_op.close()?;

    Ok(results)
}

fn execute_upsert_to_rows<Ctx: ExecutionContext>(
    collection: String,
    key: String,
    assignments: Vec<crate::query::ast::Assignment>,
    replace: bool,
    ctx: &Ctx,
    deadline: QueryDeadline,
) -> Result<Vec<Row>, ExecuteError> {
    check_deadline(deadline)?;
    let doc_id = format!("{}:{}", collection, key);

    // Check if document exists
    let existing = ctx
        .get_document(&collection, &key)
        .map_err(ExecuteError::Storage)?;

    // Build fields: start from existing (merge) or empty (replace/create)
    let mut fields = if replace {
        std::collections::HashMap::new()
    } else {
        existing
            .as_ref()
            .map(|d| d.fields.clone())
            .unwrap_or_default()
    };

    // Create a dummy row for expression evaluation context
    let eval_row = if let Some(ref doc) = existing {
        Row::from_doc(doc.clone())
    } else {
        Row::from_doc(Document {
            id: doc_id.clone(),
            fields: fields.clone(),
        })
    };

    // Evaluate and apply assignments
    for assignment in &assignments {
        let value = super::eval::eval_expr(&assignment.expr, &eval_row, None, None, ctx)?;
        super::eval::set_nested_field(&mut fields, &assignment.path, value)?;
    }

    let doc = Document { id: doc_id, fields };

    check_deadline(deadline)?;
    ctx.set_document(&doc).map_err(ExecuteError::Storage)?;
    Ok(vec![Row::from_doc(doc)])
}

fn execute_delete_to_rows<Ctx: ExecutionContext>(
    collection: String,
    input: PhysicalOp,
    ctx: &Ctx,
    deadline: QueryDeadline,
) -> Result<Vec<Row>, ExecuteError> {
    let builder = OperatorBuilder::new(ctx);
    let mut input_op = builder.build(input);

    let mut results = Vec::new();
    input_op.open()?;
    while let Some(row) = input_op.next()? {
        check_deadline(deadline)?;
        ctx.delete_document(&collection, row.doc.key())
            .map_err(ExecuteError::Storage)?;
        results.push(row);
    }
    input_op.close()?;

    Ok(results)
}

/// Execute EXPLAIN command and return rows
fn execute_explain_to_rows<Ctx: ExecutionContext>(
    plan: LogicalPlan,
    analyze: bool,
    materialize_time_micros: Option<u64>,
    ctx: &Ctx,
    _deadline: QueryDeadline,
) -> Result<Vec<Row>, ExecuteError> {
    // Optimize the inner plan
    let optimizer = Optimizer::new(ctx);
    let physical = optimizer.optimize(plan);

    // Build explain tree
    let mut explain_node = ExplainNode::from_physical(&physical);

    if analyze {
        // Build instrumented operators
        let builder = OperatorBuilder::new(ctx);
        let (mut op, stats_list) = builder.build_instrumented(physical);

        // Execute to completion
        op.open()?;
        while op.next()?.is_some() {}
        op.close()?;

        // Populate stats into explain node (pre-order to match tree structure)
        populate_stats(&mut explain_node, &stats_list, &mut 0);

        // Compute self times (total - children)
        explain_node.compute_self_times();
    }

    // Format and return as single scalar row
    let mut text = String::new();
    if let Some(micros) = materialize_time_micros {
        text.push_str(&format!(
            "Materialize subquery: {:.2}ms\n",
            micros as f64 / 1000.0
        ));
    }
    text.push_str(&explain_node.format(0));
    Ok(vec![Row::scalar(Value::String(text))])
}

/// Recursively populate stats into explain tree (post-order traversal
/// to match build_instrumented_inner which pushes children before parent)
fn populate_stats(
    node: &mut ExplainNode,
    stats_list: &[Rc<RefCell<OperatorStats>>],
    index: &mut usize,
) {
    for child in &mut node.children {
        populate_stats(child, stats_list, index);
    }

    if *index < stats_list.len() {
        node.stats = Some(stats_list[*index].borrow().clone());
        *index += 1;
    }
}

// ---------------------------------------------------------------------------
// JSON ↔ Value conversion helpers (used by engine and subquery modules)
// ---------------------------------------------------------------------------

/// Convert a document::Value to serde_json::Value.
/// This is a thin wrapper around the From<Value> implementation for cases
/// where we need a function reference.
pub(super) fn value_to_json(v: &Value) -> serde_json::Value {
    serde_json::Value::from(v.clone())
}

/// Execute a statement end-to-end, including correlated subquery post-processing.
///
/// This is the main entry point for executing a bound statement. It handles:
/// 1. Planning (with subquery data source materialization)
/// 2. Execution to rows (not JSON)
/// 3. Post-processing correlated subqueries on rows (ParentContext has full Document)
/// 4. Post-processing traversal expressions on rows
/// 5. Post-processing FETCH clauses (resolving document references)
/// 6. Returns ExecuteResult with rows (no JSON conversion)
pub fn execute_statement<Ctx: ExecutionContext>(
    stmt: Statement,
    ctx: &Ctx,
    deadline: QueryDeadline,
) -> Result<super::ExecuteResult, ExecuteError> {
    // Handle RELATE statements directly (they don't use the logical plan)
    if let Statement::Relate(ref relate) = stmt {
        let rows = super::relate::execute_relate(relate, ctx)?;
        return Ok(super::ExecuteResult::new(rows));
    }

    // Extract fetch items from SELECT before modifying the statement
    let fetch_items = if let Statement::Select(ref s) = stmt {
        s.fetch.clone()
    } else {
        None
    };

    // First, resolve non-correlated subqueries in projection
    let stmt = match stmt {
        Statement::Select(mut s) => {
            if projection_has_non_correlated_subquery(&s.projection) {
                s.projection = resolve_subqueries_in_projection(&s.projection, ctx)?;
            }
            Statement::Select(s)
        }
        other => other,
    };

    // Check if this is a SELECT with correlated subqueries in projection
    // If so, we need to defer projection to preserve all fields for parent context
    let (stmt, correlated_projection) = match stmt {
        Statement::Select(mut s) => {
            if projection_has_subquery(&s.projection) {
                let orig_projection = s.projection.clone();
                // Defer projection: use Projection::All during execution
                // so that post_process_correlated_subqueries_rows has access to all fields
                s.projection = Projection::All;
                (Statement::Select(s), Some(orig_projection))
            } else {
                (Statement::Select(s), None)
            }
        }
        other => (other, None),
    };

    // Check if this is a SELECT with traversal expressions in projection
    let traversal_projection = match &stmt {
        Statement::Select(s) => {
            // Note: if we deferred projection above, s.projection is now All,
            // so we check correlated_projection instead
            if correlated_projection
                .as_ref()
                .map(projection_has_traversal)
                .unwrap_or_else(|| projection_has_traversal(&s.projection))
            {
                correlated_projection
                    .as_ref()
                    .cloned()
                    .or_else(|| Some(s.projection.clone()))
            } else {
                None
            }
        }
        _ => None,
    };

    // Defer VALUE mode when traversals are present in the value expression.
    // ValueOp calls eval_expr directly, which can't handle Expr::Traversal.
    let deferred_value = if traversal_projection.is_some() {
        match &stmt {
            Statement::Select(s) if s.value_mode => Some(s.distinct),
            _ => None,
        }
    } else {
        None
    };

    let stmt = if deferred_value.is_some() {
        match stmt {
            Statement::Select(mut s) => {
                s.value_mode = false;
                Statement::Select(s)
            }
            other => other,
        }
    } else {
        stmt
    };

    // Execute to rows (not JSON)
    // If correlated_projection is Some, the statement has Projection::All,
    // so we get rows with ALL document fields intact for parent context access
    let plan = plan_from_statement(stmt, ctx)?;
    let mut rows = execute(plan, ctx, deadline)?;

    // Post-process correlated subqueries on Rows (ParentContext has full Document)
    if let Some(ref projection) = correlated_projection {
        rows = post_process_correlated_subqueries_rows(rows, projection, ctx, None)?;
    }

    // Post-process traversals on Rows
    if let Some(ref projection) = traversal_projection {
        rows = post_process_traversals_rows(rows, projection, ctx)?;
    }

    // Apply deferred VALUE extraction after traversals have been resolved
    if let Some(distinct) = deferred_value
        && let Some(ref projection) = traversal_projection
    {
        rows = apply_deferred_value(rows, projection, distinct);
    }

    // If we deferred projection, apply it now to get only the requested fields
    if let Some(ref projection) = correlated_projection {
        rows = apply_final_projection(rows, projection, ctx)?;
    }

    // Post-process FETCH clauses (resolve document references)
    if let Some(ref fetch) = fetch_items
        && !fetch.is_empty()
    {
        rows = super::fetch::execute_fetch(rows, fetch, ctx)?;
    }

    // Return ExecuteResult with rows (no JSON conversion here)
    Ok(super::ExecuteResult::new(rows))
}

/// Apply final projection to rows, keeping only the fields specified in the projection.
/// This is used after correlated subquery post-processing when we deferred projection.
fn apply_final_projection<Ctx: ExecutionContext>(
    rows: Vec<Row>,
    projection: &Projection,
    ctx: &Ctx,
) -> Result<Vec<Row>, ExecuteError> {
    use crate::query::execute::eval::eval_expr;
    use crate::query::execute::operators::project::expr_display_name;
    use crate::query::execute::subquery::{expr_has_subquery, expr_has_traversal};

    let items = match projection {
        Projection::All => return Ok(rows),
        Projection::Items(items) => items,
    };

    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let mut projected_fields = std::collections::HashMap::new();

        for item in items {
            let name = item
                .alias
                .clone()
                .unwrap_or_else(|| expr_display_name(&item.expr));

            // For subquery and traversal expressions, the value was already added
            // to row.doc.fields by post_process_correlated_subqueries_rows
            if expr_has_subquery(&item.expr) || expr_has_traversal(&item.expr) {
                if let Some(value) = row.doc.fields.get(&name) {
                    projected_fields.insert(name, value.clone());
                }
                continue;
            }

            // id is handled separately (in doc.id)
            if name == "id" && item.alias.is_none() {
                continue;
            }

            // Evaluate other expressions
            let value = eval_expr(&item.expr, &row, None, None, ctx)?;
            projected_fields.insert(name, value);
        }

        result.push(Row {
            doc: Document {
                id: row.doc.id.clone(),
                fields: projected_fields,
            },
            scalar_value: row.scalar_value.clone(),
            scalar_alias: row.scalar_alias.clone(),
        });
    }

    Ok(result)
}

/// Apply VALUE extraction after traversal post-processing.
/// Extracts the single projected field from each row and returns it as a scalar Row.
/// If the extracted value is an array, each element becomes its own scalar row
/// (matching how SELECT VALUE normally flattens single-expression results).
fn apply_deferred_value(rows: Vec<Row>, projection: &Projection, distinct: bool) -> Vec<Row> {
    let items = match projection {
        Projection::Items(items) if items.len() == 1 => items,
        _ => return rows,
    };

    let alias = items[0]
        .alias
        .clone()
        .unwrap_or_else(|| super::operators::project::expr_display_name(&items[0].expr));

    let mut result = Vec::with_capacity(rows.len());
    let mut seen = std::collections::HashSet::new();

    for row in rows {
        let value = row.doc.fields.get(&alias).cloned().unwrap_or(Value::Null);

        // Traversals produce arrays — flatten them so each element is a separate row
        let values: Vec<Value> = match value {
            Value::Array(elements) => elements,
            other => vec![other],
        };

        for v in values {
            if distinct {
                let key = format!("{:?}", v);
                if !seen.insert(key) {
                    continue;
                }
            }
            result.push(Row::scalar(v));
        }
    }

    result
}

/// Build a LogicalPlan from a Statement, materializing non-collection data sources.
///
/// Collection sources are converted directly to logical plans. Other sources are
/// materialized into an in-memory scan before planning.
pub fn plan_from_statement<Ctx: ExecutionContext>(
    stmt: Statement,
    ctx: &Ctx,
) -> Result<LogicalPlan, ExecuteError> {
    match stmt {
        Statement::Select(ref s) => match &s.source {
            DataSource::Collection(_) => Ok(LogicalPlan::from_ast(stmt)),
            source => {
                let rows = materialize_source(source, ctx)?;
                let scalar_output = matches!(source, DataSource::None)
                    || should_produce_scalar_output(source, &s.projection);
                Ok(build_in_memory_scan(s, rows, scalar_output))
            }
        },
        Statement::Explain(ref e) => {
            let inner_stmt = *e.statement.clone();
            match &inner_stmt {
                Statement::Select(s) => match &s.source {
                    DataSource::Collection(_) => Ok(LogicalPlan::from_ast(stmt)),
                    source => {
                        let start = Instant::now();
                        let rows = materialize_source(source, ctx)?;
                        let materialize_micros = if e.analyze {
                            Some(start.elapsed().as_micros() as u64)
                        } else {
                            None
                        };
                        let scalar_output = matches!(source, DataSource::None)
                            || should_produce_scalar_output(source, &s.projection);
                        let inner_plan = build_in_memory_scan(s, rows, scalar_output);
                        Ok(LogicalPlan::Explain {
                            analyze: e.analyze,
                            plan: Box::new(inner_plan),
                            materialize_time_micros: materialize_micros,
                        })
                    }
                },
                _ => Ok(LogicalPlan::from_ast(stmt)),
            }
        }
        _ => Ok(LogicalPlan::from_ast(stmt)),
    }
}

/// Materialize a non-Collection data source into rows.
fn materialize_source<Ctx: ExecutionContext>(
    source: &DataSource,
    ctx: &Ctx,
) -> Result<Vec<Row>, ExecuteError> {
    match source {
        DataSource::None => {
            let empty_row = Row::from_doc(Document {
                id: String::new(),
                fields: std::collections::HashMap::new(),
            });
            Ok(vec![empty_row])
        }
        DataSource::FieldPath {
            target,
            path,
            postfix,
        } => expand_field_path_source(target, path, postfix, ctx),
        DataSource::Expr(expr) => expand_expr_source(expr, ctx),
        DataSource::Collection(_) => unreachable!("Collection handled before materialize_source"),
    }
}

/// Build an InMemoryScan plan from a SelectAst and pre-materialized rows.
fn build_in_memory_scan(
    s: &crate::query::ast::SelectAst,
    rows: Vec<Row>,
    scalar_output: bool,
) -> LogicalPlan {
    let value_expr = if s.value_mode {
        extract_value_expr(&s.projection)
    } else {
        None
    };
    LogicalPlan::InMemoryScan {
        rows,
        filter: s.filter.clone(),
        projection: s.projection.clone(),
        order: s.order.clone(),
        limit: s.limit,
        offset: s.offset,
        scalar_output,
        distinct: s.distinct,
        value_mode: s.value_mode,
        value_expr,
        fetch: s.fetch.clone(),
    }
}

/// Expand a Value into rows.
/// Used by expand_expr_source and subquery module.
pub fn expand_value_to_rows(value: &Value) -> Result<Vec<Row>, ExecuteError> {
    match value {
        Value::Array(elements) => {
            let mut rows = Vec::with_capacity(elements.len());
            for elem in elements {
                match elem {
                    Value::Object(obj_fields) => {
                        let doc = Document {
                            id: String::new(),
                            fields: obj_fields.clone(),
                        };
                        rows.push(Row::from_doc(doc));
                    }
                    other => {
                        rows.push(Row::scalar(other.clone()));
                    }
                }
            }
            Ok(rows)
        }
        Value::Object(obj_fields) => {
            let doc = Document {
                id: String::new(),
                fields: obj_fields.clone(),
            };
            Ok(vec![Row::from_doc(doc)])
        }
        Value::Null => Ok(vec![]),
        Value::Range { start, end } => {
            // Expand range into array of integers
            match (start.as_ref(), end.as_ref()) {
                (Value::Int(s), Value::Int(e)) => {
                    if e < s {
                        return Ok(vec![]);
                    }
                    Ok((*s..=*e).map(|i| Row::scalar(Value::Int(i))).collect())
                }
                _ => Err(ExecuteError::Internal(format!(
                    "range requires integer bounds, got {}..{}",
                    start.type_name(),
                    end.type_name()
                ))),
            }
        }
        other => {
            // Scalars become a single row
            Ok(vec![Row::scalar(other.clone())])
        }
    }
}

/// Expand field path source (FROM collection:key.field.path[postfix]) into rows.
/// Fetches the document, navigates the field path, applies postfix ops, and expands the result.
fn expand_field_path_source<Ctx: ExecutionContext>(
    target: &crate::query::ast::Target,
    path: &[String],
    postfix: &[crate::query::ast::PostfixOp],
    ctx: &Ctx,
) -> Result<Vec<Row>, ExecuteError> {
    use crate::query::execute::eval::access::{eval_field_access, eval_index, eval_slice};

    let key = target.key.as_ref().ok_or_else(|| {
        ExecuteError::Internal(
            "field path data source requires a document key (e.g. collection:key.field)".into(),
        )
    })?;

    let doc = ctx
        .get_document(&target.collection, key)
        .map_err(ExecuteError::Storage)?
        .ok_or_else(|| {
            ExecuteError::Internal(format!("document {}:{} not found", target.collection, key))
        })?;

    // Navigate the field path
    let mut value = Value::Object(doc.fields);
    for segment in path {
        value = eval_field_access(value, segment, ctx)?;
    }

    // Apply postfix operations (slice, index, field access)
    if !postfix.is_empty() {
        let empty_row = Row::from_doc(Document {
            id: String::new(),
            fields: std::collections::HashMap::new(),
        });
        for op in postfix {
            match op {
                crate::query::ast::PostfixOp::FieldAccess(field) => {
                    value = eval_field_access(value, field, ctx)?;
                }
                crate::query::ast::PostfixOp::Index(expr) => {
                    let idx = crate::query::execute::eval::eval_expr(
                        expr,
                        &empty_row,
                        None,
                        ctx.vars(),
                        ctx,
                    )?;
                    value = eval_index(value, idx)?;
                }
                crate::query::ast::PostfixOp::Slice { start, end } => {
                    let start_val = start
                        .as_ref()
                        .map(|e| {
                            crate::query::execute::eval::eval_expr(
                                e,
                                &empty_row,
                                None,
                                ctx.vars(),
                                ctx,
                            )
                        })
                        .transpose()?;
                    let end_val = end
                        .as_ref()
                        .map(|e| {
                            crate::query::execute::eval::eval_expr(
                                e,
                                &empty_row,
                                None,
                                ctx.vars(),
                                ctx,
                            )
                        })
                        .transpose()?;
                    value = eval_slice(value, start_val, end_val)?;
                }
            }
        }
    }

    expand_value_to_rows(&value)
}

/// Expand expression source (FROM (expr)) into rows
/// If the expression evaluates to an array, each element becomes a row.
/// If it evaluates to an object, that becomes a single row.
/// Otherwise, it's a scalar single row.
fn expand_expr_source<Ctx: ExecutionContext>(
    expr: &crate::query::ast::Expr,
    ctx: &Ctx,
) -> Result<Vec<Row>, ExecuteError> {
    use crate::query::execute::eval::eval_expr;

    // Resolve any subqueries in the expression first
    let resolved_expr = resolve_subqueries_in_expr(expr, ctx)?;

    // Empty row for evaluating the expression
    let empty_row = Row::from_doc(Document {
        id: String::new(),
        fields: std::collections::HashMap::new(),
    });

    let value = eval_expr(&resolved_expr, &empty_row, None, ctx.vars(), ctx)?;

    match value {
        Value::Array(elements) => {
            let mut rows = Vec::with_capacity(elements.len());
            for elem in elements {
                match elem {
                    Value::Object(obj_fields) => {
                        let doc = Document {
                            id: String::new(),
                            fields: obj_fields,
                        };
                        rows.push(Row::from_doc(doc));
                    }
                    other => {
                        rows.push(Row::scalar(other));
                    }
                }
            }
            Ok(rows)
        }
        Value::Object(obj_fields) => {
            let doc = Document {
                id: String::new(),
                fields: obj_fields,
            };
            Ok(vec![Row::from_doc(doc)])
        }
        Value::Null => Ok(vec![]),
        Value::Range { start, end } => {
            // Expand range into array of integers
            match (start.as_ref(), end.as_ref()) {
                (Value::Int(s), Value::Int(e)) => {
                    if e < s {
                        return Ok(vec![]);
                    }
                    Ok((*s..=*e).map(|i| Row::scalar(Value::Int(i))).collect())
                }
                _ => Err(ExecuteError::Internal(format!(
                    "range requires integer bounds, got {}..{}",
                    start.type_name(),
                    end.type_name()
                ))),
            }
        }
        other => {
            // Scalars become a single row
            Ok(vec![Row::scalar(other)])
        }
    }
}

/// Determine whether an expression source query should produce flat scalar output.
/// Only for `SELECT * FROM (expr)` where expr is an array - returns scalars as-is.
/// For `SELECT field FROM (expr)` - returns objects with the field (normal behavior).
/// For `SELECT VALUE ...` - ValueOp handles flat output separately.
fn should_produce_scalar_output(source: &DataSource, projection: &Projection) -> bool {
    // Only enable scalar_output for Expr/FieldPath sources with SELECT *
    if !matches!(source, DataSource::Expr(_) | DataSource::FieldPath { .. }) {
        return false;
    }
    matches!(projection, Projection::All)
}

/// Extract the expression for SELECT VALUE.
/// For SELECT VALUE, we expect exactly one expression in the projection.
fn extract_value_expr(projection: &Projection) -> Option<Expr> {
    match projection {
        Projection::Items(items) if items.len() == 1 => Some(items[0].expr.clone()),
        Projection::All => None, // SELECT VALUE * doesn't make sense
        _ => None,
    }
}

/// Execute edge deletion
///
/// If `to` is Some, deletes a specific edge.
/// If `to` is None, deletes all edges with the given label from the source.
fn execute_delete_edge<Ctx: ExecutionContext>(
    from: &str,
    label: &str,
    to: Option<&str>,
    ctx: &Ctx,
    deadline: QueryDeadline,
) -> Result<Vec<Row>, ExecuteError> {
    check_deadline(deadline)?;
    let count = if let Some(to) = to {
        // Delete specific edge
        let deleted = ctx
            .delete_edge(from, label, to)
            .map_err(ExecuteError::Storage)?;
        deleted as i64
    } else {
        // Delete all edges with this label from source
        let mut count = 0i64;
        // Iterate until no more edges with this label
        loop {
            let (edges, _) = ctx
                .list_edges_out(from, label, None, 1000)
                .map_err(ExecuteError::Storage)?;
            if edges.is_empty() {
                break;
            }
            for edge in edges {
                check_deadline(deadline)?;
                ctx.delete_edge(&edge.from, &edge.label, &edge.to)
                    .map_err(ExecuteError::Storage)?;
                count += 1;
            }
        }
        count
    };

    // Return a row with the deleted count
    let mut fields = std::collections::HashMap::new();
    fields.insert("deleted".to_string(), Value::Int(count));
    Ok(vec![Row::from_doc(Document {
        id: String::new(),
        fields,
    })])
}

#[cfg(test)]
mod tests;
