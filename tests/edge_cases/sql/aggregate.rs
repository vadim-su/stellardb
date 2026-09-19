//! Aggregate functions integration tests

use crate::common;
use serde_json::json;

async fn setup_test_data(addr: &std::net::SocketAddr, client: &reqwest::Client) {
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION agg_test"}))
        .send()
        .await
        .unwrap();

    for (id, name, score, active) in [
        ("a", "Alice", 10, true),
        ("b", "Bob", 20, true),
        ("c", "Charlie", 30, false),
        ("d", "David", 20, true),
    ] {
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": format!(
                    r#"INSERT INTO agg_test {{"id": "{}", "name": "{}", "score": {}, "active": {}}}"#,
                    id, name, score, active
                )
            }))
            .send()
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn test_count_star() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT COUNT(*) FROM agg_test"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"];
    assert_eq!(data[0]["count(*)"], 4);
}

#[tokio::test]
async fn test_sum() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT SUM(score) FROM agg_test"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"];
    assert_eq!(data[0]["sum(score)"], 80.0);
}

#[tokio::test]
async fn test_avg() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT AVG(score) FROM agg_test"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"];
    assert_eq!(data[0]["avg(score)"], 20.0);
}

#[tokio::test]
async fn test_min_max() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT MIN(score), MAX(score) FROM agg_test"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"];
    assert_eq!(data[0]["min(score)"], 10);
    assert_eq!(data[0]["max(score)"], 30);
}

#[tokio::test]
async fn test_count_with_where() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT COUNT(*) FROM agg_test WHERE score >= 20"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"];
    assert_eq!(data[0]["count(*)"], 3);
}

#[tokio::test]
async fn test_multiple_aggregates() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT COUNT(*), SUM(score), AVG(score) FROM agg_test"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"];
    assert_eq!(data[0]["count(*)"], 4);
    assert_eq!(data[0]["sum(score)"], 80.0);
    assert_eq!(data[0]["avg(score)"], 20.0);
}

#[tokio::test]
async fn test_aggregate_empty_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION empty_agg"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT COUNT(*), SUM(x) FROM empty_agg"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"];
    assert_eq!(data[0]["count(*)"], 0);
    assert!(data[0]["sum(x)"].is_null());
}

#[tokio::test]
async fn test_namespaced_aggregate() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT builtin::count(*) FROM agg_test"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"];
    assert_eq!(data[0]["count(*)"], 4);
}

#[tokio::test]
async fn test_explain_aggregate() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "EXPLAIN SELECT COUNT(*), SUM(score) FROM agg_test"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let text = body["results"][0]["data"][0].as_str().unwrap();
    assert!(
        text.contains("Aggregate"),
        "Expected Aggregate in plan: {}",
        text
    );
    assert!(
        text.contains("TableScan"),
        "Expected TableScan in plan: {}",
        text
    );
}
