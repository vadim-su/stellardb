mod common;

use serde_json::json;

/// Helper to check if a response has an error (can be string or object)
fn has_error(body: &serde_json::Value) -> bool {
    let err = &body["error"];
    err.is_object() || err.is_string()
}

/// Helper to get the error message from a response
fn get_error_message(body: &serde_json::Value) -> String {
    let err = &body["error"];
    if let Some(s) = err.as_str() {
        s.to_string()
    } else if err.is_object() {
        err["message"].as_str().unwrap_or("").to_string()
    } else {
        String::new()
    }
}

#[tokio::test]
async fn test_collection_without_schema_allows_any_fields() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Insert document with arbitrary fields - should work (flexible by default)
    let res = client
        .post(format!("http://{}/collections/test/documents", addr))
        .json(&json!({"id": "test:1", "anything": "goes", "nested": {"deep": true}}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 201);

    // Verify the document was stored correctly
    let res = client
        .get(format!("http://{}/collections/test/documents/1", addr))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let doc: serde_json::Value = res.json().await.unwrap();
    assert_eq!(doc["anything"], "goes");
    assert_eq!(doc["nested"]["deep"], true);
}

#[tokio::test]
async fn test_define_collection_strict() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED, age int DEFAULT 0)"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["status"], "created");
    assert_eq!(body["results"][0]["data"][0]["collection"], "user");
}

#[tokio::test]
async fn test_define_collection_with_index() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, email string, INDEX ON email UNIQUE)"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["status"], "created");
}

#[tokio::test]
async fn test_describe_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // First define a collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED, age int DEFAULT 0)"
        }))
        .send()
        .await
        .unwrap();

    // Then describe it
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DESCRIBE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let data = &body["results"][0]["data"][0];
    assert_eq!(data["name"], "user");
    assert_eq!(data["mode"], "strict");
    assert_eq!(data["fields"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_describe_collections() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create a couple of collections
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION users (SCHEMA STRICT)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION posts"}))
        .send()
        .await
        .unwrap();

    // Describe all
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DESCRIBE COLLECTIONS"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let data = &body["results"][0]["data"][0];
    assert!(data["count"].as_u64().unwrap() >= 2);
}

#[tokio::test]
async fn test_drop_collection_empty() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define a collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION temp"}))
        .send()
        .await
        .unwrap();

    // Drop it (should succeed since empty)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DROP COLLECTION temp"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["status"], "dropped");
}

#[tokio::test]
async fn test_drop_collection_not_empty_without_cascade() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create a document (this creates the collection implicitly)
    client
        .post(format!("http://{}/collections/temp/documents", addr))
        .json(&json!({"id": "temp:1", "data": "test"}))
        .send()
        .await
        .unwrap();

    // Try to drop without CASCADE - should fail
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DROP COLLECTION temp"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    // Should have an error (structured error object)
    assert!(body["error"].is_object(), "Expected error: {:?}", body);
}

#[tokio::test]
async fn test_drop_collection_cascade() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create a document
    client
        .post(format!("http://{}/collections/temp/documents", addr))
        .json(&json!({"id": "temp:1", "data": "test"}))
        .send()
        .await
        .unwrap();

    // Drop with CASCADE - should succeed
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DROP COLLECTION temp CASCADE"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["status"], "dropped");

    // Verify collection is gone
    let res = client
        .get(format!("http://{}/collections/temp/documents/1", addr))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}

// =============================================================================
// Schema Binding Integration Tests
// =============================================================================

#[tokio::test]
async fn test_insert_with_strict_schema_validates_fields() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define a strict collection
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED, age int DEFAULT 0)"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    // Insert with valid fields - should work
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice"}"#
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Should not have error: {:?}", body);

    // Verify default was applied
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["age"], 0);
}

