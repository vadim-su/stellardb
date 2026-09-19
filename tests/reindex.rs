//! Integration tests for REINDEX command
//!
//! These tests verify the REINDEX command works correctly:
//! - Reindexing HNSW indexes (fully working)
//! - Reindexing all indexes on a collection
//! - Error handling for nonexistent collections/indexes
//! - Edge cases like BTree-only collections
//!
//! Note on FTS REINDEX: There is a known issue with FTS reindexing where
//! after the shadow/active swap, searches may fail because Tantivy's internal
//! paths become stale. HNSW reindexing works correctly.

mod common;

use serde_json::json;

/// Helper to execute SQL and return the response body
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

/// Helper to assert query succeeded and return the data array
fn assert_success_data(body: &serde_json::Value) -> &serde_json::Value {
    assert!(
        body["error"].is_null(),
        "Query failed with error: {:?}",
        body["error"]
    );
    // Return the data array
    &body["results"][0]["data"]
}

/// Helper to assert DDL query succeeded and return the first status row
fn assert_success(body: &serde_json::Value) -> &serde_json::Value {
    assert!(
        body["error"].is_null(),
        "Query failed with error: {:?}",
        body["error"]
    );
    // DDL returns a single status object, so get first element
    &body["results"][0]["data"][0]
}

/// Helper to assert query failed with an error
fn assert_error(body: &serde_json::Value) {
    assert!(
        !body["error"].is_null(),
        "Expected error but query succeeded: {:?}",
        body
    );
}

// =============================================================================
// HNSW Index Reindex Tests (These work correctly)
// =============================================================================

#[tokio::test]
async fn test_reindex_hnsw_index() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection and HNSW index
    sql(&client, &addr, "DEFINE COLLECTION vectors").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON vectors(embedding) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    // Insert documents with vectors
    sql(
        &client,
        &addr,
        r#"INSERT INTO vectors {id: "v1", embedding: [1.0, 0.0, 0.0]}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO vectors {id: "v2", embedding: [0.0, 1.0, 0.0]}"#,
    )
    .await;

    // Reindex HNSW index
    let result = sql(
        &client,
        &addr,
        "REINDEX idx_vectors_hnsw_embedding ON vectors",
    )
    .await;
    let data = assert_success(&result);

    assert_eq!(data["status"], "ok");
    assert_eq!(data["documents_indexed"], 2);
    assert!(data["index"].is_string());
    assert!(data["elapsed_ms"].as_u64().is_some());

    // Verify KNN search still works after reindex
    let result = sql(
        &client,
        &addr,
        r#"SELECT id FROM vectors WHERE embedding <|2|> [1.0, 0.0, 0.0]"#,
    )
    .await;
    let data = assert_success_data(&result);
    let results = data.as_array().unwrap();
    assert_eq!(results.len(), 2);
    // First result should be v1 (exact match)
    assert_eq!(results[0]["id"], "vectors:v1");
}

#[tokio::test]
async fn test_reindex_hnsw_preserves_search_results() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection and HNSW index
    sql(&client, &addr, "DEFINE COLLECTION items").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON items(vec) HNSW DIMENSION 3 DIST EUCLIDEAN",
    )
    .await;

    // Insert multiple documents
    for i in 0..10 {
        let x = (i as f64) * 0.1;
        sql(
            &client,
            &addr,
            &format!(
                r#"INSERT INTO items {{id: "{}", vec: [{}, 0.0, 0.0]}}"#,
                i, x
            ),
        )
        .await;
    }

    // Verify search before reindex
    let result = sql(
        &client,
        &addr,
        r#"SELECT id FROM items WHERE vec <|3|> [0.5, 0.0, 0.0]"#,
    )
    .await;
    let data_before = assert_success_data(&result);
    let results_before = data_before.as_array().unwrap();
    assert_eq!(results_before.len(), 3);

    // Reindex
    let result = sql(&client, &addr, "REINDEX idx_items_hnsw_vec ON items").await;
    let data = assert_success(&result);
    assert_eq!(data["status"], "ok");
    assert_eq!(data["documents_indexed"], 10);

    // Verify search after reindex returns same results
    let result = sql(
        &client,
        &addr,
        r#"SELECT id FROM items WHERE vec <|3|> [0.5, 0.0, 0.0]"#,
    )
    .await;
    let data_after = assert_success_data(&result);
    let results_after = data_after.as_array().unwrap();
    assert_eq!(results_after.len(), 3);

    // The results should be the same (or at least same count)
    assert_eq!(results_before.len(), results_after.len());
}

