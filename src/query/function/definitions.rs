//! Function and constant definitions using the define_functions! macro.

use crate::document::{DecimalValue, Value};
use crate::query::error::ExecuteError;
use crate::query::execute::eval::value_type_name;
use regex::Regex;

// Math functions defined using the macro
define_functions! {
    namespace math {
        fn abs(x: Numeric) -> Numeric {
            match x {
                Value::Null => Ok(Value::Null),
                Value::Int(i) => Ok(Value::Int(i.abs())),
                Value::Float(f) => Ok(Value::Float(f.abs())),
                other => Err(ExecuteError::TypeError {
                    expected: "Numeric",
                    got: value_type_name(&other).to_string(),
                    op: "math::abs",
                }),
            }
        }

        fn round(x: Numeric) -> Int {
            match x {
                Value::Null => Ok(Value::Null),
                Value::Int(i) => Ok(Value::Int(i)),
                Value::Float(f) => Ok(Value::Int(f.round() as i64)),
                other => Err(ExecuteError::TypeError {
                    expected: "Numeric",
                    got: value_type_name(&other).to_string(),
                    op: "math::round",
                }),
            }
        }

        fn floor(x: Numeric) -> Int {
            match x {
                Value::Null => Ok(Value::Null),
                Value::Int(i) => Ok(Value::Int(i)),
                Value::Float(f) => Ok(Value::Int(f.floor() as i64)),
                other => Err(ExecuteError::TypeError {
                    expected: "Numeric",
                    got: value_type_name(&other).to_string(),
                    op: "math::floor",
                }),
            }
        }

        fn ceil(x: Numeric) -> Int {
            match x {
                Value::Null => Ok(Value::Null),
                Value::Int(i) => Ok(Value::Int(i)),
                Value::Float(f) => Ok(Value::Int(f.ceil() as i64)),
                other => Err(ExecuteError::TypeError {
                    expected: "Numeric",
                    got: value_type_name(&other).to_string(),
                    op: "math::ceil",
                }),
            }
        }

        fn trunc(x: Numeric) -> Int {
            match x {
                Value::Null => Ok(Value::Null),
                Value::Int(i) => Ok(Value::Int(i)),
                Value::Float(f) => Ok(Value::Int(f.trunc() as i64)),
                other => Err(ExecuteError::TypeError {
                    expected: "Numeric",
                    got: value_type_name(&other).to_string(),
                    op: "math::trunc",
                }),
            }
        }

        fn sign(x: Numeric) -> Int {
            match x {
                Value::Null => Ok(Value::Null),
                Value::Int(i) => Ok(Value::Int(i.signum())),
                Value::Float(f) => {
                    let s = if f > 0.0 { 1 } else if f < 0.0 { -1 } else { 0 };
                    Ok(Value::Int(s))
                }
                other => Err(ExecuteError::TypeError {
                    expected: "Numeric",
                    got: value_type_name(&other).to_string(),
                    op: "math::sign",
                }),
            }
        }

        fn sqrt(x: Numeric) -> Float {
            match x {
                Value::Null => Ok(Value::Null),
                Value::Int(i) => Ok(Value::Float((i as f64).sqrt())),
                Value::Float(f) => Ok(Value::Float(f.sqrt())),
                other => Err(ExecuteError::TypeError {
                    expected: "Numeric",
                    got: value_type_name(&other).to_string(),
                    op: "math::sqrt",
                }),
            }
        }

        fn pow(base: Numeric, exp: Numeric) -> Float {
            match (&base, &exp) {
                (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                (Value::Int(b), Value::Int(e)) => Ok(Value::Float((*b as f64).powf(*e as f64))),
                (Value::Int(b), Value::Float(e)) => Ok(Value::Float((*b as f64).powf(*e))),
                (Value::Float(b), Value::Int(e)) => Ok(Value::Float(b.powf(*e as f64))),
                (Value::Float(b), Value::Float(e)) => Ok(Value::Float(b.powf(*e))),
                _ => Err(ExecuteError::TypeError {
                    expected: "Numeric, Numeric",
                    got: format!("{}, {}", value_type_name(&base), value_type_name(&exp)),
                    op: "math::pow",
                }),
            }
        }

        fn exp(x: Numeric) -> Float {
            match x {
                Value::Null => Ok(Value::Null),
                Value::Int(i) => Ok(Value::Float((i as f64).exp())),
                Value::Float(f) => Ok(Value::Float(f.exp())),
                other => Err(ExecuteError::TypeError {
                    expected: "Numeric",
                    got: value_type_name(&other).to_string(),
                    op: "math::exp",
                }),
            }
        }

        fn log(x: Numeric, base: Numeric) -> Float {
            match (&x, &base) {
                (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                (Value::Int(v), Value::Int(b)) => Ok(Value::Float((*v as f64).log(*b as f64))),
                (Value::Int(v), Value::Float(b)) => Ok(Value::Float((*v as f64).log(*b))),
                (Value::Float(v), Value::Int(b)) => Ok(Value::Float(v.log(*b as f64))),
                (Value::Float(v), Value::Float(b)) => Ok(Value::Float(v.log(*b))),
                _ => Err(ExecuteError::TypeError {
                    expected: "Numeric, Numeric",
                    got: format!("{}, {}", value_type_name(&x), value_type_name(&base)),
                    op: "math::log",
                }),
            }
        }

        fn modulo(a: Numeric, b: Numeric) -> Numeric {
            match (&a, &b) {
                (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                (Value::Int(_), Value::Int(0)) => Err(ExecuteError::DivisionByZero),
                (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x % y)),
                (Value::Float(_), Value::Float(y)) if *y == 0.0 => Err(ExecuteError::DivisionByZero),
                (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x % y)),
                (Value::Int(_), Value::Float(y)) if *y == 0.0 => Err(ExecuteError::DivisionByZero),
                (Value::Int(x), Value::Float(y)) => Ok(Value::Float((*x as f64) % y)),
                (Value::Float(_), Value::Int(0)) => Err(ExecuteError::DivisionByZero),
                (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x % (*y as f64))),
                _ => Err(ExecuteError::TypeError {
                    expected: "Numeric, Numeric",
                    got: format!("{}, {}", value_type_name(&a), value_type_name(&b)),
                    op: "math::mod",
                }),
            }
        }

        fn min(a: Numeric, b: Numeric) -> Numeric {
            match (&a, &b) {
                (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                (Value::Int(x), Value::Int(y)) => Ok(Value::Int(*x.min(y))),
                (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x.min(*y))),
                (Value::Int(x), Value::Float(y)) => Ok(Value::Float((*x as f64).min(*y))),
                (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x.min(*y as f64))),
                _ => Err(ExecuteError::TypeError {
                    expected: "Numeric, Numeric",
                    got: format!("{}, {}", value_type_name(&a), value_type_name(&b)),
                    op: "math::min",
                }),
            }
        }

        fn max(a: Numeric, b: Numeric) -> Numeric {
            match (&a, &b) {
                (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                (Value::Int(x), Value::Int(y)) => Ok(Value::Int(*x.max(y))),
                (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x.max(*y))),
                (Value::Int(x), Value::Float(y)) => Ok(Value::Float((*x as f64).max(*y))),
                (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x.max(*y as f64))),
                _ => Err(ExecuteError::TypeError {
                    expected: "Numeric, Numeric",
                    got: format!("{}, {}", value_type_name(&a), value_type_name(&b)),
                    op: "math::max",
                }),
            }
        }

        fn sin(x: Numeric) -> Float {
            match x {
                Value::Null => Ok(Value::Null),
                Value::Int(i) => Ok(Value::Float((i as f64).sin())),
                Value::Float(f) => Ok(Value::Float(f.sin())),
                other => Err(ExecuteError::TypeError {
                    expected: "Numeric",
                    got: value_type_name(&other).to_string(),
                    op: "math::sin",
                }),
            }
        }

        fn cos(x: Numeric) -> Float {
            match x {
                Value::Null => Ok(Value::Null),
                Value::Int(i) => Ok(Value::Float((i as f64).cos())),
                Value::Float(f) => Ok(Value::Float(f.cos())),
                other => Err(ExecuteError::TypeError {
                    expected: "Numeric",
                    got: value_type_name(&other).to_string(),
                    op: "math::cos",
                }),
            }
        }

        fn tan(x: Numeric) -> Float {
            match x {
                Value::Null => Ok(Value::Null),
                Value::Int(i) => Ok(Value::Float((i as f64).tan())),
                Value::Float(f) => Ok(Value::Float(f.tan())),
                other => Err(ExecuteError::TypeError {
                    expected: "Numeric",
                    got: value_type_name(&other).to_string(),
                    op: "math::tan",
                }),
            }
        }

        const pi: Float = std::f64::consts::PI;
        const e: Float = std::f64::consts::E;
        const tau: Float = std::f64::consts::TAU;
    }

    namespace string {
        fn upper(s: String) -> String {
            match s {
                Value::Null => Ok(Value::Null),
                Value::String(s) => Ok(Value::String(s.to_uppercase())),
                other => Err(ExecuteError::TypeError {
                    expected: "String",
                    got: value_type_name(&other).to_string(),
                    op: "string::upper",
                }),
            }
        }

        fn lower(s: String) -> String {
            match s {
                Value::Null => Ok(Value::Null),
                Value::String(s) => Ok(Value::String(s.to_lowercase())),
                other => Err(ExecuteError::TypeError {
                    expected: "String",
                    got: value_type_name(&other).to_string(),
                    op: "string::lower",
                }),
            }
        }

        fn length(s: String) -> Int {
            match s {
                Value::Null => Ok(Value::Null),
                Value::String(s) => Ok(Value::Int(s.chars().count() as i64)),
                other => Err(ExecuteError::TypeError {
                    expected: "String",
                    got: value_type_name(&other).to_string(),
                    op: "string::length",
                }),
            }
        }

        fn trim(s: String) -> String {
            match s {
                Value::Null => Ok(Value::Null),
                Value::String(s) => Ok(Value::String(s.trim().to_string())),
                other => Err(ExecuteError::TypeError {
                    expected: "String",
                    got: value_type_name(&other).to_string(),
                    op: "string::trim",
                }),
            }
        }

        fn replace(s: String, from: String, to: String) -> String {
            match (&s, &from, &to) {
                (Value::Null, _, _) | (_, Value::Null, _) | (_, _, Value::Null) => Ok(Value::Null),
                (Value::String(s), Value::String(from), Value::String(to)) => {
                    Ok(Value::String(s.replace(from.as_str(), to.as_str())))
                }
                _ => Err(ExecuteError::TypeError {
                    expected: "String, String, String",
                    got: format!("{}, {}, {}", value_type_name(&s), value_type_name(&from), value_type_name(&to)),
                    op: "string::replace",
                }),
            }
        }

        fn split(s: String, delim: String) -> Array {
            match (&s, &delim) {
                (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                (Value::String(str_val), Value::String(delim_val)) => {
                    let parts: Vec<Value> = str_val
                        .split(delim_val.as_str())
                        .map(|p| Value::String(p.to_string()))
                        .collect();
                    Ok(Value::Array(parts))
                }
                _ => Err(ExecuteError::TypeError {
                    expected: "String, String",
                    got: format!("{}, {}", value_type_name(&s), value_type_name(&delim)),
                    op: "string::split",
                }),
            }
        }

        fn starts_with(s: String, prefix: String) -> Bool {
            match (&s, &prefix) {
                (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                (Value::String(str_val), Value::String(prefix_val)) => {
                    Ok(Value::Bool(str_val.starts_with(prefix_val.as_str())))
                }
                _ => Err(ExecuteError::TypeError {
                    expected: "String, String",
                    got: format!("{}, {}", value_type_name(&s), value_type_name(&prefix)),
                    op: "string::starts_with",
                }),
            }
        }

        fn ends_with(s: String, suffix: String) -> Bool {
            match (&s, &suffix) {
                (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                (Value::String(str_val), Value::String(suffix_val)) => {
                    Ok(Value::Bool(str_val.ends_with(suffix_val.as_str())))
                }
                _ => Err(ExecuteError::TypeError {
                    expected: "String, String",
                    got: format!("{}, {}", value_type_name(&s), value_type_name(&suffix)),
                    op: "string::ends_with",
                }),
            }
        }

        fn matches(text: String, pattern: String) -> Bool {
            match (&text, &pattern) {
                (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                (Value::String(text_val), Value::String(pattern_val)) => {
                    match Regex::new(pattern_val) {
                        Ok(re) => Ok(Value::Bool(re.is_match(text_val))),
                        Err(_) => Err(ExecuteError::Internal(
                            format!("string::matches: invalid regex pattern '{}'", pattern_val)
                        )),
                    }
                }
                _ => Err(ExecuteError::TypeError {
                    expected: "String, String",
                    got: format!("{}, {}", value_type_name(&text), value_type_name(&pattern)),
                    op: "string::matches",
                }),
            }
        }

        // Variadic concat - concatenates any number of values
        variadic fn concat(args) -> String {
            let mut result = String::new();
            for arg in args {
                match arg {
                    Value::Null => {}
                    Value::String(s) => result.push_str(s),
                    Value::Int(i) => result.push_str(&i.to_string()),
                    Value::Float(f) => result.push_str(&f.to_string()),
                    Value::Decimal(d) => result.push_str(&d.as_decimal().to_string()),
                    Value::Bool(b) => result.push_str(&b.to_string()),
                    Value::Array(_) => result.push_str("[array]"),
                    Value::Object(_) => result.push_str("[object]"),
                    Value::Reference(id) => result.push_str(&format!("ref:{}", id)),
                    Value::Datetime(ms) => {
                        if let Some(dt) = chrono::DateTime::from_timestamp_millis(*ms) {
                            result.push_str(&dt.to_rfc3339());
                        } else {
                            result.push_str(&ms.to_string());
                        }
                    }
                    Value::Duration(ns) => result.push_str(&format!("{}ns", ns)),
                    Value::Bytes(_) => result.push_str("[bytes]"),
                    Value::Range { start, end } => result.push_str(&format!("{}..{}", start, end)),
                }
            }
            Ok(Value::String(result))
        }

        // substring(s, start) or substring(s, start, len) - variadic to handle 2-3 args
        variadic fn substring(args) -> String {
            if args.len() < 2 || args.len() > 3 {
                return Err(ExecuteError::ArityMismatch {
                    function: "string::substring".to_string(),
                    expected: "2-3".to_string(),
                    got: args.len(),
                });
            }
            let s = &args[0];
            let start = &args[1];
            let len = args.get(2);

            match (s, start) {
                (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                (Value::String(str_val), Value::Int(start_idx)) => {
                    let chars: Vec<char> = str_val.chars().collect();
                    let start_pos = *start_idx as usize;
                    let take_len = match len {
                        Some(Value::Int(l)) => *l as usize,
                        Some(Value::Null) | None => chars.len().saturating_sub(start_pos),
                        Some(other) => return Err(ExecuteError::TypeError {
                            expected: "Int",
                            got: value_type_name(other).to_string(),
                            op: "string::substring length",
                        }),
                    };
                    let result: String = chars.iter().skip(start_pos).take(take_len).collect();
                    Ok(Value::String(result))
                }
                _ => Err(ExecuteError::TypeError {
                    expected: "String, Int",
                    got: format!("{}, {}", value_type_name(s), value_type_name(start)),
                    op: "string::substring",
                }),
            }
        }
    }

    namespace array {
        fn length(arr: Array) -> Int {
            match arr {
                Value::Null => Ok(Value::Null),
                Value::Array(arr) => Ok(Value::Int(arr.len() as i64)),
                other => Err(ExecuteError::TypeError {
                    expected: "Array",
                    got: value_type_name(&other).to_string(),
                    op: "array::length",
                }),
            }
        }

        fn contains(arr: Array, needle: Any) -> Bool {
            match arr {
                Value::Null => Ok(Value::Null),
                Value::Array(arr) => {
                    let found = arr.iter().any(|item| values_equal(item, &needle));
                    Ok(Value::Bool(found))
                }
                other => Err(ExecuteError::TypeError {
                    expected: "Array",
                    got: value_type_name(&other).to_string(),
                    op: "array::contains",
                }),
            }
        }

        fn append(arr: Array, item: Any) -> Array {
            match arr {
                Value::Null => Ok(Value::Null),
                Value::Array(mut arr) => {
                    arr.push(item);
                    Ok(Value::Array(arr))
                }
                other => Err(ExecuteError::TypeError {
                    expected: "Array",
                    got: value_type_name(&other).to_string(),
                    op: "array::append",
                }),
            }
        }

        fn reverse(arr: Array) -> Array {
            match arr {
                Value::Null => Ok(Value::Null),
                Value::Array(mut arr) => {
                    arr.reverse();
                    Ok(Value::Array(arr))
                }
                other => Err(ExecuteError::TypeError {
                    expected: "Array",
                    got: value_type_name(&other).to_string(),
                    op: "array::reverse",
                }),
            }
        }

        fn flatten(arr: Array) -> Array {
            match arr {
                Value::Null => Ok(Value::Null),
                Value::Array(arr) => {
                    let mut result = Vec::new();
                    for item in arr {
                        match item {
                            Value::Array(inner) => result.extend(inner),
                            other => result.push(other),
                        }
                    }
                    Ok(Value::Array(result))
                }
                other => Err(ExecuteError::TypeError {
                    expected: "Array",
                    got: value_type_name(&other).to_string(),
                    op: "array::flatten",
                }),
            }
        }

        fn distinct(arr: Array) -> Array {
            use std::collections::HashSet;

            fn serialize_val(v: &Value) -> String {
                match v {
                    Value::Null => "null".to_string(),
                    Value::Bool(b) => b.to_string(),
                    Value::Int(i) => i.to_string(),
                    Value::Float(f) => f.to_string(),
                    Value::Decimal(d) => d.as_decimal().to_string(),
                    Value::String(s) => format!("\"{}\"", s),
                    Value::Array(a) => format!("{:?}", a),
                    Value::Object(o) => format!("{:?}", o),
                    Value::Reference(id) => format!("ref:{}", id),
                    Value::Datetime(ms) => format!("datetime:{}", ms),
                    Value::Duration(ns) => format!("duration:{}", ns),
                    Value::Bytes(b) => format!("bytes:{:?}", b),
                    Value::Range { start, end } => format!("range:{}..{}", start, end),
                }
            }

            match arr {
                Value::Null => Ok(Value::Null),
                Value::Array(arr) => {
                    let mut seen = HashSet::new();
                    let mut result = Vec::new();
                    for val in arr {
                        let key = serialize_val(&val);
                        if seen.insert(key) {
                            result.push(val);
                        }
                    }
                    Ok(Value::Array(result))
                }
                other => Err(ExecuteError::TypeError {
                    expected: "Array",
                    got: value_type_name(&other).to_string(),
                    op: "array::distinct",
                }),
            }
        }
    }

    namespace type {
        fn to_int(val: Any) -> Int {
            match val {
                Value::Null => Ok(Value::Null),
                Value::Int(i) => Ok(Value::Int(i)),
                Value::Float(f) => Ok(Value::Int(f as i64)),
                Value::Bool(b) => Ok(Value::Int(if b { 1 } else { 0 })),
                Value::String(s) => match s.trim().parse::<i64>() {
                    Ok(i) => Ok(Value::Int(i)),
                    Err(_) => Ok(Value::Null),
                },
                _ => Ok(Value::Null),
            }
        }

        fn to_float(val: Any) -> Float {
            match val {
                Value::Null => Ok(Value::Null),
                Value::Int(i) => Ok(Value::Float(i as f64)),
                Value::Float(f) => Ok(Value::Float(f)),
                Value::Bool(b) => Ok(Value::Float(if b { 1.0 } else { 0.0 })),
                Value::String(s) => match s.trim().parse::<f64>() {
                    Ok(f) => Ok(Value::Float(f)),
                    Err(_) => Ok(Value::Null),
                },
                _ => Ok(Value::Null),
            }
        }

        fn to_decimal(val: Any) -> Decimal {
            use rust_decimal::Decimal;
            match val {
                Value::Null => Ok(Value::Null),
                Value::Int(i) => Ok(Value::Decimal(DecimalValue::new(Decimal::from(i)))),
                Value::Float(f) => match Decimal::try_from(f) {
                    Ok(d) => Ok(Value::Decimal(DecimalValue::new(d))),
                    Err(_) => Ok(Value::Null),
                },
                Value::Decimal(d) => Ok(Value::Decimal(d)),
                Value::Bool(b) => Ok(Value::Decimal(DecimalValue::new(Decimal::from(if b { 1 } else { 0 })))),
                Value::String(s) => match s.trim().parse::<Decimal>() {
                    Ok(d) => Ok(Value::Decimal(DecimalValue::new(d))),
                    Err(_) => Ok(Value::Null),
                },
                _ => Ok(Value::Null),
            }
        }

        fn to_string(val: Any) -> String {
            let s = match val {
                Value::Null => "null".to_string(),
                Value::Int(i) => i.to_string(),
                Value::Float(f) => f.to_string(),
                Value::Decimal(d) => d.as_decimal().to_string(),
                Value::Bool(b) => b.to_string(),
                Value::String(s) => s,
                Value::Array(_) => "[array]".to_string(),
                Value::Object(_) => "[object]".to_string(),
                Value::Reference(id) => format!("ref:{}", id),
                Value::Datetime(ms) => {
                    chrono::DateTime::from_timestamp_millis(ms)
                        .map(|dt| dt.to_rfc3339())
                        .unwrap_or_else(|| ms.to_string())
                }
                Value::Duration(ns) => format!("{}ns", ns),
                Value::Bytes(_) => "[bytes]".to_string(),
                Value::Range { start, end } => format!("{}..{}", start, end),
            };
            Ok(Value::String(s))
        }

        fn to_bool(val: Any) -> Bool {
            // Use Value's is_truthy method for consistency
            Ok(Value::Bool(val.is_truthy()))
        }

        fn is_null(val: Any) -> Bool {
            Ok(Value::Bool(matches!(val, Value::Null)))
        }

        fn is_int(val: Any) -> Bool {
            Ok(Value::Bool(matches!(val, Value::Int(_))))
        }

        fn is_float(val: Any) -> Bool {
            Ok(Value::Bool(matches!(val, Value::Float(_))))
        }

        fn is_number(val: Any) -> Bool {
            Ok(Value::Bool(matches!(val, Value::Int(_) | Value::Float(_))))
        }

        fn is_string(val: Any) -> Bool {
            Ok(Value::Bool(matches!(val, Value::String(_))))
        }

        fn is_bool(val: Any) -> Bool {
            Ok(Value::Bool(matches!(val, Value::Bool(_))))
        }

        fn is_array(val: Any) -> Bool {
            Ok(Value::Bool(matches!(val, Value::Array(_))))
        }

        fn is_object(val: Any) -> Bool {
            Ok(Value::Bool(matches!(val, Value::Object(_))))
        }

        fn is_reference(val: Any) -> Bool {
            Ok(Value::Bool(matches!(val, Value::Reference(_))))
        }

        // Convert a string "collection:key" to a Reference
        fn ref(val: Any) -> Any {
            match val {
                Value::String(s) => {
                    // Validate format: must contain ":"
                    if s.contains(':') {
                        Ok(Value::Reference(s))
                    } else {
                        Ok(Value::Null)
                    }
                }
                Value::Reference(r) => Ok(Value::Reference(r)), // Already a reference
                _ => Ok(Value::Null),
            }
        }

        fn of(val: Any) -> String {
            // Use Value's type_name method for consistency
            Ok(Value::String(val.type_name().to_string()))
        }

        fn default(val: Any, default_val: Any) -> Any {
            if matches!(val, Value::Null) {
                Ok(default_val)
            } else {
                Ok(val)
            }
        }

        // Variadic coalesce - returns first non-null argument
        variadic fn coalesce(args) -> Any {
            for arg in args {
                if !matches!(arg, Value::Null) {
                    return Ok(arg.clone());
                }
            }
            Ok(Value::Null)
        }
    }

    // Vector functions - distance
    namespace vector {
        // vector::distance() - Returns distance from KNN search.
        // The actual distance value comes from the $distance field in the row.
        // Returns Null when called outside of vector search context.
        fn distance() -> Float {
            // Default: return Null (actual value injected by vector search operator)
            Ok(Value::Null)
        }
    }

    // FTS functions - hybrid_score, score, highlight
    namespace fts {
        // fts::hybrid_score() - Returns RRF score for hybrid search.
        // Returns Null when called outside of hybrid search context.
        fn hybrid_score() -> Float {
            // Default: return Null (actual value injected by hybrid search operator)
            Ok(Value::Null)
        }

        // fts::score() - Returns FTS relevance score for current row.
        // Usage: score() for unnamed @@ operator, score("name") for named @:name@ operator
        // Returns Null when called outside of FTS query context.
        variadic fn score(args) -> Float {
            // Validate arity: 0 or 1 arguments
            if args.len() > 1 {
                return Err(ExecuteError::ArityMismatch {
                    function: "fts::score".to_string(),
                    expected: "0-1".to_string(),
                    got: args.len(),
                });
            }
            // Validate optional name argument is a string
            if let Some(name) = args.first()
                && !matches!(name, Value::String(_) | Value::Null)
            {
                return Err(ExecuteError::TypeError {
                    expected: "String",
                    got: value_type_name(name).to_string(),
                    op: "fts::score name parameter",
                });
            }
            // Default: return Null (actual value injected by FTS operator)
            Ok(Value::Null)
        }

        // fts::highlight() - Returns highlighted text with matched terms wrapped in tags.
        // Usage: highlight(), highlight("name"), or highlight("name", "<b>", "</b>")
        // Returns Null when called outside of FTS query context.
        variadic fn highlight(args) -> String {
            // Validate arity: 0, 1, or 3 arguments
            if args.len() == 2 || args.len() > 3 {
                return Err(ExecuteError::ArityMismatch {
                    function: "fts::highlight".to_string(),
                    expected: "0, 1, or 3".to_string(),
                    got: args.len(),
                });
            }
            // Validate all arguments are strings (or null)
            for (i, arg) in args.iter().enumerate() {
                if !matches!(arg, Value::String(_) | Value::Null) {
                    let op_str: &'static str = match i {
                        0 => "fts::highlight name parameter",
                        1 => "fts::highlight open_tag parameter",
                        2 => "fts::highlight close_tag parameter",
                        _ => "fts::highlight argument",
                    };
                    return Err(ExecuteError::TypeError {
                        expected: "String",
                        got: value_type_name(arg).to_string(),
                        op: op_str,
                    });
                }
            }
            // Default: return Null (actual value injected by FTS operator)
            Ok(Value::Null)
        }
    }

    // Time functions - datetime manipulation
    namespace time {
        // time::now() - returns current UTC datetime
        fn now() -> Any {
            Ok(Value::from_datetime(chrono::Utc::now()))
        }

        // time::year(dt) - extract year from datetime
        fn year(dt: Any) -> Int {
            match dt {
                Value::Datetime(ms) => {
                    if let Some(dt) = chrono::DateTime::from_timestamp_millis(ms) {
                        Ok(Value::Int(chrono::Datelike::year(&dt) as i64))
                    } else {
                        Ok(Value::Null)
                    }
                }
                Value::Null => Ok(Value::Null),
                other => Err(ExecuteError::TypeError {
                    expected: "datetime",
                    got: value_type_name(&other).to_string(),
                    op: "time::year",
                }),
            }
        }

        // time::month(dt) - extract month from datetime (1-12)
        fn month(dt: Any) -> Int {
            match dt {
                Value::Datetime(ms) => {
                    if let Some(dt) = chrono::DateTime::from_timestamp_millis(ms) {
                        Ok(Value::Int(chrono::Datelike::month(&dt) as i64))
                    } else {
                        Ok(Value::Null)
                    }
                }
                Value::Null => Ok(Value::Null),
                other => Err(ExecuteError::TypeError {
                    expected: "datetime",
                    got: value_type_name(&other).to_string(),
                    op: "time::month",
                }),
            }
        }

        // time::day(dt) - extract day of month from datetime (1-31)
        fn day(dt: Any) -> Int {
            match dt {
                Value::Datetime(ms) => {
                    if let Some(dt) = chrono::DateTime::from_timestamp_millis(ms) {
                        Ok(Value::Int(chrono::Datelike::day(&dt) as i64))
                    } else {
                        Ok(Value::Null)
                    }
                }
                Value::Null => Ok(Value::Null),
                other => Err(ExecuteError::TypeError {
                    expected: "datetime",
                    got: value_type_name(&other).to_string(),
                    op: "time::day",
                }),
            }
        }

        // time::hour(dt) - extract hour from datetime (0-23)
        fn hour(dt: Any) -> Int {
            match dt {
                Value::Datetime(ms) => {
                    if let Some(dt) = chrono::DateTime::from_timestamp_millis(ms) {
                        Ok(Value::Int(chrono::Timelike::hour(&dt) as i64))
                    } else {
                        Ok(Value::Null)
                    }
                }
                Value::Null => Ok(Value::Null),
                other => Err(ExecuteError::TypeError {
                    expected: "datetime",
                    got: value_type_name(&other).to_string(),
                    op: "time::hour",
                }),
            }
        }

        // time::minute(dt) - extract minute from datetime (0-59)
        fn minute(dt: Any) -> Int {
            match dt {
                Value::Datetime(ms) => {
                    if let Some(dt) = chrono::DateTime::from_timestamp_millis(ms) {
                        Ok(Value::Int(chrono::Timelike::minute(&dt) as i64))
                    } else {
                        Ok(Value::Null)
                    }
                }
                Value::Null => Ok(Value::Null),
                other => Err(ExecuteError::TypeError {
                    expected: "datetime",
                    got: value_type_name(&other).to_string(),
                    op: "time::minute",
                }),
            }
        }

        // time::second(dt) - extract second from datetime (0-59)
        fn second(dt: Any) -> Int {
            match dt {
                Value::Datetime(ms) => {
                    if let Some(dt) = chrono::DateTime::from_timestamp_millis(ms) {
                        Ok(Value::Int(chrono::Timelike::second(&dt) as i64))
                    } else {
                        Ok(Value::Null)
                    }
                }
                Value::Null => Ok(Value::Null),
                other => Err(ExecuteError::TypeError {
                    expected: "datetime",
                    got: value_type_name(&other).to_string(),
                    op: "time::second",
                }),
            }
        }
    }

    // Duration functions - duration manipulation
    namespace duration {
        // duration::from_secs(n) - create duration from seconds
        fn from_secs(n: Any) -> Any {
            match n {
                Value::Int(s) => Ok(Value::from_duration(std::time::Duration::from_secs(s as u64))),
                Value::Null => Ok(Value::Null),
                other => Err(ExecuteError::TypeError {
                    expected: "int",
                    got: value_type_name(&other).to_string(),
                    op: "duration::from_secs",
                }),
            }
        }

        // duration::from_millis(n) - create duration from milliseconds
        fn from_millis(n: Any) -> Any {
            match n {
                Value::Int(ms) => Ok(Value::from_duration(std::time::Duration::from_millis(ms as u64))),
                Value::Null => Ok(Value::Null),
                other => Err(ExecuteError::TypeError {
                    expected: "int",
                    got: value_type_name(&other).to_string(),
                    op: "duration::from_millis",
                }),
            }
        }

        // duration::secs(d) - get total seconds from duration
        fn secs(d: Any) -> Int {
            match d {
                Value::Duration(ns) => {
                    let secs = ns / 1_000_000_000;
                    Ok(Value::Int(secs as i64))
                }
                Value::Null => Ok(Value::Null),
                other => Err(ExecuteError::TypeError {
                    expected: "duration",
                    got: value_type_name(&other).to_string(),
                    op: "duration::secs",
                }),
            }
        }

        // duration::millis(d) - get total milliseconds from duration
        fn millis(d: Any) -> Int {
            match d {
                Value::Duration(ns) => {
                    let ms = ns / 1_000_000;
                    Ok(Value::Int(ms as i64))
                }
                Value::Null => Ok(Value::Null),
                other => Err(ExecuteError::TypeError {
                    expected: "duration",
                    got: value_type_name(&other).to_string(),
                    op: "duration::millis",
                }),
            }
        }
    }

    // Bytes functions - binary data manipulation
    namespace bytes {
        // bytes::len(b) - get byte length
        fn len(b: Any) -> Int {
            match b {
                Value::Bytes(bytes) => Ok(Value::Int(bytes.len() as i64)),
                Value::Null => Ok(Value::Null),
                other => Err(ExecuteError::TypeError {
                    expected: "bytes",
                    got: value_type_name(&other).to_string(),
                    op: "bytes::len",
                }),
            }
        }

        // bytes::base64_encode(b) - encode bytes as base64 string
        fn base64_encode(b: Any) -> String {
            use base64::{Engine, engine::general_purpose::STANDARD};
            match b {
                Value::Bytes(bytes) => Ok(Value::String(STANDARD.encode(&bytes))),
                Value::Null => Ok(Value::Null),
                other => Err(ExecuteError::TypeError {
                    expected: "bytes",
                    got: value_type_name(&other).to_string(),
                    op: "bytes::base64_encode",
                }),
            }
        }

        // bytes::base64_decode(s) - decode base64 string to bytes
        fn base64_decode(s: Any) -> Any {
            use base64::{Engine, engine::general_purpose::STANDARD};
            match s {
                Value::String(str_val) => {
                    let bytes = STANDARD.decode(&str_val).map_err(|e| {
                        ExecuteError::InvalidEncoding(format!("base64: {}", e))
                    })?;
                    Ok(Value::Bytes(bytes))
                }
                Value::Null => Ok(Value::Null),
                other => Err(ExecuteError::TypeError {
                    expected: "string",
                    got: value_type_name(&other).to_string(),
                    op: "bytes::base64_decode",
                }),
            }
        }
    }

    // Search functions - Reciprocal Rank Fusion for hybrid search
    namespace search {
        // Reciprocal Rank Fusion for combining search results
        // search::rrf(lists, limit, k?)
        //
        // RRF_score(doc) = Σ 1 / (k + rank_i(doc))
        // where k = constant (default 60), rank_i = document position in list i
        variadic fn rrf(args) -> Array {
            use std::collections::HashMap;

            // Validate args: first is array of arrays, second is limit, third is optional k
            if args.len() < 2 || args.len() > 3 {
                return Err(ExecuteError::ArityMismatch {
                    function: "search::rrf".to_string(),
                    expected: "2-3".to_string(),
                    got: args.len(),
                });
            }

            let lists = match &args[0] {
                Value::Array(arr) => arr,
                _ => return Err(ExecuteError::TypeError {
                    expected: "Array of arrays",
                    got: value_type_name(&args[0]).to_string(),
                    op: "search::rrf first argument",
                }),
            };

            let limit = match &args[1] {
                Value::Int(n) => *n as usize,
                _ => return Err(ExecuteError::TypeError {
                    expected: "Int",
                    got: value_type_name(&args[1]).to_string(),
                    op: "search::rrf limit",
                }),
            };

            let k = args.get(2)
                .and_then(|v| match v { Value::Int(n) => Some(*n as f64), Value::Float(f) => Some(*f), _ => None })
                .unwrap_or(60.0);

            // First pass: find common fields across all non-empty lists
            use std::collections::HashSet;
            let mut common_fields: Option<HashSet<String>> = None;

            for list in lists.iter() {
                let items = match list {
                    Value::Array(arr) if !arr.is_empty() => arr,
                    _ => continue, // Skip empty or non-array lists
                };

                // Collect fields from this list (union of all items' fields)
                let mut list_fields: HashSet<String> = HashSet::new();
                for item in items.iter() {
                    if let Value::Object(map) = item {
                        for key in map.keys() {
                            list_fields.insert(key.clone());
                        }
                    }
                }

                // Skip if no fields found (shouldn't happen for non-empty list with objects)
                if list_fields.is_empty() {
                    continue;
                }

                // Intersect with common fields
                common_fields = Some(match common_fields {
                    None => list_fields,
                    Some(existing) => existing.intersection(&list_fields).cloned().collect(),
                });
            }

            let common_fields = common_fields.unwrap_or_default();

            // Second pass: build RRF scores and collect docs with only common fields
            let mut scores: HashMap<String, f64> = HashMap::new();
            let mut docs: HashMap<String, HashMap<String, Value>> = HashMap::new();

            for list in lists.iter() {
                let items = match list {
                    Value::Array(arr) => arr,
                    _ => continue,
                };

                for (rank, item) in items.iter().enumerate() {
                    let obj = match item {
                        Value::Object(map) => map,
                        _ => continue,
                    };

                    let id = match obj.get("id") {
                        Some(Value::String(s)) => s.clone(),
                        Some(Value::Reference(s)) => s.clone(),
                        _ => continue,
                    };

                    let rrf_contribution = 1.0 / (k + (rank + 1) as f64);
                    *scores.entry(id.clone()).or_insert(0.0) += rrf_contribution;

                    // Store only common fields (first occurrence wins for values)
                    docs.entry(id).or_insert_with(|| {
                        obj.iter()
                            .filter(|(k, _)| common_fields.contains(*k))
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect()
                    });
                }
            }

            // Sort by RRF score descending
            let mut sorted: Vec<_> = scores.into_iter().collect();
            sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            // Build result with _rrf_score
            let result: Vec<Value> = sorted.into_iter()
                .take(limit)
                .filter_map(|(id, score)| {
                    docs.get(&id).map(|fields| {
                        let mut map = fields.clone();
                        map.insert("_rrf_score".to_string(), Value::Float(score));
                        Value::Object(map)
                    })
                })
                .collect();

            Ok(Value::Array(result))
        }
    }
}

// Helper function for array::contains
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => (x - y).abs() < f64::EPSILON,
        (Value::Int(x), Value::Float(y)) | (Value::Float(y), Value::Int(x)) => {
            ((*x as f64) - y).abs() < f64::EPSILON
        }
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(a, b)| values_equal(a, b))
        }
        _ => false,
    }
}
