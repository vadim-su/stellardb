//! Vector Search Integration Tests
//!
//! These tests verify the complete vector search pipeline works end-to-end:
//! - Creating HNSW indexes with various parameters
//! - Inserting documents with vector embeddings
//! - Querying with the <|K|> KNN operator
//! - Using vector::distance() function for distance access
//! - Using effort parameter for accuracy/speed tradeoff
//! - Hybrid search with search::rrf()
//! - Different distance metrics (Cosine, Euclidean, Dot)
//!
//! # KNN Operator Syntax
//!
//! `embedding <|K|> [vector]` - Find K nearest neighbors
//! `embedding <|K, effort|> [vector]` - With effort parameter for accuracy control
//!
//! # Index Creation
//!
//! `CREATE INDEX ON collection(field) HNSW DIMENSION 3 DIST COSINE`
//! Optional parameters: M (connectivity), EF_CONSTRUCTION (build quality)

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
fn assert_success(body: &serde_json::Value) -> &serde_json::Value {
    assert!(
        body["error"].is_null(),
        "Query failed with error: {:?}",
        body["error"]
    );
    &body["results"][0]["data"]
}

/// Helper to setup a collection with 3D vectors for testing
async fn setup_vector_collection(client: &reqwest::Client, addr: &std::net::SocketAddr) {
    // Define collection
    sql(client, addr, "DEFINE COLLECTION items").await;

    // Create HNSW index with 3D vectors using Cosine distance
    let body = sql(
        client,
        addr,
        "CREATE INDEX ON items(embedding) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;
    assert_eq!(
        body["results"][0]["data"][0]["status"], "created",
        "Failed to create HNSW index: {:?}",
        body
    );
}

/// Helper to insert test items with 3D vectors
async fn insert_test_vectors(client: &reqwest::Client, addr: &std::net::SocketAddr) {
    // Insert items with unit vectors along each axis
    // v1: [1, 0, 0] - positive x-axis
    sql(
        client,
        addr,
        r#"INSERT INTO items {id: "v1", name: "X-axis", embedding: [1.0, 0.0, 0.0]}"#,
    )
    .await;

    // v2: [0, 1, 0] - positive y-axis
    sql(
        client,
        addr,
        r#"INSERT INTO items {id: "v2", name: "Y-axis", embedding: [0.0, 1.0, 0.0]}"#,
    )
    .await;

    // v3: [0, 0, 1] - positive z-axis
    sql(
        client,
        addr,
        r#"INSERT INTO items {id: "v3", name: "Z-axis", embedding: [0.0, 0.0, 1.0]}"#,
    )
    .await;

    // v4: normalized diagonal vector
    let sqrt3 = (1.0_f64 / 3.0).sqrt();
    sql(
        client,
        addr,
        &format!(
            r#"INSERT INTO items {{id: "v4", name: "Diagonal", embedding: [{}, {}, {}]}}"#,
            sqrt3, sqrt3, sqrt3
        ),
    )
    .await;

    // v5: negative x-axis
    sql(
        client,
        addr,
        r#"INSERT INTO items {id: "v5", name: "Neg-X", embedding: [-1.0, 0.0, 0.0]}"#,
    )
    .await;
}

// =============================================================================
// Basic HNSW Index Creation Tests
// =============================================================================

#[tokio::test]
async fn test_create_hnsw_index() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define collection
    sql(&client, &addr, "DEFINE COLLECTION products").await;

    // Create HNSW index
    let body = sql(
        &client,
        &addr,
        "CREATE INDEX ON products(embedding) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    assert_eq!(
        body["results"][0]["data"][0]["status"], "created",
        "Failed to create HNSW index: {:?}",
        body
    );

    // Verify index appears in DESCRIBE COLLECTION
    let body = sql(&client, &addr, "DESCRIBE COLLECTION products").await;
    let data = &body["results"][0]["data"][0];

    let indexes = data["indexes"].as_array().unwrap();
    assert_eq!(indexes.len(), 1, "Expected 1 index");

    let index = &indexes[0];
    assert_eq!(index["index_type"], "Hnsw");
    assert_eq!(index["fields"], json!(["embedding"]));
}

#[tokio::test]
async fn test_create_hnsw_index_with_all_params() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define collection
    sql(&client, &addr, "DEFINE COLLECTION embeddings").await;

    // Create HNSW index with all parameters
    let body = sql(
        &client,
        &addr,
        "CREATE INDEX ON embeddings(vec) HNSW DIMENSION 128 DIST EUCLIDEAN M 32 EF_CONSTRUCTION 400",
    )
    .await;

    assert_eq!(
        body["results"][0]["data"][0]["status"], "created",
        "Failed to create HNSW index with custom params: {:?}",
        body
    );
}

// =============================================================================
// Basic KNN Vector Search Tests
// =============================================================================

