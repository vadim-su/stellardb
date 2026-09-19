use std::collections::HashMap;

use rkyv::{
    Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize,
    rancor::{Fallible, Source},
    ser::{Allocator, Writer},
    validation::ArchiveContext,
};
pub use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

mod datetime_serde {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(ms: &i64, s: S) -> Result<S::Ok, S::Error> {
        // Serialize as ISO8601 string
        if let Some(dt) = DateTime::from_timestamp_millis(*ms) {
            s.serialize_str(&dt.to_rfc3339())
        } else {
            s.serialize_i64(*ms)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
        use serde::de::Error;

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum DatetimeValue {
            String(String),
            Int(i64),
        }

        match DatetimeValue::deserialize(d)? {
            DatetimeValue::String(s) => DateTime::parse_from_rfc3339(&s)
                .map(|dt| dt.with_timezone(&Utc).timestamp_millis())
                .map_err(|e| D::Error::custom(format!("Invalid datetime: {}", e))),
            DatetimeValue::Int(ms) => Ok(ms),
        }
    }
}

mod duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct DurationRepr {
        #[serde(rename = "_type")]
        type_tag: String,
        nanos: u64,
    }

    pub fn serialize<S: Serializer>(ns: &u64, s: S) -> Result<S::Ok, S::Error> {
        DurationRepr {
            type_tag: "duration".to_string(),
            nanos: *ns,
        }
        .serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum DurationValue {
            Object { nanos: u64 },
            Int(u64),
        }

        match DurationValue::deserialize(d)? {
            DurationValue::Object { nanos } => Ok(nanos),
            DurationValue::Int(ns) => Ok(ns),
        }
    }
}

mod base64_serde {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        STANDARD.decode(&s).map_err(serde::de::Error::custom)
    }
}

/// A wrapper for Decimal that supports rkyv serialization via 16-byte representation.
/// Internally stores bytes for zero-copy rkyv support.
#[derive(Debug, Clone, Copy, PartialEq, Archive, RkyvSerialize, RkyvDeserialize)]
pub struct DecimalValue {
    bytes: [u8; 16],
}

impl DecimalValue {
    /// Create a new DecimalValue from a Decimal
    pub fn new(d: Decimal) -> Self {
        Self {
            bytes: d.serialize(),
        }
    }

    /// Get the inner Decimal value
    pub fn as_decimal(&self) -> Decimal {
        Decimal::deserialize(self.bytes)
    }
}

impl From<Decimal> for DecimalValue {
    fn from(d: Decimal) -> Self {
        Self::new(d)
    }
}

impl From<DecimalValue> for Decimal {
    fn from(d: DecimalValue) -> Decimal {
        d.as_decimal()
    }
}

impl std::ops::Deref for DecimalValue {
    type Target = [u8; 16];
    fn deref(&self) -> &Self::Target {
        &self.bytes
    }
}

impl Serialize for DecimalValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize as a string for JSON compatibility
        serializer.serialize_str(&self.as_decimal().to_string())
    }
}

impl<'de> Deserialize<'de> for DecimalValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let d: Decimal = s.parse().map_err(serde::de::Error::custom)?;
        Ok(DecimalValue::new(d))
    }
}

