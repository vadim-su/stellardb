//! Tests for time::, duration::, bytes:: functions

mod common;

use serde_json::json;

async fn query(
    client: &reqwest::Client,
    addr: &std::net::SocketAddr,
    sql: &str,
) -> serde_json::Value {
    let resp = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({ "query": sql }))
        .send()
        .await
        .unwrap();
    resp.json().await.unwrap()
}

// =============================================================================
// time:: namespace tests
// =============================================================================

#[tokio::test]
async fn test_time_now() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "time::now()").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    // Should return a datetime (serialized as ISO8601 string)
    let data = &result["results"][0]["data"][0];
    assert!(data.is_string(), "Expected datetime string, got {:?}", data);
}

#[tokio::test]
async fn test_time_year() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, r#"time::year(d"2024-06-15T10:30:00Z")"#).await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], 2024);
}

#[tokio::test]
async fn test_time_month() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, r#"time::month(d"2024-06-15T10:30:00Z")"#).await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], 6);
}

#[tokio::test]
async fn test_time_day() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, r#"time::day(d"2024-06-15T10:30:00Z")"#).await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], 15);
}

#[tokio::test]
async fn test_time_hour() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, r#"time::hour(d"2024-06-15T10:30:00Z")"#).await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], 10);
}

#[tokio::test]
async fn test_time_minute() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, r#"time::minute(d"2024-06-15T10:30:00Z")"#).await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], 30);
}

#[tokio::test]
async fn test_time_second() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, r#"time::second(d"2024-06-15T10:30:45Z")"#).await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], 45);
}

#[tokio::test]
async fn test_time_functions_with_null() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "time::year(null)").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert!(result["results"][0]["data"][0].is_null());
}

#[tokio::test]
async fn test_time_year_type_error() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, r#"time::year("not a datetime")"#).await;
    assert!(
        !result["error"].is_null() || result["results"][0]["error"].is_object(),
        "Expected error for invalid type"
    );
}

// =============================================================================
// duration:: namespace tests
// =============================================================================

#[tokio::test]
async fn test_duration_from_secs() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "duration::secs(duration::from_secs(3600))").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], 3600);
}

#[tokio::test]
async fn test_duration_from_millis() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(
        &client,
        &addr,
        "duration::millis(duration::from_millis(1500))",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], 1500);
}

#[tokio::test]
async fn test_duration_secs_from_literal() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Using duration literal syntax: 5s = 5 seconds
    let result = query(&client, &addr, "duration::secs(5s)").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], 5);
}

#[tokio::test]
async fn test_duration_millis_from_literal() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Using duration literal syntax: 500ms = 500 milliseconds
    let result = query(&client, &addr, "duration::millis(500ms)").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], 500);
}

#[tokio::test]
async fn test_duration_secs_from_minutes() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // 2 minutes = 120 seconds
    let result = query(&client, &addr, "duration::secs(2m)").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], 120);
}

#[tokio::test]
async fn test_duration_functions_with_null() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "duration::secs(null)").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert!(result["results"][0]["data"][0].is_null());
}

#[tokio::test]
async fn test_duration_from_secs_type_error() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, r#"duration::from_secs("not a number")"#).await;
    assert!(
        !result["error"].is_null() || result["results"][0]["error"].is_object(),
        "Expected error for invalid type"
    );
}

// =============================================================================
// bytes:: namespace tests
// =============================================================================

#[tokio::test]
async fn test_bytes_len() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // b"SGVsbG8=" is base64 for "Hello" which is 5 bytes
    let result = query(&client, &addr, r#"bytes::len(b"SGVsbG8=")"#).await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], 5);
}

#[tokio::test]
async fn test_bytes_base64_encode() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Encode bytes back to base64
    let result = query(&client, &addr, r#"bytes::base64_encode(b"SGVsbG8=")"#).await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], "SGVsbG8=");
}

#[tokio::test]
async fn test_bytes_base64_decode() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Decode base64 string to bytes, then get length
    let result = query(
        &client,
        &addr,
        r#"bytes::len(bytes::base64_decode("SGVsbG8="))"#,
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], 5);
}

#[tokio::test]
async fn test_bytes_roundtrip() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // SGVsbG8= is base64 for "Hello"
    let result = query(
        &client,
        &addr,
        r#"bytes::base64_encode(bytes::base64_decode("SGVsbG8="))"#,
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], "SGVsbG8=");
}

#[tokio::test]
async fn test_bytes_functions_with_null() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "bytes::len(null)").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert!(result["results"][0]["data"][0].is_null());
}

#[tokio::test]
async fn test_bytes_base64_decode_invalid() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Invalid base64 should error
    let result = query(&client, &addr, r#"bytes::base64_decode("!!invalid!!")"#).await;
    assert!(
        !result["error"].is_null() || result["results"][0]["error"].is_object(),
        "Expected error for invalid base64"
    );
}

#[tokio::test]
async fn test_bytes_len_type_error() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, r#"bytes::len("not bytes")"#).await;
    assert!(
        !result["error"].is_null() || result["results"][0]["error"].is_object(),
        "Expected error for invalid type"
    );
}

// =============================================================================
// Integration tests - using functions with documents
// =============================================================================

#[tokio::test]
async fn test_time_functions_with_object_expr() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Using object expressions with datetime literals (works in SELECT FROM [array])
    // Note: INSERT INTO doesn't support datetime literals in object values yet
    let result = query(
        &client,
        &addr,
        r#"SELECT id, time::year(timestamp) AS year, time::month(timestamp) AS month, time::day(timestamp) AS day FROM [{id: "e1", timestamp: d"2024-12-25T15:30:45Z"}]"#,
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);

    let row = &result["results"][0]["data"][0];
    assert_eq!(row["year"], 2024);
    assert_eq!(row["month"], 12);
    assert_eq!(row["day"], 25);
}

#[tokio::test]
async fn test_duration_functions_with_object_expr() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Using object expressions with duration literals (works in SELECT FROM [array])
    // 1 hour 30 minutes = 5400 seconds
    let result = query(
        &client,
        &addr,
        r#"SELECT id, duration::secs(duration) AS total_secs FROM [{id: "t1", duration: 1h30m}]"#,
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);

    let row = &result["results"][0]["data"][0];
    assert_eq!(row["total_secs"], 5400);
}

#[tokio::test]
async fn test_time_functions_in_where_clause() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Using object expressions with datetime literals (works in SELECT FROM [array])
    // Query logs from 2024
    let result = query(
        &client,
        &addr,
        r#"SELECT id FROM [{id: "l1", timestamp: d"2024-06-15T10:00:00Z"}, {id: "l2", timestamp: d"2024-12-25T10:00:00Z"}, {id: "l3", timestamp: d"2023-06-15T10:00:00Z"}] WHERE time::year(timestamp) = 2024"#,
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["count"], 2);
}
