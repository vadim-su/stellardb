//! Expression source integration tests
//!
//! Tests for FROM (expr) syntax that allows using any expression as a data source.
//! Examples:
//!   - SELECT * FROM (search::rrf([...], 10))
//!   - SELECT * FROM ([1, 2, 3])
//!   - SELECT * FROM (func_returning_array())

use crate::common;
use serde_json::json;

// =============================================================================
// Helpers
// =============================================================================

async fn query(
    addr: &std::net::SocketAddr,
    client: &reqwest::Client,
    sql: &str,
) -> serde_json::Value {
    let resp = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": sql}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].is_null(), "SQL error: {:?}", body);
    body["results"][0]["data"].clone()
}

// =============================================================================
// Basic expression source tests
// =============================================================================

#[tokio::test]
async fn test_from_expr_array_literal() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Array of scalars
    let data = query(&addr, &client, "SELECT * FROM ([1, 2, 3])").await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 3, "Expected 3 elements, got: {:?}", arr);
}

#[tokio::test]
async fn test_from_expr_array_of_objects() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let data = query(
        &addr,
        &client,
        r#"SELECT * FROM ([{name: "alice", age: 30}, {name: "bob", age: 25}])"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 2, "Expected 2 objects, got: {:?}", arr);
    assert_eq!(arr[0]["name"], "alice");
    assert_eq!(arr[1]["name"], "bob");
}

#[tokio::test]
async fn test_from_expr_with_where() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let data = query(
        &addr,
        &client,
        r#"SELECT * FROM ([{name: "alice", age: 30}, {name: "bob", age: 25}]) WHERE age > 26"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 1, "Expected only alice, got: {:?}", arr);
    assert_eq!(arr[0]["name"], "alice");
}

#[tokio::test]
async fn test_from_expr_with_order() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let data = query(
        &addr,
        &client,
        r#"SELECT * FROM ([{name: "bob"}, {name: "alice"}, {name: "carol"}]) ORDER name"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0]["name"], "alice");
    assert_eq!(arr[1]["name"], "bob");
    assert_eq!(arr[2]["name"], "carol");
}

#[tokio::test]
async fn test_from_expr_with_limit() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let data = query(
        &addr,
        &client,
        r#"SELECT * FROM ([{n: 1}, {n: 2}, {n: 3}]) LIMIT 2"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 2, "Expected 2 rows from LIMIT 2");
}

#[tokio::test]
async fn test_from_expr_with_projection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let data = query(
        &addr,
        &client,
        r#"SELECT name FROM ([{name: "alice", age: 30}, {name: "bob", age: 25}])"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    // Each row should only have "name", not "age"
    for row in arr {
        assert!(row["name"].is_string());
    }
}

// =============================================================================
// Function call as expression source
// =============================================================================

#[tokio::test]
async fn test_from_expr_function_array_reverse() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // array::reverse with literal array
    let data = query(&addr, &client, "SELECT * FROM (array::reverse([1, 2, 3]))").await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    // Scalars are returned directly (not as objects with "value" field)
    assert_eq!(arr[0], 3);
    assert_eq!(arr[1], 2);
    assert_eq!(arr[2], 1);
}

#[tokio::test]
async fn test_from_expr_function_array_flatten() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // array::flatten with nested arrays
    let data = query(
        &addr,
        &client,
        "SELECT * FROM (array::flatten([[1, 2], [3, 4]]))",
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(
        arr.len(),
        4,
        "Expected 4 elements after flatten, got: {:?}",
        arr
    );
}

// =============================================================================
// Complex expression combinations
// =============================================================================

#[tokio::test]
async fn test_from_expr_single_object() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Single object (not array) should become a single row
    let data = query(
        &addr,
        &client,
        r#"SELECT * FROM ({name: "only", value: 42})"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 1, "Single object should produce single row");
    assert_eq!(arr[0]["name"], "only");
    assert_eq!(arr[0]["value"], 42);
}

#[tokio::test]
async fn test_from_expr_scalar_returns_single_row() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Scalar should become single row
    let data = query(&addr, &client, "SELECT * FROM (42)").await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 1, "Scalar should produce single row");
}

