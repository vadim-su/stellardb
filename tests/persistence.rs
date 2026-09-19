//! Persistence integration tests
//!
//! Tests that verify data survives server "restarts" (simulated by
//! dropping and recreating the storage).

mod common;

use std::path::Path;
use std::sync::Arc;

use serde_json::json;
use stellardb::namespace::Namespace;
use stellardb::server::create_router;
use tempfile::TempDir;

/// Helper to run a query against a server (returns first result)
async fn query(
    addr: &std::net::SocketAddr,
    client: &reqwest::Client,
    sql: &str,
) -> serde_json::Value {
    query_nth(addr, client, sql, 0).await
}

/// Helper to run a query and return nth result
async fn query_nth(
    addr: &std::net::SocketAddr,
    client: &reqwest::Client,
    sql: &str,
    n: usize,
) -> serde_json::Value {
    let resp = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({ "query": sql }))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].is_null(), "SQL error: {:?}", body);
    body["results"][n]["data"].clone()
}

/// Run queries in a fresh server context, then sync and close.
/// Returns the path for reuse.
async fn run_with_server<F, Fut>(path: &Path, f: F)
where
    F: FnOnce(std::net::SocketAddr, reqwest::Client) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let namespace = Arc::new(Namespace::open(path).unwrap());

    // Ensure test database exists
    if !namespace.database_exists(common::TEST_DB) {
        namespace.create_database(common::TEST_DB).unwrap();
    }

    let db = namespace.get_database(common::TEST_DB).unwrap();

    let (app, _state) = create_router(namespace, None, None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = common::test_client();
    f(addr, client).await;

    // Sync before dropping
    db.sync().unwrap();

    // Abort server (it doesn't matter, storage is what we care about)
    server_handle.abort();

    // Drop db to release lock
    drop(db);

    // Small delay to ensure file lock is released
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
}

#[tokio::test]
async fn test_hnsw_index_persists_after_restart() {
    let tmp = TempDir::new().unwrap();

    // Phase 1: Create data and HNSW index
    run_with_server(tmp.path(), |addr, client| async move {
        // Create collection with HNSW index
        query(&addr, &client, "DEFINE COLLECTION products").await;
        query(
            &addr,
            &client,
            "CREATE INDEX ON products(embedding) HNSW DIMENSION 3 DIST COSINE",
        )
        .await;

        // Insert documents with vectors
        query(
            &addr,
            &client,
            r#"INSERT INTO products {id: "laptop", name: "Laptop", embedding: [1.0, 0.0, 0.0]}"#,
        )
        .await;
        query(
            &addr,
            &client,
            r#"INSERT INTO products {id: "phone", name: "Phone", embedding: [0.0, 1.0, 0.0]}"#,
        )
        .await;
        query(
            &addr,
            &client,
            r#"INSERT INTO products {id: "tablet", name: "Tablet", embedding: [0.0, 0.0, 1.0]}"#,
        )
        .await;

        // Verify search works
        let data = query(
            &addr,
            &client,
            "SELECT name FROM products WHERE embedding <|3|> [1.0, 0.1, 0.0]",
        )
        .await;
        assert_eq!(data.as_array().unwrap().len(), 3, "Should find 3 products");
    })
    .await;

    // Phase 2: "Restart" and verify data persists
    run_with_server(tmp.path(), |addr, client| async move {
        // Vector search should work and return correct doc IDs
        let data = query(
            &addr,
            &client,
            "SELECT name FROM products WHERE embedding <|3|> [1.0, 0.1, 0.0]",
        )
        .await;
        let arr = data.as_array().unwrap();

        assert_eq!(arr.len(), 3, "Should still find 3 products after restart");

        // Check that the first result is laptop (closest to query vector)
        assert_eq!(
            arr[0]["name"], "Laptop",
            "Laptop should be the closest match"
        );
    })
    .await;
}

#[tokio::test]
async fn test_fts_index_persists_after_restart() {
    let tmp = TempDir::new().unwrap();

    // Phase 1: Create data and FTS index
    run_with_server(tmp.path(), |addr, client| async move {
        query(&addr, &client, "DEFINE COLLECTION articles").await;
        query(
            &addr,
            &client,
            "CREATE INDEX ON articles(title, body) FULLTEXT",
        )
        .await;

        query(
            &addr,
            &client,
            r#"INSERT INTO articles {id: "1", title: "Rust Programming", body: "Learn about systems programming"}"#,
        )
        .await;
        query(
            &addr,
            &client,
            r#"INSERT INTO articles {id: "2", title: "Database Design", body: "How to build efficient databases"}"#,
        )
        .await;

        // Wait for FTS background commit (100ms interval + buffer)
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

        // Verify FTS works
        let data = query(
            &addr,
            &client,
            r#"SELECT title FROM articles WHERE title @@ "rust""#,
        )
        .await;
        assert_eq!(data.as_array().unwrap().len(), 1);
    })
    .await;

    // Phase 2: "Restart" and verify FTS still works
    run_with_server(tmp.path(), |addr, client| async move {
        let data = query(
            &addr,
            &client,
            r#"SELECT title FROM articles WHERE title @@ "rust""#,
        )
        .await;
        let arr = data.as_array().unwrap();

        assert_eq!(arr.len(), 1, "Should still find article after restart");
        assert_eq!(arr[0]["title"], "Rust Programming");

        // Also test searching in body
        let data = query(
            &addr,
            &client,
            r#"SELECT title FROM articles WHERE body @@ "databases""#,
        )
        .await;
        let arr = data.as_array().unwrap();
        assert_eq!(arr.len(), 1, "Should find article by body after restart");
        assert_eq!(arr[0]["title"], "Database Design");
    })
    .await;
}

