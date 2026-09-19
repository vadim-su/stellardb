//! Tests for expression evaluation.

use super::*;
use crate::document::Document;
use crate::query::ast::BinaryOp;
use std::collections::HashMap;

// Re-export for testing internal binary functions
pub(super) use super::binary::eval_binary;

use crate::query::execute::context::NullContext;

fn make_doc(id: &str, fields: Vec<(&str, Value)>) -> Document {
    Document {
        id: id.to_string(),
        fields: fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    }
}

fn empty_doc() -> Document {
    Document {
        id: "test:1".to_string(),
        fields: HashMap::new(),
    }
}

#[test]
fn aggregate_function_with_expression_is_typed_invalid_operation() {
    let row = row_from_doc(&empty_doc());

    for name in ["COUNT", "SUM", "AVG", "MIN", "MAX"] {
        let expr = Expr::FunctionCall {
            name: name.to_string(),
            namespace: None,
            args: vec![Expr::Literal(Value::Array(vec![Value::Int(1)]))],
        };
        let error = eval_expr(&expr, &row, None, None, &NullContext).unwrap_err();
        assert!(matches!(error, ExecuteError::InvalidOperation(_)));
    }
}

#[test]
fn test_literal() {
    let doc = empty_doc();
    let expr = Expr::Literal(Value::Int(42));
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Int(42)
    );
}

#[test]
fn test_field_lookup() {
    let doc = make_doc("user:alice", vec![("name", Value::String("Alice".into()))]);
    let expr = Expr::Field("name".to_string());
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::String("Alice".into())
    );
}

#[test]
fn test_field_id() {
    let doc = make_doc("user:alice", vec![]);
    let expr = Expr::Field("id".to_string());
    // id returns Reference for FETCH support
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Reference("user:alice".into())
    );
}

#[test]
fn test_missing_field_is_null() {
    let doc = empty_doc();
    let expr = Expr::Field("missing".to_string());
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Null
    );
}

#[test]
fn test_arithmetic_int() {
    let doc = empty_doc();
    let expr = Expr::BinaryOp {
        left: Box::new(Expr::Literal(Value::Int(10))),
        op: BinaryOp::Add,
        right: Box::new(Expr::Literal(Value::Int(5))),
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Int(15)
    );
}

#[test]
fn test_arithmetic_float_coercion() {
    let doc = empty_doc();
    let expr = Expr::BinaryOp {
        left: Box::new(Expr::Literal(Value::Int(10))),
        op: BinaryOp::Mul,
        right: Box::new(Expr::Literal(Value::Float(2.5))),
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Float(25.0)
    );
}

#[test]
fn test_division_by_zero() {
    let doc = empty_doc();
    let expr = Expr::BinaryOp {
        left: Box::new(Expr::Literal(Value::Int(10))),
        op: BinaryOp::Div,
        right: Box::new(Expr::Literal(Value::Int(0))),
    };
    assert!(matches!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext),
        Err(ExecuteError::DivisionByZero)
    ));
}

#[test]
fn test_null_propagation() {
    let doc = empty_doc();
    let expr = Expr::BinaryOp {
        left: Box::new(Expr::Literal(Value::Int(10))),
        op: BinaryOp::Add,
        right: Box::new(Expr::Literal(Value::Null)),
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Null
    );
}

#[test]
fn test_three_valued_and() {
    // NULL AND false -> false
    assert_eq!(
        eval_binary(BinaryOp::And, &Value::Null, &Value::Bool(false)).unwrap(),
        Value::Bool(false)
    );
    // NULL AND true -> NULL
    assert_eq!(
        eval_binary(BinaryOp::And, &Value::Null, &Value::Bool(true)).unwrap(),
        Value::Null
    );
}

#[test]
fn test_three_valued_or() {
    // NULL OR true -> true
    assert_eq!(
        eval_binary(BinaryOp::Or, &Value::Null, &Value::Bool(true)).unwrap(),
        Value::Bool(true)
    );
    // NULL OR false -> NULL
    assert_eq!(
        eval_binary(BinaryOp::Or, &Value::Null, &Value::Bool(false)).unwrap(),
        Value::Null
    );
}