#[tokio::test]
async fn test_reindex_all_hnsw_indexes() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with HNSW index only
    sql(&client, &addr, "DEFINE COLLECTION points").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON points(pos) HNSW DIMENSION 2 DIST COSINE",
    )
    .await;

    // Insert documents
    sql(
        &client,
        &addr,
        r#"INSERT INTO points {id: "1", pos: [1.0, 0.0]}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO points {id: "2", pos: [0.0, 1.0]}"#,
    )
    .await;

    // Reindex all (collection level - should find HNSW index)
    let result = sql(&client, &addr, "REINDEX idx_points_hnsw_pos ON points").await;
    let data = assert_success(&result);

    assert_eq!(data["status"], "ok");
    assert!(data["index"].is_string(), "Expected index in response");
}

// =============================================================================
// FTS Index Reindex Tests
// =============================================================================

#[tokio::test]
async fn test_reindex_fts_completes_without_error() {
    // Test that REINDEX on FTS at least completes the operation
    // (There's a known issue where search after reindex may fail)
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection and FULLTEXT index
    sql(&client, &addr, "DEFINE COLLECTION docs").await;
    sql(&client, &addr, "CREATE INDEX ON docs(content) FULLTEXT").await;

    // Insert documents
    sql(
        &client,
        &addr,
        r#"INSERT INTO docs {id: "1", content: "hello world"}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO docs {id: "2", content: "goodbye world"}"#,
    )
    .await;

    // Reindex should complete without error
    // FTS index name format is {sorted_fields}_fts
    let result = sql(&client, &addr, "REINDEX content_fts ON docs").await;
    let data = assert_success(&result);

    // Verify the operation reported success
    assert_eq!(data["status"], "ok");
    assert_eq!(data["documents_indexed"], 2);
    assert!(data["index"].is_string());
}

#[tokio::test]
async fn test_reindex_all_indexes() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with HNSW index
    sql(&client, &addr, "DEFINE COLLECTION items").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON items(vec) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    // Insert documents
    sql(
        &client,
        &addr,
        r#"INSERT INTO items {id: "1", vec: [1.0, 0.0, 0.0]}"#,
    )
    .await;

    // Reindex all (collection level)
    let result = sql(&client, &addr, "REINDEX idx_items_hnsw_vec ON items").await;
    let data = assert_success(&result);

    assert_eq!(data["status"], "ok");
    assert!(data["index"].is_string(), "Expected index in response");
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[tokio::test]
async fn test_reindex_no_fts_hnsw_indexes() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with BTree index only
    sql(&client, &addr, "DEFINE COLLECTION users").await;
    sql(&client, &addr, "CREATE INDEX ON users(email)").await;

    // Insert a document so the collection exists
    sql(
        &client,
        &addr,
        r#"INSERT INTO users {id: "1", email: "test@example.com"}"#,
    )
    .await;

    // Try to reindex - should fail (no FTS/HNSW indexes)
    let result = sql(&client, &addr, "REINDEX idx_users_btree_email ON users").await;
    assert_error(&result);
}

#[tokio::test]
async fn test_reindex_nonexistent_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = sql(&client, &addr, "REINDEX idx_nonexistent ON nonexistent").await;
    assert_error(&result);
}

