//! Subquery materialization and correlated subquery support.
//!
//! Handles two kinds of subqueries:
//! - **IN-subqueries**: `WHERE x IN (SELECT ...)` — materialized before execution
//!   by running the inner SELECT and converting to an IN-list.
//! - **Correlated subqueries**: `SELECT (SELECT ... WHERE $parent.id = ...) FROM ...`
//!   — post-processed after the main query by re-executing the inner SELECT
//!   for each parent row with `$parent` references substituted.

use crate::document::{Document, Value};
use crate::query::ast::{Expr, ParentRef, Projection, ProjectionItem, SelectAst, Statement};
use crate::query::error::ExecuteError;
use crate::query::execute::context::ExecutionContext;
use crate::query::execute::parent_context::ParentContext;
use crate::query::plan::LogicalPlan;

use super::engine::{execute, plan_from_statement, value_to_json};
use super::operators::operator::Row;
use super::operators::project::expr_display_name;

// ---------------------------------------------------------------------------
// IN-subquery materialization
// ---------------------------------------------------------------------------

/// Walk a LogicalPlan and materialize any InSubquery expressions in filters.
pub fn materialize_subqueries_in_plan<Ctx: ExecutionContext>(
    plan: LogicalPlan,
    ctx: &Ctx,
) -> Result<LogicalPlan, ExecuteError> {
    match plan {
        LogicalPlan::Scan {
            collection,
            key,
            filter,
            projection,
            order,
            limit,
            offset,
            distinct,
            value_mode,
            value_expr,
            fetch,
        } => {
            let filter = match filter {
                Some(expr) => Some(materialize_subqueries_in_expr(expr, ctx)?),
                None => None,
            };
            Ok(LogicalPlan::Scan {
                collection,
                key,
                filter,
                projection,
                order,
                limit,
                offset,
                distinct,
                value_mode,
                value_expr,
                fetch,
            })
        }
        LogicalPlan::Aggregate {
            collection,
            filter,
            aggregates,
            post_projection,
        } => {
            let filter = match filter {
                Some(expr) => Some(materialize_subqueries_in_expr(expr, ctx)?),
                None => None,
            };
            Ok(LogicalPlan::Aggregate {
                collection,
                filter,
                aggregates,
                post_projection,
            })
        }
        LogicalPlan::GroupAggregate {
            collection,
            filter,
            group_fields,
            aggregates,
            post_projection,
            order,
        } => {
            let filter = match filter {
                Some(expr) => Some(materialize_subqueries_in_expr(expr, ctx)?),
                None => None,
            };
            Ok(LogicalPlan::GroupAggregate {
                collection,
                filter,
                group_fields,
                aggregates,
                post_projection,
                order,
            })
        }
        LogicalPlan::Update {
            collection,
            key,
            filter,
            assignments,
        } => {
            let filter = match filter {
                Some(expr) => Some(materialize_subqueries_in_expr(expr, ctx)?),
                None => None,
            };
            Ok(LogicalPlan::Update {
                collection,
                key,
                filter,
                assignments,
            })
        }
        LogicalPlan::Delete {
            collection,
            key,
            filter,
        } => {
            let filter = match filter {
                Some(expr) => Some(materialize_subqueries_in_expr(expr, ctx)?),
                None => None,
            };
            Ok(LogicalPlan::Delete {
                collection,
                key,
                filter,
            })
        }
        LogicalPlan::InMemoryScan {
            rows,
            filter,
            projection,
            order,
            limit,
            offset,
            scalar_output,
            distinct,
            value_mode,
            value_expr,
            fetch,
        } => {
            let filter = match filter {
                Some(expr) => Some(materialize_subqueries_in_expr(expr, ctx)?),
                None => None,
            };
            Ok(LogicalPlan::InMemoryScan {
                rows,
                filter,
                projection,
                order,
                limit,
                offset,
                scalar_output,
                distinct,
                value_mode,
                value_expr,
                fetch,
            })
        }
        // All other plan types have no filter expressions with possible subqueries
        other => Ok(other),
    }
}

