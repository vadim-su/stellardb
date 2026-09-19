// Query timeout integration tests
// Tests for SET QUERY_TIMEOUT, X-Query-Timeout header, and duration formats

use crate::common;

// =============================================================================
// SET QUERY_TIMEOUT
// =============================================================================

#[tokio::test]
async fn test_set_query_timeout() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let body: serde_json::Value = client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "SET QUERY_TIMEOUT = 5s"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(!common::has_error(&body), "SET should succeed: {:?}", body);
    assert_eq!(body["results"][0]["data"][0]["key"], "QUERY_TIMEOUT");
    assert_eq!(body["results"][0]["data"][0]["status"], "ok");
}

#[tokio::test]
async fn test_set_query_timeout_zero_disables() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let body: serde_json::Value = client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "SET QUERY_TIMEOUT = 0"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(!common::has_error(&body));
}

#[tokio::test]
async fn test_set_query_timeout_formats() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    for dur in &["500ms", "1s", "1m30s", "2m"] {
        let body: serde_json::Value = client
            .post(format!("http://{addr}/sql"))
            .json(&serde_json::json!({"query": format!("SET QUERY_TIMEOUT = {}", dur)}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert!(!common::has_error(&body), "Failed for {}: {:?}", dur, body);
    }
}

// =============================================================================
// X-Query-Timeout Header
// =============================================================================

#[tokio::test]
async fn test_x_query_timeout_header() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "DEFINE COLLECTION test"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = client
        .post(format!("http://{addr}/sql"))
        .header("X-Query-Timeout", "10s")
        .json(&serde_json::json!({"query": "SELECT * FROM test"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(!common::has_error(&body));
}

#[tokio::test]
async fn test_fast_query_completes_within_timeout() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "DEFINE COLLECTION test"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{addr}/sql"))
        .json(&serde_json::json!({"query": "CREATE test:1 SET name = 'a'; CREATE test:2 SET name = 'b'"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = client
        .post(format!("http://{addr}/sql"))
        .header("X-Query-Timeout", "5s")
        .json(&serde_json::json!({"query": "SELECT * FROM test"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(!common::has_error(&body));
    assert_eq!(body["results"][0]["data"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn invalid_x_query_timeout_is_not_silently_ignored() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let response = client
        .post(format!("http://{addr}/v1/query"))
        .header("X-Query-Timeout", "tomorrow")
        .json(&serde_json::json!({"query": "SELECT VALUE 1"}))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "SDB-ST008");
}

#[tokio::test]
async fn shorter_client_timeout_wins_with_query_timeout_code() {
    let (addr, _tmp) = common::spawn_server().await;
    let response = common::test_client()
        .post(format!("http://{addr}/v1/query"))
        .header("X-Query-Timeout", "1ns")
        .json(&serde_json::json!({"query": "SELECT VALUE 1"}))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::GATEWAY_TIMEOUT);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "SDB-QE014");
}

#[tokio::test]
async fn shorter_db_deadline_wins_with_server_code() {
    let (addr, _tmp) =
        common::spawn_server_with_admission(stellardb::server::admission::AdmissionConfig {
            db_deadline: std::time::Duration::from_nanos(1),
            ..Default::default()
        })
        .await;
    let response = common::test_client()
        .post(format!("http://{addr}/v1/query"))
        .header("X-Query-Timeout", "1s")
        .json(&serde_json::json!({"query": "SELECT VALUE 1"}))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::GATEWAY_TIMEOUT);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "SDB-SV003");
}

#[tokio::test]
async fn explicit_zero_disables_query_timeout_but_not_db_deadline() {
    let (addr, _tmp) =
        common::spawn_server_with_admission(stellardb::server::admission::AdmissionConfig {
            db_deadline: std::time::Duration::from_nanos(1),
            ..Default::default()
        })
        .await;
    let response = common::test_client()
        .post(format!("http://{addr}/v1/query"))
        .header("X-Query-Timeout", "0")
        .json(&serde_json::json!({"query": "SELECT VALUE 1"}))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::GATEWAY_TIMEOUT);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "SDB-SV003");
}

#[tokio::test]
async fn absent_query_timeout_still_uses_db_deadline() {
    let (addr, _tmp) =
        common::spawn_server_with_admission(stellardb::server::admission::AdmissionConfig {
            db_deadline: std::time::Duration::from_nanos(1),
            ..Default::default()
        })
        .await;
    let response = common::test_client()
        .post(format!("http://{addr}/v1/query"))
        .json(&serde_json::json!({"query": "SELECT VALUE 1"}))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::GATEWAY_TIMEOUT);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "SDB-SV003");
}

#[tokio::test]
async fn equal_query_and_db_deadlines_use_query_code_as_documented_tie_breaker() {
    let (addr, _tmp) =
        common::spawn_server_with_admission(stellardb::server::admission::AdmissionConfig {
            db_deadline: std::time::Duration::from_nanos(1),
            ..Default::default()
        })
        .await;
    let response = common::test_client()
        .post(format!("http://{addr}/v1/query"))
        .header("X-Query-Timeout", "1ns")
        .json(&serde_json::json!({"query": "SELECT VALUE 1"}))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::GATEWAY_TIMEOUT);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "SDB-QE014");
}
