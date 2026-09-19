//! Multi-Index Atomic Operation Tests
//!
//! Tests that verify operations on documents with multiple index types
//! are handled atomically.

mod common;

use serde_json::json;

async fn wait_for_fts_commit() {
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
}

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

fn assert_success(body: &serde_json::Value) -> &serde_json::Value {
    assert!(
        body["error"].is_null(),
        "Query failed with error: {:?}",
        body["error"]
    );
    &body["results"][0]["data"]
}

/// Deleting a document should remove it from both BTree and FTS indexes.
#[tokio::test]
async fn test_delete_with_btree_and_fts() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Setup: collection with BTree and FTS indexes
    sql(&client, &addr, "DEFINE COLLECTION items").await;
    sql(&client, &addr, "CREATE INDEX ON items(category)").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON items(description) FULLTEXT",
    )
    .await;

    // Insert document
    sql(
        &client,
        &addr,
        r#"INSERT INTO items {id: "i1", category: "electronics", description: "laptop computer"}"#,
    )
    .await;
    wait_for_fts_commit().await;

    // Verify: BTree finds it
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM items WHERE category = "electronics""#,
    )
    .await;
    assert_eq!(assert_success(&body).as_array().unwrap().len(), 1);

    // Verify: FTS finds it
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM items WHERE description @@ "laptop""#,
    )
    .await;
    assert_eq!(assert_success(&body).as_array().unwrap().len(), 1);

    // Delete
    sql(&client, &addr, "DELETE items:i1").await;
    wait_for_fts_commit().await;

    // Verify: BTree no longer finds it
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM items WHERE category = "electronics""#,
    )
    .await;
    assert_eq!(
        assert_success(&body).as_array().unwrap().len(),
        0,
        "BTree should not find deleted document"
    );

    // Verify: FTS no longer finds it
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM items WHERE description @@ "laptop""#,
    )
    .await;
    assert_eq!(
        assert_success(&body).as_array().unwrap().len(),
        0,
        "FTS should not find deleted document"
    );
}

/// Deleting a document should remove it from both BTree and HNSW indexes.
#[tokio::test]
async fn test_delete_with_btree_and_vector() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    sql(&client, &addr, "DEFINE COLLECTION items").await;
    sql(&client, &addr, "CREATE INDEX ON items(category)").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON items(embedding) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    sql(
        &client,
        &addr,
        r#"INSERT INTO items {id: "i1", category: "electronics", embedding: [1.0, 0.0, 0.0]}"#,
    )
    .await;

    // Verify both indexes find it
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM items WHERE category = "electronics""#,
    )
    .await;
    assert_eq!(assert_success(&body).as_array().unwrap().len(), 1);

    let body = sql(
        &client,
        &addr,
        "SELECT id FROM items WHERE embedding <|1|> [1.0, 0.0, 0.0]",
    )
    .await;
    assert_eq!(assert_success(&body).as_array().unwrap().len(), 1);

    // Delete
    sql(&client, &addr, "DELETE items:i1").await;

    // Verify both indexes are updated
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM items WHERE category = "electronics""#,
    )
    .await;
    assert_eq!(
        assert_success(&body).as_array().unwrap().len(),
        0,
        "BTree should not find deleted document"
    );

    let body = sql(
        &client,
        &addr,
        "SELECT id FROM items WHERE embedding <|1|> [1.0, 0.0, 0.0]",
    )
    .await;
    assert_eq!(
        assert_success(&body).as_array().unwrap().len(),
        0,
        "HNSW should not find deleted document"
    );
}

/// Deleting a document should remove it from both FTS and HNSW indexes.
#[tokio::test]
async fn test_delete_with_fts_and_vector() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    sql(&client, &addr, "DEFINE COLLECTION items").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON items(description) FULLTEXT",
    )
    .await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON items(embedding) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    sql(
        &client,
        &addr,
        r#"INSERT INTO items {id: "i1", description: "searchable content", embedding: [1.0, 0.0, 0.0]}"#,
    )
    .await;
    wait_for_fts_commit().await;

    // Verify both find it
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM items WHERE description @@ "searchable""#,
    )
    .await;
    assert_eq!(assert_success(&body).as_array().unwrap().len(), 1);

    let body = sql(
        &client,
        &addr,
        "SELECT id FROM items WHERE embedding <|1|> [1.0, 0.0, 0.0]",
    )
    .await;
    assert_eq!(assert_success(&body).as_array().unwrap().len(), 1);

    // Delete
    sql(&client, &addr, "DELETE items:i1").await;
    wait_for_fts_commit().await;

    // Verify both updated
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM items WHERE description @@ "searchable""#,
    )
    .await;
    assert_eq!(
        assert_success(&body).as_array().unwrap().len(),
        0,
        "FTS should not find deleted document"
    );

    let body = sql(
        &client,
        &addr,
        "SELECT id FROM items WHERE embedding <|1|> [1.0, 0.0, 0.0]",
    )
    .await;
    assert_eq!(
        assert_success(&body).as_array().unwrap().len(),
        0,
        "HNSW should not find deleted document"
    );
}

/// Deleting a document should remove it from ALL index types atomically.
#[tokio::test]
async fn test_delete_with_all_three_index_types() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    sql(&client, &addr, "DEFINE COLLECTION items").await;
    sql(&client, &addr, "CREATE INDEX ON items(category)").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON items(description) FULLTEXT",
    )
    .await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON items(embedding) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    sql(
        &client,
        &addr,
        r#"INSERT INTO items {
            id: "i1",
            category: "electronics",
            description: "laptop computer device",
            embedding: [1.0, 0.0, 0.0]
        }"#,
    )
    .await;
    wait_for_fts_commit().await;

    // Verify all three find it
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM items WHERE category = "electronics""#,
    )
    .await;
    assert_eq!(
        assert_success(&body).as_array().unwrap().len(),
        1,
        "BTree should find it"
    );

    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM items WHERE description @@ "laptop""#,
    )
    .await;
    assert_eq!(
        assert_success(&body).as_array().unwrap().len(),
        1,
        "FTS should find it"
    );

    let body = sql(
        &client,
        &addr,
        "SELECT id FROM items WHERE embedding <|1|> [1.0, 0.0, 0.0]",
    )
    .await;
    assert_eq!(
        assert_success(&body).as_array().unwrap().len(),
        1,
        "HNSW should find it"
    );

    // Delete
    sql(&client, &addr, "DELETE items:i1").await;
    wait_for_fts_commit().await;

    // Verify ALL THREE are updated
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM items WHERE category = "electronics""#,
    )
    .await;
    assert_eq!(
        assert_success(&body).as_array().unwrap().len(),
        0,
        "BTree should not find deleted document"
    );

    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM items WHERE description @@ "laptop""#,
    )
    .await;
    assert_eq!(
        assert_success(&body).as_array().unwrap().len(),
        0,
        "FTS should not find deleted document"
    );

    let body = sql(
        &client,
        &addr,
        "SELECT id FROM items WHERE embedding <|1|> [1.0, 0.0, 0.0]",
    )
    .await;
    assert_eq!(
        assert_success(&body).as_array().unwrap().len(),
        0,
        "HNSW should not find deleted document"
    );
}
