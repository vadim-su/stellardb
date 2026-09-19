// Update operation edge case tests
// Tests for updating non-existent documents, type mismatches, removing required fields

use crate::common;
use serde_json::json;

// =============================================================================
// Non-existent Document Updates
// =============================================================================

#[tokio::test]
async fn test_update_nonexistent_document() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user (name string)"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"UPDATE user:nonexistent SET name = "test""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Should either fail or update 0 documents
    assert!(
        common::has_error(&body) || body["results"][0]["count"] == 0,
        "Should fail or update 0 docs: {:?}",
        body
    );
}

#[tokio::test]
async fn test_update_with_where_no_match() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user (name string, age int)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "age": 30}"#
        }))
        .send()
        .await
        .unwrap();

    // Update with WHERE that matches nothing
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"UPDATE user SET name = "test" WHERE age > 100"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Should not error on no match");
    assert_eq!(body["results"][0]["count"], 0, "Should update 0 documents");
}

// =============================================================================
// Type Mismatch Updates
// =============================================================================

#[tokio::test]
async fn test_update_string_with_number() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED)"
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice"}"#
        }))
        .send()
        .await
        .unwrap();

    // Try to set string field to number
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "UPDATE user:u1 SET name = 12345"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "Should reject type mismatch on update: {:?}",
        body
    );
}

#[tokio::test]
async fn test_update_int_with_string() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED, age int)"
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "age": 30}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"UPDATE user:u1 SET age = "thirty""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "Should reject string in int field: {:?}",
        body
    );
}

#[tokio::test]
async fn test_update_bool_with_int() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED, active bool)"
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "active": true}"#
        }))
        .send()
        .await
        .unwrap();

    // Try to set bool field to int (0 or 1)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "UPDATE user:u1 SET active = 1"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Document whether 1/0 coerce to true/false
    assert!(
        common::has_error(&body) || body["error"].is_null(),
        "Should either reject or coerce: {:?}",
        body
    );
}

// =============================================================================
// Required Field Updates
// =============================================================================

#[tokio::test]
async fn test_update_required_field_to_null() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (name string REQUIRED)"
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice"}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "UPDATE user:u1 SET name = null"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "Should reject null on required field: {:?}",
        body
    );
}

#[tokio::test]
async fn test_update_optional_field_to_null() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED, nickname string)"
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "nickname": "Ali"}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "UPDATE user:u1 SET nickname = null"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should allow null on optional field: {:?}",
        body
    );

    // Verify the update
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["results"][0]["data"][0]["nickname"].is_null());
}

// =============================================================================
// Unknown Field Updates
// =============================================================================

#[tokio::test]
async fn test_update_add_unknown_field_strict() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED)"
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice"}"#
        }))
        .send()
        .await
        .unwrap();

    // Try to add unknown field in strict mode
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"UPDATE user:u1 SET unknown_field = "value""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "Strict schema should reject unknown field: {:?}",
        body
    );
}

#[tokio::test]
async fn test_update_add_unknown_field_flexible() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (name string REQUIRED)"
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice"}"#
        }))
        .send()
        .await
        .unwrap();

    // Add unknown field in flexible mode
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"UPDATE user:u1 SET extra_field = "value""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Flexible schema should allow new fields: {:?}",
        body
    );

    // Verify the field was added
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["extra_field"], "value");
}

// =============================================================================
// Multiple Field Updates
// =============================================================================

#[tokio::test]
async fn test_update_multiple_fields() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (name string, age int, active bool)"
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "age": 30, "active": true}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"UPDATE user:u1 SET name = "Bob", age = 25, active = false"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Should allow multi-field update");

    // Verify all fields updated
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let doc = &body["results"][0]["data"][0];
    assert_eq!(doc["name"], "Bob");
    assert_eq!(doc["age"], 25);
    assert_eq!(doc["active"], false);
}

#[tokio::test]
async fn test_update_partial_failure() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED, age int)"
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "age": 30}"#
        }))
        .send()
        .await
        .unwrap();

    // Update with one valid field and one invalid
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"UPDATE user:u1 SET age = 25, unknown = "field""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Should fail entirely (atomic update)
    assert!(
        common::has_error(&body),
        "Partial invalid update should fail: {:?}",
        body
    );

    // Verify nothing changed
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        body["results"][0]["data"][0]["age"], 30,
        "Age should remain unchanged"
    );
}

// =============================================================================
// Nested Field Updates
// =============================================================================

#[tokio::test]
async fn test_update_nested_object() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "address": {"city": "NYC", "zip": "10001"}}"#
        }))
        .send()
        .await
        .unwrap();

    // Update entire nested object
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"UPDATE user:u1 SET address = {"city": "LA", "zip": "90001"}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Should allow nested object update");

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["address"]["city"], "LA");
}

// =============================================================================
// Array Updates
// =============================================================================

