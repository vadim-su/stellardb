//! Tests for transaction edge listing with pending writes.
//!
//! Verifies that edges created/deleted within a transaction are visible
//! in traversal queries before commit (read-your-writes semantics).

mod common;

use serde_json::json;

async fn sql(
    client: &reqwest::Client,
    addr: &std::net::SocketAddr,
    query: &str,
) -> serde_json::Value {
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": query}))
        .send()
        .await
        .unwrap();
    res.json().await.unwrap()
}

async fn sql_in_tx(
    client: &reqwest::Client,
    addr: &std::net::SocketAddr,
    session_id: &str,
    query: &str,
) -> serde_json::Value {
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", session_id)
        .json(&json!({"query": query}))
        .send()
        .await
        .unwrap();
    res.json().await.unwrap()
}

async fn begin_tx(client: &reqwest::Client, addr: &std::net::SocketAddr) -> String {
    let body = sql(client, addr, "BEGIN").await;
    body["results"][0]["session_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn assert_no_error(body: &serde_json::Value) {
    assert!(
        body["error"].is_null(),
        "Query returned error: {:?}",
        body["error"]
    );
}

fn get_data(body: &serde_json::Value) -> &serde_json::Value {
    &body["results"][0]["data"]
}

// =============================================================================
// Edge Creation Visible in Transaction
// =============================================================================

/// Edges created via RELATE inside a transaction should be visible
/// in traversal queries within the same transaction.
#[tokio::test]
async fn test_relate_visible_in_transaction_traversal() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Setup collections and documents
    sql(&client, &addr, "DEFINE COLLECTION users").await;
    sql(
        &client,
        &addr,
        "INSERT INTO users {id: 'alice', name: 'Alice'}",
    )
    .await;
    sql(&client, &addr, "INSERT INTO users {id: 'bob', name: 'Bob'}").await;
    sql(
        &client,
        &addr,
        "INSERT INTO users {id: 'charlie', name: 'Charlie'}",
    )
    .await;

    // Begin transaction
    let session_id = begin_tx(&client, &addr).await;

    // Create edges inside transaction
    let body = sql_in_tx(
        &client,
        &addr,
        &session_id,
        "RELATE users:alice -> follows -> users:bob",
    )
    .await;
    assert_no_error(&body);

    let body = sql_in_tx(
        &client,
        &addr,
        &session_id,
        "RELATE users:alice -> follows -> users:charlie",
    )
    .await;
    assert_no_error(&body);

    // Traverse within transaction: alice's outgoing edges should be visible
    let body = sql_in_tx(
        &client,
        &addr,
        &session_id,
        "SELECT ->follows->users.name AS friends FROM users:alice",
    )
    .await;
    assert_no_error(&body);
    let data = get_data(&body);
    let friends = data[0]["friends"].as_array().unwrap();
    assert_eq!(
        friends.len(),
        2,
        "Should see 2 friends in transaction traversal, got: {:?}",
        friends
    );

    // Outside transaction: edges should NOT be visible
    let body = sql(
        &client,
        &addr,
        "SELECT ->follows->users.name AS friends FROM users:alice",
    )
    .await;
    assert_no_error(&body);
    let data = get_data(&body);
    let friends = data[0]["friends"].as_array().unwrap();
    assert_eq!(
        friends.len(),
        0,
        "Should see 0 friends outside transaction before commit"
    );

    // Commit
    sql_in_tx(&client, &addr, &session_id, "COMMIT").await;

    // After commit: edges should be visible
    let body = sql(
        &client,
        &addr,
        "SELECT ->follows->users.name AS friends FROM users:alice",
    )
    .await;
    assert_no_error(&body);
    let data = get_data(&body);
    let friends = data[0]["friends"].as_array().unwrap();
    assert_eq!(friends.len(), 2, "Should see 2 friends after commit");
}

