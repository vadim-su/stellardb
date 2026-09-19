//! RELATE statement execution.

use std::collections::HashMap;

use crate::document::{Document, Value};
use crate::edge::Edge;
use crate::query::ast::{RelateAst, RelateData, RelateSource, ReturnClause, Statement};
use crate::query::error::ExecuteError;
use crate::query::execute::context::ExecutionContext;
use crate::query::execute::eval::eval_expr;
use crate::query::execute::operators::Row;

use super::engine::{execute, plan_from_statement};

fn edge_to_row(edge: &Edge) -> Row {
    let mut fields = HashMap::new();
    fields.insert("id".to_string(), Value::String(edge.id.clone()));
    fields.insert("from".to_string(), Value::String(edge.from.clone()));
    fields.insert("to".to_string(), Value::String(edge.to.clone()));

    for (k, v) in &edge.fields {
        fields.insert(k.clone(), v.clone());
    }

    Row::from_doc(Document {
        id: edge.id.clone(),
        fields,
    })
}

/// Execute RELATE statement
pub fn execute_relate<Ctx: ExecutionContext>(
    relate: &RelateAst,
    ctx: &Ctx,
) -> Result<Vec<Row>, ExecuteError> {
    // 1. Resolve sources to record IDs
    let from_ids = resolve_source(&relate.from, ctx)?;
    let to_ids = resolve_source(&relate.to, ctx)?;

    // 2. Create edges (cartesian product)
    // Note: create_edge validates document existence atomically via OCC transaction
    let mut created_edges = Vec::new();

    for from_id in &from_ids {
        for to_id in &to_ids {
            let mut edge = Edge::new(from_id.clone(), relate.label.clone(), to_id.clone());

            // Apply SET or CONTENT
            if let Some(data) = &relate.data {
                apply_edge_data(&mut edge, data, ctx)?;
            }

            // Create edge (uses upsert semantics - update if exists)
            ctx.create_edge(&edge).map_err(ExecuteError::Storage)?;
            created_edges.push(edge);
        }
    }

    // 3. Return based on clause
    match &relate.return_clause {
        ReturnClause::None => {
            // Return count as status row
            let mut m = HashMap::new();
            m.insert("count".to_string(), Value::Int(created_edges.len() as i64));
            Ok(vec![Row::from_doc(Document {
                id: String::new(),
                fields: m,
            })])
        }
        ReturnClause::After | ReturnClause::Before | ReturnClause::Fields(_) => {
            Ok(created_edges.iter().map(edge_to_row).collect())
        }
    }
}

fn resolve_source<Ctx: ExecutionContext>(
    source: &RelateSource,
    ctx: &Ctx,
) -> Result<Vec<String>, ExecuteError> {
    match source {
        RelateSource::Record(target) => {
            let id = if let Some(key) = &target.key {
                format!("{}:{}", target.collection, key)
            } else {
                // Collection-only target means all documents in collection
                return Err(ExecuteError::InvalidOperation(
                    "RELATE requires specific record IDs".into(),
                ));
            };
            Ok(vec![id])
        }
        RelateSource::Array(targets) => targets
            .iter()
            .map(|t| {
                if let Some(key) = &t.key {
                    Ok(format!("{}:{}", t.collection, key))
                } else {
                    Err(ExecuteError::InvalidOperation(
                        "RELATE requires specific record IDs".into(),
                    ))
                }
            })
            .collect(),
        RelateSource::Subquery(select) => {
            // Execute subquery and extract IDs
            let stmt = Statement::Select(*select.clone());
            let plan = plan_from_statement(stmt, ctx)?;
            let rows = execute(plan, ctx, None)?;

            rows.iter()
                .map(|row| {
                    if !row.doc.id.is_empty() {
                        Ok(row.doc.id.clone())
                    } else {
                        Err(ExecuteError::InvalidOperation(
                            "Subquery results must have id".into(),
                        ))
                    }
                })
                .collect()
        }
    }
}

/// Protected edge fields that cannot be modified via SET or CONTENT.
/// These are system-managed fields.
const PROTECTED_EDGE_FIELDS: &[&str] = &["id", "from", "to"];

fn validate_edge_field(key: &str) -> Result<(), ExecuteError> {
    if PROTECTED_EDGE_FIELDS.contains(&key) {
        return Err(ExecuteError::InvalidOperation(format!(
            "Cannot modify protected edge field '{}'. Fields 'id', 'from', and 'to' are immutable.",
            key
        )));
    }
    Ok(())
}

fn apply_edge_data<Ctx: ExecutionContext>(
    edge: &mut Edge,
    data: &RelateData,
    _ctx: &Ctx,
) -> Result<(), ExecuteError> {
    match data {
        RelateData::Set(assignments) => {
            // Create an empty row for expression evaluation
            let dummy_row = Row::from_doc(Document {
                id: String::new(),
                fields: std::collections::HashMap::new(),
            });
            for assignment in assignments {
                let key = assignment.path.join(".");
                validate_edge_field(&key)?;
                let value = eval_expr(&assignment.expr, &dummy_row, None, None, _ctx)?;
                edge.fields.insert(key, value);
            }
        }
        RelateData::Content(obj) => {
            for (key, value) in &obj.fields {
                validate_edge_field(key)?;
                edge.fields.insert(key.clone(), value.clone());
            }
        }
    }
    Ok(())
}