#[tokio::test]
async fn test_from_expr_empty_array() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let data = query(&addr, &client, "SELECT * FROM ([])").await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 0, "Empty array should produce no rows");
}

// =============================================================================
// Aggregate on expression source
// =============================================================================
// NOTE: Aggregates on InMemoryScan sources (array, subquery, expr) are not yet
// supported. This is a known limitation that applies to all in-memory sources.

// =============================================================================
// DISTINCT on expression source
// =============================================================================

#[tokio::test]
async fn test_from_expr_distinct() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let data = query(
        &addr,
        &client,
        r#"SELECT DISTINCT category FROM ([{category: "a"}, {category: "b"}, {category: "a"}])"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 2, "Expected 2 distinct categories");

    let cats: Vec<&str> = arr.iter().filter_map(|v| v["category"].as_str()).collect();
    assert!(cats.contains(&"a"));
    assert!(cats.contains(&"b"));
}

// =============================================================================
// VALUE mode with expression source
// =============================================================================

#[tokio::test]
async fn test_from_expr_value_mode() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let data = query(
        &addr,
        &client,
        r#"SELECT VALUE name FROM ([{name: "x"}, {name: "y"}])"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0], "x");
    assert_eq!(arr[1], "y");
}

// =============================================================================
// Arithmetic expressions
// =============================================================================

#[tokio::test]
async fn test_from_expr_computed_array() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Concatenate arrays using +
    let data = query(
        &addr,
        &client,
        r#"SELECT * FROM ([{a: 1}, {a: 2}] + [{a: 3}])"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 3, "Expected 3 elements after concatenation");
}

// =============================================================================
// Nested FROM expr
// =============================================================================

#[tokio::test]
async fn test_from_expr_nested_in_subquery() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Subquery that uses expr source, then outer query on that
    let data = query(
        &addr,
        &client,
        r#"SELECT * FROM (SELECT * FROM ([{v: 1}, {v: 2}, {v: 3}]) WHERE v > 1)"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 2, "Expected v=2 and v=3");
}

// =============================================================================
// Variable-based expression source
// =============================================================================

async fn query_multi(
    addr: &std::net::SocketAddr,
    client: &reqwest::Client,
    sql: &str,
    statement_index: usize,
) -> serde_json::Value {
    let resp = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": sql}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].is_null(), "SQL error: {:?}", body);
    body["results"][statement_index]["data"].clone()
}

#[tokio::test]
async fn test_from_expr_variable() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let data = query_multi(
        &addr,
        &client,
        r#"LET items = [{name: "sword", dmg: 10}, {name: "shield", dmg: 5}]; SELECT * FROM ($items)"#,
        1, // Second statement (SELECT)
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(
        arr.len(),
        2,
        "Expected 2 items from variable, got: {:?}",
        arr
    );

    let names: Vec<&str> = arr.iter().filter_map(|v| v["name"].as_str()).collect();
    assert!(names.contains(&"sword"));
    assert!(names.contains(&"shield"));
}

#[tokio::test]
async fn test_from_expr_variable_with_filter() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let data = query_multi(
        &addr,
        &client,
        r#"LET items = [{name: "sword", dmg: 10}, {name: "shield", dmg: 5}]; SELECT * FROM ($items) WHERE dmg > 7"#,
        1,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 1, "Expected only sword, got: {:?}", arr);
    assert_eq!(arr[0]["name"], "sword");
}

#[tokio::test]
async fn test_from_expr_subquery_in_variable() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Setup collection
    query(&addr, &client, "DEFINE COLLECTION items").await;
    query(
        &addr,
        &client,
        r#"INSERT INTO items {id: "1", name: "apple"}, {id: "2", name: "banana"}"#,
    )
    .await;

    // Get items via subquery stored in variable, then use as expr source
    let data = query_multi(
        &addr,
        &client,
        r#"LET all_items = (SELECT * FROM items); SELECT name FROM ($all_items) ORDER name"#,
        1,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["name"], "apple");
    assert_eq!(arr[1]["name"], "banana");
}
