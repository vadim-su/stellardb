// Collection edge case tests
// Tests for non-existent collections, invalid names, and collection operations

use crate::common;
use serde_json::json;

// =============================================================================
// Non-existent Collection Tests
// =============================================================================

#[tokio::test]
async fn test_select_from_nonexistent_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM nonexistent"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Should either return empty result or error
    if body["error"].is_null() {
        assert_eq!(
            body["results"][0]["count"], 0,
            "Non-existent collection should return 0 results"
        );
    } else {
        let error_msg = body["error"]["message"].as_str().unwrap_or("");
        assert!(
            error_msg.to_lowercase().contains("not found")
                || error_msg.to_lowercase().contains("does not exist")
                || error_msg.to_lowercase().contains("unknown"),
            "Error should indicate collection not found: {}",
            error_msg
        );
    }
}

#[tokio::test]
async fn test_update_in_nonexistent_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"UPDATE nonexistent:doc1 SET name = "test""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Should fail - can't update in non-existent collection
    assert!(
        common::has_error(&body) || body["results"][0]["count"] == 0,
        "Should fail or affect 0 docs: {:?}",
        body
    );
}

#[tokio::test]
async fn test_delete_from_nonexistent_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DELETE nonexistent:doc1"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Should fail or report 0 affected
    // DELETE uses rows_affected, not count
    assert!(
        common::has_error(&body) || body["results"][0]["rows_affected"] == 0,
        "Should fail or affect 0 docs: {:?}",
        body
    );
}

#[tokio::test]
async fn test_describe_nonexistent_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DESCRIBE COLLECTION nonexistent"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "Should error on describing non-existent collection: {:?}",
        body
    );
}

#[tokio::test]
async fn test_drop_nonexistent_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DROP COLLECTION nonexistent"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // DROP is idempotent - returns status: "not_found" for non-existent collection
    // This is valid behavior (similar to "IF EXISTS" semantics)
    if body["error"].is_null() {
        assert_eq!(
            body["results"][0]["data"][0]["status"].as_str(),
            Some("not_found"),
            "Should indicate collection was not found: {:?}",
            body
        );
    }
}

// =============================================================================
// Collection Name Validation Tests
// =============================================================================

#[tokio::test]
async fn test_empty_collection_name() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION \"\""}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Parse error is returned as string
    assert!(
        !body["error"].is_null(),
        "Should reject empty collection name: {:?}",
        body
    );
}

#[tokio::test]
async fn test_collection_name_with_spaces() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION \"my collection\""}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Document whether spaces are allowed in quoted names
    if body["error"].is_null() {
        // If allowed, verify it works
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "DESCRIBE COLLECTION \"my collection\""}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(body["error"].is_null() || body["results"][0]["data"][0]["name"].is_string());
    }
}

#[tokio::test]
async fn test_collection_name_with_special_chars() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let special_names = vec!["user-data", "user_data", "user.data", "user@data"];

    for name in special_names {
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": format!("DEFINE COLLECTION {}", name)
            }))
            .send()
            .await
            .unwrap();

        let body: serde_json::Value = res.json().await.unwrap();
        // Document which characters are allowed
        // Just ensure we get a clear response (success or error)
        assert!(
            !body["error"].is_null() || body["results"][0]["data"][0]["status"].is_string(),
            "Should handle collection name '{}': {:?}",
            name,
            body
        );
    }
}

#[tokio::test]
async fn test_collection_name_starting_with_number() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION 123users"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Most databases don't allow names starting with numbers
    // Document actual behavior
    if common::has_error(&body) {
        let error_msg = common::get_error_message(&body);
        assert!(
            error_msg.to_lowercase().contains("invalid")
                || error_msg.to_lowercase().contains("identifier")
                || error_msg.to_lowercase().contains("syntax")
                || error_msg.to_lowercase().contains("parse"),
            "Should indicate invalid identifier: {}",
            error_msg
        );
    }
}

#[tokio::test]
async fn test_collection_name_with_unicode() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION ユーザー"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Unicode collection names are supported
    assert!(
        body["error"].is_null(),
        "Should allow Unicode collection name: {:?}",
        body
    );

    // Verify the collection can be described
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DESCRIBE COLLECTION ユーザー"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should describe Unicode collection: {:?}",
        body
    );

    // Verify we can insert and select from Unicode collection
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO ユーザー {"id": "太郎", "名前": "田中"}"#
        }))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should insert into Unicode collection: {:?}",
        body
    );

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM ユーザー:太郎"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["名前"], "田中");
}