#[tokio::test]
async fn test_insert_strict_rejects_unknown_field() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define a strict collection
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED)"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    // Insert with unknown field - should fail
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "unknown": "field"}"#
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();

    // Should have an error about unknown field
    assert!(has_error(&body), "Should have error: {:?}", body);
    let error_msg = get_error_message(&body);
    assert!(
        error_msg.to_lowercase().contains("unknown") || error_msg.to_lowercase().contains("field"),
        "Error should mention unknown field: {}",
        error_msg
    );
}

#[tokio::test]
async fn test_insert_strict_rejects_missing_required() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define a strict collection
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED, email string REQUIRED)"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    // Insert missing required field - should fail
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice"}"#
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();

    // Should have an error about missing required field
    assert!(has_error(&body), "Should have error: {:?}", body);
    let error_msg = get_error_message(&body);
    assert!(
        error_msg.to_lowercase().contains("required")
            || error_msg.to_lowercase().contains("missing"),
        "Error should mention required/missing: {}",
        error_msg
    );
}

#[tokio::test]
async fn test_insert_flexible_allows_extra_fields() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define a flexible collection
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (name string REQUIRED)"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    // Insert with extra fields - should work
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "extra": "allowed"}"#
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Should not have error: {:?}", body);

    // Verify extra field is stored
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["extra"], "allowed");
}

#[tokio::test]
async fn test_create_validates_against_schema() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define a strict collection
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED, score int DEFAULT 100)"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    // CREATE with SET - should apply defaults
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"CREATE user:bob SET name = "Bob""#
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Should not have error: {:?}", body);

    // Verify default was applied
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:bob"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["score"], 100);
}

#[tokio::test]
async fn test_update_rejects_null_on_required() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define collection and insert document
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (name string REQUIRED)"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice"}"#
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    // Try to set required field to null - should fail
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "UPDATE user:u1 SET name = null"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();

    // Should have an error about null on required field
    assert!(has_error(&body), "Should have error: {:?}", body);
    let error_msg = get_error_message(&body);
    assert!(
        error_msg.to_lowercase().contains("null") || error_msg.to_lowercase().contains("required"),
        "Error should mention null/required: {}",
        error_msg
    );
}

#[tokio::test]
async fn test_insert_type_mismatch_rejected() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define collection with int field
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED, age int)"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    // Insert with wrong type for age - should fail
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO user {"id": "u1", "name": "Alice", "age": "not a number"}"#
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();

    // Should have an error about type mismatch
    assert!(has_error(&body), "Should have error: {:?}", body);
    let error_msg = get_error_message(&body);
    assert!(
        error_msg.to_lowercase().contains("type")
            || error_msg.to_lowercase().contains("mismatch")
            || error_msg.to_lowercase().contains("expected"),
        "Error should mention type/mismatch: {}",
        error_msg
    );
}

#[tokio::test]
async fn test_collection_without_schema_allows_anything() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // First define a flexible (schemaless) collection
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION freeform"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    // Insert with arbitrary fields - should work
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO freeform {"id": "f1", "any": "field", "works": true, "nested": {"deep": 1}}"#
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Should not have error: {:?}", body);

    // Verify everything is stored
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM freeform:f1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["any"], "field");
    assert_eq!(body["results"][0]["data"][0]["works"], true);
    assert_eq!(body["results"][0]["data"][0]["nested"]["deep"], 1);
}

#[tokio::test]
async fn test_insert_into_unknown_collection_fails() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Try to insert into undeclared collection - should fail
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO unknown {"id": "x1", "name": "test"}"#
        }))
        .send()
        .await
        .unwrap();

    // Should return error
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        has_error(&body) || !body["results"][0]["data"].is_null(),
        "Should have error for unknown collection: {:?}",
        body
    );
}

// =============================================================================
// DROP COLLECTION with Indexes Tests
// =============================================================================

