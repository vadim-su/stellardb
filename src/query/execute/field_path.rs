//! Field path expansion for `SELECT * FROM collection:key.path` syntax.
//!
//! Loads a document, navigates a dotted field path, and expands the
//! resulting value into rows suitable for the query pipeline.

use std::collections::HashMap;

use crate::document::{Document, Value};
use crate::query::error::ExecuteError;
use crate::query::execute::context::ExecutionContext;
use crate::query::execute::operators::operator::Row;

/// Load a document, navigate to a field path, and expand the value into rows.
pub fn expand_field_path<Ctx: ExecutionContext>(
    ctx: &Ctx,
    collection: &str,
    key: &str,
    path: &str,
) -> Result<Vec<Row>, ExecuteError> {
    // 1. Load the document
    let doc = ctx
        .get_document(collection, key)
        .map_err(ExecuteError::Storage)?
        .ok_or_else(|| ExecuteError::NotFound(format!("{}:{}", collection, key)))?;

    // 2. Navigate the field path (supports dotted paths like "address.items")
    let value = navigate_field_path(&doc, path).ok_or_else(|| {
        ExecuteError::NotFound(format!(
            "Field '{}' not found in document '{}:{}'",
            path, collection, key
        ))
    })?;

    // 3. Expand the value into rows
    expand_value_to_rows(value, collection, key, path)
}

/// Navigate a dotted field path within a document, returning a reference to the value.
fn navigate_field_path<'a>(doc: &'a Document, path: &str) -> Option<&'a Value> {
    let segments: Vec<&str> = path.split('.').collect();
    let mut current: &Value = doc.fields.get(segments[0])?;

    for segment in &segments[1..] {
        match current {
            Value::Object(map) => {
                current = map.get(*segment)?;
            }
            _ => return None,
        }
    }

    Some(current)
}

/// Expand a Value into a Vec<Row> according to the type-based expansion rules.
fn expand_value_to_rows(
    value: &Value,
    collection: &str,
    key: &str,
    path: &str,
) -> Result<Vec<Row>, ExecuteError> {
    match value {
        Value::Array(items) => {
            let mut rows = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                let id = format!("{}:{}.{}[{}]", collection, key, path, i);
                let doc = value_to_document(item, id);
                rows.push(Row::from_doc(doc));
            }
            Ok(rows)
        }
        Value::Object(fields) => {
            let id = format!("{}:{}.{}", collection, key, path);
            let doc = Document {
                id,
                fields: fields.clone(),
            };
            Ok(vec![Row::from_doc(doc)])
        }
        Value::String(s) => {
            let mut rows = Vec::with_capacity(s.len());
            for (i, ch) in s.chars().enumerate() {
                let id = format!("{}:{}.{}[{}]", collection, key, path, i);
                let mut fields = HashMap::new();
                fields.insert("value".to_string(), Value::String(ch.to_string()));
                rows.push(Row::from_doc(Document { id, fields }));
            }
            Ok(rows)
        }
        other => {
            let type_name = match other {
                Value::Int(_) => "Int",
                Value::Float(_) => "Float",
                Value::Bool(_) => "Bool",
                Value::Null => "Null",
                Value::Decimal(_) => "Decimal",
                Value::Datetime(_) => "Datetime",
                Value::Duration(_) => "Duration",
                Value::Bytes(_) => "Bytes",
                Value::Reference(_) => "Reference",
                Value::Range { .. } => "Range",
                // Array, Object, String are handled above
                _ => unreachable!(),
            };
            Err(ExecuteError::Internal(format!(
                "Cannot iterate over field '{}' of '{}:{}': expected Array, Object, or String, got {}",
                path, collection, key, type_name
            )))
        }
    }
}

/// Convert a single Value into a Document row.
fn value_to_document(value: &Value, id: String) -> Document {
    match value {
        Value::Object(fields) => Document {
            id,
            fields: fields.clone(),
        },
        other => {
            let mut fields = HashMap::new();
            fields.insert("value".to_string(), other.clone());
            Document { id, fields }
        }
    }
}
