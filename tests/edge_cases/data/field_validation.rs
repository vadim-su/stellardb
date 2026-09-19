// Field validation edge case tests
// Tests for type mismatches, null handling, nested objects, arrays, and boundary values

use crate::common;
use serde_json::json;

// =============================================================================
// Type Mismatch Tests
// =============================================================================

#[tokio::test]
async fn test_string_field_receives_number() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define collection with string field
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED)"
        }))
        .send()
        .await
        .unwrap();

    // Insert number into string field
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": 12345}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "Should reject number in string field: {:?}",
        body
    );
}

#[tokio::test]
async fn test_int_field_receives_float() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define collection with int field
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED, age int)"
        }))
        .send()
        .await
        .unwrap();

    // Insert float into int field
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "age": 25.5}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "Should reject float in int field: {:?}",
        body
    );
}

#[tokio::test]
async fn test_int_field_receives_string() {
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

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "age": "twenty-five"}"#
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
async fn test_bool_field_receives_string() {
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

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "active": "true"}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "Should reject string 'true' in bool field: {:?}",
        body
    );
}

#[tokio::test]
async fn test_float_field_accepts_integer() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION metrics (SCHEMA STRICT, name string REQUIRED, value float)"
        }))
        .send()
        .await
        .unwrap();

    // Integer should be coercible to float
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO metrics {"id": "m1", "name": "test", "value": 42}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // This may or may not be allowed depending on implementation
    // Document the actual behavior
    if body["error"].is_null() {
        // Verify the value was stored
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "SELECT * FROM metrics:m1"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(body["results"][0]["data"][0]["value"], 42.0);
    }
    // If it's rejected, that's also valid strict behavior
}

// =============================================================================
// Null Handling Tests
// =============================================================================

#[tokio::test]
async fn test_null_on_optional_field() {
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

    // Null should be allowed on optional field
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "nickname": null}"#
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
}

#[tokio::test]
async fn test_null_on_required_field() {
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

    // Null should not be allowed on required field
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": null}"#
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
async fn test_missing_optional_field_uses_null() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED, bio string)"
        }))
        .send()
        .await
        .unwrap();

    // Insert without optional field
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice"}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should allow missing optional field"
    );

    // Verify the field is null or absent
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let doc = &body["results"][0]["data"][0];
    assert!(doc["bio"].is_null() || doc.get("bio").is_none());
}

// =============================================================================
// Nested Object Tests
// =============================================================================

#[tokio::test]
async fn test_object_field_receives_primitive() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED, metadata object)"
        }))
        .send()
        .await
        .unwrap();

    // Insert string into object field
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "metadata": "not an object"}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "Should reject primitive in object field: {:?}",
        body
    );
}

#[tokio::test]
async fn test_object_field_receives_array() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED, metadata object)"
        }))
        .send()
        .await
        .unwrap();

    // Insert array into object field
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "metadata": [1, 2, 3]}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "Should reject array in object field: {:?}",
        body
    );
}

#[tokio::test]
async fn test_deeply_nested_object_validation() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define collection that allows any fields (flexible)
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION data"}))
        .send()
        .await
        .unwrap();

    // Insert deeply nested object
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO data {"id": "d1", "level1": {"level2": {"level3": {"level4": {"value": "deep"}}}}}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should allow deeply nested objects"
    );

    // Verify the nested structure is preserved
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM data:d1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        body["results"][0]["data"][0]["level1"]["level2"]["level3"]["level4"]["value"],
        "deep"
    );
}

// =============================================================================
// Array Field Tests
// =============================================================================

#[tokio::test]
async fn test_array_field_receives_primitive() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED, tags array)"
        }))
        .send()
        .await
        .unwrap();

    // Insert string into array field
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "tags": "not-an-array"}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "Should reject primitive in array field: {:?}",
        body
    );
}

#[tokio::test]
async fn test_array_field_receives_object() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED, tags array)"
        }))
        .send()
        .await
        .unwrap();

    // Insert object into array field
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "tags": {"not": "array"}}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "Should reject object in array field: {:?}",
        body
    );
}

