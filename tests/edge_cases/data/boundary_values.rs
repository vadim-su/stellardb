// Boundary value edge case tests
// Tests for extreme values, precision, and limits

use crate::common;
use serde_json::json;

// =============================================================================
// Integer Boundary Tests
// =============================================================================

#[tokio::test]
async fn test_i64_max() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION int_bounds (value int)"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": format!(r#"INSERT INTO int_bounds {{"id": "max", "value": {}}}"#, i64::MAX)
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "i64::MAX should be accepted");

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM int_bounds:max"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["value"], i64::MAX);
}

#[tokio::test]
async fn test_i64_min() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION int_min (value int)"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": format!(r#"INSERT INTO int_min {{"id": "min", "value": {}}}"#, i64::MIN)
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "i64::MIN should be accepted");

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM int_min:min"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["value"], i64::MIN);
}

#[tokio::test]
async fn test_zero() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION zeros (int_val int, float_val float)"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO zeros {"id": "z1", "int_val": 0, "float_val": 0.0}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM zeros:z1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["int_val"], 0);
    assert_eq!(body["results"][0]["data"][0]["float_val"], 0.0);
}

// =============================================================================
// Float Boundary Tests
// =============================================================================

#[tokio::test]
async fn test_float_precision() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION float_prec (value float)"}))
        .send()
        .await
        .unwrap();

    // Classic floating point issue: 0.1 + 0.2
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO float_prec {"id": "f1", "value": 0.1}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM float_prec:f1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    // Check that value is approximately 0.1
    let value = body["results"][0]["data"][0]["value"].as_f64().unwrap();
    assert!(
        (value - 0.1).abs() < 0.0001,
        "Value should be close to 0.1: {}",
        value
    );
}

#[tokio::test]
async fn test_very_small_float() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION small_float (value float)"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO small_float {"id": "s1", "value": 0.000000001}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Very small float should be accepted"
    );

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM small_float:s1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let value = body["results"][0]["data"][0]["value"].as_f64().unwrap();
    assert!(
        value > 0.0 && value < 0.00001,
        "Value should be very small: {}",
        value
    );
}

#[tokio::test]
async fn test_very_large_float() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION large_float (value float)"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO large_float {"id": "l1", "value": 1.7976931348623157e308}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // May or may not accept very large floats
    // Document behavior
    if body["error"].is_null() {
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "SELECT * FROM large_float:l1"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        let value = body["results"][0]["data"][0]["value"].as_f64().unwrap();
        assert!(value > 1e307, "Value should be very large");
    }
}

#[tokio::test]
async fn test_negative_float() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION neg_float (value float)"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO neg_float {"id": "n1", "value": -123.456}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM neg_float:n1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let value = body["results"][0]["data"][0]["value"].as_f64().unwrap();
    assert!((value - (-123.456)).abs() < 0.001);
}

// =============================================================================
// String Boundary Tests
// =============================================================================

#[tokio::test]
async fn test_very_long_string_1mb() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION long_str"}))
        .send()
        .await
        .unwrap();

    let long_string = "x".repeat(1_000_000); // 1MB
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": format!(r#"INSERT INTO long_str {{"id": "l1", "content": "{}"}}"#, long_string)
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Document whether 1MB strings are accepted
    if body["error"].is_null() {
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "SELECT * FROM long_str:l1"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        let content = body["results"][0]["data"][0]["content"].as_str().unwrap();
        assert_eq!(content.len(), 1_000_000);
    }
}

#[tokio::test]
async fn test_empty_string() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION empty_str (value string)"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO empty_str {"id": "e1", "value": ""}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM empty_str:e1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["value"], "");
}

// =============================================================================
// Array Boundary Tests
// =============================================================================

