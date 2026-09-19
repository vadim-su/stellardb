//! Shared accumulator logic for aggregate and group_aggregate operators.
//!
//! Also contains value comparison and serialization utilities used by
//! sort, distinct, and value operators.

use std::cmp::Ordering;
use std::collections::HashSet;

use rust_decimal::Decimal;

use crate::document::{DecimalValue, Value};
use crate::query::function::AggregateFunction;

/// Tracks what numeric types we've seen for SUM/AVG
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumericMode {
    /// No values seen yet
    None,
    /// Only Int values seen - accumulate as Decimal for precision
    IntOnly,
    /// At least one Decimal seen (no Float) - use Decimal arithmetic
    Decimal,
    /// At least one Float seen - use Float arithmetic (loses precision)
    Float,
}

pub struct Accumulator {
    function: AggregateFunction,
    distinct: bool,
    distinct_values: HashSet<String>,
    count: i64,
    /// Float sum (used when any Float is encountered)
    sum_float: f64,
    /// Decimal sum (used for Int-only or Decimal values)
    sum_decimal: Decimal,
    /// Tracks numeric type mode for SUM/AVG result type
    numeric_mode: NumericMode,
    min: Option<Value>,
    max: Option<Value>,
}

impl Accumulator {
    pub fn new(function: AggregateFunction, distinct: bool) -> Self {
        Self {
            function,
            distinct,
            distinct_values: HashSet::new(),
            count: 0,
            sum_float: 0.0,
            sum_decimal: Decimal::ZERO,
            numeric_mode: NumericMode::None,
            min: None,
            max: None,
        }
    }

    /// Feed a wildcard row (COUNT(*) — always counts)
    pub fn feed_wildcard(&mut self) {
        self.count += 1;
    }

