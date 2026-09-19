//! Expression evaluation module.
//!
//! This module evaluates AST expressions against document rows, returning Values.
//! It supports all expression types: literals, fields, operators, functions, etc.

pub(crate) mod access;
mod binary;
mod compare;

use std::collections::HashMap;

#[cfg(test)]
use crate::document::Document;
use crate::document::{DecimalValue, Value};
use crate::query::ast::{BinaryOp, Expr, UnaryOp};
use crate::query::error::ExecuteError;
use crate::query::execute::VarScope;
use crate::query::execute::operators::operator::Row;
use crate::query::function::global_registry;

// Re-export public items
pub use access::set_nested_field;
pub use compare::{is_truthy, value_repr, value_type_name, values_equal};

// Internal use
use access::{eval_field_access, eval_index, eval_slice, navigate_value, resolve_nested_field};
use binary::eval_binary;
use compare::is_truthy as is_truthy_internal;

/// Evaluate an expression against a row, returning a Value.
///
/// `parent` is an optional parent row for correlated subqueries ($parent.field).
/// `vars` optionally overrides the execution context's scope for $variable references.
/// `ctx` supplies the default variable scope and reference dereferencing.
pub fn eval_expr<Ctx: crate::query::execute::context::ExecutionContext>(
    expr: &Expr,
    row: &Row,
    parent: Option<&Row>,
    vars: Option<&VarScope>,
    ctx: &Ctx,
) -> Result<Value, ExecuteError> {
    match expr {
        Expr::Literal(v) => Ok(v.clone()),

        Expr::Field(name) => {
            // Nested field paths need special handling with reference dereferencing
            if name.contains('.') {
                let segments: Vec<&str> = name.split('.').collect();
                // Get the first field from the row
                let mut current = row.get_field(segments[0]).unwrap_or(Value::Null);
                // Walk the rest of the path, dereferencing References as needed
                for segment in &segments[1..] {
                    current = eval_field_access(current, segment, ctx)?;
                }
                Ok(current)
            } else {
                Ok(row.get_field(name).unwrap_or(Value::Null))
            }
        }

        Expr::BinaryOp { left, op, right } => match op {
            // Coalesce needs short-circuit evaluation
            BinaryOp::Coalesce => eval_coalesce_with_vars(left, right, row, parent, vars, ctx),
            _ => {
                let l = eval_expr(left, row, parent, vars, ctx)?;
                let r = eval_expr(right, row, parent, vars, ctx)?;
                eval_binary(*op, &l, &r)
            }
        },

        Expr::UnaryOp { op, expr: inner } => match op {
            UnaryOp::Not => {
                let v = eval_expr(inner, row, parent, vars, ctx)?;
                match v {
                    Value::Bool(b) => Ok(Value::Bool(!b)),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError {
                        expected: "Bool",
                        got: value_repr(&v),
                        op: "NOT",
                    }),
                }
            }
            UnaryOp::Neg => {
                let v = eval_expr(inner, row, parent, vars, ctx)?;
                match v {
                    Value::Int(i) => Ok(Value::Int(-i)),
                    Value::Float(f) => Ok(Value::Float(-f)),
                    Value::Decimal(d) => Ok(Value::Decimal(DecimalValue::new(-d.as_decimal()))),
                    Value::Null => Ok(Value::Null),
                    _ => Err(ExecuteError::TypeError {
                        expected: "Int, Float, or Decimal",
                        got: value_repr(&v),
                        op: "unary minus",
                    }),
                }
            }
            UnaryOp::IsNull => {
                let v = eval_expr(inner, row, parent, vars, ctx)?;
                Ok(Value::Bool(matches!(v, Value::Null)))
            }
            UnaryOp::IsNotNull => {
                let v = eval_expr(inner, row, parent, vars, ctx)?;
                Ok(Value::Bool(!matches!(v, Value::Null)))
            }
            UnaryOp::IsNone => match inner.as_ref() {
                Expr::Field(name) => {
                    if name.contains('.') {
                        Ok(Value::Bool(
                            resolve_nested_field(&row.doc.fields, name).is_none(),
                        ))
                    } else {
                        Ok(Value::Bool(!row.has_field(name)))
                    }
                }
                _ => Ok(Value::Bool(false)), // non-field is never "none"
            },
            UnaryOp::IsNotNone => match inner.as_ref() {
                Expr::Field(name) => {
                    if name.contains('.') {
                        Ok(Value::Bool(
                            resolve_nested_field(&row.doc.fields, name).is_some(),
                        ))
                    } else {
                        Ok(Value::Bool(row.has_field(name)))
                    }
                }
                _ => Ok(Value::Bool(true)), // non-field always exists
            },
        },

        Expr::FunctionCall {
            name,
            namespace,
            args,
        } => {
            let evaluated: Vec<Value> = args
                .iter()
                .map(|a| eval_expr(a, row, parent, vars, ctx))
                .collect::<Result<Vec<_>, _>>()?;

            let registry = global_registry();

            // Namespaced functions must be in Registry
            if let Some(ns) = namespace {
                // Special handling for fts::score() - retrieve from row's $score field
                if ns == "fts" && name == "score" {
                    // For named operators: fts::score("name") -> look for $score_name
                    // For simple operators: fts::score() -> look for $score
                    let score_field = if let Some(Value::String(score_name)) = evaluated.first() {
                        format!("$score_{}", score_name)
                    } else {
                        "$score".to_string()
                    };
                    return Ok(row.get_field(&score_field).unwrap_or(Value::Null));
                }

                // Special handling for vector::distance() - retrieve from row's $distance field
                if ns == "vector" && name == "distance" {
                    return Ok(row.get_field("$distance").unwrap_or(Value::Null));
                }

                if let Some(func_def) = registry.resolve_function(ns, name) {
                    // call() validates arity before invoking eval
                    return func_def.call(&evaluated);
                }

                // Function not found - try to suggest similar names
                let similar = registry.find_similar_functions(ns, name);
                let msg = if similar.is_empty() {
                    format!("{}::{}", ns, name)
                } else {
                    format!(
                        "{}::{}. Did you mean: {}?",
                        ns,
                        name,
                        similar
                            .iter()
                            .map(|s| format!("{}::{}", ns, s))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                return Err(ExecuteError::UnknownFunction(msg));
            }

            // Non-namespaced function calls are not supported
            // Provide helpful error for aggregate function names used with expressions
            let name_lower = name.to_lowercase();
            if name_lower == "count" {
                return Err(ExecuteError::InvalidOperation(
                    "COUNT with expression argument is not an aggregate. \
                     Use array::length(expr) to count array elements."
                        .to_string(),
                ));
            }
            if matches!(name_lower.as_str(), "sum" | "avg" | "min" | "max") {
                return Err(ExecuteError::InvalidOperation(format!(
                    "{} with expression argument is not supported. \
                     Aggregates like {}(field) only work on field references.",
                    name.to_uppercase(),
                    name.to_uppercase()
                )));
            }

            Err(ExecuteError::UnknownFunction(format!(
                "{}. Use namespaced calls like string::upper(), math::abs(), etc.",
                name
            )))
        }

        Expr::Aggregate(_) => Err(ExecuteError::UnexpectedExpression("Aggregate")),

        Expr::InList {
            expr: inner,
            list,
            negated,
        } => {
            let val = eval_expr(inner, row, parent, vars, ctx)?;
            if matches!(val, Value::Null) {
                return Ok(Value::Null);
            }
            let mut found = false;
            let mut has_null = false;
            for item_expr in list {
                let item = eval_expr(item_expr, row, parent, vars, ctx)?;
                if matches!(item, Value::Null) {
                    has_null = true;
                    continue;
                }
                if values_equal(&val, &item) {
                    found = true;
                    break;
                }
            }
            let result = if found {
                true
            } else if has_null {
                // SQL semantics: if not found and list contains NULL, result is NULL
                return Ok(Value::Null);
            } else {
                false
            };
            Ok(Value::Bool(if *negated { !result } else { result }))
        }

        Expr::InSubquery { .. } => Err(ExecuteError::UnexpectedExpression("InSubquery")),

        Expr::InExpr {
            expr: inner,
            target,
            negated,
        } => {
            // Evaluate the value to check
            let val = eval_expr(inner, row, parent, vars, ctx)?;
            if matches!(val, Value::Null) {
                return Ok(Value::Null);
            }
            // Evaluate the target (should be an array)
            let target_val = eval_expr(target, row, parent, vars, ctx)?;
            let list = match target_val {
                Value::Array(arr) => arr,
                Value::Null => return Ok(Value::Null),
                _ => {
                    return Err(ExecuteError::TypeError {
                        expected: "Array",
                        got: value_repr(&target_val),
                        op: "IN",
                    });
                }
            };
            // Check membership
            let mut found = false;
            let mut has_null = false;
            for item in &list {
                if matches!(item, Value::Null) {
                    has_null = true;
                    continue;
                }
                if values_equal(&val, item) {
                    found = true;
                    break;
                }
            }
            let result = if found {
                true
            } else if has_null {
                return Ok(Value::Null);
            } else {
                false
            };
            Ok(Value::Bool(if *negated { !result } else { result }))
        }

        Expr::ParentRef(_) => Err(ExecuteError::UnexpectedExpression("ParentRef")),

        Expr::Subquery(_) => Err(ExecuteError::UnexpectedExpression("Subquery")),

        Expr::ValueRef(path) => {
            let base = row.get_value();
            match path {
                None => Ok(base),
                Some(field_path) => Ok(navigate_value(&base, field_path)),
            }
        }

        Expr::Array(items) => {
            let values = items
                .iter()
                .map(|e| eval_expr(e, row, parent, vars, ctx))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Value::Array(values))
        }

        Expr::Object(pairs) => {
            let map = pairs
                .iter()
                .map(|(k, e)| Ok((k.clone(), eval_expr(e, row, parent, vars, ctx)?)))
                .collect::<Result<HashMap<String, Value>, ExecuteError>>()?;
            Ok(Value::Object(map))
        }

        Expr::Index { base, index } => {
            let base_val = eval_expr(base, row, parent, vars, ctx)?;
            let index_val = eval_expr(index, row, parent, vars, ctx)?;
            eval_index(base_val, index_val)
        }

        Expr::Slice { base, start, end } => {
            let base_val = eval_expr(base, row, parent, vars, ctx)?;
            let start_val = start
                .as_ref()
                .map(|e| eval_expr(e, row, parent, vars, ctx))
                .transpose()?;
            let end_val = end
                .as_ref()
                .map(|e| eval_expr(e, row, parent, vars, ctx))
                .transpose()?;
            eval_slice(base_val, start_val, end_val)
        }

        Expr::FieldAccess { base, field } => {
            let base_val = eval_expr(base, row, parent, vars, ctx)?;
            eval_field_access(base_val, field, ctx)
        }

        Expr::Constant { namespace, name } => {
            let registry = global_registry();
            if let Some(const_def) = registry.resolve_constant(namespace, name) {
                Ok(const_def.value.clone())
            } else {
                Err(ExecuteError::Internal(format!(
                    "Unknown constant {}::{}",
                    namespace, name
                )))
            }
        }

        Expr::Variable(name) => {
            // Look up variable in VarScope
            match vars.or_else(|| ctx.vars()) {
                Some(scope) => scope
                    .get(name)
                    .cloned()
                    .ok_or_else(|| ExecuteError::UndefinedVariable(name.clone())),
                None => Err(ExecuteError::UndefinedVariable(name.clone())),
            }
        }

        Expr::Fts { .. } => Err(ExecuteError::UnexpectedExpression("Fts")),

        Expr::KnnSearch { .. } => Err(ExecuteError::UnexpectedExpression("KnnSearch")),

        Expr::Traversal(_) => Err(ExecuteError::UnexpectedExpression("Traversal")),

        Expr::If { .. } => Err(ExecuteError::UnexpectedExpression("If")),

        Expr::For { .. } => Err(ExecuteError::UnexpectedExpression("For")),

        Expr::Break => Err(ExecuteError::UnexpectedExpression("Break")),

        Expr::Continue => Err(ExecuteError::UnexpectedExpression("Continue")),

        Expr::Range { start, end } => {
            let start_val = eval_expr(start, row, parent, vars, ctx)?;
            let end_val = eval_expr(end, row, parent, vars, ctx)?;
            Ok(Value::Range {
                start: Box::new(start_val),
                end: Box::new(end_val),
            })
        }
    }
}