/// Recursively walk an Expr and convert InSubquery/Subquery nodes to InList/Literal.
fn materialize_subqueries_in_expr<Ctx: ExecutionContext>(
    expr: Expr,
    ctx: &Ctx,
) -> Result<Expr, ExecuteError> {
    use crate::query::ast::TransformResult;
    expr.transform(&mut |e| match e {
        Expr::InSubquery {
            expr: inner_expr,
            subquery,
            negated,
        } => {
            // Execute the subquery
            let sub_plan = LogicalPlan::from_ast(Statement::Select(*subquery));
            let rows = execute(sub_plan, ctx, None)?;

            // Extract values from result rows (first non-id column or scalar)
            let mut values = Vec::new();
            for row in rows {
                if let Some(scalar) = row.scalar_value {
                    values.push(Expr::Literal(scalar));
                } else {
                    let val = row
                        .doc
                        .fields
                        .iter()
                        .find(|(k, _)| k.as_str() != "id")
                        .map(|(_, v)| v.clone());
                    if let Some(v) = val {
                        values.push(Expr::Literal(v));
                    }
                }
            }

            // Recurse into the inner expr as well
            let inner_expr = Box::new(materialize_subqueries_in_expr(*inner_expr, ctx)?);

            Ok(TransformResult::Replace(Expr::InList {
                expr: inner_expr,
                list: values,
                negated,
            }))
        }
        Expr::Subquery(select) => {
            let sub_plan = LogicalPlan::from_ast(Statement::Select(*select));
            let rows = execute(sub_plan, ctx, None)?;

            let values: Vec<Value> = rows
                .into_iter()
                .map(|row| {
                    if let Some(scalar) = row.scalar_value {
                        scalar
                    } else {
                        Value::Object(row.doc.fields)
                    }
                })
                .collect();

            Ok(TransformResult::Replace(Expr::Literal(Value::Array(
                values,
            ))))
        }
        other => Ok(TransformResult::Recurse(other)),
    })
}

// ---------------------------------------------------------------------------
// Correlated subquery support
// ---------------------------------------------------------------------------

/// Check if a projection contains any Subquery expressions (including wrapped in Index/FieldAccess).
pub fn projection_has_subquery(projection: &Projection) -> bool {
    match projection {
        Projection::All => false,
        Projection::Items(items) => items.iter().any(|i| expr_has_subquery(&i.expr)),
    }
}

/// Post-process correlated subqueries on Row data (not JSON).
/// This version has direct access to all Document fields via ParentContext.
///
/// The key advantage over `post_process_correlated_subqueries` is that the parent context
/// receives the FULL Row with all Document fields, not a Row reconstructed from JSON
/// that only has projected fields.
pub fn post_process_correlated_subqueries_rows<Ctx: ExecutionContext>(
    rows: Vec<Row>,
    projection: &Projection,
    ctx: &Ctx,
    parent_ctx: Option<&ParentContext>,
) -> Result<Vec<Row>, ExecuteError> {
    let items = match projection {
        Projection::Items(items) => items,
        Projection::All => return Ok(rows),
    };

    // Find all projection items that contain subqueries
    let subquery_item_indices: Vec<(usize, &ProjectionItem)> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| expr_has_subquery(&item.expr))
        .collect();

    if subquery_item_indices.is_empty() {
        return Ok(rows);
    }

    let mut new_rows = Vec::with_capacity(rows.len());
    for row in rows {
        // Create new parent context from FULL Row (not JSON-reconstructed)
        let new_parent_ctx = match parent_ctx {
            Some(p) => p.push(row.clone()),
            None => ParentContext::new(row.clone()),
        };

        let mut new_fields = row.doc.fields.clone();

        // Execute and resolve each expression containing subqueries
        for (_idx, item) in &subquery_item_indices {
            let resolved_expr =
                resolve_subqueries_in_assignment_expr_with_ctx(&item.expr, &new_parent_ctx, ctx)?;

            let result_value = eval_resolved_expr(&resolved_expr, &row)?;

            let alias_name = item
                .alias
                .clone()
                .unwrap_or_else(|| expr_display_name(&item.expr));
            new_fields.insert(alias_name, result_value);
        }

        new_rows.push(Row {
            doc: Document {
                id: row.doc.id.clone(),
                fields: new_fields,
            },
            scalar_value: row.scalar_value.clone(),
            scalar_alias: row.scalar_alias.clone(),
        });
    }

    Ok(new_rows)
}

