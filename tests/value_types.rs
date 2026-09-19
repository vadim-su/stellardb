//! Tests for new Value types: Datetime, Duration, Bytes, Range and truthiness

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
// Truthiness Tests
// =============================================================================

#[tokio::test]
async fn test_truthiness_null_is_falsy() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "SELECT * FROM [{v: 1}, {v: 2}] WHERE null").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 0); // null is falsy, no rows pass
}

#[tokio::test]
async fn test_truthiness_zero_is_falsy() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(
        &client,
        &addr,
        "SELECT * FROM [{v: 0}, {v: 1}, {v: 2}] WHERE v",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 2); // 0 is falsy, only v=1 and v=2 pass
}

#[tokio::test]
async fn test_truthiness_empty_string_is_falsy() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(
        &client,
        &addr,
        r#"SELECT * FROM [{s: ""}, {s: "a"}] WHERE s"#,
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["s"], "a");
}

#[tokio::test]
async fn test_truthiness_empty_array_is_falsy() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "SELECT * FROM [{a: []}, {a: [1]}] WHERE a").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
}

#[tokio::test]
async fn test_truthiness_empty_object_is_falsy() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(
        &client,
        &addr,
        "SELECT * FROM [{o: {}}, {o: {x: 1}}] WHERE o",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
}

#[tokio::test]
async fn test_truthiness_false_is_falsy() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(
        &client,
        &addr,
        "SELECT * FROM [{b: false}, {b: true}] WHERE b",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["b"], true);
}

#[tokio::test]
async fn test_truthiness_non_empty_values_are_truthy() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Test non-empty string
    let result = query(&client, &addr, r#"SELECT * FROM [{v: 1}] WHERE "hello""#).await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);

    // Test non-zero number
    let result = query(&client, &addr, "SELECT * FROM [{v: 1}] WHERE 42").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);

    // Test non-empty array
    let result = query(&client, &addr, "SELECT * FROM [{v: 1}] WHERE [1, 2, 3]").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
}

// =============================================================================
// Datetime Literal Tests
// =============================================================================

#[tokio::test]
async fn test_datetime_literal_full() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Full datetime with timezone
    let result = query(&client, &addr, r#"SELECT d"2024-01-15T10:30:00Z" as dt"#).await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    // Datetime is serialized as ISO8601 string
    let dt = rows[0]["dt"].as_str().unwrap();
    assert!(dt.contains("2024-01-15"), "Expected date in output: {}", dt);
}

#[tokio::test]
async fn test_datetime_literal_date_only() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Date only
    let result = query(&client, &addr, r#"SELECT d"2024-01-15" as dt"#).await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    let dt = rows[0]["dt"].as_str().unwrap();
    assert!(dt.contains("2024-01-15"), "Expected date in output: {}", dt);
}

#[tokio::test]
async fn test_datetime_literal_with_offset() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Datetime with timezone offset
    let result = query(
        &client,
        &addr,
        r#"SELECT d"2024-01-15T10:30:00+05:00" as dt"#,
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
}

#[tokio::test]
async fn test_datetime_literal_invalid() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Invalid datetime
    let result = query(&client, &addr, r#"SELECT d"not-a-date" as dt"#).await;
    assert!(
        result["error"].is_object(),
        "Expected error for invalid datetime, got: {:?}",
        result
    );
    assert!(
        result["error"]["code"]
            .as_str()
            .unwrap()
            .starts_with("SDB-QP"),
        "Expected parse error code for invalid datetime"
    );
}

// =============================================================================
// Duration Literal Tests
// =============================================================================

#[tokio::test]
async fn test_duration_literal_simple() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Simple duration
    let result = query(&client, &addr, "SELECT 1h as dur").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    // Duration is serialized as a formatted string like "1h" or nanoseconds
    let dur = &rows[0]["dur"];
    assert!(!dur.is_null(), "Expected duration value");
}

#[tokio::test]
async fn test_duration_literal_compound() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Compound duration: 1h30m
    let result = query(&client, &addr, "SELECT 1h30m as dur").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
}

#[tokio::test]
async fn test_duration_literal_milliseconds() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Milliseconds
    let result = query(&client, &addr, "SELECT 500ms as dur").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
}

