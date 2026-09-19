// Expression integration tests
// Tests for arithmetic expressions in projections and WHERE clauses

use crate::common;
use serde_json::json;

async fn setup_orders(addr: &std::net::SocketAddr, client: &reqwest::Client) {
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION orders (item string, price int, quantity int)"}))
        .send()
        .await
        .unwrap();

    for (id, item, price, quantity) in [
        ("o1", "Widget", 5, 10),
        ("o2", "Gadget", 8, 3),
        ("o3", "Doohickey", 3, 7),
    ] {
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": format!(
                    r#"INSERT INTO orders {{"id": "{}", "item": "{}", "price": {}, "quantity": {}}}"#,
                    id, item, price, quantity
                )
            }))
            .send()
            .await
            .unwrap();
    }
}

// =============================================================================
// Arithmetic in Projection
// =============================================================================

#[tokio::test]
async fn test_arithmetic_in_projection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT price * quantity FROM orders"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"];
    let arr = data.as_array().expect("data should be an array");
    assert_eq!(arr.len(), 3, "Should return 3 rows");

    // The display name for price * quantity should be "price * quantity"
    let values: Vec<i64> = arr
        .iter()
        .map(|row| {
            // The key could be "price * quantity" based on expr_display_name
            let val = &row["price * quantity"];
            val.as_i64().expect("Should be an integer value")
        })
        .collect();

    let mut sorted = values.clone();
    sorted.sort();
    assert_eq!(sorted, vec![21, 24, 50], "Expected 5*10=50, 8*3=24, 3*7=21");
}

#[tokio::test]
async fn test_arithmetic_with_alias() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT price * quantity AS total FROM orders"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"];
    let arr = data.as_array().expect("data should be an array");
    assert_eq!(arr.len(), 3, "Should return 3 rows");

    // Check that the alias "total" is used as the key
    for row in arr {
        assert!(
            row.get("total").is_some() && !row["total"].is_null(),
            "Expected 'total' key in row: {:?}",
            row
        );
    }

    let mut values: Vec<i64> = arr
        .iter()
        .map(|row| row["total"].as_i64().unwrap())
        .collect();
    values.sort();
    assert_eq!(values, vec![21, 24, 50]);
}

// =============================================================================
// Expression in WHERE Clause
// =============================================================================

#[tokio::test]
async fn test_expression_in_where() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    // price * quantity > 25 => Widget(50) and Gadget(24 is NOT > 25), Doohickey(21 NOT > 25)
    // Only Widget qualifies: 5*10=50 > 25
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT * FROM orders WHERE price * quantity > 25"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(
        body["results"][0]["count"], 1,
        "Only Widget (50) should match > 25"
    );
    assert_eq!(body["results"][0]["data"][0]["item"], "Widget");
}

// =============================================================================
// Unary Negation in Projection
// =============================================================================

#[tokio::test]
async fn test_unary_neg_in_projection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT -price AS neg_price FROM orders"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"];
    let arr = data.as_array().expect("data should be an array");
    assert_eq!(arr.len(), 3);

    for row in arr {
        let neg = row["neg_price"].as_i64().expect("neg_price should be int");
        assert!(neg < 0, "neg_price should be negative, got {}", neg);
    }
}

// =============================================================================
// Object and Array Expressions in Projection
// =============================================================================

#[tokio::test]
async fn test_object_expr_in_select() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT {'item_name': item, 'total': price * quantity} AS info FROM orders"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = body["results"][0]["data"].as_array().unwrap();
    assert_eq!(data.len(), 3);

    for row in data {
        let info = &row["info"];
        assert!(info.is_object(), "info should be an object: {:?}", info);
        assert!(info["item_name"].is_string(), "item_name should be string");
        assert!(info["total"].is_number(), "total should be number");
    }
}

#[tokio::test]
async fn test_array_expr_in_select() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT [item, price, quantity] AS arr FROM orders"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = body["results"][0]["data"].as_array().unwrap();
    assert_eq!(data.len(), 3);

    for row in data {
        let arr = row["arr"].as_array().expect("arr should be an array");
        assert_eq!(arr.len(), 3);
        assert!(arr[0].is_string(), "first element should be string (item)");
        assert!(
            arr[1].is_number(),
            "second element should be number (price)"
        );
        assert!(
            arr[2].is_number(),
            "third element should be number (quantity)"
        );
    }
}

#[tokio::test]
async fn test_empty_object_expr() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT {} AS empty, item FROM orders LIMIT 1"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let row = &body["results"][0]["data"][0];
    assert_eq!(row["empty"], json!({}));
}

#[tokio::test]
async fn test_empty_array_expr() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT [] AS empty, item FROM orders LIMIT 1"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let row = &body["results"][0]["data"][0];
    assert_eq!(row["empty"], json!([]));
}

#[tokio::test]
async fn test_nested_object_in_array_expr() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT [{'name': item}, price] AS data FROM orders LIMIT 1"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let row = &body["results"][0]["data"][0];
    let data = row["data"].as_array().unwrap();
    assert_eq!(data.len(), 2);
    assert!(data[0].is_object(), "first element should be object");
    assert!(data[1].is_number(), "second element should be number");
}