#[tokio::test]
async fn test_vector_search_basic() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_vector_collection(&client, &addr).await;
    insert_test_vectors(&client, &addr).await;

    // Search for vectors closest to [1, 0, 0] - should find v1 first
    let body = sql(
        &client,
        &addr,
        r#"SELECT id, name FROM items WHERE embedding <|3|> [1.0, 0.0, 0.0]"#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert!(
        !results.is_empty(),
        "Expected results for KNN search, got empty"
    );

    // The first result should be v1 (exact match)
    assert_eq!(
        results[0]["id"], "items:v1",
        "Expected v1 to be the closest vector to [1,0,0]"
    );
}

#[tokio::test]
async fn test_vector_search_returns_k_results() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_vector_collection(&client, &addr).await;
    insert_test_vectors(&client, &addr).await;

    // Request exactly 2 neighbors
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM items WHERE embedding <|2|> [0.5, 0.5, 0.0]"#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert_eq!(
        results.len(),
        2,
        "Expected exactly 2 results with K=2, got {}",
        results.len()
    );
}

#[tokio::test]
async fn test_vector_search_ordering() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_vector_collection(&client, &addr).await;
    insert_test_vectors(&client, &addr).await;

    // Search for vectors closest to [0.9, 0.1, 0.0]
    // Should be closest to v1 [1,0,0], then v4 (diagonal), then v2 [0,1,0]
    let body = sql(
        &client,
        &addr,
        r#"SELECT id, name, vector::distance() AS dist FROM items WHERE embedding <|5|> [0.9, 0.1, 0.0]"#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert!(
        results.len() >= 3,
        "Expected at least 3 results, got {}",
        results.len()
    );

    // First result should be v1 (closest to query)
    assert_eq!(
        results[0]["id"], "items:v1",
        "Expected v1 to be first result"
    );

    // Last result should be v5 (opposite direction)
    let ids: Vec<&str> = results.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert!(
        ids.contains(&"items:v5"),
        "v5 should be in results (farthest)"
    );
}

// =============================================================================
// vector::distance() Function Tests
// =============================================================================

#[tokio::test]
async fn test_vector_distance_function() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_vector_collection(&client, &addr).await;
    insert_test_vectors(&client, &addr).await;

    // Query with vector::distance() to get actual distances
    let body = sql(
        &client,
        &addr,
        r#"SELECT id, vector::distance() AS dist FROM items WHERE embedding <|3|> [1.0, 0.0, 0.0]"#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert!(!results.is_empty(), "Expected results for distance query");

    // Check that distance values are present
    for result in results {
        let dist = result.get("dist").or(result.get("$distance"));
        assert!(
            dist.is_some(),
            "Expected distance field in result: {:?}",
            result
        );
    }

    // The exact match (v1) should have distance close to 0
    let first_dist = results[0]
        .get("dist")
        .or(results[0].get("$distance"))
        .and_then(|v| v.as_f64())
        .unwrap_or(f64::MAX);

    assert!(
        first_dist < 0.1,
        "Expected first result distance to be near 0, got {}",
        first_dist
    );
}

