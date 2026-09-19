mod common;

use serde_json::json;

#[tokio::test]
async fn test_transaction_begin_commit() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // First create the collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    // BEGIN transaction
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "BEGIN"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["completed"], 1);
    assert_eq!(body["results"][0]["statement_type"], "BEGIN");
    let session_id = body["results"][0]["session_id"].as_str().unwrap();
    assert!(!session_id.is_empty());

    // INSERT within transaction
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", session_id)
        .json(&json!({"query": "INSERT INTO user {id: 'tx_alice', name: 'Alice'}"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["id"], "user:tx_alice");

    // SELECT within transaction (read-your-writes)
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", session_id)
        .json(&json!({"query": "SELECT * FROM user:tx_alice"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["name"], "Alice");

    // Document should NOT be visible outside transaction (no session header)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:tx_alice"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 0);

    // COMMIT transaction
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", session_id)
        .json(&json!({"query": "COMMIT"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["statement_type"], "COMMIT");

    // Now document should be visible (committed)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:tx_alice"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["name"], "Alice");
}

#[tokio::test]
async fn test_transaction_rollback() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // First create the collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    // BEGIN transaction
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "BEGIN"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let session_id = body["results"][0]["session_id"].as_str().unwrap();

    // INSERT within transaction
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", session_id)
        .json(&json!({"query": "INSERT INTO user {id: 'rollback_test', name: 'ToBeRolledBack'}"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);

    // Verify visible in transaction
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", session_id)
        .json(&json!({"query": "SELECT * FROM user:rollback_test"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1);

    // ROLLBACK transaction
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", session_id)
        .json(&json!({"query": "ROLLBACK"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["statement_type"], "ROLLBACK");

    // Document should NOT exist after rollback
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:rollback_test"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 0);
}

#[tokio::test]
async fn test_transaction_update_in_tx() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // First create the collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    // Create a document outside transaction
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'update_test', name: 'Original', age: 20}"}))
        .send()
        .await
        .unwrap();

    // BEGIN transaction
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "BEGIN"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let session_id = body["results"][0]["session_id"].as_str().unwrap();

    // UPDATE within transaction
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", session_id)
        .json(&json!({"query": "UPDATE user:update_test SET name = 'Updated', age = 25"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);

    // Verify change visible in transaction
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", session_id)
        .json(&json!({"query": "SELECT * FROM user:update_test"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["name"], "Updated");
    assert_eq!(body["results"][0]["data"][0]["age"], 25);

    // Outside transaction should still see original
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:update_test"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["name"], "Original");
    assert_eq!(body["results"][0]["data"][0]["age"], 20);

    // COMMIT
    client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", session_id)
        .json(&json!({"query": "COMMIT"}))
        .send()
        .await
        .unwrap();

    // Now outside should see the update
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:update_test"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0]["name"], "Updated");
    assert_eq!(body["results"][0]["data"][0]["age"], 25);
}

#[tokio::test]
async fn test_transaction_delete_in_tx() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // First create the collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    // Create a document outside transaction
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'delete_test', name: 'ToDelete'}"}))
        .send()
        .await
        .unwrap();

    // BEGIN transaction
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "BEGIN"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let session_id = body["results"][0]["session_id"].as_str().unwrap();

    // DELETE within transaction
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", session_id)
        .json(&json!({"query": "DELETE user:delete_test"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);

    // Should not be visible in transaction
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", session_id)
        .json(&json!({"query": "SELECT * FROM user:delete_test"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 0);

    // Should still be visible outside transaction
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:delete_test"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1);

    // COMMIT
    client
        .post(format!("http://{}/sql", addr))
        .header("X-Session-Id", session_id)
        .json(&json!({"query": "COMMIT"}))
        .send()
        .await
        .unwrap();

    // Now should be deleted outside as well
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:delete_test"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 0);
}
