//! Bidirectional conversion between internal types and proto types.
//!
//! This is the protobuf equivalent of `server/serialize.rs` (which handles JSON).

use std::collections::HashMap;

use crate::document::{Document, Value};
use crate::edge::Edge;
use crate::query::execute::Row;
use crate::query::result::{
    BatchError as InternalBatchError, BatchResult, StatementResult as InternalStatementResult,
};

use super::proto;

// =============================================================================
// Value conversions
// =============================================================================

/// Convert internal Value to proto Value.
pub fn value_to_proto(v: &Value) -> proto::Value {
    use proto::value::Kind;

    let kind = match v {
        Value::Null => Some(Kind::NullValue(0)),
        Value::Bool(b) => Some(Kind::BoolValue(*b)),
        Value::Int(i) => Some(Kind::IntValue(*i)),
        Value::Float(f) => Some(Kind::FloatValue(*f)),
        Value::Decimal(d) => Some(Kind::DecimalValue(d.as_decimal().to_string())),
        Value::String(s) => Some(Kind::StringValue(s.clone())),
        Value::Bytes(b) => Some(Kind::BytesValue(b.clone())),
        Value::Datetime(ms) => Some(Kind::DatetimeValue(*ms)),
        Value::Duration(ns) => Some(Kind::DurationValue(*ns)),
        Value::Reference(id) => Some(Kind::ReferenceValue(id.clone())),
        Value::Array(arr) => {
            let values = arr.iter().map(value_to_proto).collect();
            Some(Kind::ArrayValue(proto::ArrayValue { values }))
        }
        Value::Object(obj) => {
            let fields = obj
                .iter()
                .map(|(k, v)| (k.clone(), value_to_proto(v)))
                .collect();
            Some(Kind::ObjectValue(proto::ObjectValue { fields }))
        }
        // Range is not directly representable in proto — encode as object
        Value::Range { start, end } => {
            let mut fields = HashMap::new();
            fields.insert("start".to_string(), value_to_proto(start));
            fields.insert("end".to_string(), value_to_proto(end));
            Some(Kind::ObjectValue(proto::ObjectValue { fields }))
        }
    };

    proto::Value { kind }
}

/// Convert proto Value to internal Value.
pub fn value_from_proto(v: &proto::Value) -> Value {
    use proto::value::Kind;

    match &v.kind {
        None | Some(Kind::NullValue(_)) => Value::Null,
        Some(Kind::BoolValue(b)) => Value::Bool(*b),
        Some(Kind::IntValue(i)) => Value::Int(*i),
        Some(Kind::FloatValue(f)) => Value::Float(*f),
        Some(Kind::DecimalValue(s)) => match s.parse::<rust_decimal::Decimal>() {
            Ok(d) => Value::Decimal(crate::document::DecimalValue::new(d)),
            Err(_) => Value::String(s.clone()),
        },
        Some(Kind::StringValue(s)) => Value::String(s.clone()),
        Some(Kind::BytesValue(b)) => Value::Bytes(b.clone()),
        Some(Kind::DatetimeValue(ms)) => Value::Datetime(*ms),
        Some(Kind::DurationValue(ns)) => Value::Duration(*ns),
        Some(Kind::ReferenceValue(id)) => Value::Reference(id.clone()),
        Some(Kind::ArrayValue(arr)) => {
            Value::Array(arr.values.iter().map(value_from_proto).collect())
        }
        Some(Kind::ObjectValue(obj)) => Value::Object(
            obj.fields
                .iter()
                .map(|(k, v)| (k.clone(), value_from_proto(v)))
                .collect(),
        ),
    }
}

/// Convert a HashMap<String, Value> to proto fields map.
pub fn fields_to_proto(fields: &HashMap<String, Value>) -> HashMap<String, proto::Value> {
    fields
        .iter()
        .map(|(k, v)| (k.clone(), value_to_proto(v)))
        .collect()
}

/// Convert proto fields map to internal HashMap<String, Value>.
pub fn fields_from_proto(fields: &HashMap<String, proto::Value>) -> HashMap<String, Value> {
    fields
        .iter()
        .map(|(k, v)| (k.clone(), value_from_proto(v)))
        .collect()
}

// =============================================================================
// Document conversions
// =============================================================================

/// Convert internal Document to proto Document.
pub fn document_to_proto(doc: &Document) -> proto::Document {
    proto::Document {
        id: doc.id.clone(),
        fields: fields_to_proto(&doc.fields),
    }
}

// =============================================================================
// Edge conversions
// =============================================================================

/// Convert internal Edge to proto Edge.
pub fn edge_to_proto(edge: &Edge) -> proto::Edge {
    proto::Edge {
        id: edge.id.clone(),
        from: edge.from.clone(),
        to: edge.to.clone(),
        label: edge.label.clone(),
        fields: fields_to_proto(&edge.fields),
    }
}

// =============================================================================
// Row / query result conversions
// =============================================================================

