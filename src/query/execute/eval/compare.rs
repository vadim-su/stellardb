//! Value comparison and type utilities.

use std::cmp::Ordering;

use crate::document::Value;

/// Compare two values for equality (handles NULL and type coercion).
pub fn values_equal(a: &Value, b: &Value) -> bool {
    // For numeric types, use unified decimal comparison
    if a.is_numeric()
        && b.is_numeric()
        && let (Some(da), Some(db)) = (a.as_decimal(), b.as_decimal())
    {
        return da == db;
    }

    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::Float(a), Value::Float(b)) => (a - b).abs() < f64::EPSILON,
        (Value::Int(a), Value::Float(b)) | (Value::Float(b), Value::Int(a)) => {
            ((*a as f64) - b).abs() < f64::EPSILON
        }
        (Value::Decimal(a), Value::Decimal(b)) => a.as_decimal() == b.as_decimal(),
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Reference(a), Value::Reference(b)) => a == b,
        // Allow Reference <-> String comparison (e.g., id = "orgs:acme")
        (Value::Reference(a), Value::String(b)) | (Value::String(b), Value::Reference(a)) => a == b,
        _ => false,
    }
}

/// Compare two values, returning an ordering if comparable.
pub fn values_compare(a: &Value, b: &Value) -> Option<Ordering> {
    // For numeric types, use unified decimal comparison
    if a.is_numeric()
        && b.is_numeric()
        && let (Some(da), Some(db)) = (a.as_decimal(), b.as_decimal())
    {
        return da.partial_cmp(&db);
    }

    match (a, b) {
        (Value::Int(a), Value::Int(b)) => a.partial_cmp(b),
        (Value::Float(a), Value::Float(b)) => a.partial_cmp(b),
        (Value::Int(a), Value::Float(b)) => (*a as f64).partial_cmp(b),
        (Value::Float(a), Value::Int(b)) => a.partial_cmp(&(*b as f64)),
        (Value::Decimal(a), Value::Decimal(b)) => a.as_decimal().partial_cmp(&b.as_decimal()),
        (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
        (Value::Bool(a), Value::Bool(b)) => a.partial_cmp(b),
        (Value::Reference(a), Value::Reference(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

/// Return a human-readable type name for a Value.
pub fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "Null",
        Value::Bool(_) => "Bool",
        Value::Int(_) => "Int",
        Value::Float(_) => "Float",
        Value::String(_) => "String",
        Value::Array(_) => "Array",
        Value::Object(_) => "Object",
        Value::Reference(_) => "Reference",
        Value::Decimal(_) => "Decimal",
        Value::Datetime(_) => "Datetime",
        Value::Duration(_) => "Duration",
        Value::Bytes(_) => "Bytes",
        Value::Range { .. } => "Range",
    }
}

/// Return a type name with a brief value representation for error messages.
/// For simple types, shows the actual value. For complex types, shows a summary.
pub fn value_repr(v: &Value) -> String {
    match v {
        Value::Null => "Null".to_string(),
        Value::Bool(b) => format!("Bool ({})", b),
        Value::Int(i) => format!("Int ({})", i),
        Value::Float(f) => format!("Float ({})", f),
        Value::String(s) => {
            if s.len() <= 20 {
                format!("String (\"{}\")", s)
            } else {
                format!("String (\"{}...\")", &s[..17])
            }
        }
        Value::Array(arr) => format!("Array ({} elements)", arr.len()),
        Value::Object(obj) => format!("Object ({} fields)", obj.len()),
        Value::Reference(r) => format!("Reference ({})", r),
        Value::Decimal(d) => format!("Decimal ({})", d.as_decimal()),
        Value::Datetime(dt) => format!("Datetime ({})", dt),
        Value::Duration(d) => format!("Duration ({})", d),
        Value::Bytes(b) => format!("Bytes ({} bytes)", b.len()),
        Value::Range { start, end } => format!("Range ({}..{})", start, end),
    }
}

/// Returns whether a value is truthy in StellarDB conditions.
pub fn is_truthy(value: &Value) -> bool {
    value.is_truthy()
}