/// A value that can be stored in a document field
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[rkyv(
    serialize_bounds(__S: Writer + Allocator, <__S as Fallible>::Error: Source),
    deserialize_bounds(<__D as Fallible>::Error: Source),
    bytecheck(bounds(__C: ArchiveContext, <__C as Fallible>::Error: Source))
)]
#[serde(untagged)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Decimal(DecimalValue),
    String(String),
    Array(#[rkyv(omit_bounds)] Vec<Value>),
    Object(#[rkyv(omit_bounds)] HashMap<String, Value>),
    Reference(String),
    // New types - stored as primitives for rkyv compatibility
    /// Datetime stored as Unix timestamp in milliseconds
    Datetime(#[serde(with = "datetime_serde")] i64),
    /// Duration stored as nanoseconds
    Duration(#[serde(with = "duration_serde")] u64),
    /// Binary data
    Bytes(#[serde(with = "base64_serde")] Vec<u8>),
    /// Range type with start and end values
    Range {
        #[rkyv(omit_bounds)]
        start: Box<Value>,
        #[rkyv(omit_bounds)]
        end: Box<Value>,
    },
}

impl Value {
    /// Create a Datetime value from a chrono DateTime
    pub fn from_datetime(dt: chrono::DateTime<chrono::Utc>) -> Self {
        Value::Datetime(dt.timestamp_millis())
    }

    /// Get the value as a chrono DateTime, if it's a Datetime variant
    pub fn as_datetime(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        match self {
            Value::Datetime(ms) => chrono::DateTime::from_timestamp_millis(*ms),
            _ => None,
        }
    }

    /// Create a Duration value from a std::time::Duration
    pub fn from_duration(d: std::time::Duration) -> Self {
        Value::Duration(d.as_nanos() as u64)
    }

    /// Get the value as a std::time::Duration, if it's a Duration variant
    pub fn as_std_duration(&self) -> Option<std::time::Duration> {
        match self {
            Value::Duration(ns) => Some(std::time::Duration::from_nanos(*ns)),
            _ => None,
        }
    }
}

impl Value {
    /// Check if value is a numeric type
    pub fn is_numeric(&self) -> bool {
        matches!(self, Value::Int(_) | Value::Float(_) | Value::Decimal(_))
    }

    /// Check if value is a temporal type
    pub fn is_temporal(&self) -> bool {
        matches!(self, Value::Datetime(_) | Value::Duration(_))
    }

    /// Get type name as string
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Decimal(_) => "decimal",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
            Value::Reference(_) => "reference",
            Value::Datetime(_) => "datetime",
            Value::Duration(_) => "duration",
            Value::Bytes(_) => "bytes",
            Value::Range { .. } => "range",
        }
    }

    /// Returns whether this value is truthy in StellarDB expressions.
    /// Null, false, numeric zero, and empty strings, arrays, objects, and bytes are falsy.
    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(n) => *n != 0,
            Value::Float(f) => *f != 0.0,
            Value::Decimal(d) => !d.as_decimal().is_zero(),
            Value::String(s) => !s.is_empty(),
            Value::Array(arr) => !arr.is_empty(),
            Value::Object(obj) => !obj.is_empty(),
            Value::Bytes(b) => !b.is_empty(),
            Value::Duration(ns) => *ns != 0,
            // Reference, Datetime, Range are always truthy
            Value::Reference(_) | Value::Datetime(_) | Value::Range { .. } => true,
        }
    }

    /// Convert numeric value to Decimal for comparison
    pub fn as_decimal(&self) -> Option<Decimal> {
        match self {
            Value::Int(n) => Some(Decimal::from(*n)),
            Value::Float(f) => Decimal::try_from(*f).ok(),
            Value::Decimal(d) => Some(d.as_decimal()),
            _ => None,
        }
    }
}

impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        use std::cmp::Ordering;

        match (self, other) {
            // Same types - direct comparison
            (Value::Null, Value::Null) => Some(Ordering::Equal),
            (Value::Bool(a), Value::Bool(b)) => a.partial_cmp(b),
            (Value::Int(a), Value::Int(b)) => a.partial_cmp(b),
            (Value::Float(a), Value::Float(b)) => a.partial_cmp(b),
            (Value::Decimal(a), Value::Decimal(b)) => a.as_decimal().partial_cmp(&b.as_decimal()),
            (Value::String(a), Value::String(b)) => a.partial_cmp(b),
            (Value::Reference(a), Value::Reference(b)) => a.partial_cmp(b),
            (Value::Datetime(a), Value::Datetime(b)) => a.partial_cmp(b),
            (Value::Duration(a), Value::Duration(b)) => a.partial_cmp(b),
            (Value::Bytes(a), Value::Bytes(b)) => a.partial_cmp(b),

            // Cross-type numeric comparison via Decimal
            (a, b) if a.is_numeric() && b.is_numeric() => {
                let a_dec = a.as_decimal()?;
                let b_dec = b.as_decimal()?;
                a_dec.partial_cmp(&b_dec)
            }

            // Different non-numeric types are not comparable
            _ => None,
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Null => write!(f, "null"),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Int(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", n),
            Value::Decimal(d) => write!(f, "{}", d.as_decimal()),
            Value::String(s) => write!(f, "{}", s),
            Value::Array(arr) => {
                write!(f, "[")?;
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            Value::Object(obj) => {
                write!(f, "{{")?;
                for (i, (k, v)) in obj.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, "}}")
            }
            Value::Reference(id) => write!(f, "{}", id),
            Value::Datetime(ms) => {
                if let Some(dt) = chrono::DateTime::from_timestamp_millis(*ms) {
                    write!(f, "d\"{}\"", dt.to_rfc3339())
                } else {
                    write!(f, "d\"{}\"", ms)
                }
            }
            Value::Duration(ns) => {
                // Display in a human-readable format
                let nanos = *ns;
                if nanos >= 1_000_000_000 * 60 * 60 {
                    // Hours
                    write!(f, "{}h", nanos / (1_000_000_000 * 60 * 60))
                } else if nanos >= 1_000_000_000 * 60 {
                    // Minutes
                    write!(f, "{}m", nanos / (1_000_000_000 * 60))
                } else if nanos >= 1_000_000_000 {
                    // Seconds
                    write!(f, "{}s", nanos / 1_000_000_000)
                } else if nanos >= 1_000_000 {
                    // Milliseconds
                    write!(f, "{}ms", nanos / 1_000_000)
                } else if nanos >= 1_000 {
                    // Microseconds
                    write!(f, "{}us", nanos / 1_000)
                } else {
                    // Nanoseconds
                    write!(f, "{}ns", nanos)
                }
            }
            Value::Bytes(b) => {
                use base64::{Engine, engine::general_purpose::STANDARD};
                write!(f, "b\"{}\"", STANDARD.encode(b))
            }
            Value::Range { start, end } => write!(f, "{}..{}", start, end),
        }
    }
}

/// A document stored in the database
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[rkyv(
    serialize_bounds(__S: Writer + Allocator, <__S as Fallible>::Error: Source),
    deserialize_bounds(<__D as Fallible>::Error: Source),
    bytecheck(bounds(__C: ArchiveContext, <__C as Fallible>::Error: Source))
)]
pub struct Document {
    /// Unique identifier in format "collection:key"
    pub id: String,
    /// User-defined fields
    #[rkyv(omit_bounds)]
    pub fields: HashMap<String, Value>,
}

impl Document {
    /// Extract collection name from id
    pub fn collection(&self) -> &str {
        self.id.split(':').next().unwrap_or(&self.id)
    }

    /// Extract key from id
    pub fn key(&self) -> &str {
        self.id.split_once(':').map(|(_, k)| k).unwrap_or(&self.id)
    }
}

/// Convert serde_json::Value to our Value type
impl From<serde_json::Value> for Value {
    fn from(v: serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Bool(b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Value::Int(i)
                } else {
                    Value::Float(n.as_f64().unwrap_or(0.0))
                }
            }
            serde_json::Value::String(s) => Value::String(s),
            serde_json::Value::Array(arr) => {
                Value::Array(arr.into_iter().map(Value::from).collect())
            }
            serde_json::Value::Object(obj) => {
                Value::Object(obj.into_iter().map(|(k, v)| (k, Value::from(v))).collect())
            }
        }
    }
}