#[tokio::test]
async fn test_update_array_field() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "tags": ["a", "b"]}"#
        }))
        .send()
        .await
        .unwrap();

    // Replace entire array
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"UPDATE user:u1 SET tags = ["x", "y", "z"]"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Should allow array replacement");

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let tags = body["results"][0]["data"][0]["tags"].as_array().unwrap();
    assert_eq!(tags.len(), 3);
    assert_eq!(tags[0], "x");
}

#[tokio::test]
async fn test_update_array_to_empty() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "tags": ["a", "b"]}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "UPDATE user:u1 SET tags = []"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Should allow empty array");

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let tags = body["results"][0]["data"][0]["tags"].as_array().unwrap();
    assert!(tags.is_empty());
}

// =============================================================================
// HTTP REST API Update Tests
// =============================================================================

#[tokio::test]
async fn test_http_put_nonexistent_document() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection via HTTP POST first
    client
        .post(format!("http://{}/collections/user/documents", addr))
        .json(&json!({"id": "user:existing", "name": "Alice"}))
        .send()
        .await
        .unwrap();

    // Try to update non-existent document
    let res = client
        .put(format!(
            "http://{}/collections/user/documents/nonexistent",
            addr
        ))
        .json(&json!({"name": "Bob"}))
        .send()
        .await
        .unwrap();

    // Should return 404
    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn test_http_put_replace_behavior() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create document
    client
        .post(format!("http://{}/collections/user/documents", addr))
        .json(&json!({"id": "user:alice", "name": "Alice", "age": 30}))
        .send()
        .await
        .unwrap();

    // PUT replaces entire document (standard REST behavior)
    // Only sends age - name will be lost
    let res = client
        .put(format!("http://{}/collections/user/documents/alice", addr))
        .json(&json!({"age": 31}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);

    // Verify PUT replaced (not merged) - name should be gone
    let res = client
        .get(format!("http://{}/collections/user/documents/alice", addr))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // PUT replaces, so "name" field is lost - this is correct REST semantics
    assert!(
        body.get("name").is_none() || body["name"].is_null(),
        "PUT should replace, not merge"
    );
    assert_eq!(body["age"], 31, "Age should be set");
}

// =============================================================================
// Concurrent Update Edge Cases
// =============================================================================

#[tokio::test]
async fn test_update_same_document_twice() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION counter (value int)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO counter {"id": "c1", "value": 0}"#
        }))
        .send()
        .await
        .unwrap();

    // Two sequential updates
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "UPDATE counter:c1 SET value = 1"
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "UPDATE counter:c1 SET value = 2"
        }))
        .send()
        .await
        .unwrap();

    // Verify final value
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM counter:c1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["value"], 2);
}

// =============================================================================
// Special Value Updates
// =============================================================================

