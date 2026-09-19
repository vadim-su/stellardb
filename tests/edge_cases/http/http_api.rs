// HTTP REST API edge case tests
// Tests for invalid requests, headers, encoding, and error handling

use crate::common;
use futures::future::join_all;
use serde_json::json;

// =============================================================================
// Invalid JSON Tests
// =============================================================================

#[tokio::test]
async fn test_invalid_json_in_sql_body() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .header("Content-Type", "application/json")
        .body(r#"{"query": "SELECT * FROM test""#) // Missing closing brace
        .send()
        .await
        .unwrap();

    // Should return 400 Bad Request
    assert_eq!(res.status(), 400, "Invalid JSON should return 400");
}

#[tokio::test]
async fn test_invalid_json_in_document_body() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // First create a collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION bad_json"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/collections/bad_json/documents", addr))
        .header("Content-Type", "application/json")
        .body(r#"{"id": "test", "name": broken}"#) // Invalid JSON
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 400, "Invalid JSON should return 400");
}

#[tokio::test]
async fn test_truncated_json() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .header("Content-Type", "application/json")
        .body(r#"{"query": "SELECT *"#) // Truncated
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 400);
}

// =============================================================================
// Empty Request Body Tests
// =============================================================================

#[tokio::test]
async fn test_empty_body_sql() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .header("Content-Type", "application/json")
        .body("")
        .send()
        .await
        .unwrap();

    // Should return error - either 400 or specific error
    assert!(res.status().is_client_error() || res.status().is_server_error());
}

#[tokio::test]
async fn test_empty_body_post_document() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION empty_body"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/collections/empty_body/documents", addr))
        .header("Content-Type", "application/json")
        .body("")
        .send()
        .await
        .unwrap();

    assert!(res.status().is_client_error());
}

#[tokio::test]
async fn test_null_body_sql() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .header("Content-Type", "application/json")
        .body("null")
        .send()
        .await
        .unwrap();

    assert!(res.status().is_client_error());
}

// =============================================================================
// Content-Type Header Tests
// =============================================================================

#[tokio::test]
async fn test_missing_content_type() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .body(r#"{"query": "SELECT * FROM test"}"#)
        .send()
        .await
        .unwrap();

    // Server may accept or reject based on implementation
    // Document the behavior
    let status = res.status();
    assert!(
        status.is_success() || status.is_client_error(),
        "Should handle missing Content-Type: {}",
        status
    );
}

#[tokio::test]
async fn test_wrong_content_type() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .header("Content-Type", "text/plain")
        .body(r#"{"query": "SELECT * FROM test"}"#)
        .send()
        .await
        .unwrap();

    // Document behavior with wrong content type
    let status = res.status();
    assert!(
        status.is_success() || status == 415 || status.is_client_error(),
        "Should handle wrong Content-Type: {}",
        status
    );
}

#[tokio::test]
async fn test_content_type_with_charset() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION charset_test"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .header("Content-Type", "application/json; charset=utf-8")
        .body(r#"{"query": "SELECT * FROM charset_test"}"#)
        .send()
        .await
        .unwrap();

    // Should accept Content-Type with charset
    assert!(
        res.status().is_success(),
        "Should accept charset in Content-Type"
    );
}

// =============================================================================
// HTTP Method Tests
// =============================================================================

#[tokio::test]
async fn test_get_on_sql_endpoint() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .get(format!("http://{}/sql", addr))
        .send()
        .await
        .unwrap();

    // SQL endpoint only accepts POST
    assert_eq!(res.status(), 405, "GET on /sql should return 405");
}

#[tokio::test]
async fn test_delete_on_sql_endpoint() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .delete(format!("http://{}/sql", addr))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 405, "DELETE on /sql should return 405");
}

#[tokio::test]
async fn test_post_to_get_document() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // GET document endpoint should not accept POST
    let res = client
        .post(format!("http://{}/collections/test/documents/doc1", addr))
        .json(&json!({"data": "test"}))
        .send()
        .await
        .unwrap();

    // Document specific endpoints usually only accept GET/PUT/DELETE
    // Document actual behavior
    let status = res.status();
    assert!(
        status == 405 || status.is_client_error() || status.is_success(),
        "POST to document endpoint: {}",
        status
    );
}

// =============================================================================
// URL Path Tests
// =============================================================================

#[tokio::test]
async fn test_nonexistent_endpoint() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .get(format!("http://{}/nonexistent/path", addr))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 404, "Unknown path should return 404");
}

#[tokio::test]
async fn test_url_with_query_params() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION qp_test"}))
        .send()
        .await
        .unwrap();

    // Query params on REST endpoint
    let res = client
        .get(format!(
            "http://{}/collections/qp_test/documents?limit=10&offset=0",
            addr
        ))
        .send()
        .await
        .unwrap();

    // Document whether query params are supported
    assert!(
        res.status().is_success() || res.status() == 400,
        "Query params handling: {}",
        res.status()
    );
}

#[tokio::test]
async fn test_double_slash_in_path() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .get(format!("http://{}/collections//documents", addr))
        .send()
        .await
        .unwrap();

    // Double slash - should return error
    assert!(
        res.status() == 404 || res.status() == 400,
        "Double slash should fail: {}",
        res.status()
    );
}