#[tokio::test]
async fn test_duration_literal_all_units() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Test all units
    let units = ["1ns", "1us", "1ms", "1s", "1m", "1h", "1d", "1w", "1y"];
    for unit in units {
        let sql = format!("SELECT {} as dur", unit);
        let result = query(&client, &addr, &sql).await;
        assert!(
            result["error"].is_null(),
            "Error for {}: {:?}",
            unit,
            result
        );
    }
}

// =============================================================================
// Duration Arithmetic Tests
// =============================================================================

#[tokio::test]
async fn test_duration_add_duration() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // 1h + 30m = 1h30m = 5400 seconds = 5400000000000 nanoseconds
    let result = query(&client, &addr, "SELECT duration::secs(1h + 30m) as total").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["total"], 5400); // 1h30m in seconds
}

#[tokio::test]
async fn test_duration_add_duration_different_units() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // 1d + 12h = 36 hours = 129600 seconds
    let result = query(&client, &addr, "SELECT duration::secs(1d + 12h) as total").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows[0]["total"], 129600);
}

#[tokio::test]
async fn test_duration_add_duration_small_units() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // 500ms + 500ms = 1000ms = 1s
    let result = query(
        &client,
        &addr,
        "SELECT duration::millis(500ms + 500ms) as total",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows[0]["total"], 1000);
}

#[tokio::test]
async fn test_duration_sub_duration() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // 2h - 30m = 1h30m = 5400 seconds
    let result = query(&client, &addr, "SELECT duration::secs(2h - 30m) as total").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows[0]["total"], 5400);
}

#[tokio::test]
async fn test_duration_sub_duration_saturating() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // 1h - 2h = 0 (saturating subtraction)
    let result = query(&client, &addr, "SELECT duration::secs(1h - 2h) as total").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows[0]["total"], 0);
}

// =============================================================================
// Datetime + Duration Arithmetic Tests
// =============================================================================

#[tokio::test]
async fn test_datetime_add_duration() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Add 1 day to a datetime
    let result = query(
        &client,
        &addr,
        r#"SELECT d"2024-01-15T00:00:00Z" + 1d as result"#,
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    let dt = rows[0]["result"].as_str().unwrap();
    assert!(
        dt.contains("2024-01-16"),
        "Expected 2024-01-16, got: {}",
        dt
    );
}

#[tokio::test]
async fn test_datetime_add_duration_hours() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Add 6 hours
    let result = query(
        &client,
        &addr,
        r#"SELECT d"2024-01-15T10:00:00Z" + 6h as result"#,
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    let dt = rows[0]["result"].as_str().unwrap();
    assert!(dt.contains("16:00:00"), "Expected 16:00:00, got: {}", dt);
}

#[tokio::test]
async fn test_duration_add_datetime() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Duration + Datetime (reversed order should work too)
    let result = query(
        &client,
        &addr,
        r#"SELECT 1d + d"2024-01-15T00:00:00Z" as result"#,
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    let dt = rows[0]["result"].as_str().unwrap();
    assert!(
        dt.contains("2024-01-16"),
        "Expected 2024-01-16, got: {}",
        dt
    );
}

#[tokio::test]
async fn test_datetime_sub_duration() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Subtract 1 day from a datetime
    let result = query(
        &client,
        &addr,
        r#"SELECT d"2024-01-15T00:00:00Z" - 1d as result"#,
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    let dt = rows[0]["result"].as_str().unwrap();
    assert!(
        dt.contains("2024-01-14"),
        "Expected 2024-01-14, got: {}",
        dt
    );
}

#[tokio::test]
async fn test_datetime_sub_datetime() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Datetime - Datetime = Duration (difference)
    let result = query(
        &client,
        &addr,
        r#"SELECT duration::secs(d"2024-01-16T00:00:00Z" - d"2024-01-15T00:00:00Z") as diff"#,
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows[0]["diff"], 86400); // 1 day = 86400 seconds
}

#[tokio::test]
async fn test_datetime_add_compound_duration() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Add compound duration 1d12h30m
    let result = query(
        &client,
        &addr,
        r#"SELECT d"2024-01-15T00:00:00Z" + 1d12h30m as result"#,
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    let dt = rows[0]["result"].as_str().unwrap();
    // 2024-01-15 00:00 + 1d12h30m = 2024-01-16 12:30
    assert!(
        dt.contains("2024-01-16"),
        "Expected 2024-01-16, got: {}",
        dt
    );
    assert!(dt.contains("12:30:00"), "Expected 12:30:00, got: {}", dt);
}

