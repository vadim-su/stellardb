// UPSERT statement integration tests
// Tests for merge (default), replace mode, expressions, nested fields, and idempotency

use crate::common;

// =============================================================================
// Basic UPSERT
// =============================================================================

#[tokio::test]
async fn test_upsert_creates_new_document() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    // UPSERT non-existing document
    let body: serde_json::Value = client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "UPSERT user:alice SET name = \"Alice\", age = 30"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(body["error"].is_null(), "unexpected error: {body}");
    assert_eq!(body["results"][0]["data"][0]["name"], "Alice");
    assert_eq!(body["results"][0]["data"][0]["age"], 30);

    // Verify with SELECT
    let body: serde_json::Value = client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "SELECT * FROM user:alice"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["results"][0]["data"][0]["name"], "Alice");
    assert_eq!(body["results"][0]["data"][0]["age"], 30);
}

#[tokio::test]
async fn test_upsert_merges_existing_document() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    // Create initial document
    client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "CREATE user:alice SET name = \"Alice\", age = 30, email = \"alice@example.com\""}))
        .send()
        .await
        .unwrap();

    // UPSERT with merge (default) - only update age
    let body: serde_json::Value = client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "UPSERT user:alice SET age = 31"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(body["error"].is_null(), "unexpected error: {body}");

    // Verify merge: name and email should still exist
    let body: serde_json::Value = client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "SELECT * FROM user:alice"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["results"][0]["data"][0]["name"], "Alice");
    assert_eq!(body["results"][0]["data"][0]["age"], 31);
    assert_eq!(body["results"][0]["data"][0]["email"], "alice@example.com");
}

// =============================================================================
// UPSERT REPLACE Mode
// =============================================================================

#[tokio::test]
async fn test_upsert_replace_overwrites_document() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    // Create initial document with multiple fields
    client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "CREATE user:alice SET name = \"Alice\", age = 30, email = \"alice@example.com\""}))
        .send()
        .await
        .unwrap();

    // UPSERT with REPLACE - overwrites everything
    let body: serde_json::Value = client
        .post(format!("http://{addr}/sql"))
        .json(
            &serde_json::json!({"query": "UPSERT user:alice REPLACE SET name = \"Alice Updated\""}),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(body["error"].is_null(), "unexpected error: {body}");

    // Verify replace: only name should exist, age and email gone
    let body: serde_json::Value = client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "SELECT * FROM user:alice"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["results"][0]["data"][0]["name"], "Alice Updated");
    assert!(body["results"][0]["data"][0]["age"].is_null());
    assert!(body["results"][0]["data"][0]["email"].is_null());
}

#[tokio::test]
async fn test_upsert_replace_creates_new_document() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    // UPSERT REPLACE on non-existing doc -- should create
    let body: serde_json::Value = client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "UPSERT user:bob REPLACE SET name = \"Bob\""}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(body["error"].is_null(), "unexpected error: {body}");
    assert_eq!(body["results"][0]["data"][0]["name"], "Bob");
}

// =============================================================================
// UPSERT with Expressions
// =============================================================================

#[tokio::test]
async fn test_upsert_with_expressions() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "DEFINE COLLECTION counter"}))
        .send()
        .await
        .unwrap();

    // Create initial document
    client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "CREATE counter:hits SET count = 10"}))
        .send()
        .await
        .unwrap();

    // UPSERT with arithmetic expression
    let body: serde_json::Value = client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "UPSERT counter:hits SET count = count + 1"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(body["error"].is_null(), "unexpected error: {body}");
    assert_eq!(body["results"][0]["data"][0]["count"], 11);
}

// =============================================================================
// UPSERT with Nested Fields
// =============================================================================

#[tokio::test]
async fn test_upsert_nested_fields() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    // Create document with nested structure using object literal
    client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "CREATE user:alice SET profile = {\"name\": \"Alice\", \"city\": \"NYC\"}"}))
        .send()
        .await
        .unwrap();

    // UPSERT: update only nested city, keep name
    let body: serde_json::Value = client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "UPSERT user:alice SET profile.city = \"LA\""}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(body["error"].is_null(), "unexpected error: {body}");

    let body: serde_json::Value = client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "SELECT * FROM user:alice"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["results"][0]["data"][0]["profile"]["name"], "Alice");
    assert_eq!(body["results"][0]["data"][0]["profile"]["city"], "LA");
}

// =============================================================================
// UPSERT Idempotency
// =============================================================================

#[tokio::test]
async fn test_upsert_idempotent() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    // Run same UPSERT twice -- should be idempotent
    for _ in 0..2 {
        let body: serde_json::Value = client
            .post(format!("http://{addr}/sql"))
            .json(&serde_json::json!({"query": "UPSERT user:alice SET name = \"Alice\", age = 30"}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert!(body["error"].is_null(), "unexpected error: {body}");
        assert_eq!(body["results"][0]["data"][0]["name"], "Alice");
        assert_eq!(body["results"][0]["data"][0]["age"], 30);
    }

    // Should still be one document
    let body: serde_json::Value = client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "SELECT * FROM user"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["results"][0]["count"], 1);
}