/// Evaluate a resolved expression (one where subqueries have been replaced with literals).
/// This handles Index, FieldAccess, etc. on top of subquery results.
fn eval_resolved_expr(expr: &Expr, row: &Row) -> Result<Value, ExecuteError> {
    use crate::query::execute::context::NullContext;
    use crate::query::execute::eval::eval_expr;
    eval_expr(expr, row, None, None, &NullContext)
}

/// Execute a correlated subquery with parent context.
/// Substitutes $parent references (at any depth) with literal values from the context,
/// then executes the inner SELECT and returns results as Value::Array.
///
/// For nested correlated subqueries, this function recursively processes subqueries
/// in the result projection, passing the parent context down the stack.
fn execute_correlated_subquery<Ctx: ExecutionContext>(
    subquery: &SelectAst,
    parent_ctx: &ParentContext,
    ctx: &Ctx,
) -> Result<Value, ExecuteError> {
    // Substitute $parent references with actual values from the context stack
    // Note: This only substitutes references at the current level. Nested subqueries
    // are NOT substituted here - they remain as Expr::Subquery and will be
    // processed by post_process_correlated_subqueries_rows with proper context.
    let substituted = substitute_parent_refs_in_select_with_ctx(subquery.clone(), parent_ctx)?;

    // Build and execute the inner query using row-based execution
    let stmt = Statement::Select(substituted.clone());
    let plan = plan_from_statement(stmt, ctx)?;
    let mut rows = execute(plan, ctx, None)?;

    // If the subquery's projection contains nested correlated subqueries,
    // recursively process them with the current parent context.
    // This enables multi-level $parent references like $parent.parent.parent.
    if projection_has_subquery(&substituted.projection) {
        rows = post_process_correlated_subqueries_rows(
            rows,
            &substituted.projection,
            ctx,
            Some(parent_ctx),
        )?;
    }

    // Convert rows to Value::Array
    let values: Vec<Value> = rows
        .iter()
        .map(|row| {
            if let Some(ref scalar) = row.scalar_value {
                scalar.clone()
            } else {
                // Include id in the object if present
                let mut obj = row.doc.fields.clone();
                if !row.doc.id.is_empty() {
                    obj.insert("id".to_string(), Value::String(row.doc.id.clone()));
                }
                Value::Object(obj)
            }
        })
        .collect();

    Ok(Value::Array(values))
}

/// Resolve subqueries in an expression for UPDATE SET.
/// Replaces Expr::Subquery nodes with Expr::Literal(Value::Array(...)).
/// This is similar to correlated subquery handling but for UPDATE context.
pub fn resolve_subqueries_in_assignment_expr<Ctx: ExecutionContext>(
    expr: &Expr,
    parent: &Document,
    ctx: &Ctx,
) -> Result<Expr, ExecuteError> {
    // Create a ParentContext from the document for subquery resolution
    let parent_row = Row::from_doc(parent.clone());
    let parent_ctx = ParentContext::new(parent_row);

    resolve_subqueries_in_assignment_expr_with_ctx(expr, &parent_ctx, ctx)
}

/// Internal implementation that uses ParentContext.
fn resolve_subqueries_in_assignment_expr_with_ctx<Ctx: ExecutionContext>(
    expr: &Expr,
    parent_ctx: &ParentContext,
    ctx: &Ctx,
) -> Result<Expr, ExecuteError> {
    use crate::query::ast::TransformResult;
    expr.clone().transform(&mut |e| match e {
        Expr::Subquery(select) => {
            // Execute the subquery with parent context
            let result = execute_correlated_subquery(&select, parent_ctx, ctx)?;
            Ok(TransformResult::Replace(Expr::Literal(result)))
        }
        other => Ok(TransformResult::Recurse(other)),
    })
}