#[tokio::test]
async fn test_very_long_collection_name() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Test that 256 chars fails (max is 120)
    let long_name = "a".repeat(256);
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": format!("DEFINE COLLECTION {}", long_name)
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(common::has_error(&body), "Should reject name > 120 chars");
    let error_msg = common::get_error_message(&body);
    assert!(
        error_msg.to_lowercase().contains("exceed")
            || error_msg.to_lowercase().contains("length")
            || error_msg.to_lowercase().contains("too long"),
        "Should indicate name too long: {}",
        error_msg
    );

    // Test that exactly 120 chars works
    let max_name = "b".repeat(120);
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": format!("DEFINE COLLECTION {}", max_name)
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        !common::has_error(&body),
        "Should accept 120 char name: {:?}",
        body
    );

    // Test that 121 chars fails
    let over_max_name = "c".repeat(121);
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": format!("DEFINE COLLECTION {}", over_max_name)
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(common::has_error(&body), "Should reject name > 120 chars");
}

// =============================================================================
// Reserved Name Tests
// =============================================================================

#[tokio::test]
async fn test_collection_name_sql_keyword() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Test SQL reserved words
    let reserved_words = vec!["SELECT", "FROM", "WHERE", "INSERT", "DELETE", "UPDATE"];

    for word in reserved_words {
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": format!("DEFINE COLLECTION {}", word)
            }))
            .send()
            .await
            .unwrap();

        let body: serde_json::Value = res.json().await.unwrap();
        // Should either reject or require quoting
        // Document actual behavior
        assert!(
            common::has_error(&body) || body["results"][0]["data"][0]["status"].is_string(),
            "Should handle reserved word '{}' as collection name: {:?}",
            word,
            body
        );
    }
}

// =============================================================================
// Collection Definition Edge Cases
// =============================================================================

#[tokio::test]
async fn test_redefine_existing_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define collection first time
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (name string REQUIRED)"
        }))
        .send()
        .await
        .unwrap();

    // Try to redefine with different schema
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (email string REQUIRED)"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Should either fail or update the schema
    // Document actual behavior
    if body["error"].is_null() {
        // If allowed, check what schema is active
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "DESCRIBE COLLECTION user"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        let fields = &body["results"][0]["data"][0]["fields"];
        // Check if it has the new schema or old one
        assert!(fields.is_array());
    }
}

#[tokio::test]
async fn test_define_collection_with_duplicate_fields() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (name string, name int)"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Should reject duplicate field names
    assert!(
        common::has_error(&body),
        "Should reject duplicate field names: {:?}",
        body
    );
}

#[tokio::test]
async fn test_define_collection_with_invalid_type() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (name unknowntype)"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Should reject unknown type - parser returns error
    assert!(
        !body["error"].is_null(),
        "Should reject unknown field type: {:?}",
        body
    );
}

#[tokio::test]
async fn test_define_collection_with_default_wrong_type() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (age int DEFAULT \"not a number\")"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Default value type should match field type
    assert!(
        common::has_error(&body),
        "Should reject default value with wrong type: {:?}",
        body
    );
}

// =============================================================================
// HTTP REST API Collection Tests
// =============================================================================

#[tokio::test]
async fn test_http_list_nonexistent_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .get(format!("http://{}/collections/nonexistent/documents", addr))
        .send()
        .await
        .unwrap();

    // Should return 404 or empty list
    let status = res.status();
    assert!(
        status == 404 || status == 200,
        "Should return 404 or 200: {}",
        status
    );

    if status == 200 {
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(
            body["documents"]
                .as_array()
                .map(|a| a.is_empty())
                .unwrap_or(true),
            "Non-existent collection should be empty"
        );
    }
}

#[tokio::test]
async fn test_http_get_from_nonexistent_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .get(format!(
            "http://{}/collections/nonexistent/documents/doc1",
            addr
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 404, "Should return 404 for non-existent doc");
}

#[tokio::test]
async fn test_http_delete_from_nonexistent_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .delete(format!(
            "http://{}/collections/nonexistent/documents/doc1",
            addr
        ))
        .send()
        .await
        .unwrap();

    // Should return 404 - document doesn't exist
    assert_eq!(res.status(), 404);
}

// =============================================================================
// Case Sensitivity Tests
// =============================================================================

#[tokio::test]
async fn test_collection_name_case_sensitivity() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define lowercase
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION users"}))
        .send()
        .await
        .unwrap();

    // Insert into lowercase
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO users {"id": "u1", "name": "Alice"}"#
        }))
        .send()
        .await
        .unwrap();

    // Try to query with different case
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM USERS"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Document case sensitivity behavior
    let count = body["results"][0]["count"].as_u64().unwrap_or(0);
    // Either finds the document (case-insensitive) or returns 0 (case-sensitive)
    assert!(
        count == 0 || count == 1,
        "Should return 0 or 1 documents: {:?}",
        body
    );
}

// =============================================================================
// Empty Collection Operations
// =============================================================================