#[tokio::test]
async fn test_drop_empty_collection_removes_indexes() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with index
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION test_idx"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "CREATE INDEX ON test_idx(name)"}))
        .send()
        .await
        .unwrap();

    // Verify index exists via DESCRIBE COLLECTION
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DESCRIBE COLLECTION test_idx"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        body["results"][0]["data"][0]["indexes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // DROP without CASCADE (collection is empty, so should work)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DROP COLLECTION test_idx"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["status"], "dropped");

    // Re-create collection and verify no indexes exist
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION test_idx"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DESCRIBE COLLECTION test_idx"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        body["results"][0]["data"][0]["indexes"]
            .as_array()
            .unwrap()
            .len(),
        0,
        "Indexes should be empty after DROP and re-create"
    );
}

#[tokio::test]
async fn test_drop_collection_cascade_removes_indexes() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with multiple indexes
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION test_cascade"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "CREATE INDEX ON test_cascade(name)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "CREATE INDEX ON test_cascade(age)"}))
        .send()
        .await
        .unwrap();

    // Insert some data
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"INSERT INTO test_cascade {id: "1", name: "Alice", age: 30}"#}))
        .send()
        .await
        .unwrap();

    // DROP with CASCADE
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DROP COLLECTION test_cascade CASCADE"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["status"], "dropped");

    // Re-create and verify clean state
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION test_cascade"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DESCRIBE COLLECTION test_cascade"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        body["results"][0]["data"][0]["indexes"]
            .as_array()
            .unwrap()
            .len(),
        0,
        "Indexes should be empty after CASCADE DROP"
    );
}

#[tokio::test]
async fn test_recreate_index_after_drop_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection, index, insert data
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION recreate_test"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "CREATE INDEX ON recreate_test(status)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"INSERT INTO recreate_test {id: "1", status: "active"}"#}))
        .send()
        .await
        .unwrap();

    // DROP CASCADE
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DROP COLLECTION recreate_test CASCADE"}))
        .send()
        .await
        .unwrap();

    // Re-create collection and index
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION recreate_test"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "CREATE INDEX ON recreate_test(status)"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        body["results"][0]["data"][0]["status"], "created",
        "Should be able to create index after DROP: {:?}",
        body
    );

    // Insert new data and query using index
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"INSERT INTO recreate_test {id: "2", status: "pending"}"#}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"SELECT * FROM recreate_test WHERE status = "pending""#}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        body["results"][0]["data"].as_array().unwrap().len(),
        1,
        "Index query should find the document"
    );
}

#[tokio::test]
async fn test_drop_collection_with_compound_index() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with compound index
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION compound_test"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "CREATE INDEX ON compound_test(status, age)"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(
            &json!({"query": r#"INSERT INTO compound_test {id: "1", status: "active", age: 25}"#}),
        )
        .send()
        .await
        .unwrap();

    // DROP CASCADE
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DROP COLLECTION compound_test CASCADE"}))
        .send()
        .await
        .unwrap();

    // Re-create and verify clean state
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION compound_test"}))
        .send()
        .await
        .unwrap();

    // Should be able to create same compound index again
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "CREATE INDEX ON compound_test(status, age)"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        body["results"][0]["data"][0]["status"], "created",
        "Should create compound index after DROP: {:?}",
        body
    );
}

// =============================================================================
// Index Type Tests
// =============================================================================

#[tokio::test]
async fn test_index_type_btree_default() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION post"}))
        .send()
        .await
        .unwrap();

    // Create a regular BTree index (default)
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "CREATE INDEX ON post(title)"}))
        .send()
        .await
        .unwrap();

    // Verify index type is BTree
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DESCRIBE COLLECTION post"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let indexes = body["results"][0]["data"][0]["indexes"].as_array().unwrap();
    assert_eq!(indexes.len(), 1);
    assert_eq!(indexes[0]["fields"], json!(["title"]));
    assert_eq!(indexes[0]["index_type"], "BTree");
}

#[tokio::test]
async fn test_index_type_fulltext() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION post"}))
        .send()
        .await
        .unwrap();

    // Create a full-text search index
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "CREATE INDEX ON post(title, body) FULLTEXT"}))
        .send()
        .await
        .unwrap();

    // Verify index type is FullText
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DESCRIBE COLLECTION post"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let indexes = body["results"][0]["data"][0]["indexes"].as_array().unwrap();
    assert_eq!(indexes.len(), 1);
    assert_eq!(indexes[0]["fields"], json!(["title", "body"]));
    assert!(!indexes[0]["unique"].as_bool().unwrap());
    assert_eq!(indexes[0]["index_type"], "FullText");
}

