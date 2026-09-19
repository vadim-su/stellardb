mod common;

use serde_json::json;

#[tokio::test]
async fn test_create_document() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/collections/user/documents", addr))
        .json(&json!({"name": "Alice", "email": "alice@example.com"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 201);
    let doc: serde_json::Value = res.json().await.unwrap();
    assert!(doc["id"].as_str().unwrap().starts_with("user:"));
    assert_eq!(doc["name"], "Alice");
    assert_eq!(doc["email"], "alice@example.com");
}

#[tokio::test]
async fn test_create_with_custom_id() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/collections/user/documents", addr))
        .json(&json!({"id": "user:alice", "name": "Alice"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 201);
    let doc: serde_json::Value = res.json().await.unwrap();
    assert_eq!(doc["id"], "user:alice");
    assert_eq!(doc["name"], "Alice");
}

#[tokio::test]
async fn test_create_conflict() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create first document
    client
        .post(format!("http://{}/collections/user/documents", addr))
        .json(&json!({"id": "user:bob", "name": "Bob"}))
        .send()
        .await
        .unwrap();

    // Try to create again with same id
    let res = client
        .post(format!("http://{}/collections/user/documents", addr))
        .json(&json!({"id": "user:bob", "name": "Bob 2"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 409);
}

#[tokio::test]
async fn test_get_document() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create document
    client
        .post(format!("http://{}/collections/user/documents", addr))
        .json(&json!({"id": "user:charlie", "name": "Charlie", "age": 30}))
        .send()
        .await
        .unwrap();

    // Get document
    let res = client
        .get(format!(
            "http://{}/collections/user/documents/charlie",
            addr
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let doc: serde_json::Value = res.json().await.unwrap();
    assert_eq!(doc["id"], "user:charlie");
    assert_eq!(doc["name"], "Charlie");
    assert_eq!(doc["age"], 30);
}

#[tokio::test]
async fn test_get_not_found() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .get(format!(
            "http://{}/collections/user/documents/nonexistent",
            addr
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn test_update_document() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create document
    client
        .post(format!("http://{}/collections/user/documents", addr))
        .json(&json!({"id": "user:dave", "name": "Dave", "age": 25}))
        .send()
        .await
        .unwrap();

    // Update document
    let res = client
        .put(format!("http://{}/collections/user/documents/dave", addr))
        .json(&json!({"name": "Dave Smith", "age": 26, "city": "NYC"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let doc: serde_json::Value = res.json().await.unwrap();
    assert_eq!(doc["id"], "user:dave");
    assert_eq!(doc["name"], "Dave Smith");
    assert_eq!(doc["age"], 26);
    assert_eq!(doc["city"], "NYC");
}

#[tokio::test]
async fn test_delete_document() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create document
    client
        .post(format!("http://{}/collections/user/documents", addr))
        .json(&json!({"id": "user:eve", "name": "Eve"}))
        .send()
        .await
        .unwrap();

    // Delete document
    let res = client
        .delete(format!("http://{}/collections/user/documents/eve", addr))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 204);

    // Verify it's deleted
    let res = client
        .get(format!("http://{}/collections/user/documents/eve", addr))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn test_delete_not_found() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .delete(format!(
            "http://{}/collections/user/documents/nonexistent",
            addr
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn test_list_documents_empty() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .get(format!("http://{}/collections/empty/documents", addr))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["documents"].as_array().unwrap().len(), 0);
    assert_eq!(body["next_cursor"], serde_json::Value::Null);
    assert_eq!(body["has_more"], false);
}

#[tokio::test]
async fn test_list_documents_pagination() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create 5 documents with sorted keys
    for name in ["alice", "bob", "carol", "dave", "eve"] {
        client
            .post(format!("http://{}/collections/user/documents", addr))
            .json(&json!({"id": format!("user:{}", name), "name": name}))
            .send()
            .await
            .unwrap();
    }

    // Get first page (limit 2)
    let res = client
        .get(format!(
            "http://{}/collections/user/documents?limit=2",
            addr
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let docs = body["documents"].as_array().unwrap();
    assert_eq!(docs.len(), 2);
    assert_eq!(docs[0]["id"], "user:alice");
    assert_eq!(docs[1]["id"], "user:bob");
    assert_eq!(body["has_more"], true);
    let cursor = body["next_cursor"].as_str().unwrap();
    assert_eq!(cursor, "bob");

    // Get second page using cursor
    let res = client
        .get(format!(
            "http://{}/collections/user/documents?limit=2&after={}",
            addr, cursor
        ))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let docs = body["documents"].as_array().unwrap();
    assert_eq!(docs.len(), 2);
    assert_eq!(docs[0]["id"], "user:carol");
    assert_eq!(docs[1]["id"], "user:dave");
    assert_eq!(body["has_more"], true);

    // Get last page
    let cursor = body["next_cursor"].as_str().unwrap();
    let res = client
        .get(format!(
            "http://{}/collections/user/documents?limit=2&after={}",
            addr, cursor
        ))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let docs = body["documents"].as_array().unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["id"], "user:eve");
    assert_eq!(body["has_more"], false);
    assert_eq!(body["next_cursor"], serde_json::Value::Null);
}

#[tokio::test]
async fn test_list_documents_default_limit() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create 3 documents
    for i in 1..=3 {
        client
            .post(format!("http://{}/collections/items/documents", addr))
            .json(&json!({"id": format!("items:item{}", i)}))
            .send()
            .await
            .unwrap();
    }

    // Get without limit (should use default 50)
    let res = client
        .get(format!("http://{}/collections/items/documents", addr))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["documents"].as_array().unwrap().len(), 3);
}
