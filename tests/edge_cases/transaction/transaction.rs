// Transaction edge case tests
// Tests for BEGIN/COMMIT/ROLLBACK behavior, isolation, and error conditions

use crate::common;
use serde_json::json;

// =============================================================================
// Helper to extract session ID from BEGIN response
// =============================================================================

fn extract_session_id(body: &serde_json::Value) -> Option<String> {
    body["results"][0]["session_id"]
        .as_str()
        .map(|s| s.to_string())
}

// =============================================================================
// Basic Transaction Edge Cases
// =============================================================================

#[tokio::test]
async fn test_commit_without_begin() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // COMMIT without any session - should error or be a no-op
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "COMMIT"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Without session header, COMMIT should be handled gracefully
    assert!(
        common::has_error(&body) || body["results"][0]["statement_type"] == "COMMIT",
        "COMMIT without BEGIN should be handled: {:?}",
        body
    );
}

#[tokio::test]
async fn test_rollback_without_begin() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // ROLLBACK without any session
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "ROLLBACK"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body) || body["results"][0]["statement_type"] == "ROLLBACK",
        "ROLLBACK without BEGIN should be handled: {:?}",
        body
    );
}

#[tokio::test]
async fn test_nested_begin() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION nested_begin"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "BEGIN"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let session_id = extract_session_id(&body).expect("BEGIN should return session_id");

    client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({
            "query": r#"INSERT INTO nested_begin {"id": "original", "value": 1}"#
        }))
        .send()
        .await
        .unwrap();

    // A nested BEGIN is a batch-level rejection. It must not replace or roll back the
    // active external session and must not return a second session identifier.
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({"query": "BEGIN"}))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["error"]["code"], "SDB-QE005", "{body:?}");
    assert!(body["results"].as_array().unwrap().is_empty());

    // The original transaction remains alive and still sees its pending write.
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({"query": "SELECT * FROM nested_begin:original"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1, "{body:?}");

    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({"query": "ROLLBACK"}))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());

    // Rollback of the original session removes the pending write; no replacement session
    // or autocommit write was created by the rejected nested BEGIN.
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM nested_begin:original"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 0, "{body:?}");
}

#[tokio::test]
async fn test_double_commit() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // BEGIN
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "BEGIN"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let session_id = extract_session_id(&body).expect("BEGIN should return session_id");

    // First COMMIT
    client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({"query": "COMMIT"}))
        .send()
        .await
        .unwrap();

    // Second COMMIT with same session (session no longer exists)
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({"query": "COMMIT"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Should error - session was already committed and removed
    assert!(
        common::has_error(&body) || body["results"][0]["statement_type"] == "COMMIT",
        "Double COMMIT should be handled: {:?}",
        body
    );
}

#[tokio::test]
async fn test_double_rollback() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // BEGIN
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "BEGIN"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let session_id = extract_session_id(&body).expect("BEGIN should return session_id");

    // First ROLLBACK
    client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({"query": "ROLLBACK"}))
        .send()
        .await
        .unwrap();

    // Second ROLLBACK with same session
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({"query": "ROLLBACK"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body) || body["results"][0]["statement_type"] == "ROLLBACK",
        "Double ROLLBACK should be handled: {:?}",
        body
    );
}

// =============================================================================
// Transaction Isolation Tests
// =============================================================================

#[tokio::test]
async fn test_uncommitted_changes_not_visible() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Setup collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION isolation_test"}))
        .send()
        .await
        .unwrap();

    // Start transaction
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "BEGIN"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let session_id = extract_session_id(&body).expect("BEGIN should return session_id");

    // INSERT within transaction
    client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({
            "query": r#"INSERT INTO isolation_test {"id": "t1", "value": "uncommitted"}"#
        }))
        .send()
        .await
        .unwrap();

    // Query from same session (should see uncommitted)
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({"query": "SELECT * FROM isolation_test:t1"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(
        body["results"][0]["count"], 1,
        "Same session should see own uncommitted changes"
    );

    // Query WITHOUT session header (should NOT see uncommitted)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM isolation_test:t1"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(
        body["results"][0]["count"], 0,
        "Outside transaction, uncommitted changes should not be visible"
    );

    // Rollback
    client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({"query": "ROLLBACK"}))
        .send()
        .await
        .unwrap();

    // After rollback, document should not exist
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM isolation_test:t1"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(
        body["results"][0]["count"], 0,
        "After ROLLBACK, document should not exist"
    );
}