#[tokio::test]
async fn test_select_all_from_empty_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION empty"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM empty"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should succeed on empty collection"
    );
    assert_eq!(body["results"][0]["count"], 0);
    assert!(
        body["results"][0]["data"]
            .as_array()
            .map(|a| a.is_empty())
            .unwrap_or(true)
    );
}

#[tokio::test]
async fn test_delete_all_from_empty_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION empty"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DELETE empty"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should succeed on empty collection: {:?}",
        body
    );
    // DELETE returns rows_affected, not count
    assert_eq!(body["results"][0]["rows_affected"], 0);
}

#[tokio::test]
async fn test_update_all_in_empty_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION empty (name string)"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"UPDATE empty SET name = "test""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should succeed on empty collection"
    );
    assert_eq!(body["results"][0]["count"], 0);
}

// =============================================================================
// DROP COLLECTION and Index Cleanup Tests
// =============================================================================

/// Test that recreating a collection after DROP works correctly
/// This was broken when DROP created tombstones instead of using clear()
#[tokio::test]
async fn test_drop_and_recreate_collection_multiple_times() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    for i in 0..3 {
        // Define collection
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "DEFINE COLLECTION cycle_test (name string)"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(
            body["error"].is_null(),
            "Iteration {}: Define should succeed: {:?}",
            i,
            body
        );

        // Insert some documents
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": format!(r#"INSERT INTO cycle_test {{"id": "doc{}", "name": "test{}"}}"#, i, i)
            }))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(
            body["error"].is_null(),
            "Iteration {}: Insert should succeed: {:?}",
            i,
            body
        );

        // Verify document exists
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "SELECT * FROM cycle_test"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(
            body["results"][0]["count"], 1,
            "Iteration {}: Should have 1 document",
            i
        );

        // Drop with cascade
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "DROP COLLECTION cycle_test CASCADE"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(
            body["error"].is_null(),
            "Iteration {}: Drop should succeed: {:?}",
            i,
            body
        );
    }
}

/// Test that recreating indexes after DROP COLLECTION works correctly
#[tokio::test]
async fn test_drop_and_recreate_collection_with_indexes() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    for i in 0..3 {
        // Define collection
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "DEFINE COLLECTION idx_cycle (status string)"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(
            body["error"].is_null(),
            "Iteration {}: Define should succeed: {:?}",
            i,
            body
        );

        // Create index separately
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "CREATE INDEX ON idx_cycle(status)"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(
            body["error"].is_null(),
            "Iteration {}: Create index should succeed: {:?}",
            i,
            body
        );

        // Insert documents
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": r#"INSERT INTO idx_cycle {"id": "a", "status": "active"}, {"id": "b", "status": "inactive"}, {"id": "c", "status": "active"}"#
            }))
            .send()
            .await
            .unwrap();

        // Query using index
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": r#"SELECT * FROM idx_cycle WHERE status = "active""#
            }))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(
            body["results"][0]["count"], 2,
            "Iteration {}: Should find 2 active",
            i
        );

        // Drop with cascade
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "DROP COLLECTION idx_cycle CASCADE"}))
            .send()
            .await
            .unwrap();
    }
}

/// Test that CREATE INDEX after DROP COLLECTION works correctly
#[tokio::test]
async fn test_create_index_after_drop_recreate() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    for i in 0..2 {
        // Define collection without index
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "DEFINE COLLECTION idx_later"}))
            .send()
            .await
            .unwrap();

        // Insert documents first
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": r#"INSERT INTO idx_later {"id": "a", "category": "widgets"}, {"id": "b", "category": "gadgets"}"#
            }))
            .send()
            .await
            .unwrap();

        // Create index on existing data
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "CREATE INDEX ON idx_later(category)"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(
            body["error"].is_null(),
            "Iteration {}: Create index should succeed: {:?}",
            i,
            body
        );

        // Verify index works
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": r#"SELECT * FROM idx_later WHERE category = "widgets""#
            }))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(
            body["results"][0]["count"], 1,
            "Iteration {}: Should find 1 widget",
            i
        );

        // Drop with cascade
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "DROP COLLECTION idx_later CASCADE"}))
            .send()
            .await
            .unwrap();
    }
}