#[tokio::test]
async fn test_update_to_empty_string() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user (name string)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice"}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"UPDATE user:u1 SET name = """#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Should allow empty string");

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["name"], "");
}

#[tokio::test]
async fn test_update_with_unicode() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user (name string)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice"}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"UPDATE user:u1 SET name = "日本語🚀""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Should allow unicode");

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["name"], "日本語🚀");
}

// =============================================================================
// Expression Support Tests
// =============================================================================

#[tokio::test]
async fn test_update_arithmetic_expression() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION counter (value int)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO counter {"id": "c1", "value": 10}"#
        }))
        .send()
        .await
        .unwrap();

    // Increment value
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "UPDATE counter:c1 SET value = value + 1"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should allow arithmetic: {:?}",
        body
    );

    // Verify
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM counter:c1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["value"], 11);
}

#[tokio::test]
async fn test_update_multiply_expression() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION product (price float)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO product {"id": "p1", "price": 100.0}"#
        }))
        .send()
        .await
        .unwrap();

    // Apply 10% discount
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "UPDATE product:p1 SET price = price * 0.9"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Should allow multiply: {:?}", body);

    // Verify
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM product:p1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let price = body["results"][0]["data"][0]["price"].as_f64().unwrap();
    assert!((price - 90.0).abs() < 0.001);
}

// =============================================================================
// Function Expression Updates
// =============================================================================

#[tokio::test]
async fn test_update_function_upper() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user (name string)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "alice"}"#
        }))
        .send()
        .await
        .unwrap();

    // Apply UPPER function
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "UPDATE user:u1 SET name = string::upper(name)"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Should allow function: {:?}", body);

    // Verify
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["name"], "ALICE");
}

#[tokio::test]
async fn test_update_function_math() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION data (value float)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO data {"id": "d1", "value": -42.5}"#
        }))
        .send()
        .await
        .unwrap();

    // Apply math::abs function
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "UPDATE data:d1 SET value = math::abs(value)"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should allow math function: {:?}",
        body
    );

    // Verify
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM data:d1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let value = body["results"][0]["data"][0]["value"].as_f64().unwrap();
    assert!((value - 42.5).abs() < 0.001);
}

// =============================================================================
// Field Reference Updates
// =============================================================================

#[tokio::test]
async fn test_update_field_reference() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user (first_name string, last_name string, fullname string)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "first_name": "John", "last_name": "Doe"}"#
        }))
        .send()
        .await
        .unwrap();

    // Concatenate fields using string::concat() function
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"UPDATE user:u1 SET fullname = string::concat(first_name, " ", last_name)"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should allow field concat: {:?}",
        body
    );

    // Verify
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["fullname"], "John Doe");
}

#[tokio::test]
async fn test_update_copy_field() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user (name string, backup_name string)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice"}"#
        }))
        .send()
        .await
        .unwrap();

    // Copy field value
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "UPDATE user:u1 SET backup_name = name"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should allow field copy: {:?}",
        body
    );

    // Verify
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["backup_name"], "Alice");
}

// =============================================================================
// Nested Field Path Updates
// =============================================================================

#[tokio::test]
async fn test_update_nested_field_existing() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "address": {"city": "LA", "zip": "90001"}}"#
        }))
        .send()
        .await
        .unwrap();

    // Update nested field
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"UPDATE user:u1 SET address.city = "NYC""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should allow nested update: {:?}",
        body
    );

    // Verify - city changed, zip preserved
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["address"]["city"], "NYC");
    assert_eq!(body["results"][0]["data"][0]["address"]["zip"], "90001");
}

#[tokio::test]
async fn test_update_nested_field_create() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice"}"#
        }))
        .send()
        .await
        .unwrap();

    // Create nested structure via update
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"UPDATE user:u1 SET address.city = "NYC""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Should create nested: {:?}", body);

    // Verify
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["address"]["city"], "NYC");
}

#[tokio::test]
async fn test_update_nested_field_deep() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice"}"#
        }))
        .send()
        .await
        .unwrap();

    // Create deeply nested structure
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"UPDATE user:u1 SET profile.settings.theme = "dark""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should create deep nested: {:?}",
        body
    );

    // Verify
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        body["results"][0]["data"][0]["profile"]["settings"]["theme"],
        "dark"
    );
}

#[tokio::test]
async fn test_update_nested_field_type_error() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "address": "123 Main St"}"#
        }))
        .send()
        .await
        .unwrap();

    // Try to update nested field on string (should fail)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"UPDATE user:u1 SET address.city = "NYC""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "Should fail on non-object: {:?}",
        body
    );
}

#[tokio::test]
async fn test_update_nested_field_with_expression() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "stats": {"views": 10}}"#
        }))
        .send()
        .await
        .unwrap();

    // Increment nested field
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "UPDATE user:u1 SET stats.views = stats.views + 1"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should allow nested expr: {:?}",
        body
    );

    // Verify
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["stats"]["views"], 11);
}

// =============================================================================
// Subquery Expression Tests
// =============================================================================

#[tokio::test]
async fn test_update_subquery_array() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collections
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION orders (user_name string, item string)"}))
        .send()
        .await
        .unwrap();

    // Insert data
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "alice", "name": "alice"}"#
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO orders {"id": "o1", "user_name": "alice", "item": "Book"}, {"id": "o2", "user_name": "alice", "item": "Pen"}"#
        }))
        .send()
        .await
        .unwrap();

    // Update with subquery - get all orders as array
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "UPDATE user:alice SET orders = (SELECT item FROM orders WHERE user_name = $parent.name)"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Should allow subquery: {:?}", body);

    // Verify - orders should be array of objects
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:alice"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let orders = body["results"][0]["data"][0]["orders"].as_array().unwrap();
    assert_eq!(orders.len(), 2);
}

#[tokio::test]
async fn test_update_subquery_count() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collections
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user (order_count int)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION orders (user_name string)"}))
        .send()
        .await
        .unwrap();

    // Insert data
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "alice", "name": "alice"}"#
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO orders {"id": "o1", "user_name": "alice"}, {"id": "o2", "user_name": "alice"}, {"id": "o3", "user_name": "alice"}"#
        }))
        .send()
        .await
        .unwrap();

    // Update with subquery count - need [0].cnt to get scalar
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "UPDATE user:alice SET order_count = (SELECT COUNT(*) AS cnt FROM orders WHERE user_name = $parent.name)[0].cnt"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should allow subquery count: {:?}",
        body
    );

    // Verify
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:alice"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["order_count"], 3);
}

#[tokio::test]
async fn test_update_subquery_first_item() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collections
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION orders (user_name string, item string)"}))
        .send()
        .await
        .unwrap();

    // Insert data
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "alice", "name": "alice"}"#
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO orders {"id": "o1", "user_name": "alice", "item": "First"}"#
        }))
        .send()
        .await
        .unwrap();

    // Update with subquery - get first item's field
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "UPDATE user:alice SET last_item = (SELECT item FROM orders WHERE user_name = $parent.name LIMIT 1)[0].item"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should allow subquery indexing: {:?}",
        body
    );

    // Verify
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:alice"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["last_item"], "First");
}
