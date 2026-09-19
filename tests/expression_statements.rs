//! Tests for expression statements and function/variable data sources

mod common;

use serde_json::json;

async fn query(
    client: &reqwest::Client,
    addr: &std::net::SocketAddr,
    sql: &str,
) -> serde_json::Value {
    let resp = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({ "query": sql }))
        .send()
        .await
        .unwrap();
    resp.json().await.unwrap()
}

// =============================================================================
// Expression Statements
// =============================================================================

#[tokio::test]
async fn test_expr_stmt_literal_number() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "123").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    // Scalar result is directly in data[0]
    assert_eq!(result["results"][0]["data"][0], 123);
}

#[tokio::test]
async fn test_expr_stmt_literal_string() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, r#""hello""#).await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], "hello");
}

#[tokio::test]
async fn test_expr_stmt_literal_array() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "[1, 2, 3]").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], json!([1, 2, 3]));
}

#[tokio::test]
async fn test_expr_stmt_math_constant() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "math::pi").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let pi = result["results"][0]["data"][0].as_f64().unwrap();
    assert!((pi - std::f64::consts::PI).abs() < 0.0001);
}

#[tokio::test]
async fn test_expr_stmt_function_call() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, r#"string::upper("hello")"#).await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], "HELLO");
}

#[tokio::test]
async fn test_expr_stmt_with_variable() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // LET uses ident without $, then variable usage is with $
    let result = query(&client, &addr, "LET x = 42; $x").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][1]["data"][0], 42);
}

#[tokio::test]
async fn test_expr_stmt_arithmetic() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "2 + 2 * 3").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], 8);
}

#[tokio::test]
async fn test_expr_stmt_object() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, r#"{a: 1, b: "test"}"#).await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    // Object scalar result
    assert_eq!(result["results"][0]["data"][0]["a"], 1);
    assert_eq!(result["results"][0]["data"][0]["b"], "test");
}

// =============================================================================
// FROM with Functions
// =============================================================================

#[tokio::test]
async fn test_from_function_flatten() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Use array::flatten to combine nested arrays into a single array
    let result = query(
        &client,
        &addr,
        "SELECT * FROM array::flatten([[{a: 1}, {a: 2}], [{a: 3}]])",
    )
    .await;

    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["a"], 1);
    assert_eq!(rows[1]["a"], 2);
    assert_eq!(rows[2]["a"], 3);
}

#[tokio::test]
async fn test_from_function_with_filter() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Use array::flatten and filter the results
    let result = query(
        &client,
        &addr,
        "SELECT * FROM array::flatten([[{x: 1}, {x: 5}], [{x: 3}, {x: 10}]]) WHERE x > 2",
    )
    .await;

    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 3); // x=5, x=3, x=10
}

#[tokio::test]
async fn test_from_function_flatten_nested() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(
        &client,
        &addr,
        "SELECT * FROM array::flatten([[{a: 1}], [{a: 2}]])",
    )
    .await;

    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
}

// =============================================================================
// FROM with Variables
// =============================================================================

#[tokio::test]
async fn test_from_variable_array() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // LET without $, variable reference with $
    let result = query(
        &client,
        &addr,
        "LET data = [{name: 'alice'}, {name: 'bob'}]; SELECT * FROM $data",
    )
    .await;

    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][1]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["name"], "alice");
    assert_eq!(rows[1]["name"], "bob");
}

#[tokio::test]
async fn test_from_variable_with_filter() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(
        &client,
        &addr,
        "LET nums = [{v: 1}, {v: 5}, {v: 3}]; SELECT * FROM $nums WHERE v > 2",
    )
    .await;

    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][1]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 2); // v=5, v=3
}

#[tokio::test]
async fn test_from_variable_object() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "LET obj = {x: 1, y: 2}; SELECT * FROM $obj").await;

    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][1]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["x"], 1);
    assert_eq!(rows[0]["y"], 2);
}

#[tokio::test]
async fn test_from_variable_null_empty() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "LET empty = null; SELECT * FROM $empty").await;

    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][1]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 0);
}

#[tokio::test]
async fn test_from_variable_scalar_single_row() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "LET num = 123; SELECT * FROM $num").await;

    // Scalars become a single row
    assert!(result["error"].is_null(), "Unexpected error: {:?}", result);
    let rows = result["results"][1]["data"].as_array().unwrap();
    assert_eq!(
        rows.len(),
        1,
        "Scalar should produce single row, got: {:?}",
        rows
    );
    assert_eq!(rows[0], 123, "Expected scalar value in row");
}

#[tokio::test]
async fn test_from_variable_with_projection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(
        &client,
        &addr,
        "LET data = [{name: 'alice', age: 30}, {name: 'bob', age: 25}]; SELECT name FROM $data",
    )
    .await;

    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][1]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["name"], "alice");
    assert!(rows[0].get("age").is_none() || rows[0]["age"].is_null());
}