/// Test that schemaless collections (created via REST API) can be dropped
#[tokio::test]
async fn test_drop_schemaless_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection implicitly via REST API (POST creates new doc)
    let res = client
        .post(format!(
            "http://{}/collections/schemaless_test/documents",
            addr
        ))
        .json(&json!({"id": "schemaless_test:doc1", "name": "test", "value": 42}))
        .send()
        .await
        .unwrap();
    assert!(
        res.status().is_success(),
        "POST should succeed: status={}",
        res.status()
    );

    // Verify document exists
    let res = client
        .get(format!(
            "http://{}/collections/schemaless_test/documents/doc1",
            addr
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    // Drop collection with cascade
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DROP COLLECTION schemaless_test CASCADE"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Drop should succeed: {:?}", body);
    assert_eq!(
        body["results"][0]["data"][0]["status"].as_str(),
        Some("dropped")
    );

    // Verify collection is gone
    let res = client
        .get(format!(
            "http://{}/collections/schemaless_test/documents/doc1",
            addr
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404, "Document should be gone after drop");
}

/// Test DROP COLLECTION with compound indexes
#[tokio::test]
async fn test_drop_collection_with_compound_index() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    for i in 0..2 {
        // Define with compound index
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "DEFINE COLLECTION compound_test"}))
            .send()
            .await
            .unwrap();

        // Create compound index
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "CREATE INDEX ON compound_test(category, status)"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(
            body["error"].is_null(),
            "Iteration {}: Create compound index should succeed: {:?}",
            i,
            body
        );

        // Insert data
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": r#"INSERT INTO compound_test {"id": "a", "category": "A", "status": "active"}"#
            }))
            .send()
            .await
            .unwrap();

        // Query with compound index
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": r#"SELECT * FROM compound_test WHERE category = "A" AND status = "active""#
            }))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(
            body["results"][0]["count"], 1,
            "Iteration {}: Should find 1",
            i
        );

        // Drop with cascade
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "DROP COLLECTION compound_test CASCADE"}))
            .send()
            .await
            .unwrap();
    }
}

/// Test that DROP without CASCADE fails for non-empty collection
#[tokio::test]
async fn test_drop_non_empty_collection_without_cascade() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define and populate
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION nonempty"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO nonempty {"id": "doc1", "data": "important"}"#
        }))
        .send()
        .await
        .unwrap();

    // Try to drop without cascade
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DROP COLLECTION nonempty"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "Should fail without CASCADE: {:?}",
        body
    );
}

/// Test that DROP without CASCADE succeeds for empty collection
#[tokio::test]
async fn test_drop_empty_collection_without_cascade() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define empty collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION truly_empty"}))
        .send()
        .await
        .unwrap();

    // Drop without cascade should succeed
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DROP COLLECTION truly_empty"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Should succeed for empty collection: {:?}",
        body
    );
    assert_eq!(
        body["results"][0]["data"][0]["status"].as_str(),
        Some("dropped")
    );
}

/// Test DESCRIBE COLLECTION after DROP COLLECTION
#[tokio::test]
async fn test_describe_collection_after_drop() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION idx_show_test (field1 string)"}))
        .send()
        .await
        .unwrap();

    // Create index separately
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "CREATE INDEX ON idx_show_test(field1)"}))
        .send()
        .await
        .unwrap();

    // Verify index exists via DESCRIBE COLLECTION
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DESCRIBE COLLECTION idx_show_test"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["results"][0]["data"][0]["indexes"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "Should have indexes before drop: {:?}",
        body
    );

    // Drop collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DROP COLLECTION idx_show_test CASCADE"}))
        .send()
        .await
        .unwrap();

    // DESCRIBE COLLECTION should fail (collection doesn't exist)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DESCRIBE COLLECTION idx_show_test"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "DESCRIBE COLLECTION should fail after drop: {:?}",
        body
    );
}

/// Test creating the same index twice returns proper error
#[tokio::test]
async fn test_create_duplicate_index() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION dup_idx"}))
        .send()
        .await
        .unwrap();

    // Create index first time
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "CREATE INDEX ON dup_idx(field1)"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "First create should succeed: {:?}",
        body
    );

    // Try to create same index again
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "CREATE INDEX ON dup_idx(field1)"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "Duplicate index should fail: {:?}",
        body
    );
    let error_msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        error_msg.to_lowercase().contains("already exists")
            || error_msg.to_lowercase().contains("duplicate"),
        "Error should mention already exists: {}",
        error_msg
    );
}

/// Test DROP INDEX then CREATE same INDEX
#[tokio::test]
async fn test_drop_and_recreate_index() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION recreate_idx"}))
        .send()
        .await
        .unwrap();

    // Insert data first
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO recreate_idx {"id": "a", "tag": "important"}, {"id": "b", "tag": "normal"}"#
        }))
        .send()
        .await
        .unwrap();

    for i in 0..3 {
        // Create index
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "CREATE INDEX ON recreate_idx(tag)"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(
            body["error"].is_null(),
            "Iteration {}: Create should succeed: {:?}",
            i,
            body
        );

        // Verify index works
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": r#"SELECT * FROM recreate_idx WHERE tag = "important""#
            }))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(
            body["results"][0]["count"], 1,
            "Iteration {}: Should find 1",
            i
        );

        // Drop index
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "DROP INDEX idx_recreate_idx_btree_tag ON recreate_idx"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(
            body["error"].is_null(),
            "Iteration {}: Drop should succeed: {:?}",
            i,
            body
        );
    }
}
