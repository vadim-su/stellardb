// Alias integration tests
// Tests for AS alias in projections

use crate::common;
use serde_json::json;

// =============================================================================
// Field Alias
// =============================================================================

#[tokio::test]
async fn test_field_alias() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION users (name string, age int)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO users {"id": "u1", "name": "Alice", "age": 30}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT name AS username FROM users"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"];
    let row = &data[0];

    // Should have "username" key with value "Alice"
    assert_eq!(
        row["username"], "Alice",
        "Expected alias 'username' with value 'Alice'"
    );

    // "name" key should not exist — the alias replaces it
    assert!(
        row.get("name").is_none() || row["name"].is_null(),
        "Original field 'name' should not be present when aliased, got: {:?}",
        row
    );
}

// =============================================================================
// Aggregate Alias
// =============================================================================

#[tokio::test]
async fn test_aggregate_alias() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION items (name string)"}))
        .send()
        .await
        .unwrap();

    for (id, name) in [("i1", "Apple"), ("i2", "Banana"), ("i3", "Cherry")] {
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": format!(r#"INSERT INTO items {{"id": "{}", "name": "{}"}}"#, id, name)
            }))
            .send()
            .await
            .unwrap();
    }

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT COUNT(*) AS cnt FROM items"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"];
    let row = &data[0];

    // Check that "cnt" key exists with value 3
    assert_eq!(row["cnt"], 3, "Expected COUNT(*) aliased as cnt = 3");
}
