//! Binary operation evaluation for expressions.

use crate::document::{DecimalValue, Value};
use crate::query::ast::BinaryOp;
use crate::query::error::ExecuteError;

use super::compare::{value_repr, values_compare, values_equal};

use rust_decimal::Decimal;
use std::cmp::Ordering;

/// Evaluate a binary operation on two values.
pub(crate) fn eval_binary(op: BinaryOp, l: &Value, r: &Value) -> Result<Value, ExecuteError> {
    match op {
        BinaryOp::And => eval_and(l, r),
        BinaryOp::Or => eval_or(l, r),
        BinaryOp::Coalesce => unreachable!("Coalesce handled in eval_expr"),

        // Null propagation for all other ops
        _ if matches!(l, Value::Null) || matches!(r, Value::Null) => Ok(Value::Null),

        BinaryOp::Add => eval_add(l, r),
        BinaryOp::Sub => eval_sub(l, r),
        BinaryOp::Mul => eval_arithmetic(
            l,
            r,
            i64::checked_mul,
            |a, b| a * b,
            |a, b| a.checked_mul(b),
            "multiplication",
        ),
        BinaryOp::Div => eval_div(l, r),

        BinaryOp::Eq => Ok(Value::Bool(values_equal(l, r))),
        BinaryOp::Ne => Ok(Value::Bool(!values_equal(l, r))),

        BinaryOp::Gt => Ok(Value::Bool(
            values_compare(l, r).is_some_and(|o| o == Ordering::Greater),
        )),
        BinaryOp::Gte => Ok(Value::Bool(
            values_compare(l, r).is_some_and(|o| o != Ordering::Less),
        )),
        BinaryOp::Lt => Ok(Value::Bool(
            values_compare(l, r).is_some_and(|o| o == Ordering::Less),
        )),
        BinaryOp::Lte => Ok(Value::Bool(
            values_compare(l, r).is_some_and(|o| o != Ordering::Greater),
        )),
    }
}

/// SQL three-valued AND: NULL AND false -> false, NULL AND true -> NULL
fn eval_and(l: &Value, r: &Value) -> Result<Value, ExecuteError> {
    match (l, r) {
        (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(*a && *b)),
        (Value::Bool(false), Value::Null) | (Value::Null, Value::Bool(false)) => {
            Ok(Value::Bool(false))
        }
        (Value::Bool(true), Value::Null) | (Value::Null, Value::Bool(true)) => Ok(Value::Null),
        (Value::Null, Value::Null) => Ok(Value::Null),
        _ => Err(ExecuteError::TypeError {
            expected: "Bool",
            got: if !matches!(l, Value::Bool(_) | Value::Null) {
                value_repr(l)
            } else {
                value_repr(r)
            },
            op: "AND",
        }),
    }
}

/// SQL three-valued OR: NULL OR true -> true, NULL OR false -> NULL
fn eval_or(l: &Value, r: &Value) -> Result<Value, ExecuteError> {
    match (l, r) {
        (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(*a || *b)),
        (Value::Bool(true), Value::Null) | (Value::Null, Value::Bool(true)) => {
            Ok(Value::Bool(true))
        }
        (Value::Bool(false), Value::Null) | (Value::Null, Value::Bool(false)) => Ok(Value::Null),
        (Value::Null, Value::Null) => Ok(Value::Null),
        _ => Err(ExecuteError::TypeError {
            expected: "Bool",
            got: if !matches!(l, Value::Bool(_) | Value::Null) {
                value_repr(l)
            } else {
                value_repr(r)
            },
            op: "OR",
        }),
    }
}

/// Addition: String + String -> concat, Array + Array -> concat,
/// Datetime + Duration -> Datetime, Duration + Duration -> Duration, numeric -> arithmetic
fn eval_add(l: &Value, r: &Value) -> Result<Value, ExecuteError> {
    match (l, r) {
        (Value::String(a), Value::String(b)) => {
            let mut result = String::with_capacity(a.len() + b.len());
            result.push_str(a);
            result.push_str(b);
            Ok(Value::String(result))
        }
        (Value::Array(a), Value::Array(b)) => {
            let mut result = Vec::with_capacity(a.len() + b.len());
            result.extend(a.iter().cloned());
            result.extend(b.iter().cloned());
            Ok(Value::Array(result))
        }
        // Datetime + Duration -> Datetime
        (Value::Datetime(ms), Value::Duration(ns)) => {
            let duration_ms = (*ns / 1_000_000) as i64;
            Ok(Value::Datetime(ms.saturating_add(duration_ms)))
        }
        (Value::Duration(ns), Value::Datetime(ms)) => {
            let duration_ms = (*ns / 1_000_000) as i64;
            Ok(Value::Datetime(ms.saturating_add(duration_ms)))
        }
        // Duration + Duration -> Duration
        (Value::Duration(a), Value::Duration(b)) => Ok(Value::Duration(a.saturating_add(*b))),
        _ => eval_arithmetic(
            l,
            r,
            i64::checked_add,
            |a, b| a + b,
            |a, b| a.checked_add(b),
            "addition",
        ),
    }
}

