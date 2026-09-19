// Coalesce operator (??) integration tests
// Verifies that null uses the fallback while non-null values are preserved

use crate::common;
use serde_json::json;

async fn setup_users(addr: &std::net::SocketAddr, client: &reqwest::Client) {
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION users (name string, nickname string, score int, active bool, tags array)"}))
        .send()
        .await
        .unwrap();

    // u1: all fields present and truthy
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO users {"id": "u1", "name": "Alice", "nickname": "Ali", "score": 100, "active": true, "tags": ["admin"]}"#
        }))
        .send()
        .await
        .unwrap();

    // u2: falsy values (0, false, empty array) and null nickname
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO users {"id": "u2", "name": "Bob", "nickname": null, "score": 0, "active": false, "tags": []}"#
        }))
        .send()
        .await
        .unwrap();

    // u3: empty string name, null nickname
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO users {"id": "u3", "name": "", "nickname": null, "score": 50, "active": true, "tags": ["user"]}"#
        }))
        .send()
        .await
        .unwrap();

    // u4: nickname missing entirely (will be null), some other fields null
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO users {"id": "u4", "name": "Dana", "score": null, "active": null, "tags": null}"#
        }))
        .send()
        .await
        .unwrap();
}

// =============================================================================
// Basic Coalesce with NULL
// =============================================================================

#[tokio::test]
async fn test_coalesce_null_with_default() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users(&addr, &client).await;

    // nickname ?? "anon" should return nickname if truthy, otherwise "anon"
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT id, nickname ?? 'anon' AS display FROM users"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = body["results"][0]["data"].as_array().unwrap();
    assert_eq!(data.len(), 4);

    // Find each user's result - ID format is "users:u1" etc.
    for row in data {
        let id = row["id"].as_str().unwrap();
        let display = row["display"].as_str().unwrap();
        match id {
            "users:u1" => assert_eq!(display, "Ali", "Alice has nickname Ali"),
            "users:u2" => assert_eq!(display, "anon", "Bob has null nickname"),
            "users:u3" => assert_eq!(display, "anon", "u3 has null nickname"),
            "users:u4" => assert_eq!(display, "anon", "Dana has missing nickname"),
            _ => panic!("Unexpected user id: {}", id),
        }
    }
}

// =============================================================================
// Coalesce Chain (a ?? b ?? c)
// =============================================================================

#[tokio::test]
async fn test_coalesce_chain() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users(&addr, &client).await;

    // nickname ?? name ?? "unknown"
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT id, nickname ?? name ?? 'unknown' AS display FROM users"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = body["results"][0]["data"].as_array().unwrap();

    for row in data {
        let id = row["id"].as_str().unwrap();
        let display = row["display"].as_str().unwrap();
        match id {
            "users:u1" => assert_eq!(display, "Ali", "Alice: nickname is truthy"),
            "users:u2" => assert_eq!(display, "Bob", "Bob: nickname null, name is truthy"),
            "users:u3" => assert_eq!(
                display, "unknown",
                "u3: nickname null, name empty -> unknown"
            ),
            "users:u4" => assert_eq!(
                display, "Dana",
                "Dana: nickname missing (null), name is truthy"
            ),
            _ => panic!("Unexpected user id: {}", id),
        }
    }
}

// =============================================================================
// Coalesce with Falsy Values (0, false, empty string, empty array)
// =============================================================================

#[tokio::test]
async fn test_coalesce_zero_is_falsy() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users(&addr, &client).await;

    // score ?? 999 - score of 0 should be falsy
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT id, score ?? 999 AS score_val FROM users WHERE id = users:u2"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"][0];
    assert_eq!(data["score_val"], 999, "0 is falsy, should return 999");
}

#[tokio::test]
async fn test_coalesce_false_is_falsy() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users(&addr, &client).await;

    // active ?? true - false should be falsy
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT id, active ?? true AS is_active FROM users WHERE id = users:u2"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"][0];
    assert_eq!(
        data["is_active"], true,
        "false is falsy, should return true"
    );
}

#[tokio::test]
async fn test_coalesce_empty_string_is_falsy() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users(&addr, &client).await;

    // name ?? "fallback" - empty string should be falsy
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT id, name ?? 'fallback' AS display FROM users WHERE id = users:u3"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"][0];
    assert_eq!(data["display"], "fallback", "empty string is falsy");
}

#[tokio::test]
async fn test_coalesce_empty_array_is_falsy() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users(&addr, &client).await;

    // tags ?? ["default"] - empty array should be falsy
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT id, tags ?? ['default'] AS tags_val FROM users WHERE id = users:u2"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"][0];
    let tags = data["tags_val"].as_array().unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0], "default", "empty array is falsy");
}

// =============================================================================
// Coalesce with Truthy Values
// =============================================================================

#[tokio::test]
async fn test_coalesce_truthy_value_returned() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users(&addr, &client).await;

    // For u1: score=100 (truthy), should return 100
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT id, score ?? 999 AS score_val FROM users WHERE id = users:u1"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"][0];
    assert_eq!(data["score_val"], 100, "100 is truthy, should return 100");
}

// =============================================================================
// Coalesce with Literals
// =============================================================================

#[tokio::test]
async fn test_coalesce_null_literal() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users(&addr, &client).await;

    // null ?? "default" should return "default"
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT null ?? 'default' AS val FROM users LIMIT 1"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"][0];
    assert_eq!(data["val"], "default");
}

#[tokio::test]
async fn test_coalesce_multiple_null_literals() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users(&addr, &client).await;

    // null ?? null ?? 42 should return 42
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT null ?? null ?? 42 AS val FROM users LIMIT 1"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"][0];
    assert_eq!(data["val"], 42);
}

// =============================================================================
// Coalesce Precedence (lower than OR)
// =============================================================================

#[tokio::test]
async fn test_coalesce_precedence_with_or() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users(&addr, &client).await;

    // Test that ?? has lower precedence than OR
    // "active OR false ?? true" parses as "(active OR false) ?? true"
    // For u1: active=true, so (true OR false) = true, which is truthy -> true
    // For u2: active=false, so (false OR false) = false, which is falsy -> true
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT id, active OR false ?? true AS val FROM users WHERE id IN ['users:u1', 'users:u2']"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = body["results"][0]["data"].as_array().unwrap();
    for row in data {
        // Both should be true due to precedence
        assert_eq!(row["val"], true, "Coalesce should apply to entire OR expr");
    }
}
