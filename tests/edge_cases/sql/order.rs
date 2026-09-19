//! ORDER clause integration tests

use crate::common;
use serde_json::json;

async fn setup_test_data(addr: &std::net::SocketAddr, client: &reqwest::Client) {
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION order_test"}))
        .send()
        .await
        .unwrap();

    for (id, name, score) in [
        ("c", "Charlie", 30),
        ("a", "Alice", 10),
        ("b", "Bob", 20),
        ("d", "David", 20),
    ] {
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": format!(r#"INSERT INTO order_test {{"id": "{}", "name": "{}", "score": {}}}"#, id, name, score)
            }))
            .send()
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn test_order_by_int_asc() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM order_test ORDER score"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"];
    assert_eq!(data[0]["id"], "order_test:a"); // score 10
}

#[tokio::test]
async fn test_order_by_int_desc() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM order_test ORDER score DESC"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());

    let data = &body["results"][0]["data"];
    assert_eq!(data[0]["id"], "order_test:c"); // score 30
}

#[tokio::test]
async fn test_order_by_string() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM order_test ORDER name"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());

    let data = &body["results"][0]["data"];
    assert_eq!(data[0]["name"], "Alice");
    assert_eq!(data[1]["name"], "Bob");
    assert_eq!(data[2]["name"], "Charlie");
    assert_eq!(data[3]["name"], "David");
}

#[tokio::test]
async fn test_order_with_limit() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM order_test ORDER score DESC LIMIT 2"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 2);

    let data = &body["results"][0]["data"];
    assert_eq!(data[0]["id"], "order_test:c"); // score 30
}

#[tokio::test]
async fn test_order_with_where() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM order_test WHERE score >= 20 ORDER name"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 3);

    let data = &body["results"][0]["data"];
    assert_eq!(data[0]["name"], "Bob");
    assert_eq!(data[1]["name"], "Charlie");
    assert_eq!(data[2]["name"], "David");
}

#[tokio::test]
async fn test_order_multiple_fields() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM order_test ORDER score ASC, name DESC"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());

    let data = &body["results"][0]["data"];
    // score 10: Alice
    assert_eq!(data[0]["name"], "Alice");
    // score 20: David, Bob (DESC by name)
    assert_eq!(data[1]["name"], "David");
    assert_eq!(data[2]["name"], "Bob");
    // score 30: Charlie
    assert_eq!(data[3]["name"], "Charlie");
}

#[tokio::test]
async fn test_order_empty_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION empty_order"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM empty_order ORDER name"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    assert_eq!(body["results"][0]["count"], 0);
}

// ========================
// ORDER BY expressions (not just fields)
// ========================

#[tokio::test]
async fn test_order_by_expression() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Setup: collection with price and quantity
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION products"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO products {name: 'A', price: 10, quantity: 3}, {name: 'B', price: 5, quantity: 8}, {name: 'C', price: 20, quantity: 1}"}))
        .send()
        .await
        .unwrap();

    // ORDER by computed expression: price * quantity
    // A: 10*3=30, B: 5*8=40, C: 20*1=20
    // Ascending: C(20), A(30), B(40)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM products ORDER price * quantity"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body["error"]);
    let data = body["results"][0]["data"].as_array().unwrap();
    assert_eq!(data.len(), 3);
    assert_eq!(data[0]["name"], "C"); // 20
    assert_eq!(data[1]["name"], "A"); // 30
    assert_eq!(data[2]["name"], "B"); // 40
}

#[tokio::test]
async fn test_order_by_expression_desc() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION products"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO products {name: 'A', price: 10, quantity: 3}, {name: 'B', price: 5, quantity: 8}, {name: 'C', price: 20, quantity: 1}"}))
        .send()
        .await
        .unwrap();

    // Descending: B(40), A(30), C(20)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM products ORDER price * quantity DESC"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body["error"]);
    let data = body["results"][0]["data"].as_array().unwrap();
    assert_eq!(data[0]["name"], "B"); // 40
    assert_eq!(data[1]["name"], "A"); // 30
    assert_eq!(data[2]["name"], "C"); // 20
}

#[tokio::test]
async fn test_order_by_function_call() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION users"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO users {name: 'Zoe'}, {name: 'alice'}, {name: 'Bob'}"}))
        .send()
        .await
        .unwrap();

    // ORDER by UPPER(name) - case-insensitive sorting
    // UPPER: ALICE, BOB, ZOE
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM users ORDER string::upper(name)"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body["error"]);
    let data = body["results"][0]["data"].as_array().unwrap();
    assert_eq!(data[0]["name"], "alice"); // ALICE
    assert_eq!(data[1]["name"], "Bob"); // BOB
    assert_eq!(data[2]["name"], "Zoe"); // ZOE
}

#[tokio::test]
async fn test_order_by_arithmetic_with_field() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION items"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO items {name: 'A', value: 100}, {name: 'B', value: 50}, {name: 'C', value: 75}"}))
        .send()
        .await
        .unwrap();

    // ORDER by value + 10 (just to test expression parsing)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM items ORDER value + 10 DESC"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body["error"]);
    let data = body["results"][0]["data"].as_array().unwrap();
    assert_eq!(data[0]["name"], "A"); // 110
    assert_eq!(data[1]["name"], "C"); // 85
    assert_eq!(data[2]["name"], "B"); // 60
}
