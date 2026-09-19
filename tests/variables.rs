// Integration tests for variables, constants, and LET statements
// Tests the full query pipeline for the new typed function system

mod common;

use serde_json::json;

// =============================================================================
// Constants without parentheses
// =============================================================================

#[tokio::test]
async fn test_constant_without_parens_pi() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION test"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"INSERT INTO test {"id": "t1"}"#}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT math::pi AS pi FROM test"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    let pi = body["results"][0]["data"][0]["pi"].as_f64().unwrap();
    assert!((pi - std::f64::consts::PI).abs() < 0.0001);
}

#[tokio::test]
async fn test_constant_without_parens_e() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION test"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"INSERT INTO test {"id": "t1"}"#}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT math::e AS e FROM test"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    let e = body["results"][0]["data"][0]["e"].as_f64().unwrap();
    assert!((e - std::f64::consts::E).abs() < 0.0001);
}

#[tokio::test]
async fn test_constant_without_parens_tau() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION test"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"INSERT INTO test {"id": "t1"}"#}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT math::tau AS tau FROM test"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    let tau = body["results"][0]["data"][0]["tau"].as_f64().unwrap();
    assert!((tau - std::f64::consts::TAU).abs() < 0.0001);
}

#[tokio::test]
async fn test_constant_in_expression() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION circles"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"INSERT INTO circles {"id": "c1", "radius": 5}"#}))
        .send()
        .await
        .unwrap();

    // Calculate circle area: pi * r^2
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT math::pi * radius * radius AS area FROM circles"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    let area = body["results"][0]["data"][0]["area"].as_f64().unwrap();
    let expected = std::f64::consts::PI * 25.0;
    assert!((area - expected).abs() < 0.0001);
}

// =============================================================================
// Constant with parentheses should error
// =============================================================================

#[tokio::test]
async fn test_constant_with_parens_error() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION test"}))
        .send()
        .await
        .unwrap();

    // math::pi() should fail - constants don't use parens
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT math::pi() AS pi FROM test"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Should have an error about unknown function
    let has_error = !body["error"].is_null() || !body["results"][0]["error"].is_null();
    assert!(has_error, "Expected error for math::pi(), got: {:?}", body);
}

// =============================================================================
// Namespaced function calls
// =============================================================================

#[tokio::test]
async fn test_namespaced_function_call() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION test"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"INSERT INTO test {"id": "t1", "val": -5}"#}))
        .send()
        .await
        .unwrap();

    // Use namespaced function
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT math::abs(val) AS abs_val FROM test"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["data"][0]["abs_val"], 5);
}

#[tokio::test]
async fn test_non_namespaced_function_error() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION test"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"INSERT INTO test {"id": "t1", "name": "hello"}"#}))
        .send()
        .await
        .unwrap();

    // Non-namespaced UPPER should error
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT UPPER(name) FROM test"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let has_error = !body["error"].is_null() || !body["results"][0]["error"].is_null();
    assert!(
        has_error,
        "Expected error for non-namespaced UPPER(), got: {:?}",
        body
    );
}

// =============================================================================
// LET Statement Tests

#[tokio::test]
async fn test_let_simple_variable() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION test"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"INSERT INTO test {"id": "t1", "price": 100}"#}))
        .send()
        .await
        .unwrap();

    // Use LET and variable in same batch
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "LET tax = 0.2; SELECT price * (1 + $tax) AS total FROM test"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][1]["data"][0]["total"], 120.0);
}

#[tokio::test]
async fn test_undefined_variable_error() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION test"}))
        .send()
        .await
        .unwrap();

    // Insert a document so that SELECT actually processes rows
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"INSERT INTO test {"id": "t1"}"#}))
        .send()
        .await
        .unwrap();

    // Use undefined variable should error
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT $undefined FROM test"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let has_error = !body["error"].is_null() || !body["results"][0]["error"].is_null();
    assert!(
        has_error,
        "Expected error for undefined variable, got: {:?}",
        body
    );
}
