// Integration tests for math functions
// Tests the full query pipeline: parsing -> binding -> execution

mod common;

use serde_json::json;

async fn setup_test_data(addr: &std::net::SocketAddr, client: &reqwest::Client) {
    // Define flexible collection (schema-less) to allow any fields
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION mathtest"}))
        .send()
        .await
        .unwrap();

    // Insert test data with various numeric values
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO mathtest {"id": "m1", "name": "pos", "val": 4.5, "x": 3, "y": 4}"#
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO mathtest {"id": "m2", "name": "neg", "val": -3.7, "x": -5, "y": 2}"#
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO mathtest {"id": "m3", "name": "zero", "val": 0, "x": 0, "y": 0}"#
        }))
        .send()
        .await
        .unwrap();
}

// =============================================================================
// math::abs - absolute value
// =============================================================================

#[tokio::test]
async fn test_math_abs_positive() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, math::abs(val) AS abs_val FROM mathtest WHERE name = "pos""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["abs_val"], 4.5);
}

#[tokio::test]
async fn test_math_abs_negative() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, math::abs(val) AS abs_val FROM mathtest WHERE name = "neg""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["abs_val"], 3.7);
}

#[tokio::test]
async fn test_math_abs_zero() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, math::abs(val) AS abs_val FROM mathtest WHERE name = "zero""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["abs_val"], 0);
}

#[tokio::test]
async fn test_math_abs_with_null() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION nullmath"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO nullmath {"id": "n1", "val": null}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT math::abs(val) AS result FROM nullmath"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert!(body["results"][0]["data"][0]["result"].is_null());
}

// =============================================================================
// math::round, math::floor, math::ceil - rounding functions
// =============================================================================

#[tokio::test]
async fn test_math_round() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT name, math::round(val) AS rounded FROM mathtest"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 3);

    let data = &body["results"][0]["data"];
    let arr = data.as_array().expect("data should be an array");

    let mut results: Vec<(&str, i64)> = arr
        .iter()
        .map(|row| {
            (
                row["name"].as_str().unwrap(),
                row["rounded"].as_i64().unwrap(),
            )
        })
        .collect();
    results.sort_by_key(|(name, _)| *name);

    // 4.5 -> 5 (rounds to nearest even or up depending on implementation)
    // -3.7 -> -4
    // 0 -> 0
    assert_eq!(results[0].0, "neg");
    assert_eq!(results[0].1, -4);
    assert_eq!(results[1].0, "pos");
    // round(4.5) can be 4 or 5 depending on rounding mode
    assert!(results[1].1 == 4 || results[1].1 == 5);
    assert_eq!(results[2].0, "zero");
    assert_eq!(results[2].1, 0);
}

#[tokio::test]
async fn test_math_floor() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT name, math::floor(val) AS floored FROM mathtest"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 3);

    let data = &body["results"][0]["data"];
    let arr = data.as_array().expect("data should be an array");

    let mut results: Vec<(&str, i64)> = arr
        .iter()
        .map(|row| {
            (
                row["name"].as_str().unwrap(),
                row["floored"].as_i64().unwrap(),
            )
        })
        .collect();
    results.sort_by_key(|(name, _)| *name);

    // floor(4.5) = 4
    // floor(-3.7) = -4
    // floor(0) = 0
    assert_eq!(results, vec![("neg", -4), ("pos", 4), ("zero", 0)]);
}

#[tokio::test]
async fn test_math_ceil() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT name, math::ceil(val) AS ceiled FROM mathtest"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 3);

    let data = &body["results"][0]["data"];
    let arr = data.as_array().expect("data should be an array");

    let mut results: Vec<(&str, i64)> = arr
        .iter()
        .map(|row| {
            (
                row["name"].as_str().unwrap(),
                row["ceiled"].as_i64().unwrap(),
            )
        })
        .collect();
    results.sort_by_key(|(name, _)| *name);

    // ceil(4.5) = 5
    // ceil(-3.7) = -3
    // ceil(0) = 0
    assert_eq!(results, vec![("neg", -3), ("pos", 5), ("zero", 0)]);
}

// =============================================================================
// math::sqrt - square root
// =============================================================================

#[tokio::test]
async fn test_math_sqrt() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION sqrttest"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO sqrttest {"id": "s1", "val": 16}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT math::sqrt(val) AS result FROM sqrttest"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["data"][0]["result"], 4.0);
}

#[tokio::test]
async fn test_math_sqrt_zero() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION sqrttest2"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO sqrttest2 {"id": "s1", "val": 0}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT math::sqrt(val) AS result FROM sqrttest2"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["data"][0]["result"], 0.0);
}

#[tokio::test]
async fn test_math_sqrt_fractional() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION sqrttest3"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO sqrttest3 {"id": "s1", "val": 2}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT math::sqrt(val) AS result FROM sqrttest3"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    let result = body["results"][0]["data"][0]["result"].as_f64().unwrap();
    // sqrt(2) ~ 1.41421356
    #[allow(clippy::approx_constant)]
    let expected = 1.41421356;
    assert!((result - expected).abs() < 0.0001);
}