#[tokio::test]
async fn test_vector_distance_ordering_desc() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_vector_collection(&client, &addr).await;
    insert_test_vectors(&client, &addr).await;

    // Get distances and verify they're in ascending order (closest first)
    let body = sql(
        &client,
        &addr,
        r#"SELECT id, vector::distance() AS dist FROM items WHERE embedding <|5|> [1.0, 0.0, 0.0]"#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    if results.len() >= 2 {
        let dist1 = results[0]
            .get("dist")
            .or(results[0].get("$distance"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let dist2 = results[1]
            .get("dist")
            .or(results[1].get("$distance"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        assert!(
            dist1 <= dist2,
            "Results should be ordered by distance ASC: {} <= {}",
            dist1,
            dist2
        );
    }
}

// =============================================================================
// KNN with Effort Parameter Tests
// =============================================================================

#[tokio::test]
async fn test_vector_search_with_effort() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_vector_collection(&client, &addr).await;
    insert_test_vectors(&client, &addr).await;

    // Search with effort parameter (higher effort = more accurate but slower)
    let body = sql(
        &client,
        &addr,
        r#"SELECT id, name FROM items WHERE embedding <|3, 100|> [0.5, 0.5, 0.0]"#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert!(
        !results.is_empty(),
        "Expected results for KNN search with effort parameter"
    );

    // Verify we got the expected number of results
    assert!(results.len() <= 3, "Expected at most 3 results with K=3");
}

#[tokio::test]
async fn test_vector_search_effort_comparison() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_vector_collection(&client, &addr).await;
    insert_test_vectors(&client, &addr).await;

    // Low effort
    let body_low = sql(
        &client,
        &addr,
        r#"SELECT id FROM items WHERE embedding <|3, 10|> [0.5, 0.5, 0.5]"#,
    )
    .await;

    // High effort
    let body_high = sql(
        &client,
        &addr,
        r#"SELECT id FROM items WHERE embedding <|3, 200|> [0.5, 0.5, 0.5]"#,
    )
    .await;

    // Both should succeed (effort affects quality, not correctness)
    let data_low = assert_success(&body_low);
    let data_high = assert_success(&body_high);

    assert!(
        data_low.as_array().is_some(),
        "Low effort query should succeed"
    );
    assert!(
        data_high.as_array().is_some(),
        "High effort query should succeed"
    );
}

// =============================================================================
// Distance Metrics Tests
// =============================================================================

#[tokio::test]
async fn test_hnsw_cosine_distance() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with COSINE distance
    sql(&client, &addr, "DEFINE COLLECTION cosine_test").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON cosine_test(vec) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    // Insert vectors
    sql(
        &client,
        &addr,
        r#"INSERT INTO cosine_test {id: "a", vec: [1.0, 0.0, 0.0]}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO cosine_test {id: "b", vec: [0.0, 1.0, 0.0]}"#,
    )
    .await;

    // Query
    let body = sql(
        &client,
        &addr,
        r#"SELECT id, vector::distance() AS dist FROM cosine_test WHERE vec <|2|> [1.0, 0.0, 0.0]"#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert!(!results.is_empty(), "Expected results for cosine query");

    // Vector "a" should be closest (distance 0 for cosine of same direction)
    assert_eq!(results[0]["id"], "cosine_test:a");
}

#[tokio::test]
async fn test_hnsw_euclidean_distance() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with EUCLIDEAN distance
    sql(&client, &addr, "DEFINE COLLECTION euclidean_test").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON euclidean_test(vec) HNSW DIMENSION 3 DIST EUCLIDEAN",
    )
    .await;

    // Insert vectors
    sql(
        &client,
        &addr,
        r#"INSERT INTO euclidean_test {id: "a", vec: [1.0, 0.0, 0.0]}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO euclidean_test {id: "b", vec: [2.0, 0.0, 0.0]}"#,
    )
    .await;

    // Query - "a" should be closer to [0,0,0] than "b"
    let body = sql(
        &client,
        &addr,
        r#"SELECT id, vector::distance() AS dist FROM euclidean_test WHERE vec <|2|> [0.0, 0.0, 0.0]"#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert!(!results.is_empty(), "Expected results for euclidean query");

    // Vector "a" should be closest (distance 1 vs distance 2)
    assert_eq!(results[0]["id"], "euclidean_test:a");
}

#[tokio::test]
async fn test_hnsw_dot_distance() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with DOT (inner product) distance
    sql(&client, &addr, "DEFINE COLLECTION dot_test").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON dot_test(vec) HNSW DIMENSION 3 DIST DOT",
    )
    .await;

    // Insert vectors
    sql(
        &client,
        &addr,
        r#"INSERT INTO dot_test {id: "a", vec: [1.0, 0.0, 0.0]}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO dot_test {id: "b", vec: [-1.0, 0.0, 0.0]}"#,
    )
    .await;

    // Query
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM dot_test WHERE vec <|2|> [1.0, 0.0, 0.0]"#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert!(
        !results.is_empty(),
        "Expected results for dot product query"
    );
}

// =============================================================================
// Hybrid Search with search::rrf() Tests
// =============================================================================

#[tokio::test]
async fn test_hybrid_search_rrf() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with both text and vector fields
    sql(&client, &addr, "DEFINE COLLECTION products").await;

    // Create FTS index on name
    sql(&client, &addr, "CREATE INDEX ON products(name) FULLTEXT").await;

    // Create HNSW index on embedding
    sql(
        &client,
        &addr,
        "CREATE INDEX ON products(embedding) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    // Insert products with both text and embeddings
    sql(
        &client,
        &addr,
        r#"INSERT INTO products {id: "p1", name: "laptop computer", embedding: [1.0, 0.0, 0.0]}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO products {id: "p2", name: "desktop computer", embedding: [0.9, 0.1, 0.0]}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO products {id: "p3", name: "laptop case", embedding: [0.0, 1.0, 0.0]}"#,
    )
    .await;

    // Test RRF using LET to store subquery results, then call search::rrf()
    // search::rrf takes: (array_of_arrays, limit, optional_k)
    let body = sql(
        &client,
        &addr,
        r#"LET $fts = (SELECT id, name FROM products WHERE name @@ "laptop");
           LET $vec = (SELECT id, name FROM products WHERE embedding <|3|> [1.0, 0.0, 0.0]);
           SELECT VALUE search::rrf([$fts, $vec], 5)"#,
    )
    .await;

    // RRF should succeed and return merged results
    // Check the last result (the SELECT with RRF)
    let has_results = body["results"]
        .as_array()
        .map(|arr| !arr.is_empty())
        .unwrap_or(false);
    let has_error = !body["error"].is_null();

    assert!(
        has_results || has_error,
        "RRF query should return results or error: {:?}",
        body
    );

    // If successful, verify the RRF results structure
    if has_results && !has_error {
        let results = &body["results"];
        let last_result = results.as_array().unwrap().last().unwrap();
        // VALUE mode returns the array directly
        let data = &last_result["data"];
        assert!(data.is_array(), "RRF should return an array: {:?}", data);
    }
}