/// Resolve non-correlated subqueries in projection expressions before evaluation.
/// Handles cases like search::rrf([(SELECT ...), (SELECT ...)], 10).
pub fn resolve_subqueries_in_projection<Ctx: ExecutionContext>(
    projection: &Projection,
    ctx: &Ctx,
) -> Result<Projection, ExecuteError> {
    match projection {
        Projection::All => Ok(Projection::All),
        Projection::Items(items) => {
            let resolved_items = items
                .iter()
                .map(|item| {
                    let resolved_expr = resolve_subqueries_in_expr(&item.expr, ctx)?;
                    Ok(ProjectionItem {
                        expr: resolved_expr,
                        alias: item.alias.clone(),
                    })
                })
                .collect::<Result<Vec<_>, ExecuteError>>()?;
            Ok(Projection::Items(resolved_items))
        }
    }
}

/// Recursively resolve subqueries in an expression.
/// Returns a new expression with all Subquery nodes replaced by their
/// evaluated results as Literal values.
pub fn resolve_subqueries_in_expr<Ctx: ExecutionContext>(
    expr: &Expr,
    ctx: &Ctx,
) -> Result<Expr, ExecuteError> {
    use crate::query::ast::TransformResult;
    expr.clone().transform(&mut |e| match e {
        Expr::Subquery(select) => {
            // Execute subquery (non-correlated, no parent context)
            let stmt = Statement::Select(*select);
            let plan = super::engine::plan_from_statement(stmt, ctx)?;
            let rows = super::engine::execute(plan, ctx, None)?;

            // Convert rows to Value::Array
            let values: Vec<Value> = rows
                .into_iter()
                .map(|row| {
                    if let Some(scalar) = row.scalar_value {
                        scalar
                    } else {
                        let mut obj = std::collections::HashMap::new();
                        if !row.doc.id.is_empty() {
                            obj.insert("id".to_string(), Value::String(row.doc.id));
                        }
                        for (k, v) in row.doc.fields {
                            obj.insert(k, v);
                        }
                        Value::Object(obj)
                    }
                })
                .collect();

            Ok(TransformResult::Replace(Expr::Literal(Value::Array(
                values,
            ))))
        }
        other => Ok(TransformResult::Recurse(other)),
    })
}

/// Check if projection has non-correlated subqueries (no $parent refs).
pub fn projection_has_non_correlated_subquery(projection: &Projection) -> bool {
    match projection {
        Projection::All => false,
        Projection::Items(items) => items
            .iter()
            .any(|i| expr_has_non_correlated_subquery(&i.expr)),
    }
}

fn expr_has_non_correlated_subquery(expr: &Expr) -> bool {
    expr.any(|e| matches!(e, Expr::Subquery(select) if !select_has_parent_ref(select)))
}

fn select_has_parent_ref(select: &SelectAst) -> bool {
    // Check data source for $parent reference (now represented as Expr)
    if let crate::query::ast::DataSource::Expr(expr) = &select.source
        && expr_has_parent_ref(expr)
    {
        return true;
    }
    if let Some(ref filter) = select.filter
        && expr_has_parent_ref(filter)
    {
        return true;
    }
    if let Projection::Items(items) = &select.projection
        && items.iter().any(|i| expr_has_parent_ref(&i.expr))
    {
        return true;
    }
    false
}

fn expr_has_parent_ref(expr: &Expr) -> bool {
    // Note: walk() does not descend into Subquery children, so we check
    // nested subqueries explicitly via select_has_parent_ref.
    expr.any(|e| match e {
        Expr::ParentRef(_) => true,
        Expr::Subquery(select) => select_has_parent_ref(select),
        _ => false,
    })
}