// =============================================================================
// URL Encoding Tests
// =============================================================================

#[tokio::test]
async fn test_url_encoded_collection_name() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION test_collection"}))
        .send()
        .await
        .unwrap();

    // URL-encoded collection name (test%5Fcollection = test_collection)
    let res = client
        .get(format!(
            "http://{}/collections/test%5Fcollection/documents",
            addr
        ))
        .send()
        .await
        .unwrap();

    // Should decode and find the collection
    assert!(
        res.status().is_success(),
        "URL-encoded name should work: {}",
        res.status()
    );
}

#[tokio::test]
async fn test_url_encoded_document_id() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create document with ID that needs encoding
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION url_enc"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/collections/url_enc/documents", addr))
        .json(&json!({"id": "url_enc:test doc", "name": "Has space"}))
        .send()
        .await
        .unwrap();

    // Retrieve with URL-encoded ID (space = %20)
    let res = client
        .get(format!(
            "http://{}/collections/url_enc/documents/test%20doc",
            addr
        ))
        .send()
        .await
        .unwrap();

    // Document whether URL-encoded IDs work
    let status = res.status();
    assert!(
        status.is_success() || status == 404,
        "URL-encoded ID: {}",
        status
    );
}

// =============================================================================
// Large Request Tests
// =============================================================================

#[tokio::test]
async fn test_large_sql_query() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Very long query string
    let padding = " ".repeat(10000);
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": format!("SELECT * FROM test{}", padding)}))
        .send()
        .await
        .unwrap();

    // Should handle large query (may succeed or fail gracefully)
    assert!(
        res.status().is_success() || res.status().is_client_error(),
        "Large query handling: {}",
        res.status()
    );
}

#[tokio::test]
async fn test_large_document() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION large_doc"}))
        .send()
        .await
        .unwrap();

    // 100KB document
    let large_data = "x".repeat(100_000);
    let res = client
        .post(format!("http://{}/collections/large_doc/documents", addr))
        .json(&json!({"id": "large_doc:big", "data": large_data}))
        .send()
        .await
        .unwrap();

    // Should accept or reject with clear error
    assert!(
        res.status().is_success() || res.status() == 413 || res.status().is_client_error(),
        "Large document handling: {}",
        res.status()
    );
}

// =============================================================================
// Concurrent Request Tests
// =============================================================================

#[tokio::test]
async fn test_concurrent_creates() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION concurrent"}))
        .send()
        .await
        .unwrap();

    // Spawn 10 concurrent requests
    let mut handles: Vec<tokio::task::JoinHandle<reqwest::StatusCode>> = Vec::new();
    for i in 0..10 {
        let client = client.clone();
        handles.push(tokio::spawn(async move {
            client
                .post(format!("http://{}/collections/concurrent/documents", addr))
                .json(&json!({"id": format!("concurrent:doc{}", i), "index": i}))
                .send()
                .await
                .unwrap()
                .status()
        }));
    }

    // Wait for all requests
    let results = join_all(handles).await;

    // All should succeed
    for (i, result) in results.into_iter().enumerate() {
        let status = result.unwrap();
        assert!(
            status.is_success(),
            "Concurrent request {} failed: {}",
            i,
            status
        );
    }

    // Verify all documents exist
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM concurrent"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 10);
}

#[tokio::test]
async fn test_concurrent_same_id() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION race"}))
        .send()
        .await
        .unwrap();

    // Spawn 5 requests trying to create same document
    let mut handles: Vec<tokio::task::JoinHandle<reqwest::StatusCode>> = Vec::new();
    for i in 0..5 {
        let client = client.clone();
        handles.push(tokio::spawn(async move {
            client
                .post(format!("http://{}/collections/race/documents", addr))
                .json(&json!({"id": "race:same", "attempt": i}))
                .send()
                .await
                .unwrap()
                .status()
        }));
    }

    let results = join_all(handles).await;

    // Count successes and conflicts
    let successes: Vec<_> = results
        .iter()
        .filter(|r| r.as_ref().unwrap().is_success())
        .collect();
    let conflicts: Vec<_> = results
        .iter()
        .filter(|r| r.as_ref().unwrap().as_u16() == 409)
        .collect();

    // With atomic operations: exactly 1 success, 4 conflicts
    assert_eq!(
        successes.len(),
        1,
        "Exactly one concurrent create should succeed, got {}",
        successes.len()
    );
    assert_eq!(
        conflicts.len(),
        4,
        "Four concurrent creates should get 409 Conflict, got {}",
        conflicts.len()
    );

    // Verify exactly one document exists
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM race"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1);
}

// =============================================================================
// Response Format Tests
// =============================================================================

#[tokio::test]
async fn test_json_response_format() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION resp_test"}))
        .send()
        .await
        .unwrap();

    // Response should have JSON content type
    let content_type = res.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(
        content_type.contains("application/json"),
        "Response should be JSON: {}",
        content_type
    );

    // Response should be valid JSON
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body.is_object(), "Response should be JSON object");
}

#[tokio::test]
async fn test_error_response_format() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INVALID SQL SYNTAX HERE"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Error response should have error field
    assert!(
        !body["error"].is_null(),
        "Error response should have error field: {:?}",
        body
    );
}
