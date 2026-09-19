// Edge list limit clamp tests
// Tests that edge list handlers clamp limit to 1..1000

use crate::common;
use serde_json::json;

async fn setup_edges(addr: &std::net::SocketAddr, client: &reqwest::Client) {
    // Create some edges: alice follows bob, carol, dave
    for target in ["user:bob", "user:carol", "user:dave"] {
        client
            .post(format!(
                "http://{}/nodes/user:alice/edges/follows/{}",
                addr, target
            ))
            .json(&json!({}))
            .send()
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn test_edge_list_limit_zero_clamped_to_one() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_edges(&addr, &client).await;

    // Request with limit=0 -- should be clamped to 1
    let res = client
        .get(format!(
            "http://{}/nodes/user:alice/out/follows?limit=0",
            addr
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let edges = body["edges"].as_array().unwrap();
    // With limit clamped to 1, we should get at most 1 edge
    assert_eq!(edges.len(), 1, "limit=0 should be clamped to 1");
}

#[tokio::test]
async fn test_edge_list_limit_very_large_clamped_to_1000() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_edges(&addr, &client).await;

    // Request with limit=99999 -- should be clamped to 1000
    // We only have 3 edges so we can't verify the exact cap,
    // but we can verify it doesn't error and returns all 3
    let res = client
        .get(format!(
            "http://{}/nodes/user:alice/out/follows?limit=99999",
            addr
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let edges = body["edges"].as_array().unwrap();
    // All 3 edges should be returned (99999 clamped to 1000, which is still > 3)
    assert_eq!(
        edges.len(),
        3,
        "limit=99999 should be clamped to 1000 and return all 3 edges"
    );
}

#[tokio::test]
async fn test_edge_list_in_limit_zero_clamped() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create edges: bob, carol follow alice
    for source in ["user:bob", "user:carol"] {
        client
            .post(format!(
                "http://{}/nodes/{}/edges/follows/user:alice",
                addr, source
            ))
            .json(&json!({}))
            .send()
            .await
            .unwrap();
    }

    // Request incoming edges with limit=0
    let res = client
        .get(format!(
            "http://{}/nodes/user:alice/in/follows?limit=0",
            addr
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let edges = body["edges"].as_array().unwrap();
    assert_eq!(
        edges.len(),
        1,
        "limit=0 on incoming edges should be clamped to 1"
    );
}