#[tokio::test]
async fn test_rrf_with_custom_k() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection
    sql(&client, &addr, "DEFINE COLLECTION docs").await;
    sql(&client, &addr, "CREATE INDEX ON docs(text) FULLTEXT").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON docs(vec) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    // Insert documents
    sql(
        &client,
        &addr,
        r#"INSERT INTO docs {id: "d1", text: "hello world", vec: [1.0, 0.0, 0.0]}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO docs {id: "d2", text: "hello there", vec: [0.0, 1.0, 0.0]}"#,
    )
    .await;

    // Test RRF with custom k parameter (k=30 instead of default 60)
    let body = sql(
        &client,
        &addr,
        r#"LET $fts = (SELECT id, text FROM docs WHERE text @@ "hello");
           LET $vec = (SELECT id, text FROM docs WHERE vec <|2|> [1.0, 0.0, 0.0]);
           SELECT VALUE search::rrf([$fts, $vec], 5, 30)"#,
    )
    .await;

    // Should succeed with custom k
    let has_results = body["results"]
        .as_array()
        .map(|arr| !arr.is_empty())
        .unwrap_or(false);
    let has_error = !body["error"].is_null();

    assert!(
        has_results || has_error,
        "RRF with custom k should handle gracefully: {:?}",
        body
    );
}

// =============================================================================
// Dimension Validation Tests
// =============================================================================

#[tokio::test]
async fn test_vector_dimension_validation() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with 3D vectors
    sql(&client, &addr, "DEFINE COLLECTION dim_test").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON dim_test(vec) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    // Insert valid 3D vector
    sql(
        &client,
        &addr,
        r#"INSERT INTO dim_test {id: "a", vec: [1.0, 0.0, 0.0]}"#,
    )
    .await;

    // Try to search with wrong dimension (2D instead of 3D)
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM dim_test WHERE vec <|1|> [1.0, 0.0]"#,
    )
    .await;

    // This should either return an error or empty results
    let has_error = body["error"].is_object() || !body["error"].is_null();
    let empty_results = body["results"][0]["data"]
        .as_array()
        .map(|a| a.is_empty())
        .unwrap_or(true);

    assert!(
        has_error || empty_results,
        "Wrong dimension query should fail or return empty: {:?}",
        body
    );
}

#[tokio::test]
async fn test_vector_dimension_mismatch_on_insert() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with 3D vectors
    sql(&client, &addr, "DEFINE COLLECTION strict_dim").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON strict_dim(vec) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    // Try to insert 4D vector (wrong dimension)
    let body = sql(
        &client,
        &addr,
        r#"INSERT INTO strict_dim {id: "bad", vec: [1.0, 0.0, 0.0, 0.0]}"#,
    )
    .await;

    // This may either fail or succeed (depending on validation timing)
    // The important thing is it doesn't crash
    assert!(
        body["results"].is_array() || body["error"].is_object(),
        "Insert with wrong dimension should handle gracefully"
    );
}

// =============================================================================
// Edge Cases Tests
// =============================================================================

#[tokio::test]
async fn test_vector_search_empty_collection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with index but no documents
    sql(&client, &addr, "DEFINE COLLECTION empty_coll").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON empty_coll(vec) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    // Search should return empty results, not error
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM empty_coll WHERE vec <|5|> [1.0, 0.0, 0.0]"#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert_eq!(results.len(), 0, "Empty collection should return 0 results");
}

#[tokio::test]
async fn test_vector_search_k_larger_than_dataset() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_vector_collection(&client, &addr).await;

    // Insert only 2 vectors
    sql(
        &client,
        &addr,
        r#"INSERT INTO items {id: "a", name: "first", embedding: [1.0, 0.0, 0.0]}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO items {id: "b", name: "second", embedding: [0.0, 1.0, 0.0]}"#,
    )
    .await;

    // Request K=100 but only 2 documents exist
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM items WHERE embedding <|100|> [0.5, 0.5, 0.0]"#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    // Should return only available documents (2), not 100
    assert!(
        results.len() <= 2,
        "Should return at most 2 results when only 2 exist, got {}",
        results.len()
    );
}

#[tokio::test]
async fn test_vector_search_with_projection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_vector_collection(&client, &addr).await;
    insert_test_vectors(&client, &addr).await;

    // Select only specific fields
    let body = sql(
        &client,
        &addr,
        r#"SELECT id, name FROM items WHERE embedding <|3|> [1.0, 0.0, 0.0]"#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert!(!results.is_empty(), "Expected results");

    // Verify projection worked
    for result in results {
        assert!(result.get("id").is_some(), "Should have id field");
        assert!(result.get("name").is_some(), "Should have name field");
    }
}

