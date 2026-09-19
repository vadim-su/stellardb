// HTTP body size limit tests
// Tests for the 10MB DefaultBodyLimit

use crate::common;

// =============================================================================
// Request Body Size Limits
// =============================================================================

#[tokio::test]
async fn test_body_larger_than_10mb_rejected() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create a body larger than 10MB
    let large_body = "x".repeat(11 * 1024 * 1024); // 11 MB
    let payload = format!(r#"{{"query": "SELECT '{}' AS big"}}"#, large_body);

    let res = client
        .post(format!("http://{}/sql", addr))
        .header("Content-Type", "application/json")
        .body(payload)
        .send()
        .await
        .unwrap();

    // Should be rejected -- axum returns 413 Payload Too Large
    assert_eq!(
        res.status().as_u16(),
        413,
        "Body over 10MB should return 413 Payload Too Large, got {}",
        res.status()
    );
}

#[tokio::test]
async fn configured_global_body_limit_is_enforced() {
    let (addr, _tmp) = common::spawn_server_with_http_body_limit(2 * 1024).await;
    let response = common::test_client()
        .post(format!("http://{addr}/sql"))
        .header("Content-Type", "application/json")
        .body(format!(
            "{{\"query\":\"SELECT '{}'\"}}",
            "x".repeat(3 * 1024)
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 413);
}

#[tokio::test]
async fn test_body_under_10mb_accepted() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // A body just under 1MB should be fine
    let padding = "x".repeat(500_000);
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&serde_json::json!({"query": format!("SELECT '{}' AS data", padding)}))
        .send()
        .await
        .unwrap();

    // Should be accepted (may fail parsing the query, but not a 413)
    assert_ne!(
        res.status().as_u16(),
        413,
        "Body under 10MB should not return 413"
    );
}
