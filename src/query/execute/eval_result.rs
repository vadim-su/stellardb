//! EvalResult type for unified expression evaluation.
//!
//! Expressions can return either a Value (scalars, arrays, objects)
//! or Rows directly (subqueries). EvalResult unifies both cases.

use crate::document::{Document, Value};
use crate::query::error::ExecuteError;
use crate::query::execute::operators::operator::Row;

/// Result of evaluating an expression.
/// Subqueries return Rows directly; everything else returns Value.
#[derive(Debug, Clone)]
pub enum EvalResult {
    Value(Value),
    Rows(Vec<Row>),
}

impl EvalResult {
    /// Convert to rows for FROM clause iteration.
    pub fn into_rows(self) -> Result<Vec<Row>, ExecuteError> {
        match self {
            EvalResult::Rows(rows) => Ok(rows),
            EvalResult::Value(v) => expand_value_to_rows(v),
        }
    }

    /// Convert to Value (for projections, filters).
    /// Rows are converted to Array of Objects.
    pub fn into_value(self) -> Value {
        match self {
            EvalResult::Value(v) => v,
            EvalResult::Rows(rows) => rows_to_value(rows),
        }
    }
}

/// Convert a Value to rows for FROM clause.
pub fn expand_value_to_rows(value: Value) -> Result<Vec<Row>, ExecuteError> {
    match value {
        Value::Array(elements) => {
            let mut rows = Vec::with_capacity(elements.len());
            for elem in elements {
                match elem {
                    Value::Object(fields) => {
                        let doc = Document {
                            id: String::new(),
                            fields,
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
        Value::Object(fields) => {
            let doc = Document {
                id: String::new(),
                fields,
            };
            Ok(vec![Row::from_doc(doc)])
        }
        Value::Null => Ok(vec![]),
        Value::Range { start, end } => expand_range_to_rows(*start, *end),
        other => {
            // Scalars become a single row
            Ok(vec![Row::scalar(other)])
        }
    }
}

/// Expand a range to rows (always inclusive).
fn expand_range_to_rows(start: Value, end: Value) -> Result<Vec<Row>, ExecuteError> {
    match (&start, &end) {
        (Value::Int(s), Value::Int(e)) => {
            if e < s {
                return Ok(vec![]);
            }
            Ok((*s..=*e).map(|i| Row::scalar(Value::Int(i))).collect())
        }
        _ => Err(ExecuteError::UnexpectedExpression(
            "non-integer range bounds",
        )),
    }
}

/// Convert rows to Value::Array for scalar contexts.
fn rows_to_value(rows: Vec<Row>) -> Value {
    let values: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            if let Some(scalar) = row.scalar_value() {
                scalar.clone()
            } else {
                Value::Object(row.fields().clone())
            }
        })
        .collect();
    Value::Array(values)
}