#[tokio::test]
async fn test_large_array() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION large_arr"}))
        .send()
        .await
        .unwrap();

    // Array with 1000 elements
    let elements: Vec<i32> = (1..=1000).collect();
    let array_str = format!("{:?}", elements);

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": format!(r#"INSERT INTO large_arr {{"id": "a1", "items": {}}}"#, array_str)
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Large array should be accepted");

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM large_arr:a1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let items = body["results"][0]["data"][0]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1000);
}

#[tokio::test]
async fn test_empty_array() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION empty_arr"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO empty_arr {"id": "e1", "items": []}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM empty_arr:e1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let items = body["results"][0]["data"][0]["items"].as_array().unwrap();
    assert!(items.is_empty());
}

#[tokio::test]
async fn test_nested_arrays() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION nested_arr"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO nested_arr {"id": "n1", "matrix": [[1, 2], [3, 4], [5, 6]]}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Nested arrays should be accepted");

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM nested_arr:n1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let matrix = body["results"][0]["data"][0]["matrix"].as_array().unwrap();
    assert_eq!(matrix.len(), 3);
    assert_eq!(matrix[0].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_array_with_nulls() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION arr_nulls"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO arr_nulls {"id": "n1", "items": [1, null, 3, null, 5]}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Array with nulls should be accepted"
    );

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM arr_nulls:n1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let items = body["results"][0]["data"][0]["items"].as_array().unwrap();
    assert!(items[1].is_null());
    assert!(items[3].is_null());
}

// =============================================================================
// Object Boundary Tests
// =============================================================================

#[tokio::test]
async fn test_many_fields() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION many_fields"}))
        .send()
        .await
        .unwrap();

    // Object with 100 fields
    let mut fields = String::new();
    for i in 1..=100 {
        if i > 1 {
            fields.push_str(", ");
        }
        fields.push_str(&format!(r#""field{}": {}"#, i, i));
    }

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": format!(r#"INSERT INTO many_fields {{"id": "m1", {}}}"#, fields)
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Object with 100 fields should be accepted"
    );

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM many_fields:m1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["field50"], 50);
}

#[tokio::test]
async fn test_deeply_nested_objects() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION deep_nest"}))
        .send()
        .await
        .unwrap();

    // 10 levels deep
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO deep_nest {"id": "d1", "l1": {"l2": {"l3": {"l4": {"l5": {"l6": {"l7": {"l8": {"l9": {"l10": "bottom"}}}}}}}}}}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "10-level nesting should be accepted"
    );

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM deep_nest:d1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        body["results"][0]["data"][0]["l1"]["l2"]["l3"]["l4"]["l5"]["l6"]["l7"]["l8"]["l9"]["l10"],
        "bottom"
    );
}

#[tokio::test]
async fn test_empty_object() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION empty_obj"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO empty_obj {"id": "e1", "data": {}}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Empty nested object should be accepted"
    );

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM empty_obj:e1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let data = &body["results"][0]["data"][0]["data"];
    assert!(data.is_object());
    assert!(data.as_object().unwrap().is_empty());
}

// =============================================================================
// Field Name Boundary Tests
// =============================================================================

#[tokio::test]
async fn test_very_long_field_name() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION long_field"}))
        .send()
        .await
        .unwrap();

    let long_name = "f".repeat(1000);
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": format!(r#"INSERT INTO long_field {{"id": "l1", "{}": "value"}}"#, long_name)
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Document whether very long field names are accepted
    if body["error"].is_null() {
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "SELECT * FROM long_field:l1"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(body["results"][0]["data"][0][&long_name], "value");
    }
}

#[tokio::test]
async fn test_single_char_field_name() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION short_field"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO short_field {"id": "s1", "x": 1, "y": 2, "z": 3}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM short_field:s1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["x"], 1);
}

#[tokio::test]
async fn test_numeric_string_field_name() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION num_field"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO num_field {"id": "n1", "123": "numeric field name"}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Document whether numeric field names are accepted
    if body["error"].is_null() {
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "SELECT * FROM num_field:n1"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(body["results"][0]["data"][0]["123"], "numeric field name");
    }
}