#[test]
fn test_comparison() {
    let doc = empty_doc();
    let expr = Expr::BinaryOp {
        left: Box::new(Expr::Literal(Value::Int(10))),
        op: BinaryOp::Gt,
        right: Box::new(Expr::Literal(Value::Int(5))),
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn test_upper_function() {
    let doc = empty_doc();
    let expr = Expr::FunctionCall {
        name: "upper".to_string(),
        namespace: Some("string".to_string()),
        args: vec![Expr::Literal(Value::String("hello".into()))],
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::String("HELLO".into())
    );
}

#[test]
fn test_lower_function() {
    let doc = empty_doc();
    let expr = Expr::FunctionCall {
        name: "lower".to_string(),
        namespace: Some("string".to_string()),
        args: vec![Expr::Literal(Value::String("HELLO".into()))],
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::String("hello".into())
    );
}

#[test]
fn test_concat_function() {
    let doc = empty_doc();
    let expr = Expr::FunctionCall {
        name: "concat".to_string(),
        namespace: Some("string".to_string()),
        args: vec![
            Expr::Literal(Value::String("hello".into())),
            Expr::Literal(Value::Null),
            Expr::Literal(Value::Int(42)),
        ],
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::String("hello42".into())
    );
}

#[test]
fn test_upper_null() {
    let doc = empty_doc();
    let expr = Expr::FunctionCall {
        name: "upper".to_string(),
        namespace: Some("string".to_string()),
        args: vec![Expr::Literal(Value::Null)],
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Null
    );
}

#[test]
fn test_in_list() {
    let doc = empty_doc();
    let expr = Expr::InList {
        expr: Box::new(Expr::Literal(Value::Int(2))),
        list: vec![
            Expr::Literal(Value::Int(1)),
            Expr::Literal(Value::Int(2)),
            Expr::Literal(Value::Int(3)),
        ],
        negated: false,
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn test_not_in_list() {
    let doc = empty_doc();
    let expr = Expr::InList {
        expr: Box::new(Expr::Literal(Value::Int(5))),
        list: vec![Expr::Literal(Value::Int(1)), Expr::Literal(Value::Int(2))],
        negated: true,
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn test_eval_filter_true() {
    let doc = make_doc("test:1", vec![("age", Value::Int(30))]);
    let expr = Expr::BinaryOp {
        left: Box::new(Expr::Field("age".to_string())),
        op: BinaryOp::Gt,
        right: Box::new(Expr::Literal(Value::Int(25))),
    };
    assert!(eval_filter(&expr, &row_from_doc(&doc), &NullContext).unwrap());
}

#[test]
fn test_eval_filter_null_is_false() {
    let doc = empty_doc();
    let expr = Expr::BinaryOp {
        left: Box::new(Expr::Field("missing".to_string())),
        op: BinaryOp::Eq,
        right: Box::new(Expr::Literal(Value::Int(1))),
    };
    // NULL = 1 -> NULL, filter treats as false
    assert!(!eval_filter(&expr, &row_from_doc(&doc), &NullContext).unwrap());
}

#[test]
fn test_unary_not() {
    let doc = empty_doc();
    let expr = Expr::UnaryOp {
        op: UnaryOp::Not,
        expr: Box::new(Expr::Literal(Value::Bool(true))),
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn test_unary_neg() {
    let doc = empty_doc();
    let expr = Expr::UnaryOp {
        op: UnaryOp::Neg,
        expr: Box::new(Expr::Literal(Value::Int(42))),
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Int(-42)
    );
}

// --- Additional comprehensive tests ---

fn binop(left: Expr, op: BinaryOp, right: Expr) -> Expr {
    Expr::BinaryOp {
        left: Box::new(left),
        op,
        right: Box::new(right),
    }
}

#[test]
fn test_float_mul() {
    let doc = empty_doc();
    let expr = binop(
        Expr::Literal(Value::Float(2.5)),
        BinaryOp::Mul,
        Expr::Literal(Value::Float(4.0)),
    );
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Float(10.0)
    );
}

#[test]
fn test_mixed_int_float_add() {
    let doc = empty_doc();
    let expr = binop(
        Expr::Literal(Value::Int(3)),
        BinaryOp::Add,
        Expr::Literal(Value::Float(0.14)),
    );
    match eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap() {
        #[allow(clippy::approx_constant)]
        Value::Float(f) => assert!((f - 3.14).abs() < f64::EPSILON),
        other => panic!("expected Float, got {:?}", other),
    }
}

#[test]
fn test_string_concat() {
    let doc = empty_doc();
    let expr = binop(
        Expr::Literal(Value::String("hello".into())),
        BinaryOp::Add,
        Expr::Literal(Value::String(" world".into())),
    );
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::String("hello world".into())
    );
}

#[test]
fn test_array_concat() {
    let doc = empty_doc();
    let expr = binop(
        Expr::Literal(Value::Array(vec![Value::Int(1), Value::Int(2)])),
        BinaryOp::Add,
        Expr::Literal(Value::Array(vec![Value::Int(3), Value::Int(4)])),
    );
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Array(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
            Value::Int(4),
        ])
    );
}

#[test]
fn test_eval_index_positive() {
    let doc = make_doc(
        "test:1",
        vec![(
            "arr",
            Value::Array(vec![Value::Int(10), Value::Int(20), Value::Int(30)]),
        )],
    );
    let expr = Expr::Index {
        base: Box::new(Expr::Field("arr".into())),
        index: Box::new(Expr::Literal(Value::Int(1))),
    };
    let result = eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap();
    assert_eq!(result, Value::Int(20));
}

#[test]
fn test_eval_index_negative() {
    let doc = make_doc(
        "test:1",
        vec![(
            "arr",
            Value::Array(vec![Value::Int(10), Value::Int(20), Value::Int(30)]),
        )],
    );
    let expr = Expr::Index {
        base: Box::new(Expr::Field("arr".into())),
        index: Box::new(Expr::Literal(Value::Int(-1))),
    };
    let result = eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap();
    assert_eq!(result, Value::Int(30));
}

#[test]
fn test_eval_slice_basic() {
    let doc = make_doc(
        "test:1",
        vec![(
            "arr",
            Value::Array(vec![
                Value::Int(10),
                Value::Int(20),
                Value::Int(30),
                Value::Int(40),
            ]),
        )],
    );
    let expr = Expr::Slice {
        base: Box::new(Expr::Field("arr".into())),
        start: Some(Box::new(Expr::Literal(Value::Int(1)))),
        end: Some(Box::new(Expr::Literal(Value::Int(2)))),
    };
    let result = eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap();
    assert_eq!(result, Value::Array(vec![Value::Int(20), Value::Int(30)]));
}

#[test]
fn test_eval_field_access() {
    let mut inner = HashMap::new();
    inner.insert("name".to_string(), Value::String("Alice".into()));
    let doc = make_doc("test:1", vec![("user", Value::Object(inner))]);
    let expr = Expr::FieldAccess {
        base: Box::new(Expr::Field("user".into())),
        field: "name".to_string(),
    };
    let result = eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap();
    assert_eq!(result, Value::String("Alice".into()));
}

#[test]
fn test_is_null_present_null_value() {
    let doc = make_doc("test:1", vec![("x", Value::Null)]);
    let expr = Expr::UnaryOp {
        op: UnaryOp::IsNull,
        expr: Box::new(Expr::Field("x".to_string())),
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn test_is_none_missing_field() {
    let doc = empty_doc();
    let expr = Expr::UnaryOp {
        op: UnaryOp::IsNone,
        expr: Box::new(Expr::Field("x".to_string())),
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn test_is_none_present_null_is_false() {
    // Key distinction: field present with null value → IS NONE = false
    let doc = make_doc("test:1", vec![("x", Value::Null)]);
    let expr = Expr::UnaryOp {
        op: UnaryOp::IsNone,
        expr: Box::new(Expr::Field("x".to_string())),
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn test_eval_array_literals() {
    let doc = empty_doc();
    let expr = Expr::Array(vec![
        Expr::Literal(Value::Int(1)),
        Expr::Literal(Value::String("two".into())),
        Expr::Literal(Value::Bool(true)),
    ]);
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Array(vec![
            Value::Int(1),
            Value::String("two".into()),
            Value::Bool(true),
        ])
    );
}

#[test]
fn test_eval_object_literals() {
    let doc = empty_doc();
    let expr = Expr::Object(vec![
        ("key".into(), Expr::Literal(Value::String("value".into()))),
        ("num".into(), Expr::Literal(Value::Int(42))),
    ]);
    let result = eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap();
    match result {
        Value::Object(map) => {
            assert_eq!(map.get("key"), Some(&Value::String("value".into())));
            assert_eq!(map.get("num"), Some(&Value::Int(42)));
        }
        _ => panic!("expected Object"),
    }
}

#[test]
fn test_math_abs_int() {
    let doc = empty_doc();
    let expr = Expr::FunctionCall {
        name: "abs".to_string(),
        namespace: Some("math".to_string()),
        args: vec![Expr::Literal(Value::Int(-42))],
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Int(42)
    );
}

#[test]
fn test_math_sqrt_int() {
    let doc = empty_doc();
    let expr = Expr::FunctionCall {
        name: "sqrt".to_string(),
        namespace: Some("math".to_string()),
        args: vec![Expr::Literal(Value::Int(16))],
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Float(4.0)
    );
}

#[test]
fn test_array_length() {
    let doc = empty_doc();
    let expr = Expr::FunctionCall {
        name: "length".to_string(),
        namespace: Some("array".to_string()),
        args: vec![Expr::Literal(Value::Array(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
        ]))],
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Int(3)
    );
}

#[test]
fn test_math_pi() {
    let doc = empty_doc();
    // pi is now a constant, not a function
    let expr = Expr::Constant {
        namespace: "math".to_string(),
        name: "pi".to_string(),
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Float(std::f64::consts::PI)
    );
}

#[test]
fn test_math_ceil_null() {
    let doc = empty_doc();
    let expr = Expr::FunctionCall {
        name: "ceil".to_string(),
        namespace: Some("math".to_string()),
        args: vec![Expr::Literal(Value::Null)],
    };
    assert_eq!(
        eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap(),
        Value::Null
    );
}

// === Array indexing with float indices ===

#[test]
fn test_eval_index_float_whole_number() {
    let doc = make_doc(
        "test:1",
        vec![(
            "arr",
            Value::Array(vec![Value::Int(10), Value::Int(20), Value::Int(30)]),
        )],
    );
    // Index with float that has no fractional part (1.0 -> index 1)
    let expr = Expr::Index {
        base: Box::new(Expr::Field("arr".into())),
        index: Box::new(Expr::Literal(Value::Float(1.0))),
    };
    let result = eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap();
    assert_eq!(result, Value::Int(20));
}

#[test]
fn test_eval_index_float_negative_whole_number() {
    let doc = make_doc(
        "test:1",
        vec![(
            "arr",
            Value::Array(vec![Value::Int(10), Value::Int(20), Value::Int(30)]),
        )],
    );
    // Negative float index (-1.0 -> last element)
    let expr = Expr::Index {
        base: Box::new(Expr::Field("arr".into())),
        index: Box::new(Expr::Literal(Value::Float(-1.0))),
    };
    let result = eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap();
    assert_eq!(result, Value::Int(30));
}

#[test]
fn test_eval_index_float_with_fraction_error() {
    let doc = make_doc(
        "test:1",
        vec![(
            "arr",
            Value::Array(vec![Value::Int(10), Value::Int(20), Value::Int(30)]),
        )],
    );
    // Float with fractional part should error
    let expr = Expr::Index {
        base: Box::new(Expr::Field("arr".into())),
        index: Box::new(Expr::Literal(Value::Float(1.5))),
    };
    let result = eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext);
    assert!(result.is_err());
}

#[test]
fn test_eval_index_out_of_bounds() {
    let doc = make_doc(
        "test:1",
        vec![("arr", Value::Array(vec![Value::Int(10), Value::Int(20)]))],
    );
    // Index beyond array length returns Null
    let expr = Expr::Index {
        base: Box::new(Expr::Field("arr".into())),
        index: Box::new(Expr::Literal(Value::Int(5))),
    };
    let result = eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap();
    assert_eq!(result, Value::Null);
}

#[test]
fn test_eval_index_negative_out_of_bounds() {
    let doc = make_doc(
        "test:1",
        vec![("arr", Value::Array(vec![Value::Int(10), Value::Int(20)]))],
    );
    // Negative index beyond array start returns Null
    let expr = Expr::Index {
        base: Box::new(Expr::Field("arr".into())),
        index: Box::new(Expr::Literal(Value::Int(-5))),
    };
    let result = eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap();
    assert_eq!(result, Value::Null);
}

#[test]
fn test_eval_index_null_propagation_base() {
    let doc = empty_doc();
    // Indexing null returns null
    let expr = Expr::Index {
        base: Box::new(Expr::Literal(Value::Null)),
        index: Box::new(Expr::Literal(Value::Int(0))),
    };
    let result = eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap();
    assert_eq!(result, Value::Null);
}

#[test]
fn test_eval_index_null_propagation_index() {
    let doc = make_doc(
        "test:1",
        vec![("arr", Value::Array(vec![Value::Int(10), Value::Int(20)]))],
    );
    // Null index returns null
    let expr = Expr::Index {
        base: Box::new(Expr::Field("arr".into())),
        index: Box::new(Expr::Literal(Value::Null)),
    };
    let result = eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap();
    assert_eq!(result, Value::Null);
}

#[test]
fn test_eval_index_array_of_objects() {
    // Simulate subquery result: array of objects
    let mut obj1 = HashMap::new();
    obj1.insert("name".to_string(), Value::String("Alice".into()));
    let mut obj2 = HashMap::new();
    obj2.insert("name".to_string(), Value::String("Bob".into()));

    let doc = make_doc(
        "test:1",
        vec![(
            "results",
            Value::Array(vec![Value::Object(obj1), Value::Object(obj2)]),
        )],
    );

    // Get first result
    let expr = Expr::Index {
        base: Box::new(Expr::Field("results".into())),
        index: Box::new(Expr::Literal(Value::Int(0))),
    };
    let result = eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap();

    match result {
        Value::Object(map) => {
            assert_eq!(map.get("name"), Some(&Value::String("Alice".into())));
        }
        _ => panic!("Expected Object, got {:?}", result),
    }
}

#[test]
fn test_eval_index_chained_with_field_access() {
    // Test: results[0].name (common subquery pattern)
    let mut obj1 = HashMap::new();
    obj1.insert("name".to_string(), Value::String("Alice".into()));
    let mut obj2 = HashMap::new();
    obj2.insert("name".to_string(), Value::String("Bob".into()));

    let doc = make_doc(
        "test:1",
        vec![(
            "results",
            Value::Array(vec![Value::Object(obj1), Value::Object(obj2)]),
        )],
    );

    // results[0].name
    let expr = Expr::FieldAccess {
        base: Box::new(Expr::Index {
            base: Box::new(Expr::Field("results".into())),
            index: Box::new(Expr::Literal(Value::Int(0))),
        }),
        field: "name".to_string(),
    };
    let result = eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap();
    assert_eq!(result, Value::String("Alice".into()));
}

#[test]
fn test_eval_index_last_element() {
    // Test: results[-1] to get last element
    let mut obj1 = HashMap::new();
    obj1.insert("name".to_string(), Value::String("First".into()));
    let mut obj2 = HashMap::new();
    obj2.insert("name".to_string(), Value::String("Last".into()));

    let doc = make_doc(
        "test:1",
        vec![(
            "results",
            Value::Array(vec![Value::Object(obj1), Value::Object(obj2)]),
        )],
    );

    // results[-1]
    let expr = Expr::Index {
        base: Box::new(Expr::Field("results".into())),
        index: Box::new(Expr::Literal(Value::Int(-1))),
    };
    let result = eval_expr(&expr, &row_from_doc(&doc), None, None, &NullContext).unwrap();

    match result {
        Value::Object(map) => {
            assert_eq!(map.get("name"), Some(&Value::String("Last".into())));
        }
        _ => panic!("Expected Object, got {:?}", result),
    }
}
