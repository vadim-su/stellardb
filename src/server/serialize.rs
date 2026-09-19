//! JSON serialization types for HTTP responses.
//!
//! These types provide JSON-serializable wrappers around the query result types,
//! which no longer implement Serialize directly (they hold Vec<Row> instead of JSON).

use crate::document::Value;
use crate::query::execute::operators::operator::{Row, rows_to_json};
use crate::query::result::{BatchError, BatchResult, StatementResult};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;

/// JSON-serializable version of StatementResult for HTTP responses
#[derive(Serialize)]
pub struct StatementResultJson {
    pub statement_index: usize,
    pub statement_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows_affected: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

impl From<&StatementResult> for StatementResultJson {
    fn from(r: &StatementResult) -> Self {
        // For SELECT and UPDATE queries, always include data array and count (even if empty)
        // For DDL and other statements, only include if there are rows
        let is_data_returning = matches!(
            r.statement_type.as_str(),
            "SELECT" | "UPDATE" | "INSERT" | "CREATE" | "EXPR"
        );
        let (data, count) = if r.rows.is_empty() {
            if is_data_returning {
                // Data-returning statement with no results: data=[], count=0
                (Some(serde_json::Value::Array(vec![])), Some(0))
            } else if r.rows_affected.is_some() {
                // DML with rows_affected (e.g., DELETE): data=null, count=rows_affected
                (None, Some(0))
            } else {
                // DDL or other with no rows: data=null, count=null
                (None, None)
            }
        } else {
            (Some(rows_to_json(&r.rows)), Some(r.rows.len()))
        };

        Self {
            statement_index: r.statement_index,
            statement_type: r.statement_type.clone(),
            data,
            rows_affected: r.rows_affected,
            count,
            session_id: r.session_id.clone(),
        }
    }
}

/// JSON-serializable version of BatchError for HTTP responses
#[derive(Serialize)]
pub struct BatchErrorJson {
    pub statement_index: usize,
    pub code: &'static str,
    pub message: String,
    pub severity: &'static str,
}

impl From<&BatchError> for BatchErrorJson {
    fn from(e: &BatchError) -> Self {
        Self {
            statement_index: e.statement_index,
            code: e.code,
            message: e.message.clone(),
            severity: "ERROR",
        }
    }
}

/// JSON-serializable version of BatchResult for HTTP responses
#[derive(Serialize)]
pub struct BatchResultJson {
    pub results: Vec<StatementResultJson>,
    pub completed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<BatchErrorJson>,
}

impl From<&BatchResult> for BatchResultJson {
    fn from(r: &BatchResult) -> Self {
        Self {
            results: r.results.iter().map(StatementResultJson::from).collect(),
            completed: r.completed,
            error: r.error.as_ref().map(BatchErrorJson::from),
        }
    }
}

fn value_to_typed_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(v) => serde_json::json!(v),
        Value::Int(v) => serde_json::json!({
            "$type": "int64",
            "value": v.to_string(),
        }),
        Value::Float(v) => serde_json::json!(v),
        Value::Decimal(v) => serde_json::json!({
            "$type": "decimal",
            "value": v.as_decimal().to_string(),
        }),
        Value::String(v) => serde_json::json!(v),
        Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(value_to_typed_json).collect())
        }
        Value::Object(fields) => {
            let mut obj = serde_json::Map::new();
            for (key, value) in fields {
                obj.insert(key.clone(), value_to_typed_json(value));
            }
            serde_json::Value::Object(obj)
        }
        Value::Reference(v) => serde_json::json!({
            "$type": "reference",
            "value": v,
        }),
        Value::Datetime(ms) => {
            let value = chrono::DateTime::from_timestamp_millis(*ms)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| ms.to_string());
            serde_json::json!({
                "$type": "datetime",
                "value": value,
            })
        }
        Value::Duration(ns) => serde_json::json!({
            "$type": "duration",
            "value": ns.to_string(),
        }),
        Value::Bytes(bytes) => serde_json::json!({
            "$type": "bytes",
            "value": STANDARD.encode(bytes),
        }),
        Value::Range { start, end } => serde_json::json!({
            "$type": "range",
            "start": value_to_typed_json(start),
            "end": value_to_typed_json(end),
        }),
    }
}

fn row_to_typed_json(row: &Row) -> serde_json::Value {
    if let Some(value) = row.scalar_value() {
        return value_to_typed_json(value);
    }

    let mut obj = serde_json::Map::new();
    if !row.doc.id.is_empty() {
        obj.insert("id".to_string(), serde_json::json!(row.doc.id));
    }
    for (key, value) in row.fields() {
        obj.insert(key.clone(), value_to_typed_json(value));
    }
    serde_json::Value::Object(obj)
}

fn rows_to_typed_json(rows: &[Row]) -> serde_json::Value {
    serde_json::Value::Array(rows.iter().map(row_to_typed_json).collect())
}

/// EJSON-serializable version of StatementResult for `/v1/query`.
#[derive(Serialize)]
pub struct TypedStatementResultJson {
    pub statement_index: usize,
    pub statement_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows_affected: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

impl From<&StatementResult> for TypedStatementResultJson {
    fn from(r: &StatementResult) -> Self {
        let is_data_returning = matches!(
            r.statement_type.as_str(),
            "SELECT" | "UPDATE" | "INSERT" | "CREATE" | "EXPR"
        );
        let (data, count) = if r.rows.is_empty() {
            if is_data_returning {
                (Some(serde_json::Value::Array(vec![])), Some(0))
            } else if r.rows_affected.is_some() {
                (None, Some(0))
            } else {
                (None, None)
            }
        } else {
            (Some(rows_to_typed_json(&r.rows)), Some(r.rows.len()))
        };

        Self {
            statement_index: r.statement_index,
            statement_type: r.statement_type.clone(),
            data,
            rows_affected: r.rows_affected,
            count,
            session_id: r.session_id.clone(),
        }
    }
}

/// EJSON-serializable version of BatchResult for `/v1/query`.
#[derive(Serialize)]
pub struct TypedBatchResultJson {
    pub results: Vec<TypedStatementResultJson>,
    pub completed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<BatchErrorJson>,
}

impl From<&BatchResult> for TypedBatchResultJson {
    fn from(r: &BatchResult) -> Self {
        Self {
            results: r
                .results
                .iter()
                .map(TypedStatementResultJson::from)
                .collect(),
            completed: r.completed,
            error: r.error.as_ref().map(BatchErrorJson::from),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_error_json_preserves_safe_code_and_message() {
        let error = BatchError::public(
            3,
            "SDB-QE033",
            crate::error::ErrorClass::Aborted,
            "transaction conflict",
        );
        let json = BatchErrorJson::from(&error);
        assert_eq!(json.statement_index, 3);
        assert_eq!(json.code, "SDB-QE033");
        assert_eq!(json.message, "transaction conflict");
    }
}