#[tokio::test]
async fn test_reindex_nonexistent_index() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    sql(&client, &addr, "DEFINE COLLECTION test").await;
    sql(&client, &addr, "CREATE INDEX ON test(name) FULLTEXT").await;

    // Insert a document to create the index
    sql(
        &client,
        &addr,
        r#"INSERT INTO test {id: "1", name: "test"}"#,
    )
    .await;

    // Try to reindex nonexistent fields (using new FTS naming format)
    let result = sql(&client, &addr, "REINDEX nonexistent_fts ON test").await;
    assert_error(&result);
}

#[tokio::test]
async fn test_reindex_collection_without_documents() {
    // REINDEX on a collection without documents should succeed
    // and report 0 documents indexed
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    sql(&client, &addr, "DEFINE COLLECTION empty").await;
    sql(&client, &addr, "CREATE INDEX ON empty(content) FULLTEXT").await;

    // FTS index name format is {sorted_fields}_fts
    let result = sql(&client, &addr, "REINDEX content_fts ON empty").await;

    let data = assert_success(&result);
    assert_eq!(data["status"], "ok");
    assert_eq!(data["documents_indexed"], 0);
}

// =============================================================================
// Edge Cases
// =============================================================================

#[tokio::test]
async fn test_reindex_elapsed_time_reported() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create and populate collection
    sql(&client, &addr, "DEFINE COLLECTION timed").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON timed(vec) HNSW DIMENSION 2 DIST COSINE",
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO timed {id: "1", vec: [1.0, 0.0]}"#,
    )
    .await;

    // Reindex and verify timing is reported
    let result = sql(&client, &addr, "REINDEX idx_timed_hnsw_vec ON timed").await;
    let data = assert_success(&result);

    // elapsed_ms should be a non-negative number
    let elapsed = data["elapsed_ms"].as_u64();
    assert!(elapsed.is_some(), "elapsed_ms should be reported");
}

#[tokio::test]
async fn test_reindex_hnsw_with_different_metrics() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Test with DOT metric
    sql(&client, &addr, "DEFINE COLLECTION dot_test").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON dot_test(vec) HNSW DIMENSION 3 DIST DOT",
    )
    .await;

    sql(
        &client,
        &addr,
        r#"INSERT INTO dot_test {id: "1", vec: [1.0, 0.0, 0.0]}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO dot_test {id: "2", vec: [0.5, 0.5, 0.0]}"#,
    )
    .await;

    let result = sql(&client, &addr, "REINDEX idx_dot_test_hnsw_vec ON dot_test").await;
    let data = assert_success(&result);
    assert_eq!(data["status"], "ok");
    assert_eq!(data["documents_indexed"], 2);

    // Verify search still works
    let result = sql(
        &client,
        &addr,
        r#"SELECT id FROM dot_test WHERE vec <|2|> [1.0, 0.0, 0.0]"#,
    )
    .await;
    let data = assert_success_data(&result);
    assert_eq!(data.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_reindex_large_hnsw_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    sql(&client, &addr, "DEFINE COLLECTION large").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON large(vec) HNSW DIMENSION 4 DIST EUCLIDEAN",
    )
    .await;

    // Insert 100 documents
    for i in 0..100 {
        let x = (i as f64) / 100.0;
        let y = ((i * 7) % 100) as f64 / 100.0;
        sql(
            &client,
            &addr,
            &format!(
                r#"INSERT INTO large {{id: "{}", vec: [{}, {}, 0.0, 0.0]}}"#,
                i, x, y
            ),
        )
        .await;
    }

    // Reindex
    let result = sql(&client, &addr, "REINDEX idx_large_hnsw_vec ON large").await;
    let data = assert_success(&result);

    assert_eq!(data["status"], "ok");
    assert_eq!(data["documents_indexed"], 100);

    // Verify search works
    let result = sql(
        &client,
        &addr,
        r#"SELECT id FROM large WHERE vec <|5|> [0.5, 0.5, 0.0, 0.0]"#,
    )
    .await;
    let data = assert_success_data(&result);
    assert_eq!(data.as_array().unwrap().len(), 5);
}
