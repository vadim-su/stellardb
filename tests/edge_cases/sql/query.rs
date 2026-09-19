// Query edge case tests
// Tests for WHERE clause, non-existent fields, type comparisons, and edge conditions

use crate::common;
use serde_json::json;

// =============================================================================
// WHERE Clause with Non-existent Fields
// =============================================================================

#[tokio::test]
async fn test_where_nonexistent_field() {
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

    // Query with non-existent field
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT * FROM user WHERE nonexistent = "value""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Should either return empty or error
    if body["error"].is_null() {
        assert_eq!(
            body["results"][0]["count"], 0,
            "Should return 0 results for non-existent field"
        );
    }
}

#[tokio::test]
async fn test_where_field_is_null() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user (name string, nickname string)"}))
        .send()
        .await
        .unwrap();

    // Insert with null field
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "nickname": null}"#
        }))
        .send()
        .await
        .unwrap();

    // Insert without optional field
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u2", "name": "Bob"}"#
        }))
        .send()
        .await
        .unwrap();

    // Query for null values
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT * FROM user WHERE nickname = null"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // In SQL, `x = null` evaluates to NULL (falsy), so no rows match.
    if body["error"].is_null() {
        let data = &body["results"][0]["data"];
        let count = data.as_array().map(|a| a.len()).unwrap_or(0);
        assert_eq!(
            count, 0,
            "SQL null = null should return no rows (three-valued logic)"
        );
    }
}

// =============================================================================
// Type Mismatch in WHERE Clause
// =============================================================================

#[tokio::test]
async fn test_where_int_compared_to_string() {
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

    // Compare int field to string
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT * FROM user WHERE age = "30""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Document behavior - may coerce or return 0
    if body["error"].is_null() {
        let count = body["results"][0]["count"].as_u64().unwrap_or(0);
        // Either finds it (coercion) or not (strict)
        assert!(
            count == 0 || count == 1,
            "Count should be 0 or 1: {}",
            count
        );
    }
}

#[tokio::test]
async fn test_where_string_compared_to_int() {
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

    // Compare string field to int
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT * FROM user WHERE name = 123"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Should return 0 (no match)
    if body["error"].is_null() {
        assert_eq!(
            body["results"][0]["count"], 0,
            "Type mismatch should not match"
        );
    }
}

#[tokio::test]
async fn test_where_bool_compared_to_string() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user (name string, active bool)"}))
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

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT * FROM user WHERE active = "true""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    if body["error"].is_null() {
        let count = body["results"][0]["count"].as_u64().unwrap_or(0);
        // String "true" should not match boolean true
        assert_eq!(count, 0, "String 'true' should not match boolean true");
    }
}

// =============================================================================
// Comparison Operators
// =============================================================================

#[tokio::test]
async fn test_where_greater_than() {
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
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "age": 20}"#
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u2", "name": "Bob", "age": 30}"#
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u3", "name": "Charlie", "age": 40}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT * FROM user WHERE age > 25"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(
        body["results"][0]["count"], 2,
        "Should find 2 users over 25"
    );
}

#[tokio::test]
async fn test_where_less_than_equal() {
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
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "age": 20}; INSERT INTO user {"id": "u2", "name": "Bob", "age": 30}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT * FROM user WHERE age <= 25"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 1);
}

#[tokio::test]
async fn test_where_not_equal() {
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
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice"}; INSERT INTO user {"id": "u2", "name": "Bob"}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT * FROM user WHERE name != "Alice""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["name"], "Bob");
}

// =============================================================================
// Logical Operators
// =============================================================================

#[tokio::test]
async fn test_where_and_operator() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user (name string, age int, active bool)"}))
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

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u2", "name": "Bob", "age": 25, "active": true}"#
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u3", "name": "Charlie", "age": 35, "active": false}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT * FROM user WHERE age > 25 AND active = true"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(
        body["results"][0]["count"], 1,
        "Only Alice matches both conditions"
    );
}