// =============================================================================
// Bytes Literal Tests
// =============================================================================

#[tokio::test]
async fn test_bytes_literal_base64() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // b"SGVsbG8=" is base64 for "Hello"
    let result = query(&client, &addr, r#"SELECT b"SGVsbG8=" as bytes"#).await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    // Bytes should be serialized as base64
    let bytes = rows[0]["bytes"].as_str().unwrap();
    assert_eq!(bytes, "SGVsbG8=");
}

#[tokio::test]
async fn test_bytes_literal_hex() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // 0x48656c6c6f is hex for "Hello"
    let result = query(&client, &addr, "SELECT 0x48656c6c6f as bytes").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    // Should decode to "Hello" and serialize back to base64
    let bytes = rows[0]["bytes"].as_str().unwrap();
    assert_eq!(bytes, "SGVsbG8=");
}

#[tokio::test]
async fn test_bytes_literal_empty() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Empty bytes
    let result = query(&client, &addr, r#"SELECT b"" as bytes"#).await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    let bytes = rows[0]["bytes"].as_str().unwrap();
    assert_eq!(bytes, "");
}

#[tokio::test]
async fn test_bytes_literal_hex_empty() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Empty hex (0x without digits) should fail to parse - requires at least one hex digit
    let result = query(&client, &addr, "SELECT 0x as bytes").await;
    assert!(
        result["error"].is_object(),
        "Expected error for empty hex literal, got: {:?}",
        result
    );
    assert!(
        result["error"]["code"]
            .as_str()
            .unwrap()
            .starts_with("SDB-"),
        "Expected structured error code for empty hex literal"
    );
}

#[tokio::test]
async fn test_bytes_literal_invalid_base64() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Invalid base64
    let result = query(&client, &addr, r#"SELECT b"!!!" as bytes"#).await;
    assert!(
        result["error"].is_object(),
        "Expected error for invalid base64, got: {:?}",
        result
    );
    assert!(
        result["error"]["code"]
            .as_str()
            .unwrap()
            .starts_with("SDB-"),
        "Expected structured error code for invalid base64"
    );
}

// =============================================================================
// Range Tests
// =============================================================================

#[tokio::test]
async fn test_range_in_from() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "SELECT * FROM 1..5").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 5); // 1, 2, 3, 4, 5 (inclusive)
}

#[tokio::test]
async fn test_range_with_filter() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "SELECT * FROM 1..10 WHERE $value > 5").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 5); // 6, 7, 8, 9, 10
}

#[tokio::test]
async fn test_range_with_value_ref() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "SELECT $value * 2 as doubled FROM 1..3").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["doubled"], 2);
    assert_eq!(rows[1]["doubled"], 4);
    assert_eq!(rows[2]["doubled"], 6);
}

// =============================================================================
// Decimal Arithmetic Tests
// =============================================================================

#[tokio::test]
async fn test_decimal_subtraction() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "SELECT 100.50dec - 30.25dec AS diff").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    // Decimal serializes as JSON number via f64 conversion
    assert_eq!(result["results"][0]["data"][0]["diff"], json!(70.25));
}

#[tokio::test]
async fn test_decimal_addition() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "SELECT 100.50dec + 30.25dec AS total").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0]["total"], json!(130.75));
}

#[tokio::test]
async fn test_decimal_multiplication() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "SELECT 10.5dec * 3dec AS product").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0]["product"], json!(31.5));
}

#[tokio::test]
async fn test_decimal_division() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "SELECT 100dec / 4dec AS quotient").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0]["quotient"], json!(25.0));
}

