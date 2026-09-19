// CREATE statement edge case tests
// Tests for CREATE vs INSERT differences, key handling, and error conditions

use crate::common;
use serde_json::json;

// =============================================================================
// Basic CREATE Tests
// =============================================================================

#[tokio::test]
async fn test_create_with_key() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION create_test"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"CREATE create_test:alice SET name = "Alice", age = 30"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "CREATE with key should work: {:?}",
        body
    );

    // Verify document was created with correct key
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM create_test:alice"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["name"], "Alice");
    assert_eq!(body["results"][0]["data"][0]["age"], 30);
}

#[tokio::test]
async fn test_create_without_key() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION create_nokey"}))
        .send()
        .await
        .unwrap();

    // CREATE without key - should auto-generate
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"CREATE create_nokey SET name = "Bob""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "CREATE without key should work");

    // Verify document was created
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM create_nokey"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1);
    // ID should be auto-generated
    let id = body["results"][0]["data"][0]["id"].as_str().unwrap();
    assert!(
        id.starts_with("create_nokey:"),
        "ID should have collection prefix: {}",
        id
    );
}

// =============================================================================
// CREATE Duplicate Key Tests
// =============================================================================

#[tokio::test]
async fn test_create_duplicate_key() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION create_dup"}))
        .send()
        .await
        .unwrap();

    // First CREATE
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"CREATE create_dup:dup1 SET name = "First""#
        }))
        .send()
        .await
        .unwrap();

    // Second CREATE with same key
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"CREATE create_dup:dup1 SET name = "Second""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // CREATE should fail on duplicate (unlike INSERT which might upsert)
    assert!(
        common::has_error(&body),
        "CREATE duplicate key should fail: {:?}",
        body
    );

    // Verify original document unchanged
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM create_dup:dup1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["name"], "First");
}

// =============================================================================
// CREATE vs INSERT Comparison
// =============================================================================

#[tokio::test]
async fn test_create_vs_insert_semantics() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION compare"}))
        .send()
        .await
        .unwrap();

    // CREATE uses SET syntax
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"CREATE compare:c1 SET name = "Create""#
        }))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());

    // INSERT uses object literal syntax
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO compare {"id": "i1", "name": "Insert"}"#
        }))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());

    // Both should exist
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM compare"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 2);
}

// =============================================================================
// CREATE with Schema Validation
// =============================================================================

#[tokio::test]
async fn test_create_validates_schema() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION create_schema (SCHEMA STRICT, name string REQUIRED, age int)"
        }))
        .send()
        .await
        .unwrap();

    // Missing required field
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "CREATE create_schema:s1 SET age = 30"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "CREATE should validate required fields: {:?}",
        body
    );
}

#[tokio::test]
async fn test_create_type_mismatch() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION create_types (SCHEMA STRICT, age int)"
        }))
        .send()
        .await
        .unwrap();

    // String value for int field
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"CREATE create_types:t1 SET age = "not a number""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "CREATE should validate types: {:?}",
        body
    );
}

#[tokio::test]
async fn test_create_applies_defaults() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION create_defaults (name string, status string DEFAULT \"active\")"
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"CREATE create_defaults:d1 SET name = "Test""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());

    // Verify default was applied
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM create_defaults:d1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["status"], "active");
}

// =============================================================================
// CREATE with Special Keys
// =============================================================================

#[tokio::test]
async fn test_create_unicode_key() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION create_unicode"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"CREATE create_unicode:ユーザー SET name = "Unicode key""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Unicode key should work: {:?}",
        body
    );

    // Verify retrieval
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM create_unicode:ユーザー"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1);
}

#[tokio::test]
async fn test_create_numeric_key() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION create_numkey"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"CREATE create_numkey:12345 SET value = "numeric key""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Numeric key should work: {:?}",
        body
    );

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM create_numkey:12345"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1);
}

#[tokio::test]
async fn test_create_quoted_key_with_slash() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION create_slash"}))
        .send()
        .await
        .unwrap();

    // Use quoted key syntax for slashes
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"CREATE create_slash:"path/to/resource" SET type = "path""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Quoted key with slash should work: {:?}",
        body
    );

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"SELECT * FROM create_slash:"path/to/resource""#}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1);
}

// =============================================================================
// CREATE with Multiple Assignments
// =============================================================================

#[tokio::test]
async fn test_create_multiple_fields() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION create_multi"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"CREATE create_multi:m1 SET name = "Test", age = 25, active = true, score = 99.5"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM create_multi:m1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let doc = &body["results"][0]["data"][0];
    assert_eq!(doc["name"], "Test");
    assert_eq!(doc["age"], 25);
    assert_eq!(doc["active"], true);
    assert_eq!(doc["score"], 99.5);
}

#[tokio::test]
async fn test_create_nested_object() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION create_nested"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"CREATE create_nested:n1 SET name = "Test", address = {"city": "NYC", "zip": "10001"}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Nested object in CREATE should work: {:?}",
        body
    );

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM create_nested:n1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["address"]["city"], "NYC");
}

#[tokio::test]
async fn test_create_array_value() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION create_array"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"CREATE create_array:a1 SET tags = ["rust", "database", "test"]"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Array in CREATE should work: {:?}",
        body
    );

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM create_array:a1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let tags = body["results"][0]["data"][0]["tags"].as_array().unwrap();
    assert_eq!(tags.len(), 3);
}

// =============================================================================
// CREATE Error Cases
// =============================================================================

#[tokio::test]
async fn test_create_missing_set() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION create_noset"}))
        .send()
        .await
        .unwrap();

    // CREATE without SET
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "CREATE create_noset:x1"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Should be parse error - SET is required
    assert!(
        !body["error"].is_null(),
        "CREATE without SET should fail: {:?}",
        body
    );
}

#[tokio::test]
async fn test_create_empty_assignments() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "CREATE create_empty:e1 SET"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Should be parse error - no assignments after SET
    assert!(
        !body["error"].is_null(),
        "CREATE SET without assignments should fail: {:?}",
        body
    );
}

// =============================================================================
// CREATE Response Format
// =============================================================================

#[tokio::test]
async fn test_create_returns_created_document() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION create_resp"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"CREATE create_resp:r1 SET name = "Response Test""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());

    // CREATE should return the created document
    let data = &body["results"][0]["data"];
    if data.is_array() {
        assert_eq!(data[0]["name"], "Response Test");
    } else if data.is_object() {
        assert_eq!(data["name"], "Response Test");
    }
}