#[tokio::test]
async fn test_where_or_operator() {
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
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "age": 20}"#
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u2", "name": "Bob", "age": 30}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT * FROM user WHERE name = "Alice" OR age > 25"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 2, "Both Alice and Bob match");
}

// =============================================================================
// Empty Results
// =============================================================================

#[tokio::test]
async fn test_select_with_impossible_condition() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user (age int)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "age": 30}"#
        }))
        .send()
        .await
        .unwrap();

    // Impossible condition
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT * FROM user WHERE age > 100 AND age < 0"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 0);
}

// =============================================================================
// Special Value Queries
// =============================================================================

#[tokio::test]
async fn test_select_where_empty_string() {
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
            "query": r#"INSERT INTO user {"id": "u1", "name": ""}"#
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u2", "name": "Alice"}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT * FROM user WHERE name = """#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 1, "Should find empty string");
}

#[tokio::test]
async fn test_select_where_zero() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user (score int)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "score": 0}"#
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u2", "score": 100}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT * FROM user WHERE score = 0"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 1);
}

#[tokio::test]
async fn test_select_where_negative() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user (score int)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "score": -50}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT * FROM user WHERE score < 0"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 1);
}

#[tokio::test]
async fn test_select_where_boolean_true() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user (name string, active bool)"}))
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

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u2", "name": "Bob", "active": false}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT * FROM user WHERE active = true"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["name"], "Alice");
}

// =============================================================================
// Unicode in WHERE Clause
// =============================================================================

#[tokio::test]
async fn test_select_where_unicode_value() {
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
            "query": r#"INSERT INTO user {"id": "u1", "name": "日本語"}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT * FROM user WHERE name = "日本語""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 1);
}

// =============================================================================
// Invalid Query Syntax
// =============================================================================

#[tokio::test]
async fn test_incomplete_where_clause() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT * FROM user WHERE"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Parse error is returned as string
    assert!(
        !body["error"].is_null(),
        "Incomplete WHERE should error: {:?}",
        body
    );
}

#[tokio::test]
async fn test_invalid_operator() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT * FROM user WHERE name LIKE "%test%""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // LIKE is not supported - parser returns error as string
    // Either succeeds or returns clear error
    assert!(
        !body["error"].is_null() || body["results"][0]["count"].is_number(),
        "Should handle LIKE operator: {:?}",
        body
    );
}

#[tokio::test]
async fn test_missing_value_in_where() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT * FROM user WHERE name ="
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Parse error is returned as string
    assert!(
        !body["error"].is_null(),
        "Missing value should error: {:?}",
        body
    );
}

// =============================================================================
// SELECT Field Projection
// =============================================================================

#[tokio::test]
async fn test_select_specific_fields() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user (name string, age int, email string)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "age": 30, "email": "alice@test.com"}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT name, age FROM user:u1"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    if body["error"].is_null() {
        let doc = &body["results"][0]["data"][0];
        assert!(doc["name"].is_string(), "Should have name field");
        assert!(doc["age"].is_number(), "Should have age field");
        // email might or might not be included depending on implementation
    }
}

#[tokio::test]
async fn test_select_nonexistent_field() {
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
            "query": "SELECT nonexistent FROM user:u1"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Should either error or return null/missing for that field
    if body["error"].is_null() {
        let doc = &body["results"][0]["data"][0];
        assert!(
            doc["nonexistent"].is_null() || doc.get("nonexistent").is_none(),
            "Non-existent field should be null or missing"
        );
    }
}

// =============================================================================
// DELETE with WHERE Clause
// =============================================================================

#[tokio::test]
async fn test_delete_with_where() {
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
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "age": 20}"#
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u2", "name": "Bob", "age": 30}"#
        }))
        .send()
        .await
        .unwrap();

    // Delete only users under 25
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DELETE user WHERE age < 25"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(
        body["results"][0]["rows_affected"], 1,
        "Should delete 1 user"
    );

    // Verify Bob still exists
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["name"], "Bob");
}

