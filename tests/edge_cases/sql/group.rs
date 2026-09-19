//! GROUP clause integration tests

use crate::common;
use serde_json::json;

async fn setup_orders(addr: &std::net::SocketAddr, client: &reqwest::Client) {
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION orders"}))
        .send()
        .await
        .unwrap();

    let queries = [
        r#"INSERT INTO orders {id: "1", status: "active", city: "NYC", amount: 10}"#,
        r#"INSERT INTO orders {id: "2", status: "inactive", city: "LA", amount: 20}"#,
        r#"INSERT INTO orders {id: "3", status: "active", city: "NYC", amount: 30}"#,
        r#"INSERT INTO orders {id: "4", status: "active", city: "LA", amount: 5}"#,
    ];

    for q in queries {
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": q}))
            .send()
            .await
            .unwrap();
    }
}

async fn query(
    addr: &std::net::SocketAddr,
    client: &reqwest::Client,
    sql: &str,
) -> serde_json::Value {
    let resp = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": sql}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].is_null(), "SQL error: {:?}", body);
    body["results"][0]["data"].clone()
}

// =============================================================================
// Basic GROUP
// =============================================================================

#[tokio::test]
async fn test_group_count() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT status, COUNT(*) FROM orders GROUP status",
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 2);

    let active = arr.iter().find(|v| v["status"] == "active").unwrap();
    assert_eq!(active["count(*)"], 3);

    let inactive = arr.iter().find(|v| v["status"] == "inactive").unwrap();
    assert_eq!(inactive["count(*)"], 1);
}

#[tokio::test]
async fn test_group_sum() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT status, SUM(amount) FROM orders GROUP status",
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 2);

    let active = arr.iter().find(|v| v["status"] == "active").unwrap();
    assert_eq!(active["sum(amount)"], 45.0);
}

#[tokio::test]
async fn test_group_multiple_aggregates() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT status, COUNT(*), SUM(amount), AVG(amount) FROM orders GROUP status",
    )
    .await;
    let arr = data.as_array().unwrap();

    let active = arr.iter().find(|v| v["status"] == "active").unwrap();
    assert_eq!(active["count(*)"], 3);
    assert_eq!(active["sum(amount)"], 45.0);
    assert_eq!(active["avg(amount)"], 15.0);
}

// =============================================================================
// Multiple GROUP fields
// =============================================================================

#[tokio::test]
async fn test_group_multiple_fields() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT status, city, COUNT(*) FROM orders GROUP status, city",
    )
    .await;
    let arr = data.as_array().unwrap();
    // active+NYC=2, active+LA=1, inactive+LA=1
    assert_eq!(arr.len(), 3);

    let active_nyc = arr
        .iter()
        .find(|v| v["status"] == "active" && v["city"] == "NYC")
        .unwrap();
    assert_eq!(active_nyc["count(*)"], 2);
}

// =============================================================================
// GROUP with WHERE
// =============================================================================

#[tokio::test]
async fn test_group_with_where() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT status, SUM(amount) FROM orders WHERE amount > 5 GROUP status",
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 2);

    // active: 10+30=40, inactive: 20
    let active = arr.iter().find(|v| v["status"] == "active").unwrap();
    assert_eq!(active["sum(amount)"], 40.0);

    let inactive = arr.iter().find(|v| v["status"] == "inactive").unwrap();
    assert_eq!(inactive["sum(amount)"], 20.0);
}

// =============================================================================
// GROUP with alias
// =============================================================================

#[tokio::test]
async fn test_group_with_alias() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT status, COUNT(*) AS total FROM orders GROUP status",
    )
    .await;
    let arr = data.as_array().unwrap();

    let active = arr.iter().find(|v| v["status"] == "active").unwrap();
    assert_eq!(active["total"], 3);
}

// =============================================================================
// GROUP empty collection
// =============================================================================

#[tokio::test]
async fn test_group_empty_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION empty_orders"}))
        .send()
        .await
        .unwrap();

    let data = query(
        &addr,
        &client,
        "SELECT status, COUNT(*) FROM empty_orders GROUP status",
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 0);
}

// =============================================================================
// GROUP with ORDER
// =============================================================================

#[tokio::test]
async fn test_group_order_by_group_field_asc() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT status, COUNT(*) FROM orders GROUP status ORDER status",
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["status"], "active");
    assert_eq!(arr[1]["status"], "inactive");
}

#[tokio::test]
async fn test_group_order_by_group_field_desc() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT status, COUNT(*) FROM orders GROUP status ORDER status DESC",
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["status"], "inactive");
    assert_eq!(arr[1]["status"], "active");
}

#[tokio::test]
async fn test_group_order_by_aggregate() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    // Use alias so ORDER can reference it as a plain field
    let data = query(
        &addr,
        &client,
        "SELECT status, SUM(amount) AS total FROM orders GROUP status ORDER total",
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    // inactive=20, active=45 → ascending
    assert_eq!(arr[0]["status"], "inactive");
    assert_eq!(arr[0]["total"], 20.0);
    assert_eq!(arr[1]["status"], "active");
    assert_eq!(arr[1]["total"], 45.0);
}

#[tokio::test]
async fn test_group_order_by_aggregate_desc() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT status, SUM(amount) AS total FROM orders GROUP status ORDER total DESC",
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["status"], "active");
    assert_eq!(arr[0]["total"], 45.0);
    assert_eq!(arr[1]["status"], "inactive");
    assert_eq!(arr[1]["total"], 20.0);
}

#[tokio::test]
async fn test_group_order_multiple_fields() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    // GROUP by status,city ORDER by status, then city
    let data = query(
        &addr,
        &client,
        "SELECT status, city, COUNT(*) FROM orders GROUP status, city ORDER status, city",
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    // active+LA, active+NYC, inactive+LA
    assert_eq!(arr[0]["status"], "active");
    assert_eq!(arr[0]["city"], "LA");
    assert_eq!(arr[1]["status"], "active");
    assert_eq!(arr[1]["city"], "NYC");
    assert_eq!(arr[2]["status"], "inactive");
    assert_eq!(arr[2]["city"], "LA");
}

#[tokio::test]
async fn test_group_order_with_where() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_orders(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT status, SUM(amount) AS total FROM orders WHERE amount > 5 GROUP status ORDER total DESC",
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    // active: 10+30=40, inactive: 20 → desc: active first
    assert_eq!(arr[0]["status"], "active");
    assert_eq!(arr[0]["total"], 40.0);
    assert_eq!(arr[1]["status"], "inactive");
    assert_eq!(arr[1]["total"], 20.0);
}

// =============================================================================
// EXPLAIN GROUP
// =============================================================================

#[tokio::test]
async fn test_explain_group() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION orders"}))
        .send()
        .await
        .unwrap();

    let data = query(
        &addr,
        &client,
        "EXPLAIN SELECT status, COUNT(*) FROM orders GROUP status",
    )
    .await;
    let text = data.as_array().unwrap()[0].as_str().unwrap();
    assert!(
        text.contains("GroupAggregate"),
        "Expected GroupAggregate in plan: {}",
        text
    );
    assert!(
        text.contains("group_by: status"),
        "Expected group field in plan: {}",
        text
    );
}