#[tokio::test]
async fn test_decimal_mixed_with_int() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Decimal + Int -> Decimal
    let result = query(&client, &addr, "SELECT 100.50dec + 10 AS sum").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0]["sum"], json!(110.5));

    // Int - Decimal -> Decimal
    let result = query(&client, &addr, "SELECT 200 - 50.75dec AS diff").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0]["diff"], json!(149.25));

    // Decimal * Int -> Decimal
    let result = query(&client, &addr, "SELECT 25.5dec * 4 AS product").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0]["product"], json!(102.0));

    // Decimal / Int -> Decimal
    let result = query(&client, &addr, "SELECT 100dec / 3 AS quotient").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let q = result["results"][0]["data"][0]["quotient"]
        .as_f64()
        .unwrap();
    assert!((q - 33.333333).abs() < 0.001, "Expected ~33.33..., got {q}");
}

#[tokio::test]
async fn test_decimal_division_by_zero() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "SELECT 100dec / 0dec AS bad").await;
    assert!(
        !result["error"].is_null(),
        "Expected division by zero error"
    );

    let result = query(&client, &addr, "SELECT 100dec / 0 AS bad").await;
    assert!(
        !result["error"].is_null(),
        "Expected division by zero error"
    );
}

#[tokio::test]
async fn test_decimal_in_stored_data() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with decimal fields, compute arithmetic
    let setup = r#"
        DEFINE COLLECTION item (price decimal, cost decimal);
        CREATE item:1 SET price = 100.50dec, cost = 60.25dec;
        CREATE item:2 SET price = 250.00dec, cost = 150.75dec
    "#;
    let r = query(&client, &addr, setup).await;
    assert!(r["error"].is_null(), "Setup error: {:?}", r);

    let result = query(
        &client,
        &addr,
        "SELECT price, cost, price - cost AS margin FROM item ORDER price DESC",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    // item:2 (price=250, cost=150.75) comes first (ORDER price DESC)
    assert_eq!(rows[0]["margin"], json!(99.25));
    // item:1 (price=100.50, cost=60.25) comes second
    assert_eq!(rows[1]["margin"], json!(40.25));
}

#[tokio::test]
async fn test_order_by_computed_expression() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let setup = r#"
        DEFINE COLLECTION product5 (name string, price decimal, cost decimal);
        CREATE product5:1 SET name = "A", price = 100dec, cost = 80dec;
        CREATE product5:2 SET name = "B", price = 200dec, cost = 50dec;
        CREATE product5:3 SET name = "C", price = 150dec, cost = 100dec
    "#;
    let r = query(&client, &addr, setup).await;
    assert!(r["error"].is_null(), "Setup error: {:?}", r);

    // ORDER BY computed expression (not alias) — price - cost DESC
    let result = query(
        &client,
        &addr,
        "SELECT name, price - cost AS margin FROM product5 ORDER price - cost DESC",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    // B: 200 - 50 = 150 (highest margin)
    assert_eq!(rows[0]["name"], "B");
    assert_eq!(rows[0]["margin"], json!(150.0));
    // C: 150 - 100 = 50
    assert_eq!(rows[1]["name"], "C");
    assert_eq!(rows[1]["margin"], json!(50.0));
    // A: 100 - 80 = 20 (lowest margin)
    assert_eq!(rows[2]["name"], "A");
    assert_eq!(rows[2]["margin"], json!(20.0));
}

#[tokio::test]
async fn test_order_by_computed_mixed_types() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // ORDER BY on Int arithmetic expression
    let result = query(
        &client,
        &addr,
        "SELECT * FROM [{a: 3, b: 1}, {a: 1, b: 5}, {a: 2, b: 10}] ORDER a + b DESC",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    // {a:2, b:10} -> 12 (highest)
    assert_eq!(rows[0]["a"], 2);
    // {a:1, b:5} -> 6
    assert_eq!(rows[1]["a"], 1);
    // {a:3, b:1} -> 4 (lowest)
    assert_eq!(rows[2]["a"], 3);
}

#[tokio::test]
async fn test_order_by_computed_expression_asc() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(
        &client,
        &addr,
        "SELECT * FROM [{x: 10, y: 3}, {x: 5, y: 1}, {x: 8, y: 6}] ORDER x - y",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let rows = result["results"][0]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    // {x:8, y:6} -> 2 (lowest)
    assert_eq!(rows[0]["x"], 8);
    // {x:5, y:1} -> 4
    assert_eq!(rows[1]["x"], 5);
    // {x:10, y:3} -> 7 (highest)
    assert_eq!(rows[2]["x"], 10);
}