/// Arithmetic with type coercion:
/// Int op Int -> Int, Float op Float -> Float, Int op Float -> Float,
/// Decimal op any numeric -> Decimal
fn eval_arithmetic(
    l: &Value,
    r: &Value,
    int_op: fn(i64, i64) -> Option<i64>,
    float_op: fn(f64, f64) -> f64,
    decimal_op: fn(Decimal, Decimal) -> Option<Decimal>,
    op_name: &'static str,
) -> Result<Value, ExecuteError> {
    match (l, r) {
        (Value::Int(a), Value::Int(b)) => int_op(*a, *b)
            .map(Value::Int)
            .ok_or(ExecuteError::Overflow(op_name)),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(float_op(*a, *b))),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(float_op(*a as f64, *b))),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(float_op(*a, *b as f64))),
        // Decimal coercion: if either operand is Decimal, promote to Decimal
        (Value::Decimal(a), Value::Decimal(b)) => {
            decimal_result(decimal_op, a.as_decimal(), b.as_decimal(), op_name)
        }
        (Value::Decimal(a), Value::Int(b)) => {
            decimal_result(decimal_op, a.as_decimal(), Decimal::from(*b), op_name)
        }
        (Value::Int(a), Value::Decimal(b)) => {
            decimal_result(decimal_op, Decimal::from(*a), b.as_decimal(), op_name)
        }
        (Value::Decimal(a), Value::Float(b)) => {
            let db = f64_to_decimal(*b, op_name)?;
            decimal_result(decimal_op, a.as_decimal(), db, op_name)
        }
        (Value::Float(a), Value::Decimal(b)) => {
            let da = f64_to_decimal(*a, op_name)?;
            decimal_result(decimal_op, da, b.as_decimal(), op_name)
        }
        _ => Err(ExecuteError::TypeError {
            expected: "numeric (Int, Float, or Decimal)",
            got: if !matches!(l, Value::Int(_) | Value::Float(_) | Value::Decimal(_)) {
                value_repr(l)
            } else {
                value_repr(r)
            },
            op: op_name,
        }),
    }
}

fn decimal_result(
    op: fn(Decimal, Decimal) -> Option<Decimal>,
    a: Decimal,
    b: Decimal,
    op_name: &'static str,
) -> Result<Value, ExecuteError> {
    op(a, b)
        .map(|d| Value::Decimal(DecimalValue::new(d)))
        .ok_or(ExecuteError::Overflow(op_name))
}

fn f64_to_decimal(f: f64, op_name: &'static str) -> Result<Decimal, ExecuteError> {
    Decimal::try_from(f).map_err(|_| ExecuteError::Overflow(op_name))
}

/// Subtraction: Datetime - Duration -> Datetime, Datetime - Datetime -> Duration,
/// Duration - Duration -> Duration, numeric -> arithmetic
fn eval_sub(l: &Value, r: &Value) -> Result<Value, ExecuteError> {
    match (l, r) {
        // Datetime - Duration -> Datetime
        (Value::Datetime(ms), Value::Duration(ns)) => {
            let duration_ms = (*ns / 1_000_000) as i64;
            Ok(Value::Datetime(ms.saturating_sub(duration_ms)))
        }
        // Datetime - Datetime -> Duration (absolute difference)
        (Value::Datetime(a), Value::Datetime(b)) => {
            let diff_ms = a.saturating_sub(*b);
            // Convert ms to ns, handle negative by taking absolute
            let diff_ns = diff_ms.unsigned_abs().saturating_mul(1_000_000);
            Ok(Value::Duration(diff_ns))
        }
        // Duration - Duration -> Duration (saturating at 0)
        (Value::Duration(a), Value::Duration(b)) => Ok(Value::Duration(a.saturating_sub(*b))),
        _ => eval_arithmetic(
            l,
            r,
            i64::checked_sub,
            |a, b| a - b,
            |a, b| a.checked_sub(b),
            "subtraction",
        ),
    }
}

/// Division with zero check
fn eval_div(l: &Value, r: &Value) -> Result<Value, ExecuteError> {
    match (l, r) {
        (Value::Int(_), Value::Int(0)) => Err(ExecuteError::DivisionByZero),
        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a / b)),
        (Value::Float(_), Value::Float(b)) if *b == 0.0 => Err(ExecuteError::DivisionByZero),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
        (Value::Int(_), Value::Float(b)) if *b == 0.0 => Err(ExecuteError::DivisionByZero),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 / b)),
        (Value::Float(_), Value::Int(0)) => Err(ExecuteError::DivisionByZero),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a / *b as f64)),
        // Decimal division with zero check
        (Value::Decimal(a), Value::Decimal(b)) => {
            let db = b.as_decimal();
            if db == Decimal::ZERO {
                return Err(ExecuteError::DivisionByZero);
            }
            decimal_result(|a, b| a.checked_div(b), a.as_decimal(), db, "division")
        }
        (Value::Decimal(a), Value::Int(b)) => {
            if *b == 0 {
                return Err(ExecuteError::DivisionByZero);
            }
            decimal_result(
                |a, b| a.checked_div(b),
                a.as_decimal(),
                Decimal::from(*b),
                "division",
            )
        }
        (Value::Int(a), Value::Decimal(b)) => {
            let db = b.as_decimal();
            if db == Decimal::ZERO {
                return Err(ExecuteError::DivisionByZero);
            }
            decimal_result(|a, b| a.checked_div(b), Decimal::from(*a), db, "division")
        }
        (Value::Decimal(a), Value::Float(b)) => {
            let db = f64_to_decimal(*b, "division")?;
            if db == Decimal::ZERO {
                return Err(ExecuteError::DivisionByZero);
            }
            decimal_result(|a, b| a.checked_div(b), a.as_decimal(), db, "division")
        }
        (Value::Float(a), Value::Decimal(b)) => {
            let db = b.as_decimal();
            if db == Decimal::ZERO {
                return Err(ExecuteError::DivisionByZero);
            }
            let da = f64_to_decimal(*a, "division")?;
            decimal_result(|a, b| a.checked_div(b), da, db, "division")
        }
        _ => Err(ExecuteError::TypeError {
            expected: "numeric (Int, Float, or Decimal)",
            got: if !matches!(l, Value::Int(_) | Value::Float(_) | Value::Decimal(_)) {
                value_repr(l)
            } else {
                value_repr(r)
            },
            op: "division",
        }),
    }
}