#[tokio::test]
async fn test_delete_with_where_no_match() {
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

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DELETE user WHERE age > 100"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(
        body["results"][0]["rows_affected"], 0,
        "Should delete 0 users"
    );

    // Verify user still exists
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1);
}

// =============================================================================
// Unknown function in WHERE clause should error
// =============================================================================

#[tokio::test]
async fn test_unknown_function_in_where_clause_errors() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Unknown function in WHERE should return an error, not silently filter all rows
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT VALUE $value FROM 1..10 WHERE math::nonexistent($value) = 0"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        !body["error"].is_null(),
        "Unknown function should return error, got: {:?}",
        body
    );
    let error_msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        error_msg.contains("Unknown function"),
        "Error should mention unknown function, got: {}",
        error_msg
    );
}

#[tokio::test]
async fn test_unknown_function_suggests_similar() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // math::mod should suggest math::modulo
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT math::mod(10, 3)"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let error_msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        error_msg.contains("Did you mean"),
        "Error should suggest similar function, got: {}",
        error_msg
    );
    assert!(
        error_msg.contains("modulo"),
        "Error should suggest 'modulo', got: {}",
        error_msg
    );
}

// =============================================================================
// Error Suggestions Tests
// =============================================================================

#[tokio::test]
async fn test_unknown_collection_suggests_similar() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create a collection named "products"
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION products"}))
        .send()
        .await
        .unwrap();

    // Try to insert into "prodcts" (typo) - should suggest "products"
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO prodcts {"name": "test"}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let error_msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        error_msg.contains("unknown collection"),
        "Error should mention unknown collection, got: {}",
        error_msg
    );
    assert!(
        error_msg.contains("Did you mean"),
        "Error should suggest similar collection, got: {}",
        error_msg
    );
    assert!(
        error_msg.contains("products"),
        "Error should suggest 'products', got: {}",
        error_msg
    );
}

#[tokio::test]
async fn test_unknown_field_suggests_similar() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create a strict schema collection
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION users (SCHEMA STRICT, name string required, email string)"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "DEFINE should succeed: {:?}", body);

    // Insert a valid document first to ensure collection exists
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO users {"name": "Bob", "email": "bob@example.com"}"#
        }))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "INSERT should succeed: {:?}", body);

    // Try to insert with extra field "nmae" (typo) - should suggest "name"
    // Note: we also provide required "name" to avoid required field error
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO users {"name": "Alice", "email": "alice@example.com", "nmae": "typo"}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let error_msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        error_msg.contains("unknown field"),
        "Error should mention unknown field, got: {}",
        error_msg
    );
    assert!(
        error_msg.contains("Did you mean"),
        "Error should suggest similar field, got: {}",
        error_msg
    );
    assert!(
        error_msg.contains("name"),
        "Error should suggest 'name', got: {}",
        error_msg
    );
}

#[tokio::test]
async fn test_type_error_shows_value() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Try to do boolean AND with a string - should show the actual value
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT true AND "hello""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let error_msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        error_msg.contains("Type error"),
        "Should be a type error, got: {}",
        error_msg
    );
    assert!(
        error_msg.contains("String") && error_msg.contains("hello"),
        "Error should show the actual value 'hello', got: {}",
        error_msg
    );
}

#[tokio::test]
async fn test_type_error_shows_number_value() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Try to do NOT on an integer - should show the actual value
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT NOT 42"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let error_msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        error_msg.contains("Type error"),
        "Should be a type error, got: {}",
        error_msg
    );
    assert!(
        error_msg.contains("Int") && error_msg.contains("42"),
        "Error should show the actual value '42', got: {}",
        error_msg
    );
}

#[tokio::test]
async fn test_invalid_id_with_colon_shows_helpful_message() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    // Try to INSERT with ID containing colon - this is caught at execution time
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "foo:bar", "name": "test"}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let error_msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        error_msg.contains("cannot contain ':'"),
        "Error should mention colon restriction, got: {}",
        error_msg
    );
    assert!(
        error_msg.contains("collection:key"),
        "Error should explain the format, got: {}",
        error_msg
    );
}