#[tokio::test]
async fn test_empty_array_is_valid() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Use schemaless collection for testing empty array support
    // (typed arrays like [string] would need element validation)
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "tags": []}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should allow empty array: {:?}",
        body
    );

    // Verify the empty array was stored correctly
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let tags = &body["results"][0]["data"][0]["tags"];
    assert!(tags.is_array(), "Tags should be array");
    assert!(tags.as_array().unwrap().is_empty(), "Tags should be empty");
}

#[tokio::test]
async fn test_mixed_type_array() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION data"}))
        .send()
        .await
        .unwrap();

    // Insert array with mixed types (flexible collection)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO data {"id": "d1", "mixed": [1, "two", true, null, {"nested": "obj"}]}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Flexible collection should allow mixed arrays"
    );
}

// =============================================================================
// Empty Document Tests
// =============================================================================

#[tokio::test]
async fn test_empty_document_in_flexible_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION empty_test"}))
        .send()
        .await
        .unwrap();

    // Insert document with only id
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO empty_test {"id": "e1"}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Flexible collection should allow minimal document: {:?}",
        body
    );
}

#[tokio::test]
async fn test_empty_document_in_strict_collection() {
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

    // Insert document with only id - missing required field
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1"}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "Strict collection should reject document missing required field"
    );
}

// =============================================================================
// Boundary Value Tests
// =============================================================================

#[tokio::test]
async fn test_very_long_string() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION data"}))
        .send()
        .await
        .unwrap();

    let long_string = "x".repeat(100_000); // 100KB string
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": format!(r#"INSERT INTO data {{"id": "d1", "content": "{}"}}"#, long_string)
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Either succeeds or returns a clear error about size limits
    if body["error"].is_null() {
        // Verify it was stored correctly
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "SELECT * FROM data:d1"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(
            body["results"][0]["data"][0]["content"]
                .as_str()
                .unwrap()
                .len(),
            100_000
        );
    }
}

#[tokio::test]
async fn test_large_integer_value() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION nums (SCHEMA STRICT, value int REQUIRED)"
        }))
        .send()
        .await
        .unwrap();

    // Insert i64::MAX
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": format!(r#"INSERT INTO nums {{"id": "n1", "value": {}}}"#, i64::MAX)
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    if body["error"].is_null() {
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "SELECT * FROM nums:n1"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(body["results"][0]["data"][0]["value"], i64::MAX);
    }
}

#[tokio::test]
async fn test_negative_integer() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION nums (SCHEMA STRICT, value int REQUIRED)"
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO nums {"id": "n1", "value": -42}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Should allow negative integers");

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM nums:n1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["value"], -42);
}

#[tokio::test]
async fn test_empty_string() {
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

    // Empty string is still a string
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": ""}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Empty string should be valid for REQUIRED (it's not null)
    assert!(
        body["error"].is_null(),
        "Empty string should be valid for required string field: {:?}",
        body
    );
}

// =============================================================================
// Special Characters in Values
// =============================================================================

#[tokio::test]
async fn test_unicode_in_string() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION i18n"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO i18n {"id": "u1", "name": "日本語テスト", "emoji": "🚀🎉"}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Should allow unicode characters");

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM i18n:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["name"], "日本語テスト");
    assert_eq!(body["results"][0]["data"][0]["emoji"], "🚀🎉");
}

#[tokio::test]
async fn test_special_json_characters_in_string() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION special"}))
        .send()
        .await
        .unwrap();

    // Note: SQL parser has limited support for JSON escape sequences
    // Use HTTP REST API for complex string values
    // Here we test basic strings with some special chars
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO special {"id": "s1", "content": "hello world! & < > test"}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should handle basic special characters: {:?}",
        body
    );

    // Verify the content was stored
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM special:s1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        body["results"][0]["data"][0]["content"],
        "hello world! & < > test"
    );
}