#[tokio::test]
async fn test_index_type_unique_btree() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    // Create a unique BTree index
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "CREATE INDEX ON user(email) UNIQUE"}))
        .send()
        .await
        .unwrap();

    // Verify index type is BTree and unique
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DESCRIBE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let indexes = body["results"][0]["data"][0]["indexes"].as_array().unwrap();
    assert_eq!(indexes.len(), 1);
    assert_eq!(indexes[0]["fields"], json!(["email"]));
    assert!(indexes[0]["unique"].as_bool().unwrap());
    assert_eq!(indexes[0]["index_type"], "BTree");
}

// =============================================================================
// New Field Type Tests: Any, Union, Duration, Bytes
// =============================================================================

/// Helper function for executing SQL queries
async fn query(
    client: &reqwest::Client,
    addr: &std::net::SocketAddr,
    sql: &str,
) -> serde_json::Value {
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": sql}))
        .send()
        .await
        .unwrap();
    res.json().await.unwrap()
}

#[tokio::test]
async fn test_schema_type_any() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define collection with TYPE any
    let result = query(
        &client,
        &addr,
        "DEFINE COLLECTION events (SCHEMA STRICT, id string, payload any)",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);

    // Insert string payload
    let result = query(
        &client,
        &addr,
        r#"CREATE events:1 SET id = "e1", payload = "string""#,
    )
    .await;
    assert!(
        result["error"].is_null(),
        "Error inserting string: {:?}",
        result
    );

    // Insert int payload
    let result = query(
        &client,
        &addr,
        r#"CREATE events:2 SET id = "e2", payload = 123"#,
    )
    .await;
    assert!(
        result["error"].is_null(),
        "Error inserting int: {:?}",
        result
    );

    // Insert array payload
    let result = query(
        &client,
        &addr,
        r#"CREATE events:3 SET id = "e3", payload = [1, 2, 3]"#,
    )
    .await;
    assert!(
        result["error"].is_null(),
        "Error inserting array: {:?}",
        result
    );

    // Insert object payload
    let result = query(
        &client,
        &addr,
        r#"CREATE events:4 SET id = "e4", payload = {nested: true}"#,
    )
    .await;
    assert!(
        result["error"].is_null(),
        "Error inserting object: {:?}",
        result
    );

    // Verify all values were stored
    let result = query(&client, &addr, "SELECT * FROM events ORDER id").await;
    let data = &result["results"][0]["data"];
    assert_eq!(data.as_array().unwrap().len(), 4);
}

#[tokio::test]
async fn test_schema_union_type() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define collection with union type
    let result = query(
        &client,
        &addr,
        "DEFINE COLLECTION config (SCHEMA STRICT, key string, value string | int | bool)",
    )
    .await;
    assert!(result["error"].is_null(), "Error defining: {:?}", result);

    // Insert string value
    let result = query(
        &client,
        &addr,
        r#"CREATE config:1 SET key = "name", value = "test""#,
    )
    .await;
    assert!(
        result["error"].is_null(),
        "Error inserting string: {:?}",
        result
    );

    // Insert int value
    let result = query(
        &client,
        &addr,
        r#"CREATE config:2 SET key = "count", value = 42"#,
    )
    .await;
    assert!(
        result["error"].is_null(),
        "Error inserting int: {:?}",
        result
    );

    // Insert bool value
    let result = query(
        &client,
        &addr,
        r#"CREATE config:3 SET key = "enabled", value = true"#,
    )
    .await;
    assert!(
        result["error"].is_null(),
        "Error inserting bool: {:?}",
        result
    );

    // Insert invalid value (array not in union) - should fail
    let result = query(
        &client,
        &addr,
        r#"CREATE config:4 SET key = "bad", value = [1, 2]"#,
    )
    .await;
    assert!(has_error(&result), "Should fail: array not in union");
}