/// Collect all field names referenced via $parent in the projection's subqueries.
/// This is used to ensure those fields are fetched even if not in the user's projection.
pub fn collect_parent_ref_fields(projection: &Projection) -> std::collections::HashSet<String> {
    let mut fields = std::collections::HashSet::new();
    if let Projection::Items(items) = projection {
        for item in items {
            collect_parent_refs_from_expr(&item.expr, &mut fields);
        }
    }
    fields
}

fn collect_parent_refs_from_expr(expr: &Expr, fields: &mut std::collections::HashSet<String>) {
    let _ = expr.walk::<()>(&mut |e| {
        match e {
            Expr::ParentRef(p) => {
                // Only collect fields from immediate parent (depth == 0)
                // Higher-level parents (depth > 0) are handled by ParentContext
                if p.depth == 0 {
                    fields.insert(p.field.clone());
                }
            }
            // walk() doesn't descend into Subquery/InSubquery children, so do it explicitly
            Expr::Subquery(select) => {
                if let crate::query::ast::DataSource::Expr(ref source_expr) = select.source {
                    collect_parent_refs_from_expr(source_expr, fields);
                }
                if let Some(ref filter) = select.filter {
                    collect_parent_refs_from_expr(filter, fields);
                }
                if let Projection::Items(items) = &select.projection {
                    for item in items {
                        collect_parent_refs_from_expr(&item.expr, fields);
                    }
                }
            }
            Expr::InSubquery {
                expr: inner,
                subquery,
                ..
            } => {
                collect_parent_refs_from_expr(inner, fields);
                if let Some(ref filter) = subquery.filter {
                    collect_parent_refs_from_expr(filter, fields);
                }
            }
            _ => {}
        }
        std::ops::ControlFlow::Continue(())
    });
}

/// Check if an expression contains any Subquery nodes
pub fn expr_has_subquery(expr: &Expr) -> bool {
    expr.any(|e| matches!(e, Expr::Subquery(_)))
}

/// Resolve a $parent reference using the ParentContext stack.
/// Returns the field value from the appropriate parent level.
fn resolve_parent_ref(
    parent_ref: &ParentRef,
    parent_ctx: &ParentContext,
) -> Result<Value, ExecuteError> {
    let row =
        parent_ctx
            .get(parent_ref.depth)
            .ok_or_else(|| ExecuteError::ParentDepthExceeded {
                requested: parent_ref.depth,
                available: parent_ctx.len(),
            })?;

    if parent_ref.field == "id" {
        Ok(Value::Reference(row.doc.id.clone()))
    } else {
        Ok(row
            .doc
            .fields
            .get(&parent_ref.field)
            .cloned()
            .unwrap_or(Value::Null))
    }
}

/// Substitute $parent references throughout a SelectAst using ParentContext.
/// This supports multi-level parent references like $parent.parent.field.
fn substitute_parent_refs_in_select_with_ctx(
    mut select: SelectAst,
    parent_ctx: &ParentContext,
) -> Result<SelectAst, ExecuteError> {
    use crate::query::ast::{DataSource, Target};

    // Substitute in data source (FROM $parent.field or FROM (expr containing $parent))
    if let DataSource::Expr(expr) = &select.source {
        // Check if this is a ParentRef expression (FROM $parent.field)
        if let Expr::ParentRef(parent_ref) = expr.as_ref() {
            // For data source, we need to resolve from the appropriate parent level
            let row = parent_ctx.get(parent_ref.depth).ok_or_else(|| {
                ExecuteError::ParentDepthExceeded {
                    requested: parent_ref.depth,
                    available: parent_ctx.len(),
                }
            })?;
            let value = if parent_ref.field == "id" {
                Value::Reference(row.doc.id.clone())
            } else {
                row.doc
                    .fields
                    .get(&parent_ref.field)
                    .cloned()
                    .unwrap_or(Value::Null)
            };
            select.source = match value {
                Value::Reference(ref_id) => {
                    // Parse "collection:key" format
                    if let Some((collection, key)) = ref_id.split_once(':') {
                        DataSource::Collection(Target {
                            collection: collection.to_string(),
                            key: Some(key.to_string()),
                        })
                    } else {
                        // Just a collection name without key
                        DataSource::Collection(Target {
                            collection: ref_id,
                            key: None,
                        })
                    }
                }
                Value::String(s) => {
                    // String value treated as reference
                    if let Some((collection, key)) = s.split_once(':') {
                        DataSource::Collection(Target {
                            collection: collection.to_string(),
                            key: Some(key.to_string()),
                        })
                    } else {
                        DataSource::Collection(Target {
                            collection: s,
                            key: None,
                        })
                    }
                }
                // For null or non-reference values, use an empty source
                _ => DataSource::None,
            };
        } else {
            // Other expressions in data source - substitute parent refs within them
            let substituted =
                substitute_parent_refs_in_expr_with_ctx(expr.as_ref().clone(), parent_ctx)?;
            select.source = DataSource::Expr(Box::new(substituted));
        }
    }

    // Substitute in filter
    if let Some(filter) = select.filter.take() {
        select.filter = Some(substitute_parent_refs_in_expr_with_ctx(filter, parent_ctx)?);
    }

    // Substitute in projection items
    if let Projection::Items(items) = select.projection {
        let substituted_items = items
            .into_iter()
            .map(|mut item| {
                item.expr = substitute_parent_refs_in_expr_with_ctx(item.expr, parent_ctx)?;
                Ok(item)
            })
            .collect::<Result<Vec<_>, ExecuteError>>()?;
        select.projection = Projection::Items(substituted_items);
    }

    Ok(select)
}