    /// Feed a field value (None means field not present — skip for all aggregates)
    pub fn feed(&mut self, value: Option<&Value>) {
        // For DISTINCT, check if we've seen this value
        if self.distinct {
            if let Some(v) = value {
                if matches!(v, Value::Null) {
                    return; // Skip nulls for DISTINCT
                }
                let key = serialize_value(v);
                if !self.distinct_values.insert(key) {
                    return; // Already seen this value
                }
            } else {
                return; // Skip None
            }
        }

        match self.function {
            AggregateFunction::Count => match value {
                None | Some(Value::Null) => {}
                Some(_) => self.count += 1,
            },
            AggregateFunction::Sum | AggregateFunction::Avg => {
                if let Some(v) = value {
                    self.feed_numeric(v);
                }
            }
            AggregateFunction::Min => {
                if let Some(val) = value {
                    if matches!(val, Value::Null) {
                        return;
                    }
                    match &self.min {
                        None => self.min = Some(val.clone()),
                        Some(current) => {
                            if compare_value_inner(val, current) == Ordering::Less {
                                self.min = Some(val.clone());
                            }
                        }
                    }
                }
            }
            AggregateFunction::Max => {
                if let Some(val) = value {
                    if matches!(val, Value::Null) {
                        return;
                    }
                    match &self.max {
                        None => self.max = Some(val.clone()),
                        Some(current) => {
                            if compare_value_inner(val, current) == Ordering::Greater {
                                self.max = Some(val.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    /// Feed a numeric value for SUM/AVG with proper type handling
    fn feed_numeric(&mut self, value: &Value) {
        match value {
            Value::Null => (),
            Value::Int(i) => {
                // Int: add to decimal sum, upgrade mode if needed
                self.sum_decimal += Decimal::from(*i);
                if self.numeric_mode == NumericMode::None {
                    self.numeric_mode = NumericMode::IntOnly;
                }
                // If already Float mode, also add to float sum
                if self.numeric_mode == NumericMode::Float {
                    self.sum_float += *i as f64;
                }
                self.count += 1;
            }
            Value::Decimal(d) => {
                // Decimal: add to decimal sum
                self.sum_decimal += d.as_decimal();
                match self.numeric_mode {
                    NumericMode::None | NumericMode::IntOnly => {
                        self.numeric_mode = NumericMode::Decimal;
                    }
                    NumericMode::Float => {
                        // Also add to float sum if we're in float mode
                        use std::str::FromStr;
                        if let Ok(f) = f64::from_str(&d.as_decimal().to_string()) {
                            self.sum_float += f;
                        }
                    }
                    NumericMode::Decimal => {}
                }
                self.count += 1;
            }
            Value::Float(f) => {
                // Float: switch to float mode, convert accumulated decimal sum
                if self.numeric_mode != NumericMode::Float {
                    // Convert decimal sum to float
                    use std::str::FromStr;
                    self.sum_float = f64::from_str(&self.sum_decimal.to_string()).unwrap_or(0.0);
                    self.numeric_mode = NumericMode::Float;
                }
                self.sum_float += *f;
                self.count += 1;
            }
            _ => {} // Non-numeric values are skipped
        }
    }

    pub fn result(&self) -> Value {
        match self.function {
            AggregateFunction::Count => {
                if self.distinct {
                    Value::Int(self.distinct_values.len() as i64)
                } else {
                    Value::Int(self.count)
                }
            }
            AggregateFunction::Sum => {
                if self.count == 0 {
                    Value::Null
                } else {
                    match self.numeric_mode {
                        NumericMode::None => Value::Null,
                        NumericMode::IntOnly | NumericMode::Decimal => {
                            Value::Decimal(DecimalValue::new(self.sum_decimal))
                        }
                        NumericMode::Float => Value::Float(self.sum_float),
                    }
                }
            }
            AggregateFunction::Avg => {
                if self.count == 0 {
                    Value::Null
                } else {
                    match self.numeric_mode {
                        NumericMode::None => Value::Null,
                        NumericMode::IntOnly | NumericMode::Decimal => {
                            let avg = self.sum_decimal / Decimal::from(self.count);
                            Value::Decimal(DecimalValue::new(avg))
                        }
                        NumericMode::Float => Value::Float(self.sum_float / self.count as f64),
                    }
                }
            }
            AggregateFunction::Min => self.min.clone().unwrap_or(Value::Null),
            AggregateFunction::Max => self.max.clone().unwrap_or(Value::Null),
        }
    }
}

/// Compare two values with a total ordering.
/// Null < Bool < Number < String < Array < Object < Reference < Datetime < Duration < Bytes < Range
pub fn compare_value_inner(a: &Value, b: &Value) -> Ordering {
    let type_ord = type_priority(a).cmp(&type_priority(b));
    if type_ord != Ordering::Equal {
        return type_ord;
    }

    // For numeric types, use Value::partial_cmp which handles cross-type comparison
    if a.is_numeric() && b.is_numeric() {
        return a.partial_cmp(b).unwrap_or(Ordering::Equal);
    }

    match (a, b) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        _ => Ordering::Equal,
    }
}

/// Compare two optional values with None sorting before Some.
pub fn compare_values(a: Option<&Value>, b: Option<&Value>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(a), Some(b)) => compare_value_inner(a, b),
    }
}

fn type_priority(v: &Value) -> u8 {
    match v {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Int(_) => 2,
        Value::Float(_) => 2,
        Value::Decimal(_) => 2, // Same priority as Int/Float for numeric types
        Value::String(_) => 3,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
        Value::Reference(_) => 6,
        Value::Datetime(_) => 7,
        Value::Duration(_) => 8,
        Value::Bytes(_) => 9,
        Value::Range { .. } => 10,
    }
}

/// Serialize a Value to a deterministic string for use as hash keys (DISTINCT, GROUP BY).
pub fn serialize_value(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Decimal(d) => d.as_decimal().to_string(),
        Value::String(s) => format!("\"{}\"", s),
        Value::Array(_) => format!("{:?}", v),
        Value::Object(_) => format!("{:?}", v),
        Value::Reference(id) => format!("ref:{}", id),
        Value::Datetime(ms) => format!("datetime:{}", ms),
        Value::Duration(ns) => format!("duration:{}", ns),
        Value::Bytes(b) => format!("bytes:{:?}", b),
        Value::Range { start, end } => format!("range:{}..{}", start, end),
    }
}