#[tokio::test]
async fn test_schema_datetime_type_defined() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define collection with datetime type
    let result = query(
        &client,
        &addr,
        "DEFINE COLLECTION logs (timestamp datetime, message string)",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);

    // Describe to verify type
    let result = query(&client, &addr, "DESCRIBE COLLECTION logs").await;
    let fields = result["results"][0]["data"][0]["fields"]
        .as_array()
        .unwrap();
    assert!(
        fields
            .iter()
            .any(|f| f["name"] == "timestamp" && f["type"] == "datetime")
    );
}

#[tokio::test]
async fn test_schema_duration_type() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define collection with duration type
    let result = query(
        &client,
        &addr,
        "DEFINE COLLECTION timers (SCHEMA STRICT, name string, interval duration)",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);

    // Describe to verify type
    let result = query(&client, &addr, "DESCRIBE COLLECTION timers").await;
    let fields = result["results"][0]["data"][0]["fields"]
        .as_array()
        .unwrap();
    assert!(
        fields
            .iter()
            .any(|f| f["name"] == "interval" && f["type"] == "duration")
    );
}

#[tokio::test]
async fn test_schema_bytes_type() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define collection with bytes type
    let result = query(
        &client,
        &addr,
        "DEFINE COLLECTION files (SCHEMA STRICT, name string, content bytes)",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);

    // Describe to verify type
    let result = query(&client, &addr, "DESCRIBE COLLECTION files").await;
    let fields = result["results"][0]["data"][0]["fields"]
        .as_array()
        .unwrap();
    assert!(
        fields
            .iter()
            .any(|f| f["name"] == "content" && f["type"] == "bytes")
    );
}

#[tokio::test]
async fn test_schema_range_type() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define collection with range type
    let result = query(
        &client,
        &addr,
        "DEFINE COLLECTION schedules (SCHEMA STRICT, name string, hours range<int>)",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);

    // Describe to verify type
    let result = query(&client, &addr, "DESCRIBE COLLECTION schedules").await;
    let fields = result["results"][0]["data"][0]["fields"]
        .as_array()
        .unwrap();
    assert!(
        fields
            .iter()
            .any(|f| f["name"] == "hours" && f["type"] == "range<int>")
    );
}

#[tokio::test]
async fn test_schema_union_with_array() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define collection with union including array type
    let result = query(
        &client,
        &addr,
        "DEFINE COLLECTION mixed (SCHEMA STRICT, id string, data string | [int])",
    )
    .await;
    assert!(result["error"].is_null(), "Error defining: {:?}", result);

    // Insert string
    let result = query(
        &client,
        &addr,
        r#"CREATE mixed:1 SET id = "a", data = "hello""#,
    )
    .await;
    assert!(
        result["error"].is_null(),
        "Error inserting string: {:?}",
        result
    );

    // Insert array of ints
    let result = query(
        &client,
        &addr,
        r#"CREATE mixed:2 SET id = "b", data = [1, 2, 3]"#,
    )
    .await;
    assert!(
        result["error"].is_null(),
        "Error inserting array: {:?}",
        result
    );
}

#[tokio::test]
async fn test_schema_describe_shows_union_types() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define collection with union type
    let result = query(
        &client,
        &addr,
        "DEFINE COLLECTION flexible_config (key string, value string | int | bool)",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);

    // Describe to verify type formatting
    let result = query(&client, &addr, "DESCRIBE COLLECTION flexible_config").await;
    let fields = result["results"][0]["data"][0]["fields"]
        .as_array()
        .unwrap();
    let value_field = fields.iter().find(|f| f["name"] == "value").unwrap();
    // Should show as union type
    let type_str = value_field["type"].as_str().unwrap();
    assert!(
        type_str.contains("|") || type_str.contains("string") && type_str.contains("int"),
        "Expected union type, got: {}",
        type_str
    );
}