/// Edges deleted inside a transaction should not appear in traversals
/// within the same transaction.
#[tokio::test]
async fn test_edge_delete_hidden_in_transaction_traversal() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Setup
    sql(&client, &addr, "DEFINE COLLECTION users").await;
    sql(
        &client,
        &addr,
        "INSERT INTO users {id: 'alice', name: 'Alice'}",
    )
    .await;
    sql(&client, &addr, "INSERT INTO users {id: 'bob', name: 'Bob'}").await;
    sql(&client, &addr, "RELATE users:alice -> follows -> users:bob").await;

    // Verify edge exists
    let body = sql(
        &client,
        &addr,
        "SELECT ->follows->users.name AS friends FROM users:alice",
    )
    .await;
    let friends = get_data(&body)[0]["friends"].as_array().unwrap();
    assert_eq!(friends.len(), 1);

    // Begin transaction and delete edge
    let session_id = begin_tx(&client, &addr).await;

    let body = sql_in_tx(
        &client,
        &addr,
        &session_id,
        "DELETE users:alice->follows->users:bob",
    )
    .await;
    assert_no_error(&body);

    // Within transaction: edge should be gone
    let body = sql_in_tx(
        &client,
        &addr,
        &session_id,
        "SELECT ->follows->users.name AS friends FROM users:alice",
    )
    .await;
    assert_no_error(&body);
    let friends = get_data(&body)[0]["friends"].as_array().unwrap();
    assert_eq!(
        friends.len(),
        0,
        "Deleted edge should not appear in transaction traversal"
    );

    // Outside transaction: edge should still be visible
    let body = sql(
        &client,
        &addr,
        "SELECT ->follows->users.name AS friends FROM users:alice",
    )
    .await;
    let friends = get_data(&body)[0]["friends"].as_array().unwrap();
    assert_eq!(
        friends.len(),
        1,
        "Edge should still be visible outside uncommitted transaction"
    );

    // Rollback
    sql_in_tx(&client, &addr, &session_id, "ROLLBACK").await;

    // After rollback: edge should still exist
    let body = sql(
        &client,
        &addr,
        "SELECT ->follows->users.name AS friends FROM users:alice",
    )
    .await;
    let friends = get_data(&body)[0]["friends"].as_array().unwrap();
    assert_eq!(friends.len(), 1, "Edge should survive rollback");
}

/// Incoming edge traversal should also see pending writes.
#[tokio::test]
async fn test_incoming_edge_visible_in_transaction() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    sql(&client, &addr, "DEFINE COLLECTION users").await;
    sql(
        &client,
        &addr,
        "INSERT INTO users {id: 'alice', name: 'Alice'}",
    )
    .await;
    sql(&client, &addr, "INSERT INTO users {id: 'bob', name: 'Bob'}").await;

    let session_id = begin_tx(&client, &addr).await;

    // Create edge: alice follows bob (so bob has incoming "follows" from alice)
    sql_in_tx(
        &client,
        &addr,
        &session_id,
        "RELATE users:alice -> follows -> users:bob",
    )
    .await;

    // Query incoming edges for bob within transaction
    let body = sql_in_tx(
        &client,
        &addr,
        &session_id,
        "SELECT <-follows<-users.name AS followers FROM users:bob",
    )
    .await;
    assert_no_error(&body);
    let followers = get_data(&body)[0]["followers"].as_array().unwrap();
    assert_eq!(
        followers.len(),
        1,
        "Incoming edge should be visible in transaction: {:?}",
        followers
    );

    sql_in_tx(&client, &addr, &session_id, "COMMIT").await;
}

/// Mixed insert + relate in a transaction should work for traversal.
#[tokio::test]
async fn test_insert_and_relate_in_same_transaction() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    sql(&client, &addr, "DEFINE COLLECTION users").await;
    sql(
        &client,
        &addr,
        "INSERT INTO users {id: 'alice', name: 'Alice'}",
    )
    .await;

    let session_id = begin_tx(&client, &addr).await;

    // Insert new doc AND create edge to it within same tx
    sql_in_tx(
        &client,
        &addr,
        &session_id,
        "INSERT INTO users {id: 'new_friend', name: 'NewFriend'}",
    )
    .await;
    sql_in_tx(
        &client,
        &addr,
        &session_id,
        "RELATE users:alice -> follows -> users:new_friend",
    )
    .await;

    // Traversal should find the new doc through the new edge
    let body = sql_in_tx(
        &client,
        &addr,
        &session_id,
        "SELECT ->follows->users.name AS friends FROM users:alice",
    )
    .await;
    assert_no_error(&body);
    let friends = get_data(&body)[0]["friends"].as_array().unwrap();
    assert!(
        !friends.is_empty(),
        "New doc should be reachable via new edge in transaction: {:?}",
        friends
    );

    sql_in_tx(&client, &addr, &session_id, "COMMIT").await;

    // Verify after commit
    let body = sql(
        &client,
        &addr,
        "SELECT ->follows->users.name AS friends FROM users:alice",
    )
    .await;
    let friends = get_data(&body)[0]["friends"].as_array().unwrap();
    assert!(!friends.is_empty(), "Traversal should work after commit");
}
