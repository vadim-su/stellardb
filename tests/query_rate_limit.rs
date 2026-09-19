mod common;

use std::time::Duration;

use reqwest::StatusCode;
use serde_json::json;
use stellardb::server::admission::AdmissionConfig;

#[tokio::test]
async fn query_endpoints_enforce_per_client_rate_limit() {
    let (addr, _tmp) = common::spawn_server_with_admission(AdmissionConfig {
        query_client_rate_per_second: f64::EPSILON,
        query_client_burst: 1,
        max_query_clients: 10,
        query_client_retention: Duration::from_secs(60),
        ..Default::default()
    })
    .await;
    let client = common::test_client();

    let first = client
        .post(format!("http://{addr}/v1/query"))
        .header("X-Database", common::TEST_DB)
        .json(&json!({"query": "SELECT 1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let second = client
        .post(format!("http://{addr}/sql"))
        .header("X-Database", common::TEST_DB)
        .json(&json!({"query": "SELECT 1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
}