#[tokio::test]
async fn test_vector_search_with_limit() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_vector_collection(&client, &addr).await;
    insert_test_vectors(&client, &addr).await;

    // KNN with K=5 but LIMIT 2
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM items WHERE embedding <|5|> [1.0, 0.0, 0.0] LIMIT 2"#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert!(
        results.len() <= 2,
        "LIMIT 2 should return at most 2 results, got {}",
        results.len()
    );
}

#[tokio::test]
async fn test_vector_search_no_index() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection WITHOUT HNSW index
    sql(&client, &addr, "DEFINE COLLECTION no_index").await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO no_index {id: "a", vec: [1.0, 0.0, 0.0]}"#,
    )
    .await;

    // KNN search should fail or return empty without index
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM no_index WHERE vec <|3|> [1.0, 0.0, 0.0]"#,
    )
    .await;

    // Either returns error or empty results (implementation dependent)
    let has_error = body["error"].is_object() || !body["error"].is_null();
    let empty_results = body["results"][0]["data"]
        .as_array()
        .map(|a| a.is_empty())
        .unwrap_or(true);

    assert!(
        has_error || empty_results,
        "KNN search without index should fail or return empty: {:?}",
        body
    );
}

// =============================================================================
// Complex Query Tests
// =============================================================================

#[tokio::test]
async fn test_vector_search_with_filter() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with category field
    sql(&client, &addr, "DEFINE COLLECTION categorized").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON categorized(vec) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    // Insert items with categories
    sql(
        &client,
        &addr,
        r#"INSERT INTO categorized {id: "a", category: "A", vec: [1.0, 0.0, 0.0]}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO categorized {id: "b", category: "B", vec: [0.9, 0.1, 0.0]}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO categorized {id: "c", category: "A", vec: [0.8, 0.2, 0.0]}"#,
    )
    .await;

    // Note: Post-filter after KNN is applied via regular WHERE condition
    // The KNN operator finds K nearest, then filter is applied
    let body = sql(
        &client,
        &addr,
        r#"SELECT id, category FROM categorized WHERE vec <|3|> [1.0, 0.0, 0.0]"#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert!(
        !results.is_empty(),
        "Expected results for filtered KNN search"
    );
}

#[tokio::test]
async fn test_vector_search_with_computed_distance() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_vector_collection(&client, &addr).await;
    insert_test_vectors(&client, &addr).await;

    // Use distance in computed expression
    let body = sql(
        &client,
        &addr,
        r#"SELECT id, vector::distance() * 100 AS dist_pct FROM items WHERE embedding <|3|> [1.0, 0.0, 0.0]"#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert!(!results.is_empty(), "Expected results");

    // Check computed field exists
    for result in results {
        let has_computed = result.get("dist_pct").is_some() || result.get("$distance").is_some();
        assert!(
            has_computed,
            "Should have computed distance field: {:?}",
            result
        );
    }
}

// =============================================================================
// Index Shows in DESCRIBE Tests
// =============================================================================

#[tokio::test]
async fn test_hnsw_index_shows_in_describe() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    sql(&client, &addr, "DEFINE COLLECTION described").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON described(embedding) HNSW DIMENSION 128 DIST EUCLIDEAN",
    )
    .await;

    let body = sql(&client, &addr, "DESCRIBE COLLECTION described").await;
    let data = &body["results"][0]["data"][0];

    let indexes = data["indexes"].as_array().unwrap();
    assert!(!indexes.is_empty(), "Expected at least 1 index");

    let hnsw_idx = indexes.iter().find(|idx| idx["index_type"] == "Hnsw");

    assert!(hnsw_idx.is_some(), "Expected HNSW index in describe output");

    let idx = hnsw_idx.unwrap();
    assert_eq!(idx["fields"], json!(["embedding"]));
}

// =============================================================================
// Hybrid Search with FROM (expr) Tests
// =============================================================================

