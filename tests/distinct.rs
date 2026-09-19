//! Integration tests for DISTINCT functionality

mod common;

use serde_json::json;

async fn setup_test_data(addr: &std::net::SocketAddr, client: &reqwest::Client) {
    // Define collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION users"}))
        .send()
        .await
        .unwrap();

    // Insert test data with duplicate cities
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO users
                {id: 'alice', name: 'Alice', city: 'NYC', age: 30},
                {id: 'bob', name: 'Bob', city: 'LA', age: 25},
                {id: 'carol', name: 'Carol', city: 'NYC', age: 35},
                {id: 'dave', name: 'Dave', city: 'Chicago', age: 30},
                {id: 'eve', name: 'Eve', city: 'LA', age: 28}"#
        }))
        .send()
        .await
        .unwrap();
}

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
// SELECT DISTINCT
// =============================================================================

#[tokio::test]
async fn test_select_distinct_single_field() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let data = query(&addr, &client, "SELECT DISTINCT city FROM users").await;
    let arr = data.as_array().expect("data should be an array");

    // Should have 3 unique cities: NYC, LA, Chicago
    let mut cities: Vec<&str> = arr
        .iter()
        .map(|row| row["city"].as_str().unwrap())
        .collect();
    cities.sort();
    assert_eq!(cities, vec!["Chicago", "LA", "NYC"]);
}

#[tokio::test]
async fn test_select_distinct_multiple_fields() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let data = query(&addr, &client, "SELECT DISTINCT city, age FROM users").await;
    let arr = data.as_array().expect("data should be an array");

    // Should have 5 unique (city, age) combinations
    assert_eq!(arr.len(), 5);
}

#[tokio::test]
async fn test_select_distinct_with_order() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let data = query(&addr, &client, "SELECT DISTINCT city FROM users ORDER city").await;
    let arr = data.as_array().expect("data should be an array");

    // Should have 3 unique cities in order
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0]["city"], "Chicago");
    assert_eq!(arr[1]["city"], "LA");
    assert_eq!(arr[2]["city"], "NYC");
}

#[tokio::test]
async fn test_select_distinct_with_where() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT DISTINCT city FROM users WHERE age >= 28",
    )
    .await;
    let arr = data.as_array().expect("data should be an array");

    // Alice(NYC,30), Carol(NYC,35), Dave(Chicago,30), Eve(LA,28)
    // Unique cities: NYC, Chicago, LA
    let mut cities: Vec<&str> = arr
        .iter()
        .map(|row| row["city"].as_str().unwrap())
        .collect();
    cities.sort();
    assert_eq!(cities, vec!["Chicago", "LA", "NYC"]);
}

// =============================================================================
// SELECT VALUE
// =============================================================================

#[tokio::test]
async fn test_select_value_returns_values() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let data = query(&addr, &client, "SELECT VALUE name FROM users").await;
    let arr = data.as_array().expect("data should be an array");

    // Should have 5 values (flat strings, not objects)
    assert_eq!(arr.len(), 5);
    // Each should be a flat string value (SELECT VALUE returns flat values)
    for row in arr {
        assert!(row.is_string(), "Expected flat string value, got {:?}", row);
    }
}

#[tokio::test]
async fn test_select_value_with_order() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let data = query(&addr, &client, "SELECT VALUE name FROM users ORDER name").await;
    let arr = data.as_array().expect("data should be an array");

    // Should have 5 values in alphabetical order (flat strings)
    assert_eq!(arr.len(), 5);
    let names: Vec<&str> = arr.iter().map(|row| row.as_str().unwrap()).collect();
    assert_eq!(names, vec!["Alice", "Bob", "Carol", "Dave", "Eve"]);
}

// =============================================================================
// SELECT DISTINCT VALUE
// =============================================================================

#[tokio::test]
async fn test_select_distinct_value() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let data = query(&addr, &client, "SELECT DISTINCT VALUE city FROM users").await;
    let arr = data.as_array().expect("data should be an array");

    // Should have 3 unique city values
    assert_eq!(arr.len(), 3);
}

#[tokio::test]
async fn test_select_distinct_value_with_order() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT DISTINCT VALUE city FROM users ORDER city",
    )
    .await;
    let arr = data.as_array().expect("data should be an array");

    // Should have 3 unique city values in order (flat strings)
    assert_eq!(arr.len(), 3);
    let cities: Vec<&str> = arr.iter().map(|row| row.as_str().unwrap()).collect();
    assert_eq!(cities, vec!["Chicago", "LA", "NYC"]);
}

#[tokio::test]
async fn test_select_distinct_value_numeric() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT DISTINCT VALUE age FROM users ORDER age",
    )
    .await;
    let arr = data.as_array().expect("data should be an array");

    // Unique ages: 25, 28, 30, 35 (flat numbers)
    assert_eq!(arr.len(), 4);
    let ages: Vec<i64> = arr.iter().map(|row| row.as_i64().unwrap()).collect();
    assert_eq!(ages, vec![25, 28, 30, 35]);
}