/// Evaluate coalesce operator with short-circuit evaluation and variable support.
fn eval_coalesce_with_vars<Ctx: crate::query::execute::context::ExecutionContext>(
    left: &Expr,
    right: &Expr,
    row: &Row,
    parent: Option<&Row>,
    vars: Option<&VarScope>,
    ctx: &Ctx,
) -> Result<Value, ExecuteError> {
    let l = eval_expr(left, row, parent, vars, ctx)?;
    // Return the left value when it is truthy.
    if is_truthy_internal(&l) {
        return Ok(l);
    }
    // Otherwise evaluate and return right
    eval_expr(right, row, parent, vars, ctx)
}

/// Evaluate an expression as a boolean filter.
/// Returns true if the expression evaluates to true, false for false/NULL/error.
pub fn eval_filter<Ctx: crate::query::execute::context::ExecutionContext>(
    expr: &Expr,
    row: &Row,
    ctx: &Ctx,
) -> Result<bool, ExecuteError> {
    eval_filter_with_vars(expr, row, ctx.vars(), ctx)
}

/// Evaluate an expression as a boolean filter with explicit variable scope.
/// Uses StellarDB truthiness rules for condition evaluation.
pub fn eval_filter_with_vars<Ctx: crate::query::execute::context::ExecutionContext>(
    expr: &Expr,
    row: &Row,
    vars: Option<&VarScope>,
    ctx: &Ctx,
) -> Result<bool, ExecuteError> {
    let value = eval_expr(expr, row, None, vars, ctx)?;
    Ok(value.is_truthy())
}

/// Helper to create a Row from a Document for tests
#[cfg(test)]
fn row_from_doc(doc: &Document) -> Row {
    Row::from_doc(doc.clone())
}

#[cfg(test)]
mod tests;