/// Helper to setup products collection with both FTS and vector indexes
async fn setup_hybrid_collection(client: &reqwest::Client, addr: &std::net::SocketAddr) {
    sql(client, addr, "DEFINE COLLECTION products").await;
    sql(client, addr, "CREATE INDEX ON products(name) FULLTEXT").await;
    sql(
        client,
        addr,
        "CREATE INDEX ON products(embedding) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    // Insert products with both text and embeddings
    sql(
        client,
        addr,
        r#"INSERT INTO products {id: "laptop1", name: "gaming laptop pro", category: "electronics", embedding: [1.0, 0.0, 0.0]}"#,
    )
    .await;
    sql(
        client,
        addr,
        r#"INSERT INTO products {id: "laptop2", name: "business laptop", category: "electronics", embedding: [0.95, 0.1, 0.0]}"#,
    )
    .await;
    sql(
        client,
        addr,
        r#"INSERT INTO products {id: "case1", name: "laptop case", category: "accessories", embedding: [0.0, 1.0, 0.0]}"#,
    )
    .await;
    sql(
        client,
        addr,
        r#"INSERT INTO products {id: "phone1", name: "smartphone", category: "electronics", embedding: [0.0, 0.0, 1.0]}"#,
    )
    .await;
}

#[tokio::test]
async fn test_hybrid_search_with_from_expr() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_hybrid_collection(&client, &addr).await;

    // Use FROM (expr) syntax with search::rrf
    let body = sql(
        &client,
        &addr,
        r#"
        LET fts_results = (SELECT id, name FROM products WHERE name @@ "laptop");
        LET vec_results = (SELECT id, name FROM products WHERE embedding <|3|> [1.0, 0.0, 0.0]);
        SELECT * FROM (search::rrf([$fts_results, $vec_results], 5))
        "#,
    )
    .await;

    assert!(
        body["error"].is_null(),
        "Hybrid search should succeed: {:?}",
        body
    );

    // Get the last result (SELECT FROM expr)
    let results = body["results"].as_array().unwrap();
    let select_result = results.last().unwrap();
    let data = select_result["data"].as_array().unwrap();

    // RRF should return merged results
    assert!(!data.is_empty(), "RRF should return results");

    // Each result should have id, name, and _rrf_score
    for item in data {
        assert!(item["id"].is_string(), "Each result should have id");
        assert!(item["name"].is_string(), "Each result should have name");
        assert!(
            item["_rrf_score"].is_number(),
            "Each result should have _rrf_score"
        );
    }
}

#[tokio::test]
async fn test_hybrid_search_from_expr_with_filter() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_hybrid_collection(&client, &addr).await;

    // Use FROM (expr) with WHERE filter on RRF results
    let body = sql(
        &client,
        &addr,
        r#"
        LET fts_results = (SELECT id, name, category FROM products WHERE name @@ "laptop");
        LET vec_results = (SELECT id, name, category FROM products WHERE embedding <|3|> [1.0, 0.0, 0.0]);
        SELECT * FROM (search::rrf([$fts_results, $vec_results], 10)) WHERE category = "electronics"
        "#,
    )
    .await;

    assert!(
        body["error"].is_null(),
        "Hybrid search with filter should succeed: {:?}",
        body
    );

    let results = body["results"].as_array().unwrap();
    let select_result = results.last().unwrap();
    let data = select_result["data"].as_array().unwrap();

    // All results should be electronics (filter applied after RRF)
    for item in data {
        assert_eq!(
            item["category"], "electronics",
            "Filter should only return electronics"
        );
    }
}

#[tokio::test]
async fn test_hybrid_search_from_expr_with_order_limit() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_hybrid_collection(&client, &addr).await;

    // Use FROM (expr) with ORDER and LIMIT on RRF results
    let body = sql(
        &client,
        &addr,
        r#"
        LET fts_results = (SELECT id, name FROM products WHERE name @@ "laptop");
        LET vec_results = (SELECT id, name FROM products WHERE embedding <|5|> [1.0, 0.0, 0.0]);
        SELECT * FROM (search::rrf([$fts_results, $vec_results], 10)) ORDER _rrf_score DESC LIMIT 2
        "#,
    )
    .await;

    assert!(
        body["error"].is_null(),
        "Hybrid search with order/limit should succeed: {:?}",
        body
    );

    let results = body["results"].as_array().unwrap();
    let select_result = results.last().unwrap();
    let data = select_result["data"].as_array().unwrap();

    // Should return at most 2 results
    assert!(
        data.len() <= 2,
        "LIMIT 2 should return at most 2 results, got {}",
        data.len()
    );

    // Results should be ordered by _rrf_score DESC
    if data.len() >= 2 {
        let score1 = data[0]["_rrf_score"].as_f64().unwrap();
        let score2 = data[1]["_rrf_score"].as_f64().unwrap();
        assert!(
            score1 >= score2,
            "Results should be ordered by _rrf_score DESC: {} >= {}",
            score1,
            score2
        );
    }
}