// =============================================================================
// math::pow - power function
// =============================================================================

#[tokio::test]
async fn test_math_pow() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION powtest"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO powtest {"id": "p1", "base": 2, "exp": 10}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT math::pow(base, exp) AS result FROM powtest"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["data"][0]["result"], 1024.0);
}

#[tokio::test]
async fn test_math_pow_zero_exponent() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION powtest2"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO powtest2 {"id": "p1", "base": 5, "exp": 0}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT math::pow(base, exp) AS result FROM powtest2"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    // Any number to the power of 0 is 1
    assert_eq!(body["results"][0]["data"][0]["result"], 1.0);
}

#[tokio::test]
async fn test_math_pow_negative_exponent() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION powtest3"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO powtest3 {"id": "p1", "base": 2, "exp": -2}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT math::pow(base, exp) AS result FROM powtest3"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    // 2^(-2) = 0.25
    assert_eq!(body["results"][0]["data"][0]["result"], 0.25);
}

// =============================================================================
// math::min, math::max - comparison functions
// =============================================================================

#[tokio::test]
async fn test_math_min() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, math::min(x, y) AS minimum FROM mathtest WHERE name = "pos""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    // min(3, 4) = 3
    assert_eq!(body["results"][0]["data"][0]["minimum"], 3);
}

#[tokio::test]
async fn test_math_max() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, math::max(x, y) AS maximum FROM mathtest WHERE name = "pos""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    // max(3, 4) = 4
    assert_eq!(body["results"][0]["data"][0]["maximum"], 4);
}

#[tokio::test]
async fn test_math_min_with_negative() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, math::min(x, y) AS minimum FROM mathtest WHERE name = "neg""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    // min(-5, 2) = -5
    assert_eq!(body["results"][0]["data"][0]["minimum"], -5);
}

#[tokio::test]
async fn test_math_max_with_negative() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, math::max(x, y) AS maximum FROM mathtest WHERE name = "neg""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    // max(-5, 2) = 2
    assert_eq!(body["results"][0]["data"][0]["maximum"], 2);
}

// =============================================================================
// math::pi and math::e - constants
// =============================================================================

#[tokio::test]
async fn test_math_pi() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION consttest"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO consttest {"id": "c1"}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT math::pi AS pi_val FROM consttest"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    let pi = body["results"][0]["data"][0]["pi_val"].as_f64().unwrap();
    // pi ~ 3.14159265
    assert!((pi - std::f64::consts::PI).abs() < 0.0001);
}

#[tokio::test]
async fn test_math_e() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION consttest2"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO consttest2 {"id": "c1"}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT math::e AS e_val FROM consttest2"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    let e = body["results"][0]["data"][0]["e_val"].as_f64().unwrap();
    // e ~ 2.71828182
    assert!((e - std::f64::consts::E).abs() < 0.0001);
}

// =============================================================================
// math::sin, math::cos - trigonometric functions
// =============================================================================

#[tokio::test]
async fn test_math_sin_zero() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION trigtest"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO trigtest {"id": "t1", "angle": 0}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT math::sin(angle) AS sin_val FROM trigtest"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    let sin_val = body["results"][0]["data"][0]["sin_val"].as_f64().unwrap();
    // sin(0) = 0
    assert!(sin_val.abs() < 0.0001);
}

#[tokio::test]
async fn test_math_cos_zero() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION trigtest2"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO trigtest2 {"id": "t1", "angle": 0}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT math::cos(angle) AS cos_val FROM trigtest2"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    let cos_val = body["results"][0]["data"][0]["cos_val"].as_f64().unwrap();
    // cos(0) = 1
    assert!((cos_val - 1.0).abs() < 0.0001);
}

#[tokio::test]
async fn test_math_sin_with_pi() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION trigtest3"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO trigtest3 {"id": "t1"}"#
        }))
        .send()
        .await
        .unwrap();

    // sin(pi) should be approximately 0
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT math::sin(math::pi) AS sin_pi FROM trigtest3"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    let sin_pi = body["results"][0]["data"][0]["sin_pi"].as_f64().unwrap();
    // sin(pi) ~ 0 (with floating point precision)
    assert!(sin_pi.abs() < 0.0001);
}

#[tokio::test]
async fn test_math_cos_with_pi() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION trigtest4"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO trigtest4 {"id": "t1"}"#
        }))
        .send()
        .await
        .unwrap();

    // cos(pi) should be approximately -1
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT math::cos(math::pi) AS cos_pi FROM trigtest4"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    let cos_pi = body["results"][0]["data"][0]["cos_pi"].as_f64().unwrap();
    // cos(pi) = -1
    assert!((cos_pi + 1.0).abs() < 0.0001);
}

// =============================================================================
// Chained math functions
// =============================================================================