/// Recursively substitute ParentRef nodes with Literal values using ParentContext.
/// Does NOT recurse into Expr::Subquery — nested subqueries are handled by
/// post_process_correlated_subqueries with the correct parent context.
fn substitute_parent_refs_in_expr_with_ctx(
    expr: Expr,
    parent_ctx: &ParentContext,
) -> Result<Expr, ExecuteError> {
    use crate::query::ast::TransformResult;
    expr.transform(&mut |e| match e {
        Expr::ParentRef(ref p) => {
            let value = resolve_parent_ref(p, parent_ctx)?;
            Ok(TransformResult::Replace(Expr::Literal(value)))
        }
        other => Ok(TransformResult::Recurse(other)),
    })
}

// ---------------------------------------------------------------------------
// Traversal expression support
// ---------------------------------------------------------------------------

use crate::query::ast::TraversalExpr;
use crate::query::execute::operators::operator::Operator;
use crate::query::execute::operators::traversal::TraversalOp;

/// Check if a projection contains any Traversal expressions.
pub fn projection_has_traversal(projection: &Projection) -> bool {
    match projection {
        Projection::All => false,
        Projection::Items(items) => items.iter().any(|i| expr_has_traversal(&i.expr)),
    }
}

/// Check if an expression contains any Traversal nodes.
pub fn expr_has_traversal(expr: &Expr) -> bool {
    expr.any(|e| matches!(e, Expr::Traversal(_)))
}

/// Post-process SELECT results to evaluate traversal expressions in projection.
/// For each row, for each Traversal expression in projection, execute the traversal
/// using the row's document ID as the start node.
pub fn post_process_traversals<Ctx: ExecutionContext>(
    results: serde_json::Value,
    projection: &Projection,
    ctx: &Ctx,
) -> Result<serde_json::Value, ExecuteError> {
    let items = match projection {
        Projection::Items(items) => items,
        Projection::All => return Ok(results),
    };

    // Find traversal items
    let traversal_items: Vec<(usize, &TraversalExpr, String)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| match &item.expr {
            Expr::Traversal(t) => {
                let alias = item
                    .alias
                    .clone()
                    .unwrap_or_else(|| expr_display_name(&item.expr));
                Some((i, t, alias))
            }
            _ => None,
        })
        .collect();

    if traversal_items.is_empty() {
        return Ok(results);
    }

    let rows = match results {
        serde_json::Value::Array(rows) => rows,
        other => return Ok(other),
    };

    let mut new_rows = Vec::with_capacity(rows.len());
    for row in rows {
        // Get the document ID to use as traversal start node
        let doc_id = row
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut row_obj = match row {
            serde_json::Value::Object(map) => map,
            other => {
                new_rows.push(other);
                continue;
            }
        };

        // Execute each traversal expression
        for (_idx, traversal_expr, alias) in &traversal_items {
            let result_value = execute_traversal((*traversal_expr).clone(), &doc_id, ctx)?;
            row_obj.insert(alias.clone(), value_to_json(&result_value));
        }

        new_rows.push(serde_json::Value::Object(row_obj));
    }

    Ok(serde_json::Value::Array(new_rows))
}

