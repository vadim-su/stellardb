// IS NULL / IS NOT NULL / IS NONE / IS NOT NONE integration tests

use crate::common;
use serde_json::json;

async fn setup_nulls(addr: &std::net::SocketAddr, client: &reqwest::Client) {
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION items (name string, tag string)"}))
        .send()
        .await
        .unwrap();

    // doc with non-null tag
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO items {"id": "a", "name": "alpha", "tag": "x"}"#
        }))
        .send()
        .await
        .unwrap();

    // doc with explicit null tag
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO items {"id": "b", "name": "beta", "tag": null}"#
        }))
        .send()
        .await
        .unwrap();

    // doc with no tag field at all
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO items {"id": "c", "name": "gamma"}"#
        }))
        .send()
        .await
        .unwrap();
}

// =============================================================================
// IS NULL — matches null value AND missing field
// =============================================================================

#[tokio::test]
async fn test_is_null_matches_null_and_missing() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_nulls(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT * FROM items WHERE tag IS NULL"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(
        body["results"][0]["count"], 2,
        "IS NULL should match both null-value (b) and missing-field (c)"
    );
}

// =============================================================================
// IS NOT NULL — excludes null value AND missing field
// =============================================================================

#[tokio::test]
async fn test_is_not_null_excludes_null_and_missing() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_nulls(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT * FROM items WHERE tag IS NOT NULL"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(
        body["results"][0]["count"], 1,
        "IS NOT NULL should match only doc with non-null tag (a)"
    );
}

// =============================================================================
// IS NONE — only missing field
// =============================================================================

#[tokio::test]
async fn test_is_none_only_missing() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_nulls(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT * FROM items WHERE tag IS NONE"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(
        body["results"][0]["count"], 1,
        "IS NONE should match only doc with missing field (c)"
    );
}

// =============================================================================
// IS NOT NONE — field exists (even if null)
// =============================================================================

#[tokio::test]
async fn test_is_not_none_includes_null_value() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_nulls(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT * FROM items WHERE tag IS NOT NONE"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(
        body["results"][0]["count"], 2,
        "IS NOT NONE should match docs with field present (a, b), even if null"
    );
}

// =============================================================================
// Combined: IS NULL AND IS NOT NONE — present with null value
// =============================================================================

#[tokio::test]
async fn test_is_null_and_is_not_none_means_present_null() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_nulls(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT * FROM items WHERE tag IS NULL AND tag IS NOT NONE"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(
        body["results"][0]["count"], 1,
        "IS NULL AND IS NOT NONE should match only doc with explicit null (b)"
    );
}