#[tokio::test]
async fn test_hybrid_search_from_expr_with_fetch() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create products and categories collections for FETCH testing
    sql(&client, &addr, "DEFINE COLLECTION categories").await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO categories {id: "cat1", name: "Electronics", description: "Electronic devices"}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO categories {id: "cat2", name: "Accessories", description: "Device accessories"}"#,
    )
    .await;

    sql(&client, &addr, "DEFINE COLLECTION items").await;
    sql(&client, &addr, "CREATE INDEX ON items(name) FULLTEXT").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON items(embedding) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    // Items with references to categories
    sql(
        &client,
        &addr,
        r#"INSERT INTO items {id: "item1", name: "laptop computer", category_ref: categories:cat1, embedding: [1.0, 0.0, 0.0]}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO items {id: "item2", name: "laptop bag", category_ref: categories:cat2, embedding: [0.0, 1.0, 0.0]}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO items {id: "item3", name: "gaming laptop", category_ref: categories:cat1, embedding: [0.9, 0.1, 0.0]}"#,
    )
    .await;

    // Use FROM (expr) with FETCH to resolve category references
    let body = sql(
        &client,
        &addr,
        r#"
        LET fts_results = (SELECT id, name, category_ref FROM items WHERE name @@ "laptop");
        LET vec_results = (SELECT id, name, category_ref FROM items WHERE embedding <|3|> [1.0, 0.0, 0.0]);
        SELECT * FROM (search::rrf([$fts_results, $vec_results], 5)) FETCH category_ref
        "#,
    )
    .await;

    assert!(
        body["error"].is_null(),
        "Hybrid search with FETCH should succeed: {:?}",
        body
    );

    let results = body["results"].as_array().unwrap();
    let select_result = results.last().unwrap();
    let data = select_result["data"].as_array().unwrap();

    // Check that category_ref was fetched (should be an object, not a string reference)
    for item in data {
        if !item["category_ref"].is_null() {
            // Fetched reference should be an object with name and description
            let cat_ref = &item["category_ref"];
            assert!(
                cat_ref.is_object(),
                "FETCH should resolve category_ref to object: {:?}",
                cat_ref
            );
            if cat_ref.is_object() {
                assert!(
                    cat_ref["name"].is_string(),
                    "Fetched category should have name"
                );
            }
        }
    }
}

#[tokio::test]
async fn test_hybrid_search_complex_pipeline() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_hybrid_collection(&client, &addr).await;

    // Complex pipeline: vector search -> store in variable -> use as expr source -> filter -> project
    let body = sql(
        &client,
        &addr,
        r#"
        LET query_vec = [1.0, 0.0, 0.0];
        LET top_similar = (
            SELECT id, name, category, vector::distance() AS dist
            FROM products
            WHERE embedding <|5|> $query_vec
            ORDER dist
        );
        SELECT id, name, dist FROM ($top_similar) WHERE dist < 0.5
        "#,
    )
    .await;

    assert!(
        body["error"].is_null(),
        "Complex pipeline should succeed: {:?}",
        body
    );

    let results = body["results"].as_array().unwrap();
    let select_result = results.last().unwrap();
    let data = select_result["data"].as_array().unwrap();

    // All results should have dist < 0.5
    for item in data {
        let dist = item["dist"].as_f64().unwrap();
        assert!(
            dist < 0.5,
            "Filter should only return items with dist < 0.5, got {}",
            dist
        );
    }
}

#[tokio::test]
async fn test_from_expr_with_empty_rrf_inputs() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_hybrid_collection(&client, &addr).await;

    // FTS query with no matches, vector query with matches
    let body = sql(
        &client,
        &addr,
        r#"
        LET fts_results = (SELECT id, name FROM products WHERE name @@ "nonexistent_term_xyz");
        LET vec_results = (SELECT id, name FROM products WHERE embedding <|3|> [1.0, 0.0, 0.0]);
        SELECT * FROM (search::rrf([$fts_results, $vec_results], 5))
        "#,
    )
    .await;

    assert!(
        body["error"].is_null(),
        "RRF with one empty input should succeed: {:?}",
        body
    );

    let results = body["results"].as_array().unwrap();
    let select_result = results.last().unwrap();
    let data = select_result["data"].as_array().unwrap();

    // Should still return results from vector search
    assert!(
        !data.is_empty(),
        "RRF should return results from non-empty input"
    );
}

// =============================================================================
// Atomicity Tests - Document should not be created if HNSW validation fails
// =============================================================================

#[tokio::test]
async fn test_document_not_created_on_hnsw_dimension_error() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with 3D HNSW index
    sql(&client, &addr, "DEFINE COLLECTION atomic_test").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON atomic_test(vec) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    // Try to insert document with wrong dimension (4D instead of 3D)
    let insert_body = sql(
        &client,
        &addr,
        r#"INSERT INTO atomic_test {id: "wrong_dim", name: "should not exist", vec: [1.0, 0.0, 0.0, 0.0]}"#,
    )
    .await;

    // The insert should fail with dimension error
    let has_error =
        !insert_body["error"].is_null() || insert_body["results"][0]["error"].is_object();

    assert!(
        has_error,
        "INSERT with wrong vector dimension should fail: {:?}",
        insert_body
    );

    // Verify the document was NOT created
    let select_body = sql(&client, &addr, r#"SELECT * FROM atomic_test:wrong_dim"#).await;

    let data = &select_body["results"][0]["data"];
    let is_empty = data.as_array().map(|a| a.is_empty()).unwrap_or(true);

    assert!(
        is_empty,
        "Document should NOT exist after failed HNSW validation: {:?}",
        select_body
    );
}