/// Execute a traversal expression starting from a given document ID.
/// Returns the results as a Value::Array.
fn execute_traversal<Ctx: ExecutionContext>(
    traversal_expr: TraversalExpr,
    start_id: &str,
    ctx: &Ctx,
) -> Result<Value, ExecuteError> {
    // Create and execute the TraversalOp
    let mut op = TraversalOp::new(ctx, traversal_expr, vec![start_id.to_string()]);
    op.open()?;

    let mut results = Vec::new();
    while let Some(row) = op.next()? {
        // Convert Row to Value - use the document's fields as an object
        let value = row_to_value(&row);
        results.push(value);
    }
    op.close()?;

    Ok(Value::Array(results))
}

/// Convert a Row to a Value for traversal results.
fn row_to_value(row: &crate::query::execute::operators::operator::Row) -> Value {
    let mut obj = std::collections::HashMap::new();
    obj.insert("id".to_string(), Value::String(row.doc.id.clone()));
    for (k, v) in &row.doc.fields {
        obj.insert(k.clone(), v.clone());
    }
    Value::Object(obj)
}

/// Post-process traversal expressions on Row data.
/// Handles both direct Expr::Traversal and nested traversals (e.g., COUNT(->follows)).
pub fn post_process_traversals_rows<Ctx: ExecutionContext>(
    rows: Vec<Row>,
    projection: &Projection,
    ctx: &Ctx,
) -> Result<Vec<Row>, ExecuteError> {
    let items = match projection {
        Projection::Items(items) => items,
        Projection::All => return Ok(rows),
    };

    // Find all items that contain traversals (including nested ones)
    let traversal_item_indices: Vec<(usize, &ProjectionItem)> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| expr_has_traversal(&item.expr))
        .collect();

    if traversal_item_indices.is_empty() {
        return Ok(rows);
    }

    let mut new_rows = Vec::with_capacity(rows.len());
    for row in rows {
        let mut new_fields = row.doc.fields.clone();

        for (_idx, item) in &traversal_item_indices {
            // Resolve all traversals in the expression to literal values
            let resolved_expr = resolve_traversals_in_expr(&item.expr, &row.doc.id, ctx)?;

            // Evaluate the resolved expression
            let result =
                crate::query::execute::eval::eval_expr(&resolved_expr, &row, None, None, ctx)?;

            let alias = item
                .alias
                .clone()
                .unwrap_or_else(|| expr_display_name(&item.expr));
            new_fields.insert(alias, result);
        }

        new_rows.push(Row {
            doc: Document {
                id: row.doc.id.clone(),
                fields: new_fields,
            },
            scalar_value: row.scalar_value.clone(),
            scalar_alias: row.scalar_alias.clone(),
        });
    }

    Ok(new_rows)
}

/// Recursively resolve traversal expressions in an expression tree.
/// Replaces Expr::Traversal nodes with Expr::Literal(Value::Array(...)) containing results.
fn resolve_traversals_in_expr<Ctx: ExecutionContext>(
    expr: &Expr,
    start_id: &str,
    ctx: &Ctx,
) -> Result<Expr, ExecuteError> {
    use crate::query::ast::TransformResult;
    expr.clone().transform(&mut |e| match e {
        Expr::Traversal(t) => {
            let result = execute_traversal(t, start_id, ctx)?;
            Ok(TransformResult::Replace(Expr::Literal(result)))
        }
        other => Ok(TransformResult::Recurse(other)),
    })
}
