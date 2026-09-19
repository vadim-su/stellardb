//! FTS Index Maintenance Tests
//!
//! Tests that verify FTS indexes are correctly updated when documents
//! are modified or deleted. These tests enforce STRICT verification -
//! no "eventual consistency" acceptance.

mod common;

use serde_json::json;

/// Wait for FTS background commit to complete.
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

/// When text content is updated, search should NOT find old content
/// and SHOULD find new content.
#[tokio::test]
async fn test_fts_update_removes_old_content() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Setup
    sql(&client, &addr, "DEFINE COLLECTION articles").await;
    sql(&client, &addr, "CREATE INDEX ON articles(content) FULLTEXT").await;

    // Insert document with "rust programming"
    sql(
        &client,
        &addr,
        r#"INSERT INTO articles {id: "a1", content: "rust programming language"}"#,
    )
    .await;
    wait_for_fts_commit().await;

    // Verify: search "rust" finds it
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM articles WHERE content @@ "rust""#,
    )
    .await;
    let data = assert_success(&body);
    assert_eq!(
        data.as_array().unwrap().len(),
        1,
        "Should find document with 'rust'"
    );

    // Update: change content to "python scripting"
    sql(
        &client,
        &addr,
        r#"UPDATE articles:a1 SET content = "python scripting language""#,
    )
    .await;
    wait_for_fts_commit().await;

    // Verify: search "rust" returns 0 results (STRICT)
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM articles WHERE content @@ "rust""#,
    )
    .await;
    let data = assert_success(&body);
    assert_eq!(
        data.as_array().unwrap().len(),
        0,
        "Old content 'rust' should NOT be found after update"
    );

    // Verify: search "python" finds the document
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM articles WHERE content @@ "python""#,
    )
    .await;
    let data = assert_success(&body);
    assert_eq!(
        data.as_array().unwrap().len(),
        1,
        "New content 'python' SHOULD be found after update"
    );
    assert_eq!(data[0]["id"], "articles:a1");
}

/// When a document is deleted, it should NOT appear in search results.
/// This test uses STRICT count verification.
#[tokio::test]
async fn test_fts_delete_removes_from_search() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Setup
    sql(&client, &addr, "DEFINE COLLECTION articles").await;
    sql(&client, &addr, "CREATE INDEX ON articles(content) FULLTEXT").await;

    // Insert 2 documents with "database"
    sql(
        &client,
        &addr,
        r#"INSERT INTO articles {id: "a1", content: "database systems"}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO articles {id: "a2", content: "database indexing"}"#,
    )
    .await;
    wait_for_fts_commit().await;

    // Verify: search "database" finds 2
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM articles WHERE content @@ "database""#,
    )
    .await;
    let data = assert_success(&body);
    assert_eq!(data.as_array().unwrap().len(), 2, "Should find 2 documents");

    // Delete one document
    sql(&client, &addr, "DELETE articles:a1").await;
    wait_for_fts_commit().await;

    // Verify: search "database" returns exactly 1 (STRICT)
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM articles WHERE content @@ "database""#,
    )
    .await;
    let data = assert_success(&body);
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 1, "Should find exactly 1 document after delete");
    assert_eq!(
        arr[0]["id"], "articles:a2",
        "Remaining document should be a2"
    );
}

/// Edge case: delete all documents, search should return empty.
#[tokio::test]
async fn test_fts_delete_all_documents() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    sql(&client, &addr, "DEFINE COLLECTION articles").await;
    sql(&client, &addr, "CREATE INDEX ON articles(content) FULLTEXT").await;

    sql(
        &client,
        &addr,
        r#"INSERT INTO articles {id: "a1", content: "searchable content"}"#,
    )
    .await;
    wait_for_fts_commit().await;

    // Verify exists
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM articles WHERE content @@ "searchable""#,
    )
    .await;
    assert_eq!(assert_success(&body).as_array().unwrap().len(), 1);

    // Delete
    sql(&client, &addr, "DELETE articles:a1").await;
    wait_for_fts_commit().await;

    // Search should return empty
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM articles WHERE content @@ "searchable""#,
    )
    .await;
    let data = assert_success(&body);
    assert_eq!(
        data.as_array().unwrap().len(),
        0,
        "Search should return empty after deleting all documents"
    );
}

/// When only part of the content changes, search should reflect both.
#[tokio::test]
async fn test_fts_update_partial_content() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    sql(&client, &addr, "DEFINE COLLECTION articles").await;
    sql(&client, &addr, "CREATE INDEX ON articles(content) FULLTEXT").await;

    // Insert with "rust database"
    sql(
        &client,
        &addr,
        r#"INSERT INTO articles {id: "a1", content: "rust database systems"}"#,
    )
    .await;
    wait_for_fts_commit().await;

    // Update to "rust indexing" (remove "database", add "indexing")
    sql(
        &client,
        &addr,
        r#"UPDATE articles:a1 SET content = "rust indexing systems""#,
    )
    .await;
    wait_for_fts_commit().await;

    // "rust" still findable
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM articles WHERE content @@ "rust""#,
    )
    .await;
    assert_eq!(assert_success(&body).as_array().unwrap().len(), 1);

    // "database" NOT findable
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM articles WHERE content @@ "database""#,
    )
    .await;
    assert_eq!(
        assert_success(&body).as_array().unwrap().len(),
        0,
        "Removed word 'database' should not be found"
    );

    // "indexing" IS findable
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM articles WHERE content @@ "indexing""#,
    )
    .await;
    assert_eq!(
        assert_success(&body).as_array().unwrap().len(),
        1,
        "Added word 'indexing' should be found"
    );
}