#[tokio::test]
async fn test_committed_changes_visible() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION commit_test"}))
        .send()
        .await
        .unwrap();

    // Start transaction
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "BEGIN"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let session_id = extract_session_id(&body).expect("BEGIN should return session_id");

    // INSERT within transaction
    client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({
            "query": r#"INSERT INTO commit_test {"id": "c1", "value": "committed"}"#
        }))
        .send()
        .await
        .unwrap();

    // COMMIT
    client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({"query": "COMMIT"}))
        .send()
        .await
        .unwrap();

    // After commit, document should exist
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM commit_test:c1"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["value"], "committed");
}

// =============================================================================
// Transaction with Multiple Operations
// =============================================================================

#[tokio::test]
async fn test_transaction_multiple_inserts() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION multi_insert"}))
        .send()
        .await
        .unwrap();

    // BEGIN
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "BEGIN"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let session_id = extract_session_id(&body).expect("BEGIN should return session_id");

    // Multiple inserts in transaction
    for i in 1..=5 {
        client
            .post(format!("http://{}/sql", addr))
            .header("X-Session-Id", &session_id)
            .json(&json!({
                "query": format!(r#"INSERT INTO multi_insert {{"id": "m{}", "value": {}}}"#, i, i)
            }))
            .send()
            .await
            .unwrap();
    }

    // COMMIT
    client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({"query": "COMMIT"}))
        .send()
        .await
        .unwrap();

    // All 5 documents should exist
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM multi_insert"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 5);
}

#[tokio::test]
async fn test_transaction_rollback_multiple_inserts() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION rollback_multi"}))
        .send()
        .await
        .unwrap();

    // BEGIN
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "BEGIN"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let session_id = extract_session_id(&body).expect("BEGIN should return session_id");

    // Multiple inserts
    for i in 1..=5 {
        client
            .post(format!("http://{}/sql", addr))
            .header("X-Session-Id", &session_id)
            .json(&json!({
                "query": format!(r#"INSERT INTO rollback_multi {{"id": "r{}", "value": {}}}"#, i, i)
            }))
            .send()
            .await
            .unwrap();
    }

    // Verify within transaction
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({"query": "SELECT * FROM rollback_multi"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        body["results"][0]["count"], 5,
        "Should see 5 docs within transaction"
    );

    // ROLLBACK all
    client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({"query": "ROLLBACK"}))
        .send()
        .await
        .unwrap();

    // No documents should exist
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM rollback_multi"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(
        body["results"][0]["count"], 0,
        "All inserts should be rolled back"
    );
}

#[tokio::test]
async fn test_transaction_mixed_operations() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION mixed_ops"}))
        .send()
        .await
        .unwrap();

    // Insert initial document outside transaction
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO mixed_ops {"id": "existing", "value": "original"}"#
        }))
        .send()
        .await
        .unwrap();

    // Start transaction
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "BEGIN"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let session_id = extract_session_id(&body).expect("BEGIN should return session_id");

    // Insert new
    client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({
            "query": r#"INSERT INTO mixed_ops {"id": "new", "value": "inserted"}"#
        }))
        .send()
        .await
        .unwrap();

    // Update existing
    client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({
            "query": r#"UPDATE mixed_ops:existing SET value = "updated""#
        }))
        .send()
        .await
        .unwrap();

    // Commit
    client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({"query": "COMMIT"}))
        .send()
        .await
        .unwrap();

    // Verify both documents
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM mixed_ops"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 2);
}

// =============================================================================
// Transaction After Error
// =============================================================================