#[tokio::test]
async fn test_chained_abs_sqrt() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    // sqrt(abs(val)) for negative value
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, math::sqrt(math::abs(val)) AS result FROM mathtest WHERE name = "neg""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    let result = body["results"][0]["data"][0]["result"].as_f64().unwrap();
    // sqrt(abs(-3.7)) = sqrt(3.7) ~ 1.9235
    assert!((result - 1.9235).abs() < 0.01);
}

#[tokio::test]
async fn test_chained_floor_pow() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    // pow(floor(val), 2) - square of floored value
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, math::pow(math::floor(val), 2) AS result FROM mathtest WHERE name = "pos""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    // floor(4.5) = 4, 4^2 = 16
    assert_eq!(body["results"][0]["data"][0]["result"], 16.0);
}

// =============================================================================
// Combining math functions with other SQL features
// =============================================================================

#[tokio::test]
async fn test_math_functions_with_order() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT name, math::abs(val) AS abs_val FROM mathtest ORDER math::abs(val) DESC"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 3);

    let data = &body["results"][0]["data"];
    let arr = data.as_array().expect("data should be an array");

    // Ordered by abs(val) descending: pos (4.5), neg (3.7), zero (0)
    assert_eq!(arr[0]["name"], "pos");
    assert_eq!(arr[1]["name"], "neg");
    assert_eq!(arr[2]["name"], "zero");
}

#[tokio::test]
async fn test_math_functions_with_where() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    // Filter where abs(val) > 3
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT name FROM mathtest WHERE math::abs(val) > 3"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 2);

    let mut names: Vec<&str> = body["results"][0]["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect();
    names.sort();
    // abs(4.5) > 3 and abs(-3.7) > 3
    assert_eq!(names, vec!["neg", "pos"]);
}

#[tokio::test]
async fn test_math_functions_with_limit() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT name, math::abs(val) AS abs_val FROM mathtest LIMIT 2"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 2);
}

#[tokio::test]
async fn test_math_in_complex_expression() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    // Calculate distance from origin: sqrt(x^2 + y^2)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, math::sqrt(math::pow(x, 2) + math::pow(y, 2)) AS dist FROM mathtest WHERE name = "pos""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    let dist = body["results"][0]["data"][0]["dist"].as_f64().unwrap();
    // sqrt(3^2 + 4^2) = sqrt(9 + 16) = sqrt(25) = 5
    assert!((dist - 5.0).abs() < 0.0001);
}

// =============================================================================
// math::pi and math::e - constants without parentheses
// =============================================================================

#[tokio::test]
async fn test_math_pi_no_parens() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION consttest_np"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO consttest_np {"id": "c1"}"#
        }))
        .send()
        .await
        .unwrap();

    // Test math::pi without parentheses
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT math::pi AS pi_val FROM consttest_np"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    let pi = body["results"][0]["data"][0]["pi_val"].as_f64().unwrap();
    assert!((pi - std::f64::consts::PI).abs() < 0.0001);
}

#[tokio::test]
async fn test_math_e_no_parens() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION consttest_np2"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO consttest_np2 {"id": "c1"}"#
        }))
        .send()
        .await
        .unwrap();

    // Test math::e without parentheses
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT math::e AS e_val FROM consttest_np2"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    let e = body["results"][0]["data"][0]["e_val"].as_f64().unwrap();
    assert!((e - std::f64::consts::E).abs() < 0.0001);
}

#[tokio::test]
async fn test_math_pi_in_expression() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION circles"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO circles {"id": "c1", "radius": 2}"#
        }))
        .send()
        .await
        .unwrap();

    // Calculate circle area: pi * r^2
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT math::pi * radius * radius AS area FROM circles"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    let area = body["results"][0]["data"][0]["area"].as_f64().unwrap();
    // pi * 4 ~ 12.566
    assert!((area - std::f64::consts::PI * 4.0).abs() < 0.0001);
}

#[tokio::test]
async fn test_math_const_with_function() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION exptest"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO exptest {"id": "e1", "x": 2}"#
        }))
        .send()
        .await
        .unwrap();

    // Test math::e as argument to pow: e^x
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT math::pow(math::e, x) AS exp_x FROM exptest"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    let exp_x = body["results"][0]["data"][0]["exp_x"].as_f64().unwrap();
    // e^2 ~ 7.389
    assert!((exp_x - std::f64::consts::E.powi(2)).abs() < 0.0001);
}

#[tokio::test]
async fn test_math_sin_with_pi_no_parens() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION trigtest_np"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO trigtest_np {"id": "t1"}"#
        }))
        .send()
        .await
        .unwrap();

    // sin(pi) should be approximately 0 - testing math::pi without parens inside a function call
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT math::sin(math::pi) AS sin_pi FROM trigtest_np"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    let sin_pi = body["results"][0]["data"][0]["sin_pi"].as_f64().unwrap();
    assert!(sin_pi.abs() < 0.0001);
}
