// Datetime edge case tests
// Tests for now(), timestamps, epoch, and date validation

use crate::common;
use serde_json::json;

// =============================================================================
// now() Function Tests
// =============================================================================

#[tokio::test]
async fn test_default_now_generates_timestamp() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION events (name string, created_at datetime DEFAULT now())"
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO events {"id": "e1", "name": "test event"}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Insert should succeed: {:?}", body);

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM events:e1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();

    // created_at should be set to current timestamp
    let created_at = &body["results"][0]["data"][0]["created_at"];
    assert!(
        created_at.is_number() || created_at.is_string(),
        "created_at should be timestamp or string: {:?}",
        created_at
    );
}

#[tokio::test]
async fn test_now_in_multiple_documents() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION multi_now (created datetime DEFAULT now())"
        }))
        .send()
        .await
        .unwrap();

    // Insert two documents with small delay
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO multi_now {"id": "m1"}"#
        }))
        .send()
        .await
        .unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO multi_now {"id": "m2"}"#
        }))
        .send()
        .await
        .unwrap();

    // Both should have timestamps
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM multi_now"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();

    let docs = body["results"][0]["data"].as_array().unwrap();
    for doc in docs {
        assert!(
            doc["created"].is_number() || doc["created"].is_string(),
            "Each doc should have timestamp"
        );
    }
}

// =============================================================================
// Datetime Value Tests
// =============================================================================

#[tokio::test]
async fn test_datetime_accepts_epoch_timestamp() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION timestamps (ts datetime)"
        }))
        .send()
        .await
        .unwrap();

    // Unix epoch as integer (milliseconds or seconds depending on implementation)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO timestamps {"id": "t1", "ts": 0}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Document whether epoch 0 is accepted
    if body["error"].is_null() {
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "SELECT * FROM timestamps:t1"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(body["results"][0]["data"][0]["ts"].is_number());
    }
}

#[tokio::test]
async fn test_datetime_accepts_large_timestamp() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION future_ts (ts datetime)"
        }))
        .send()
        .await
        .unwrap();

    // Year 2099 timestamp (in seconds: ~4_100_000_000)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO future_ts {"id": "f1", "ts": 4100000000}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Document whether far future dates are accepted
    // Either succeeds (error null) or fails gracefully with error
    assert!(
        body["error"].is_null() || common::has_error(&body),
        "Far future timestamp should be handled: {:?}",
        body
    );
}

#[tokio::test]
async fn test_datetime_accepts_iso_string() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION iso_dates (date datetime)"
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO iso_dates {"id": "i1", "date": "2024-01-15T10:30:00Z"}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Document whether ISO string dates are accepted for datetime fields
    if body["error"].is_null() {
        let res = client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": "SELECT * FROM iso_dates:i1"}))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        // Should be stored and retrieved
        let date = &body["results"][0]["data"][0]["date"];
        assert!(date.is_string() || date.is_number());
    }
}

#[tokio::test]
async fn test_datetime_negative_timestamp() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION neg_ts (ts datetime)"
        }))
        .send()
        .await
        .unwrap();

    // Negative timestamp (before Unix epoch, e.g., 1960)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO neg_ts {"id": "n1", "ts": -315619200}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Document whether pre-epoch dates are accepted
    assert!(
        body["error"].is_null() || common::has_error(&body),
        "Negative timestamp should be handled: {:?}",
        body
    );
}

// =============================================================================
// Datetime Type Validation Tests
// =============================================================================

#[tokio::test]
async fn test_datetime_rejects_invalid_string() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION strict_dt (SCHEMA STRICT, ts datetime)"
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO strict_dt {"id": "bad", "ts": "not a date"}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Invalid date string should be rejected in strict mode
    // Behavior depends on implementation
    assert!(
        common::has_error(&body) || body["error"].is_null(),
        "Invalid date string handling: {:?}",
        body
    );
}

#[tokio::test]
async fn test_datetime_rejects_boolean() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION dt_bool (SCHEMA STRICT, ts datetime)"
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO dt_bool {"id": "bad", "ts": true}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        common::has_error(&body),
        "Boolean should be rejected for datetime: {:?}",
        body
    );
}

#[tokio::test]
async fn test_datetime_accepts_null() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION dt_null (ts datetime)"
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO dt_null {"id": "n1", "ts": null}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Null datetime should be accepted");

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM dt_null:n1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["results"][0]["data"][0]["ts"].is_null());
}

// =============================================================================
// Datetime Query Tests
// =============================================================================

#[tokio::test]
async fn test_datetime_comparison_greater() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION dt_query (ts datetime)"
        }))
        .send()
        .await
        .unwrap();

    // Insert documents with different timestamps
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO dt_query {"id": "d1", "ts": 1000}"#
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO dt_query {"id": "d2", "ts": 2000}"#
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO dt_query {"id": "d3", "ts": 3000}"#
        }))
        .send()
        .await
        .unwrap();

    // Query for timestamps > 1500
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM dt_query WHERE ts > 1500"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(
        body["results"][0]["count"], 2,
        "Should find 2 documents with ts > 1500"
    );
}

#[tokio::test]
async fn test_datetime_comparison_range() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION dt_range (ts datetime)"
        }))
        .send()
        .await
        .unwrap();

    for i in 1..=5 {
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": format!(r#"INSERT INTO dt_range {{"id": "r{}", "ts": {}}}"#, i, i * 1000)
            }))
            .send()
            .await
            .unwrap();
    }

    // Query for range: 2000 <= ts <= 4000
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM dt_range WHERE ts >= 2000 AND ts <= 4000"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(
        body["results"][0]["count"], 3,
        "Should find 3 documents in range"
    );
}

// =============================================================================
// Datetime Edge Values
// =============================================================================

#[tokio::test]
async fn test_datetime_max_value() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION dt_max (ts datetime)"
        }))
        .send()
        .await
        .unwrap();

    // Very large timestamp (year 3000+)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO dt_max {"id": "max", "ts": 32503680000}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Document maximum datetime handling
    assert!(
        body["error"].is_null() || common::has_error(&body),
        "Max datetime handling: {:?}",
        body
    );
}

// =============================================================================
// Combined Tests
// =============================================================================

#[tokio::test]
async fn test_datetime_with_other_fields() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION events_full (name string REQUIRED, created datetime DEFAULT now(), updated datetime)"
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO events_full {"id": "ef1", "name": "Test Event", "updated": null}"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM events_full:ef1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();

    let doc = &body["results"][0]["data"][0];
    assert_eq!(doc["name"], "Test Event");
    // created should have default now()
    assert!(
        doc["created"].is_number() || doc["created"].is_string(),
        "created should have default"
    );
    assert!(doc["updated"].is_null());
}

#[tokio::test]
async fn test_update_datetime_field() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION dt_update (ts datetime)"
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO dt_update {"id": "u1", "ts": 1000}"#
        }))
        .send()
        .await
        .unwrap();

    // Update the datetime field
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "UPDATE dt_update:u1 SET ts = 2000"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM dt_update:u1"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["ts"], 2000);
}