#[tokio::test]
async fn test_edges_persist_after_restart() {
    let tmp = TempDir::new().unwrap();

    // Phase 1: Create nodes and edges
    run_with_server(tmp.path(), |addr, client| async move {
        query(&addr, &client, "DEFINE COLLECTION user").await;

        query(&addr, &client, r#"CREATE user:alice SET name = "Alice""#).await;
        query(&addr, &client, r#"CREATE user:bob SET name = "Bob""#).await;
        query(&addr, &client, r#"CREATE user:carol SET name = "Carol""#).await;

        // Create edges
        query(
            &addr,
            &client,
            "RELATE user:alice->follows->user:bob SET since = 2023",
        )
        .await;
        query(
            &addr,
            &client,
            "RELATE user:alice->follows->user:carol SET since = 2024",
        )
        .await;
        query(&addr, &client, "RELATE user:bob->likes->user:carol").await;

        // Verify edges work
        let data = query(
            &addr,
            &client,
            "SELECT ->follows AS following FROM user:alice",
        )
        .await;
        let arr = data.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        let following = arr[0]["following"].as_array().unwrap();
        assert_eq!(following.len(), 2, "Alice should follow 2 users");
    })
    .await;

    // Phase 2: "Restart" and verify edges persist
    run_with_server(tmp.path(), |addr, client| async move {
        // Test outgoing edges
        let data = query(
            &addr,
            &client,
            "SELECT ->follows AS following FROM user:alice",
        )
        .await;
        let arr = data.as_array().unwrap();
        assert_eq!(arr.len(), 1, "Should find user:alice after restart");
        let following = arr[0]["following"].as_array().unwrap();
        assert_eq!(
            following.len(),
            2,
            "Alice should still follow 2 users after restart"
        );

        // Test incoming edges
        let data = query(
            &addr,
            &client,
            "SELECT <-follows AS followers FROM user:bob",
        )
        .await;
        let arr = data.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        let followers = arr[0]["followers"].as_array().unwrap();
        assert_eq!(
            followers.len(),
            1,
            "Bob should have 1 follower after restart"
        );

        // Test different edge type
        let data = query(&addr, &client, "SELECT ->likes AS likes FROM user:bob").await;
        let arr = data.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        let likes = arr[0]["likes"].as_array().unwrap();
        assert_eq!(likes.len(), 1, "Bob should still like Carol after restart");

        // Test multi-hop traversal: ->follows returns edges with from/to
        // Alice follows [Bob, Carol], so ->follows returns 2 edges with all fields
        let data = query(&addr, &client, "SELECT ->follows AS edges FROM user:alice").await;
        let arr = data.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        let edges = arr[0]["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 2, "Alice should have 2 follows edges");
        // Verify edges have from/to fields (EdgeAll default)
        assert!(
            edges[0].get("from").is_some(),
            "Edge should have from field"
        );
        assert!(edges[0].get("to").is_some(), "Edge should have to field");
    })
    .await;
}

#[tokio::test]
async fn test_hybrid_search_persists_after_restart() {
    let tmp = TempDir::new().unwrap();

    // Phase 1: Create data with both HNSW and FTS indexes
    run_with_server(tmp.path(), |addr, client| async move {
        query(&addr, &client, "DEFINE COLLECTION items").await;
        query(
            &addr,
            &client,
            "CREATE INDEX ON items(vec) HNSW DIMENSION 3 DIST COSINE",
        )
        .await;
        query(&addr, &client, "CREATE INDEX ON items(name) FULLTEXT").await;

        query(
            &addr,
            &client,
            r#"INSERT INTO items {id: "a", name: "alpha item", vec: [1.0, 0.0, 0.0]}"#,
        )
        .await;
        query(
            &addr,
            &client,
            r#"INSERT INTO items {id: "b", name: "beta product", vec: [0.0, 1.0, 0.0]}"#,
        )
        .await;
        query(
            &addr,
            &client,
            r#"INSERT INTO items {id: "c", name: "gamma item", vec: [0.0, 0.0, 1.0]}"#,
        )
        .await;
    })
    .await;

    // Phase 2: "Restart" and verify hybrid search works
    run_with_server(tmp.path(), |addr, client| async move {
        // Test vector search after restart
        let data = query(
            &addr,
            &client,
            "SELECT name FROM items WHERE vec <|3|> [1.0, 0.0, 0.0]",
        )
        .await;
        let arr = data.as_array().unwrap();
        assert_eq!(
            arr.len(),
            3,
            "Vector search should return results after restart"
        );
        assert_eq!(arr[0]["name"], "alpha item", "Closest should be alpha");

        // Test FTS search after restart
        let data = query(
            &addr,
            &client,
            r#"SELECT name FROM items WHERE name @@ "item""#,
        )
        .await;
        let arr = data.as_array().unwrap();
        assert_eq!(arr.len(), 2, "FTS search should find 2 items after restart");

        // Test RRF hybrid search (3rd statement, index 2)
        let data = query_nth(
            &addr,
            &client,
            r#"
            LET vec_search = (SELECT id, name FROM items WHERE vec <|3|> [1.0, 0.0, 0.0]);
            LET fts_search = (SELECT id, name FROM items WHERE name @@ "item");
            SELECT * FROM (search::rrf([$vec_search, $fts_search], 10))
            "#,
            2, // Third statement (SELECT)
        )
        .await;

        let arr = data.as_array().unwrap();
        assert!(
            !arr.is_empty(),
            "Hybrid search should return results after restart"
        );
    })
    .await;
}