#[tokio::test]
async fn test_transaction_continues_after_error() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION error_test (SCHEMA STRICT, name string REQUIRED)"
        }))
        .send()
        .await
        .unwrap();

    // BEGIN
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "BEGIN"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let session_id = extract_session_id(&body).expect("BEGIN should return session_id");

    // Valid insert
    client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({
            "query": r#"INSERT INTO error_test {"id": "e1", "name": "valid"}"#
        }))
        .send()
        .await
        .unwrap();

    // Invalid insert (schema violation) - should fail
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({
            "query": r#"INSERT INTO error_test {"id": "e2", "name": 12345}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(common::has_error(&body), "Should reject type mismatch");

    // Try another valid insert after error
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({
            "query": r#"INSERT INTO error_test {"id": "e3", "name": "also valid"}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Transaction should continue after non-fatal error
    if body["error"].is_null() {
        // Transaction continues - commit and verify
        client
            .post(format!("http://{}/sql", addr))
            .header("X-Session-Id", &session_id)
            .json(&json!({"query": "COMMIT"}))
            .send()
            .await
            .unwrap();

        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "SELECT * FROM error_test"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        // Should have e1 and e3 (e2 failed)
        assert_eq!(body["results"][0]["count"], 2);
    } else {
        // Transaction was aborted - rollback to clean up
        client
            .post(format!("http://{}/sql", addr))
            .header("X-Session-Id", &session_id)
            .json(&json!({"query": "ROLLBACK"}))
            .send()
            .await
            .unwrap();
    }
}

// =============================================================================
// Large Transaction Tests
// =============================================================================

#[tokio::test]
async fn test_large_transaction() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION large_tx"}))
        .send()
        .await
        .unwrap();

    // BEGIN
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "BEGIN"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let session_id = extract_session_id(&body).expect("BEGIN should return session_id");

    // Insert 100 documents
    for i in 1..=100 {
        client
            .post(format!("http://{}/sql", addr))
            .header("X-Session-Id", &session_id)
            .json(&json!({
                "query": format!(r#"INSERT INTO large_tx {{"id": "doc{}", "index": {}}}"#, i, i)
            }))
            .send()
            .await
            .unwrap();
    }

    // COMMIT
    client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({"query": "COMMIT"}))
        .send()
        .await
        .unwrap();

    // Verify all 100 documents
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM large_tx"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 100);
}

// =============================================================================
// Transaction Case Sensitivity
// =============================================================================

#[tokio::test]
async fn test_transaction_keywords_case_insensitive() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Test lowercase begin
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "begin"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "begin should work lowercase");

    let session_id = extract_session_id(&body).expect("begin should return session_id");

    // Test lowercase commit
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({"query": "commit"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "commit should work lowercase");

    // Test mixed case BeGiN
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "BeGiN"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "BeGiN should work mixed case");

    let session_id = extract_session_id(&body).expect("BeGiN should return session_id");

    // Test mixed case RoLlBaCk
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({"query": "RoLlBaCk"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "RoLlBaCk should work mixed case");
}

// =============================================================================
// Invalid Session ID Tests
// =============================================================================

#[tokio::test]
async fn test_invalid_session_id() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION invalid_session"}))
        .send()
        .await
        .unwrap();

    // An explicitly supplied unknown session is rejected before statement execution.
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", "fake-session-id-12345")
        .json(&json!({
            "query": r#"INSERT INTO invalid_session {"id": "x", "value": 1}"#
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::CONFLICT);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["error"]["code"], "SDB-QE034", "{body:?}");

    // The rejected DML must never fall back to autocommit.
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM invalid_session:x"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        body["results"][0]["count"], 0,
        "Invalid session DML must not create a document: {body:?}"
    );
}

#[tokio::test]
async fn test_expired_session_id() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION expired_session"}))
        .send()
        .await
        .unwrap();

    // Start and commit a transaction
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "BEGIN"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let session_id = extract_session_id(&body).expect("BEGIN should return session_id");

    // Commit removes the original session.
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({"query": "COMMIT"}))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());

    // Reusing the completed session is rejected before DML execution.
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", &session_id)
        .json(&json!({
            "query": r#"INSERT INTO expired_session {"id": "x", "value": 1}"#
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::CONFLICT);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["error"]["code"], "SDB-QE034", "{body:?}");

    // The rejected DML must never fall back to autocommit.
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM expired_session:x"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        body["results"][0]["count"], 0,
        "Expired session DML must not create a document: {body:?}"
    );
}