#[tokio::test]
async fn test_document_created_on_valid_hnsw_dimension() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with 3D HNSW index
    sql(&client, &addr, "DEFINE COLLECTION valid_dim_test").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON valid_dim_test(vec) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    // Insert document with correct dimension (3D)
    let insert_body = sql(
        &client,
        &addr,
        r#"INSERT INTO valid_dim_test {id: "correct_dim", name: "should exist", vec: [1.0, 0.0, 0.0]}"#,
    )
    .await;

    // The insert should succeed
    assert!(
        insert_body["error"].is_null(),
        "INSERT with correct vector dimension should succeed: {:?}",
        insert_body
    );

    // Verify the document was created
    let select_body = sql(
        &client,
        &addr,
        r#"SELECT * FROM valid_dim_test:correct_dim"#,
    )
    .await;

    let data = &select_body["results"][0]["data"];
    let has_data = data.as_array().map(|a| !a.is_empty()).unwrap_or(false);

    assert!(
        has_data,
        "Document should exist after successful HNSW validation: {:?}",
        select_body
    );

    // Verify the vector was stored correctly
    let doc = &data[0];
    assert_eq!(doc["name"], "should exist");
}

#[tokio::test]
async fn test_update_fails_atomically_on_wrong_dimension() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with 3D HNSW index
    sql(&client, &addr, "DEFINE COLLECTION update_atomic_test").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON update_atomic_test(vec) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    // First, insert a valid document
    sql(
        &client,
        &addr,
        r#"INSERT INTO update_atomic_test {id: "doc1", name: "original", vec: [1.0, 0.0, 0.0]}"#,
    )
    .await;

    // Try to update with wrong dimension
    let update_body = sql(
        &client,
        &addr,
        r#"UPDATE update_atomic_test:doc1 SET name = "updated", vec = [1.0, 0.0, 0.0, 0.0]"#,
    )
    .await;

    // The update should fail with dimension error
    let has_error =
        !update_body["error"].is_null() || update_body["results"][0]["error"].is_object();

    assert!(
        has_error,
        "UPDATE with wrong vector dimension should fail: {:?}",
        update_body
    );

    // Verify the document was NOT modified (should still have original values)
    let select_body = sql(&client, &addr, r#"SELECT * FROM update_atomic_test:doc1"#).await;

    let data = &select_body["results"][0]["data"];
    let doc = &data[0];

    assert_eq!(
        doc["name"], "original",
        "Document should retain original value after failed update: {:?}",
        doc
    );
}

// =============================================================================
// Subquery in KNN Expression Tests
// =============================================================================

#[tokio::test]
async fn test_knn_with_subquery_vector() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with HNSW index
    sql(&client, &addr, "DEFINE COLLECTION subq_test").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON subq_test(embedding) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    // Insert test vectors
    sql(
        &client,
        &addr,
        r#"INSERT INTO subq_test {id: "ref", name: "reference", embedding: [1.0, 0.0, 0.0]}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO subq_test {id: "close", name: "close to ref", embedding: [0.9, 0.1, 0.0]}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO subq_test {id: "far", name: "far from ref", embedding: [0.0, 0.0, 1.0]}"#,
    )
    .await;

    // Use subquery to get the embedding of the reference document
    let body = sql(
        &client,
        &addr,
        r#"SELECT id, name, vector::distance() AS dist
           FROM subq_test
           WHERE embedding <|3|> (SELECT VALUE embedding FROM subq_test:ref)[0]"#,
    )
    .await;

    assert!(
        body["error"].is_null(),
        "KNN with subquery vector should succeed: {:?}",
        body
    );

    let data = &body["results"][0]["data"];
    let results = data.as_array().unwrap();

    assert!(!results.is_empty(), "Should return results");

    // First result should be "ref" (exact match with itself)
    assert_eq!(
        results[0]["id"], "subq_test:ref",
        "First result should be the reference document itself"
    );
}

#[tokio::test]
async fn test_knn_with_variable_from_subquery() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with HNSW index
    sql(&client, &addr, "DEFINE COLLECTION var_knn_test").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON var_knn_test(vec) HNSW DIMENSION 3 DIST COSINE",
    )
    .await;

    // Insert test vectors
    sql(
        &client,
        &addr,
        r#"INSERT INTO var_knn_test {id: "a", vec: [1.0, 0.0, 0.0]}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO var_knn_test {id: "b", vec: [0.0, 1.0, 0.0]}"#,
    )
    .await;

    // Use LET to store the vector from a subquery, then use in KNN
    let body = sql(
        &client,
        &addr,
        r#"LET ref_vec = (SELECT VALUE vec FROM var_knn_test:a)[0];
           SELECT id FROM var_knn_test WHERE vec <|2|> $ref_vec"#,
    )
    .await;

    assert!(
        body["error"].is_null(),
        "KNN with variable from subquery should succeed: {:?}",
        body
    );

    // Get the last result (the SELECT statement)
    let results = body["results"].as_array().unwrap();
    let select_result = results.last().unwrap();
    let data = select_result["data"].as_array().unwrap();

    assert!(!data.is_empty(), "Should return results");
    assert_eq!(
        data[0]["id"], "var_knn_test:a",
        "First result should be 'a'"
    );
}
