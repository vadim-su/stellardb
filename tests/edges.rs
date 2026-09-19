mod common;

use serde_json::json;

#[tokio::test]
async fn test_create_edge() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!(
            "http://{}/nodes/user:alice/edges/follows/user:bob",
            addr
        ))
        .json(&json!({"since": 1705000000, "close": true}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 201);
    let edge: serde_json::Value = res.json().await.unwrap();
    // ID is now in format follows:<nanoid>
    assert!(edge["id"].as_str().unwrap().starts_with("follows:"));
    assert_eq!(edge["from"], "user:alice");
    assert_eq!(edge["to"], "user:bob");
    assert_eq!(edge["since"], 1705000000);
    assert_eq!(edge["close"], true);
}

#[tokio::test]
async fn test_get_edge() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create edge
    client
        .post(format!(
            "http://{}/nodes/user:alice/edges/follows/user:bob",
            addr
        ))
        .json(&json!({"weight": 5}))
        .send()
        .await
        .unwrap();

    // Get edge
    let res = client
        .get(format!(
            "http://{}/nodes/user:alice/edges/follows/user:bob",
            addr
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let edge: serde_json::Value = res.json().await.unwrap();
    // ID is now in format follows:<nanoid>
    assert!(edge["id"].as_str().unwrap().starts_with("follows:"));
    assert_eq!(edge["weight"], 5);
}

#[tokio::test]
async fn test_get_edge_not_found() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .get(format!(
            "http://{}/nodes/user:alice/edges/follows/user:nobody",
            addr
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn test_delete_edge() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create edge
    client
        .post(format!(
            "http://{}/nodes/user:alice/edges/likes/post:123",
            addr
        ))
        .json(&json!({}))
        .send()
        .await
        .unwrap();

    // Delete edge
    let res = client
        .delete(format!(
            "http://{}/nodes/user:alice/edges/likes/post:123",
            addr
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 204);

    // Verify deleted
    let res = client
        .get(format!(
            "http://{}/nodes/user:alice/edges/likes/post:123",
            addr
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn test_list_edges_out() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create edges: alice follows bob, carol, dave
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

    // List outgoing follows from alice
    let res = client
        .get(format!("http://{}/nodes/user:alice/out/follows", addr))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let edges = body["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 3);

    // Verify all targets are present
    let targets: Vec<&str> = edges.iter().map(|e| e["to"].as_str().unwrap()).collect();
    assert!(targets.contains(&"user:bob"));
    assert!(targets.contains(&"user:carol"));
    assert!(targets.contains(&"user:dave"));
}

#[tokio::test]
async fn test_list_edges_in() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create edges: bob, carol, dave follow alice
    for source in ["user:bob", "user:carol", "user:dave"] {
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

    // List incoming follows to alice
    let res = client
        .get(format!("http://{}/nodes/user:alice/in/follows", addr))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let edges = body["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 3);

    // Verify all sources are present
    let sources: Vec<&str> = edges.iter().map(|e| e["from"].as_str().unwrap()).collect();
    assert!(sources.contains(&"user:bob"));
    assert!(sources.contains(&"user:carol"));
    assert!(sources.contains(&"user:dave"));
}

#[tokio::test]
async fn test_traversal_expression_in_select() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create some users
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'alice', name: 'Alice'}"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'bob', name: 'Bob'}"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'carol', name: 'Carol'}"}))
        .send()
        .await
        .unwrap();

    // Create edges: alice follows bob and carol
    client
        .post(format!(
            "http://{}/nodes/user:alice/edges/follows/user:bob",
            addr
        ))
        .json(&json!({"since": 2024}))
        .send()
        .await
        .unwrap();

    client
        .post(format!(
            "http://{}/nodes/user:alice/edges/follows/user:carol",
            addr
        ))
        .json(&json!({"since": 2023}))
        .send()
        .await
        .unwrap();

    // Execute traversal query: get who alice follows
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT ->follows AS following FROM user:alice"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1);

    // The result should have a "following" field with an array of traversal results
    let data = &body["results"][0]["data"][0];
    assert!(data["following"].is_array());
    let following = data["following"].as_array().unwrap();
    assert_eq!(following.len(), 2);

    // Verify the traversal returned 2 edges with IDs starting with "follows:"
    // Note: basic traversal (->follows) returns only {id: ...}
    for edge in following {
        let id = edge["id"].as_str().unwrap_or("");
        assert!(
            id.starts_with("follows:"),
            "Expected ID starting with 'follows:', got: {}",
            id
        );
    }
}

#[tokio::test]
async fn test_traversal_to_node() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create users
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'alice', name: 'Alice'}"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'bob', name: 'Bob'}"}))
        .send()
        .await
        .unwrap();

    // Create edge: alice follows bob
    client
        .post(format!(
            "http://{}/nodes/user:alice/edges/follows/user:bob",
            addr
        ))
        .json(&json!({}))
        .send()
        .await
        .unwrap();

    // Execute traversal to node: get who alice follows (their user documents)
    // Use ->follows->user.* to get all fields of target nodes
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT ->follows->user.* AS friends FROM user:alice"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1);

    // The result should have "friends" with the target user nodes
    let data = &body["results"][0]["data"][0];
    assert!(data["friends"].is_array());
    let friends = data["friends"].as_array().unwrap();
    assert_eq!(friends.len(), 1);
    // With ->user.*, we get the full document including the name field
    assert_eq!(friends[0]["id"], "user:bob");
    assert_eq!(friends[0]["name"], "Bob");
}

#[tokio::test]
async fn test_delete_edge_sql() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create users
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'alice', name: 'Alice'}"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'bob', name: 'Bob'}"}))
        .send()
        .await
        .unwrap();

    // Create edge using RELATE
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "RELATE user:alice->follows->user:bob SET since = 2024"}))
        .send()
        .await
        .unwrap();

    // Verify edge exists
    let res = client
        .get(format!(
            "http://{}/nodes/user:alice/edges/follows/user:bob",
            addr
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    // Delete edge using SQL
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DELETE user:alice->follows->user:bob"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    // DeleteEdge returns data with "deleted" count
    assert_eq!(body["results"][0]["data"][0]["deleted"], 1);

    // Verify edge is gone
    let res = client
        .get(format!(
            "http://{}/nodes/user:alice/edges/follows/user:bob",
            addr
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn test_delete_all_edges_by_label_sql() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create users
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    for name in ["alice", "bob", "carol", "dave"] {
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": format!("INSERT INTO user {{id: '{}', name: '{}'}}", name, name.to_uppercase())}))
            .send()
            .await
            .unwrap();
    }

    // Create multiple edges: alice follows bob, carol, and dave
    for target in ["bob", "carol", "dave"] {
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({"query": format!("RELATE user:alice->follows->user:{}", target)}))
            .send()
            .await
            .unwrap();
    }

    // Verify edges exist
    let res = client
        .get(format!("http://{}/nodes/user:alice/out/follows", addr))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let edges = body["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 3);

    // Delete all follows edges from alice
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DELETE user:alice->follows"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    // DeleteEdge returns data with "deleted" count
    assert_eq!(body["results"][0]["data"][0]["deleted"], 3);

    // Verify all edges are gone
    let res = client
        .get(format!("http://{}/nodes/user:alice/out/follows", addr))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let edges = body["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 0);
}
