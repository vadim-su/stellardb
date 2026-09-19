//! Field access, indexing, and slicing operations.

use std::collections::HashMap;

use crate::document::Value;
use crate::query::error::ExecuteError;

use super::compare::value_type_name;

/// Resolve a dot-separated field path against a document's fields map.
/// Returns `Some(value)` if the full path exists, `None` if any segment is missing.
pub fn resolve_nested_field(fields: &HashMap<String, Value>, path: &str) -> Option<Value> {
    let mut segments = path.split('.');
    let root = segments.next()?;
    let mut current = fields.get(root)?;
    for segment in segments {
        match current {
            Value::Object(map) => current = map.get(segment)?,
            _ => return None,
        }
    }
    Some(current.clone())
}

/// Navigate into a Value using a dot-separated field path.
pub fn navigate_value(value: &Value, path: &str) -> Value {
    let segments: Vec<&str> = path.split('.').collect();
    let mut current = value.clone();
    for segment in segments {
        match current {
            Value::Object(ref map) => {
                current = map.get(segment).cloned().unwrap_or(Value::Null);
            }
            _ => return Value::Null,
        }
    }
    current
}

/// Evaluate array indexing: base[index]
///
/// Supports integer indices and float indices with no fractional part.
/// Negative indices count from the end (e.g., -1 is the last element).
/// Out-of-bounds indices return Null.
pub fn eval_index(base: Value, index: Value) -> Result<Value, ExecuteError> {
    match base {
        Value::Null => Ok(Value::Null), // null propagation
        Value::Array(arr) => {
            let idx = match &index {
                Value::Int(i) => *i,
                Value::Float(f) if f.fract() == 0.0 => *f as i64,
                Value::Null => return Ok(Value::Null),
                _ => {
                    return Err(ExecuteError::TypeError {
                        expected: "Int",
                        got: value_type_name(&index).to_string(),
                        op: "array index",
                    });
                }
            };
            let len = arr.len() as i64;
            let normalized = if idx < 0 { len + idx } else { idx };
            if normalized < 0 || normalized >= len {
                Ok(Value::Null)
            } else {
                Ok(arr[normalized as usize].clone())
            }
        }
        _ => Err(ExecuteError::TypeError {
            expected: "Array",
            got: value_type_name(&base).to_string(),
            op: "indexing",
        }),
    }
}

/// Evaluate array slicing: base[start..end]
pub fn eval_slice(
    base: Value,
    start: Option<Value>,
    end: Option<Value>,
) -> Result<Value, ExecuteError> {
    match base {
        Value::Null => Ok(Value::Null), // null propagation
        Value::Array(arr) => {
            let len = arr.len() as i64;

            // Extract integer bounds with null propagation
            let start_idx = match start {
                None => 0,
                Some(Value::Int(i)) => i,
                Some(Value::Null) => return Ok(Value::Null),
                Some(v) => {
                    return Err(ExecuteError::TypeError {
                        expected: "Int",
                        got: value_type_name(&v).to_string(),
                        op: "slice start",
                    });
                }
            };
            let end_idx = match end {
                None => len - 1,
                Some(Value::Int(i)) => i,
                Some(Value::Null) => return Ok(Value::Null),
                Some(v) => {
                    return Err(ExecuteError::TypeError {
                        expected: "Int",
                        got: value_type_name(&v).to_string(),
                        op: "slice end",
                    });
                }
            };

            // Normalize negative indices
            let s = if start_idx < 0 {
                len + start_idx
            } else {
                start_idx
            };
            let e = if end_idx < 0 { len + end_idx } else { end_idx };

            // Clamp to array bounds
            let s = s.clamp(0, len);
            let e = e.clamp(-1, len - 1);

            if s > e || len == 0 {
                Ok(Value::Array(vec![]))
            } else {
                Ok(Value::Array(arr[s as usize..=e as usize].to_vec()))
            }
        }
        _ => Err(ExecuteError::TypeError {
            expected: "Array",
            got: value_type_name(&base).to_string(),
            op: "slicing",
        }),
    }
}

/// Evaluate field access: base.field
/// If base is a Reference, automatically dereferences it.
pub fn eval_field_access<Ctx: crate::query::execute::context::ExecutionContext>(
    base: Value,
    field: &str,
    ctx: &Ctx,
) -> Result<Value, ExecuteError> {
    match base {
        Value::Null => Ok(Value::Null),
        Value::Object(map) => Ok(map.get(field).cloned().unwrap_or(Value::Null)),
        Value::Reference(ref_id) => {
            // Dereference: load document and get field
            if let Some((collection, key)) = ref_id.split_once(':') {
                match ctx.get_document(collection, key) {
                    Ok(Some(doc)) => Ok(doc.fields.get(field).cloned().unwrap_or(Value::Null)),
                    Ok(None) => Ok(Value::Null),
                    Err(e) => Err(ExecuteError::Storage(e)),
                }
            } else {
                Ok(Value::Null)
            }
        }
        Value::Array(_) => Err(ExecuteError::TypeError {
            expected: "Object or Reference",
            got: "array".to_string(),
            op: "field access",
        }),
        _ => Err(ExecuteError::TypeError {
            expected: "Object or Reference",
            got: value_type_name(&base).to_string(),
            op: "field access",
        }),
    }
}

/// Set a value at a nested path in a fields map.
/// Creates intermediate objects as needed.
///
/// Example: set_nested_field(fields, &["address", "city"], Value::String("NYC"))
/// - If address doesn't exist, creates it as empty object
/// - If address exists but is not an object, returns TypeError
pub fn set_nested_field(
    fields: &mut HashMap<String, Value>,
    path: &[String],
    value: Value,
) -> Result<(), ExecuteError> {
    match path {
        [] => Err(ExecuteError::UnexpectedExpression("empty assignment path")),

        // Simple case: top-level field
        [field] => {
            fields.insert(field.clone(), value);
            Ok(())
        }

        // Nested path: address.city
        [first, rest @ ..] => {
            let nested = fields
                .entry(first.clone())
                .or_insert_with(|| Value::Object(HashMap::new()));

            match nested {
                Value::Object(obj) => set_nested_field(obj, rest, value),
                _ => Err(ExecuteError::TypeError {
                    expected: "Object",
                    got: value_type_name(nested).to_string(),
                    op: "nested field update",
                }),
            }
        }
    }
}
