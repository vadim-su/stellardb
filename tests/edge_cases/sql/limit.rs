// LIMIT clause edge case tests
// Tests for LIMIT 0, negative, large values, and combinations with WHERE

use crate::common;
use serde_json::json;

// =============================================================================
// Setup Helper
// =============================================================================

async fn setup_test_data(addr: &std::net::SocketAddr, client: &reqwest::Client) {
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION limit_test (name string, value int)"}))
        .send()
        .await
        .unwrap();

    // Insert 10 documents
    for i in 1..=10 {
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": format!(r#"INSERT INTO limit_test {{"id": "l{}", "name": "item{}", "value": {}}}"#, i, i, i * 10)
            }))
            .send()
            .await
            .unwrap();
    }
}

// =============================================================================
// Basic LIMIT Tests
// =============================================================================

#[tokio::test]
async fn test_limit_zero() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM limit_test LIMIT 0"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // LIMIT 0 should return 0 results
    if body["error"].is_null() {
        assert_eq!(
            body["results"][0]["count"], 0,
            "LIMIT 0 should return 0 results"
        );
    }
}

#[tokio::test]
async fn test_limit_one() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM limit_test LIMIT 1"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 1);
}

#[tokio::test]
async fn test_limit_exact_count() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_test_data(&addr, &client).await;

    // LIMIT exactly equals document count
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM limit_test LIMIT 10"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 10);
}

#[tokio::test]
async fn test_limit_exceeds_count() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_test_data(&addr, &client).await;

    // LIMIT larger than document count
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM limit_test LIMIT 100"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    // Should return all 10, not error
    assert_eq!(body["results"][0]["count"], 10);
}

#[tokio::test]
async fn test_limit_very_large() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_test_data(&addr, &client).await;

    // Very large LIMIT
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM limit_test LIMIT 1000000"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 10);
}

// =============================================================================
// LIMIT with WHERE Clause
// =============================================================================

#[tokio::test]
async fn test_limit_with_where() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_test_data(&addr, &client).await;

    // WHERE returns 5 docs (value > 50), LIMIT 3
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM limit_test WHERE value > 50 LIMIT 3"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 3);
}

#[tokio::test]
async fn test_limit_with_where_exceeds_filtered() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_test_data(&addr, &client).await;

    // WHERE returns 2 docs (value > 80), LIMIT 10
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM limit_test WHERE value > 80 LIMIT 10"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    // Only 2 match, so return 2
    assert_eq!(body["results"][0]["count"], 2);
}

#[tokio::test]
async fn test_limit_with_where_no_match() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_test_data(&addr, &client).await;

    // WHERE returns 0 docs
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM limit_test WHERE value > 1000 LIMIT 5"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 0);
}

// =============================================================================
// LIMIT on Empty Collection
// =============================================================================

#[tokio::test]
async fn test_limit_empty_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION empty_limit"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM empty_limit LIMIT 10"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 0);
}

// =============================================================================
// Invalid LIMIT Values
// =============================================================================

#[tokio::test]
async fn test_limit_negative() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM limit_test LIMIT -1"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Negative LIMIT should be rejected (parse error)
    assert!(
        !body["error"].is_null(),
        "Negative LIMIT should be rejected: {:?}",
        body
    );
}

#[tokio::test]
async fn test_limit_float() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM limit_test LIMIT 5.5"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Float LIMIT should be rejected (parse error)
    assert!(
        !body["error"].is_null(),
        "Float LIMIT should be rejected: {:?}",
        body
    );
}

#[tokio::test]
async fn test_limit_string() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"SELECT * FROM limit_test LIMIT "five""#}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        !body["error"].is_null(),
        "String LIMIT should be rejected: {:?}",
        body
    );
}

#[tokio::test]
async fn test_limit_missing_value() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM limit_test LIMIT"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        !body["error"].is_null(),
        "LIMIT without value should be rejected: {:?}",
        body
    );
}

// =============================================================================
// LIMIT with Single Document Lookup
// =============================================================================

#[tokio::test]
async fn test_limit_on_single_doc_lookup() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_test_data(&addr, &client).await;

    // LIMIT on document-by-ID lookup (should still work, though unnecessary)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM limit_test:l1 LIMIT 10"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    // Only 1 document can be returned for specific ID
    assert_eq!(body["results"][0]["count"], 1);
}

// =============================================================================
// LIMIT Consistency
// =============================================================================

#[tokio::test]
async fn test_limit_returns_consistent_count() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_test_data(&addr, &client).await;

    // Run same query multiple times, should get same count
    for _ in 0..5 {
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "SELECT * FROM limit_test LIMIT 5"}))
            .send()
            .await
            .unwrap();

        let body: serde_json::Value = res.json().await.unwrap();
        assert!(body["error"].is_null());
        assert_eq!(body["results"][0]["count"], 5);
    }
}
