// Expression error integration tests
// Tests for division by zero and type errors in expressions

use crate::common;
use serde_json::json;

// =============================================================================
// Division By Zero
// =============================================================================

#[tokio::test]
async fn test_division_by_zero() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION nums (x int)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO nums {"id": "n1", "x": 42}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT x / 0 FROM nums"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        !body["error"].is_null(),
        "Division by zero should produce an error, got: {:?}",
        body
    );
}

// =============================================================================
// Type Error: String + Int
// =============================================================================

#[tokio::test]
async fn test_type_error_string_add() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION people (name string)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO people {"id": "p1", "name": "Alice"}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT name + 1 FROM people"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        !body["error"].is_null(),
        "Adding string + int should produce a type error, got: {:?}",
        body
    );
}

// =============================================================================
// Float Division By Zero
// =============================================================================

#[tokio::test]
async fn test_float_divided_by_float_zero() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION fdiv"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"INSERT INTO fdiv {"id": "f1", "x": 1.0}"#}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT x / 0.0 FROM fdiv"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        !body["error"].is_null(),
        "1.0 / 0.0 should produce an error, got: {:?}",
        body
    );
}

#[tokio::test]
async fn test_int_divided_by_float_zero() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION fdiv2"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"INSERT INTO fdiv2 {"id": "f1", "x": 1}"#}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT x / 0.0 FROM fdiv2"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        !body["error"].is_null(),
        "1 / 0.0 should produce an error, got: {:?}",
        body
    );
}

#[tokio::test]
async fn test_float_divided_by_int_zero() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION fdiv3"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"INSERT INTO fdiv3 {"id": "f1", "x": 1.0}"#}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT x / 0 FROM fdiv3"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        !body["error"].is_null(),
        "1.0 / 0 should produce an error, got: {:?}",
        body
    );
}

// =============================================================================
// Modulo By Zero (via math::modulo function)
// =============================================================================

#[tokio::test]
async fn test_int_modulo_int_zero() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION modt"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"INSERT INTO modt {"id": "m1", "x": 10}"#}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT math::modulo(x, 0) FROM modt"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        !body["error"].is_null(),
        "10 % 0 should produce an error, got: {:?}",
        body
    );
}

#[tokio::test]
async fn test_float_modulo_float_zero() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION modt2"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"INSERT INTO modt2 {"id": "m1", "x": 10.0}"#}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT math::modulo(x, 0.0) FROM modt2"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        !body["error"].is_null(),
        "10.0 % 0.0 should produce an error, got: {:?}",
        body
    );
}

#[tokio::test]
async fn test_int_modulo_float_zero() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION modt3"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"INSERT INTO modt3 {"id": "m1", "x": 10}"#}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT math::modulo(x, 0.0) FROM modt3"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        !body["error"].is_null(),
        "10 % 0.0 should produce an error, got: {:?}",
        body
    );
}

#[tokio::test]
async fn test_float_modulo_int_zero() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION modt4"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"INSERT INTO modt4 {"id": "m1", "x": 10.0}"#}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT math::modulo(x, 0) FROM modt4"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        !body["error"].is_null(),
        "10.0 % 0 should produce an error, got: {:?}",
        body
    );
}