/// Convert our Value type to serde_json::Value
impl From<Value> for serde_json::Value {
    fn from(v: Value) -> Self {
        match v {
            Value::Null => serde_json::Value::Null,
            Value::Bool(b) => serde_json::Value::Bool(b),
            Value::Int(i) => serde_json::Value::Number(i.into()),
            Value::Float(f) => serde_json::Number::from_f64(f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Value::Decimal(d) => {
                // Convert Decimal to f64 for JSON numeric representation
                // Fall back to string if conversion fails (extremely large/small values)
                use rust_decimal::prelude::ToPrimitive;
                match d.as_decimal().to_f64() {
                    Some(f) => serde_json::Number::from_f64(f)
                        .map(serde_json::Value::Number)
                        .unwrap_or(serde_json::Value::Null),
                    None => serde_json::Value::String(d.as_decimal().to_string()),
                }
            }
            Value::String(s) => serde_json::Value::String(s),
            Value::Array(arr) => {
                serde_json::Value::Array(arr.into_iter().map(serde_json::Value::from).collect())
            }
            Value::Object(obj) => serde_json::Value::Object(
                obj.into_iter()
                    .map(|(k, v)| (k, serde_json::Value::from(v)))
                    .collect(),
            ),
            Value::Reference(id) => serde_json::Value::String(id),
            Value::Datetime(ms) => {
                // Convert timestamp millis to ISO8601 string
                chrono::DateTime::from_timestamp_millis(ms)
                    .map(|dt| serde_json::Value::String(dt.to_rfc3339()))
                    .unwrap_or(serde_json::Value::Number(ms.into()))
            }
            Value::Duration(ns) => {
                // Store as object with type tag for proper deserialization
                let mut obj = serde_json::Map::new();
                obj.insert(
                    "_type".to_string(),
                    serde_json::Value::String("duration".to_string()),
                );
                obj.insert("nanos".to_string(), serde_json::Value::Number(ns.into()));
                serde_json::Value::Object(obj)
            }
            Value::Bytes(b) => {
                use base64::{Engine, engine::general_purpose::STANDARD};
                serde_json::Value::String(STANDARD.encode(&b))
            }
            Value::Range { start, end } => {
                let mut obj = serde_json::Map::new();
                obj.insert("start".to_string(), serde_json::Value::from(*start));
                obj.insert("end".to_string(), serde_json::Value::from(*end));
                serde_json::Value::Object(obj)
            }
        }
    }
}

/// Reserved field names that cannot be used in user data
pub const RESERVED_FIELDS: &[&str] = &["id"];

/// Check if a field name is reserved
pub fn is_reserved_field(name: &str) -> bool {
    RESERVED_FIELDS.contains(&name)
}

/// Extract user fields from JSON, filtering out reserved fields
pub fn extract_user_fields(json: serde_json::Value) -> HashMap<String, Value> {
    match json {
        serde_json::Value::Object(obj) => obj
            .into_iter()
            .filter(|(k, _)| !is_reserved_field(k))
            .map(|(k, v)| (k, Value::from(v)))
            .collect(),
        _ => HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reference_value() {
        let r = Value::Reference("user:alice".to_string());
        assert!(matches!(r, Value::Reference(_)));

        if let Value::Reference(id) = &r {
            let (collection, key) = id.split_once(':').unwrap();
            assert_eq!(collection, "user");
            assert_eq!(key, "alice");
        }
    }

    #[test]
    fn test_reference_json_conversion() {
        let r = Value::Reference("post:123".to_string());
        let json: serde_json::Value = r.clone().into();
        assert_eq!(json, serde_json::Value::String("post:123".to_string()));
    }

    #[test]
    fn test_value_cross_type_ordering() {
        // Int vs Float
        assert!(Value::Int(1) < Value::Float(1.5));
        assert!(Value::Float(0.5) < Value::Int(1));

        // Int vs Decimal
        assert!(Value::Int(1) < Value::Decimal(DecimalValue::new("1.5".parse().unwrap())));

        // Float vs Decimal
        assert!(Value::Float(1.5) < Value::Decimal(DecimalValue::new("2".parse().unwrap())));

        // Equal values across types
        assert!(Value::Int(5).partial_cmp(&Value::Float(5.0)) == Some(std::cmp::Ordering::Equal));
        assert!(
            Value::Int(5).partial_cmp(&Value::Decimal(DecimalValue::new("5".parse().unwrap())))
                == Some(std::cmp::Ordering::Equal)
        );
    }
}