// =============================================================================
// COUNT(DISTINCT field)
// =============================================================================

#[tokio::test]
async fn test_count_distinct() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let data = query(&addr, &client, "SELECT COUNT(DISTINCT city) FROM users").await;
    let arr = data.as_array().expect("data should be an array");

    assert_eq!(arr.len(), 1);
    // Alias should be "count(distinct city)" or similar
    let count = arr[0]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .as_i64()
        .unwrap();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn test_count_distinct_with_alias() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT COUNT(DISTINCT city) AS unique_cities FROM users",
    )
    .await;
    let arr = data.as_array().expect("data should be an array");

    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["unique_cities"], 3);
}

#[tokio::test]
async fn test_count_distinct_with_group() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT age, COUNT(DISTINCT city) AS unique_cities FROM users GROUP age ORDER age",
    )
    .await;
    let arr = data.as_array().expect("data should be an array");

    // Should have groups for ages 25, 28, 30, 35
    assert_eq!(arr.len(), 4);

    // Age 25: Bob (LA) -> 1 unique city
    assert_eq!(arr[0]["age"], 25);
    assert_eq!(arr[0]["unique_cities"], 1);

    // Age 28: Eve (LA) -> 1 unique city
    assert_eq!(arr[1]["age"], 28);
    assert_eq!(arr[1]["unique_cities"], 1);

    // Age 30: Alice (NYC), Dave (Chicago) -> 2 unique cities
    assert_eq!(arr[2]["age"], 30);
    assert_eq!(arr[2]["unique_cities"], 2);

    // Age 35: Carol (NYC) -> 1 unique city
    assert_eq!(arr[3]["age"], 35);
    assert_eq!(arr[3]["unique_cities"], 1);
}

// =============================================================================
// EXPLAIN shows new operators
// =============================================================================

#[tokio::test]
async fn test_explain_shows_distinct() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let data = query(&addr, &client, "EXPLAIN SELECT DISTINCT city FROM users").await;
    let text = data.as_array().unwrap()[0].as_str().unwrap();
    assert!(
        text.contains("Distinct"),
        "Expected Distinct in plan: {}",
        text
    );
}

#[tokio::test]
async fn test_explain_shows_value() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let data = query(&addr, &client, "EXPLAIN SELECT VALUE name FROM users").await;
    let text = data.as_array().unwrap()[0].as_str().unwrap();
    assert!(text.contains("Value"), "Expected Value in plan: {}", text);
}

// =============================================================================
// Edge cases
// =============================================================================

#[tokio::test]
async fn test_distinct_empty_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION empty_users"}))
        .send()
        .await
        .unwrap();

    let data = query(&addr, &client, "SELECT DISTINCT city FROM empty_users").await;
    let arr = data.as_array().expect("data should be an array");
    assert_eq!(arr.len(), 0);
}

#[tokio::test]
async fn test_distinct_all_same_values() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION same_city"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO same_city
                {id: '1', city: 'NYC'},
                {id: '2', city: 'NYC'},
                {id: '3', city: 'NYC'}"#
        }))
        .send()
        .await
        .unwrap();

    let data = query(&addr, &client, "SELECT DISTINCT city FROM same_city").await;
    let arr = data.as_array().expect("data should be an array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["city"], "NYC");
}

#[tokio::test]
async fn test_distinct_with_null_values() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION null_cities"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO null_cities
                {id: '1', city: 'NYC'},
                {id: '2', city: null},
                {id: '3', city: 'LA'},
                {id: '4', city: null}"#
        }))
        .send()
        .await
        .unwrap();

    let data = query(&addr, &client, "SELECT DISTINCT city FROM null_cities").await;
    let arr = data.as_array().expect("data should be an array");
    // Should have 3 unique values: NYC, LA, null (null appears once)
    assert_eq!(arr.len(), 3);
}

// =============================================================================
// Field name "value" should not conflict with VALUE keyword
// =============================================================================

#[tokio::test]
async fn test_value_as_field_name_in_array_source() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // "value" as a field name should work, not be parsed as VALUE keyword
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT value FROM [{value: 'test'}, {value: 'hello'}]"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"];
    let arr = data.as_array().expect("data should be an array");
    assert_eq!(arr.len(), 2);
    // SELECT value (without VALUE keyword) returns objects with the field
    assert_eq!(arr[0]["value"], "test");
    assert_eq!(arr[1]["value"], "hello");
}

#[tokio::test]
async fn test_value_as_field_name_in_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with "value" field
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION items"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO items {id: '1', value: 100}, {id: '2', value: 200}"#
        }))
        .send()
        .await
        .unwrap();

    // SELECT value FROM collection should work
    let data = query(&addr, &client, "SELECT value FROM items").await;
    let arr = data.as_array().expect("data should be an array");
    assert_eq!(arr.len(), 2);

    let mut values: Vec<i64> = arr.iter().map(|r| r["value"].as_i64().unwrap()).collect();
    values.sort();
    assert_eq!(values, vec![100, 200]);
}