/// Convert a Row to proto RowData.
///
/// Mirrors the logic of `row_to_json` in operator.rs:
/// - Scalar rows produce a single field (alias or "$value")
/// - Document rows produce id + all fields
pub fn row_to_proto(row: &Row) -> proto::RowData {
    let mut fields = HashMap::new();

    if let Some(ref scalar) = row.scalar_value {
        let key = row.scalar_alias.as_deref().unwrap_or("$value").to_string();
        fields.insert(key, value_to_proto(scalar));
    } else {
        if !row.doc.id.is_empty() {
            fields.insert(
                "id".to_string(),
                value_to_proto(&Value::String(row.doc.id.clone())),
            );
        }
        for (k, v) in &row.doc.fields {
            fields.insert(k.clone(), value_to_proto(v));
        }
    }

    proto::RowData { fields }
}

/// Convert InternalStatementResult to proto StatementResult.
pub fn statement_result_to_proto(r: &InternalStatementResult) -> proto::StatementResult {
    let rows: Vec<proto::RowData> = r.rows.iter().map(row_to_proto).collect();
    let count = if !r.rows.is_empty() {
        Some(r.rows.len() as i32)
    } else {
        // Match HTTP behavior: data-returning statements get count=0
        let is_data_returning = matches!(
            r.statement_type.as_str(),
            "SELECT" | "UPDATE" | "INSERT" | "CREATE" | "EXPR"
        );
        if is_data_returning || r.rows_affected.is_some() {
            Some(0)
        } else {
            None
        }
    };

    proto::StatementResult {
        statement_index: r.statement_index as i32,
        statement_type: r.statement_type.clone(),
        rows,
        rows_affected: r.rows_affected.map(|n| n as i64),
        count,
        session_id: r.session_id.clone(),
    }
}

/// Convert InternalBatchError to proto BatchError.
pub fn batch_error_to_proto(e: &InternalBatchError) -> proto::BatchError {
    proto::BatchError {
        statement_index: e.statement_index as i32,
        message: e.message.clone(),
        code: e.code.to_string(),
    }
}

/// Convert a full BatchResult to proto ExecuteResponse.
pub fn batch_result_to_proto(batch: &BatchResult, elapsed: String) -> proto::ExecuteResponse {
    proto::ExecuteResponse {
        results: batch
            .results
            .iter()
            .map(statement_result_to_proto)
            .collect(),
        completed: batch.completed as i32,
        error: batch.error.as_ref().map(batch_error_to_proto),
        elapsed,
        operation: None,
    }
}

pub fn ddl_job_to_proto(job: &crate::server::ddl_jobs::DdlJob) -> proto::DdlOperation {
    proto::DdlOperation {
        operation_id: job.operation_id.clone(),
        operation_type: job.operation_type.clone(),
        database: job.database.clone(),
        collection: job.collection.clone(),
        index_name: job.index_name.clone(),
        index_type: job.index_type.clone(),
        state: job.state.as_str().to_string(),
        phase: job.phase.as_str().to_string(),
        processed_items: job.processed_items as u64,
        total_items: job.total_items as u64,
        progress_percent: job.progress_percent,
        created_at: job.created_at.to_rfc3339(),
        started_at: job.started_at.map(|value| value.to_rfc3339()),
        finished_at: job.finished_at.map(|value| value.to_rfc3339()),
        error: job.error.as_ref().map(|error| proto::DdlOperationError {
            code: error.code.clone(),
            message: error.message.clone(),
        }),
    }
}

pub fn admission_operation_to_proto(
    operation: &crate::server::admission::TrackedOperationSnapshot,
) -> proto::DdlOperation {
    let (state, phase, error) = match &operation.status {
        crate::server::admission::OperationStatus::Running => ("BUILDING", "BUILDING", None),
        crate::server::admission::OperationStatus::Completed { .. } => ("READY", "COMPLETE", None),
        crate::server::admission::OperationStatus::Failed { code, message } => (
            "FAILED",
            "COMPLETE",
            Some(proto::DdlOperationError {
                code: code.clone(),
                message: message.clone(),
            }),
        ),
    };
    proto::DdlOperation {
        operation_id: operation.operation_id.clone(),
        operation_type: operation.operation_type.clone(),
        database: operation.database.clone(),
        collection: String::new(),
        index_name: String::new(),
        index_type: String::new(),
        state: state.to_string(),
        phase: phase.to_string(),
        processed_items: 0,
        total_items: 0,
        progress_percent: 0.0,
        created_at: operation.created_at.to_rfc3339(),
        started_at: Some(operation.created_at.to_rfc3339()),
        finished_at: operation.finished_at.map(|value| value.to_rfc3339()),
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_error_proto_preserves_safe_code_and_message() {
        let error = InternalBatchError::public(
            3,
            "SDB-QE033",
            crate::error::ErrorClass::Aborted,
            "transaction conflict",
        );
        let proto = batch_error_to_proto(&error);
        assert_eq!(proto.statement_index, 3);
        assert_eq!(proto.code, "SDB-QE033");
        assert_eq!(proto.message, "transaction conflict");
    }
}
