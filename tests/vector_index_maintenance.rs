//! Vector (HNSW) Index Maintenance Tests
//!
//! Tests that verify HNSW indexes are correctly updated when documents
//! are modified or deleted.

use std::sync::Arc;
use stellardb::Database;
use tempfile::TempDir;

fn setup() -> (TempDir, Arc<Database>) {
    let tmp = TempDir::new().unwrap();
    let storage = Arc::new(Database::open(tmp.path()).unwrap());
    (tmp, storage)
}

fn run(storage: &Arc<Database>, sql: &str) -> serde_json::Value {
    stellardb::query_json(storage.clone(), sql)
}

/// When a vector field is updated, KNN search should find the document
/// at its new position, not the old one.
#[test]
fn test_hnsw_update_vector_field() {
    let (_tmp, storage) = setup();

    // Setup: collection with HNSW index
    run(&storage, "DEFINE COLLECTION items");
    run(
        &storage,
        "CREATE INDEX ON items(embedding) HNSW DIMENSION 3 DIST COSINE",
    );

    // Insert document with vector pointing along X-axis
    run(
        &storage,
        r#"INSERT INTO items {id: "v1", name: "test", embedding: [1.0, 0.0, 0.0]}"#,
    );

    // Verify: KNN near X-axis finds it
    let result = run(
        &storage,
        "SELECT id FROM items WHERE embedding <|1|> [1.0, 0.0, 0.0]",
    );
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "items:v1");

    // Update: move vector to Y-axis
    run(
        &storage,
        r#"UPDATE items:v1 SET embedding = [0.0, 1.0, 0.0]"#,
    );

    // Verify: KNN near Y-axis now finds it
    let result = run(
        &storage,
        "SELECT id FROM items WHERE embedding <|1|> [0.0, 1.0, 0.0]",
    );
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "items:v1");

    // Verify: KNN near X-axis does NOT find it as closest
    // Insert another vector at X-axis to prove the point
    run(
        &storage,
        r#"INSERT INTO items {id: "v2", name: "x-axis", embedding: [1.0, 0.0, 0.0]}"#,
    );

    let result = run(
        &storage,
        "SELECT id FROM items WHERE embedding <|1|> [1.0, 0.0, 0.0]",
    );
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(
        arr[0]["id"], "items:v2",
        "v1 should no longer be closest to X-axis"
    );
}

/// When a document is deleted, it should no longer appear in KNN results.
#[test]
fn test_hnsw_delete_removes_from_knn() {
    let (_tmp, storage) = setup();

    // Setup
    run(&storage, "DEFINE COLLECTION items");
    run(
        &storage,
        "CREATE INDEX ON items(embedding) HNSW DIMENSION 3 DIST COSINE",
    );

    // Insert 3 documents
    run(
        &storage,
        r#"INSERT INTO items {id: "v1", embedding: [1.0, 0.0, 0.0]}"#,
    );
    run(
        &storage,
        r#"INSERT INTO items {id: "v2", embedding: [0.0, 1.0, 0.0]}"#,
    );
    run(
        &storage,
        r#"INSERT INTO items {id: "v3", embedding: [0.0, 0.0, 1.0]}"#,
    );

    // Verify: KNN k=3 returns all 3
    let result = run(
        &storage,
        "SELECT id FROM items WHERE embedding <|3|> [0.5, 0.5, 0.5]",
    );
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 3, "Should find all 3 documents");

    // Delete one document
    run(&storage, "DELETE items:v2");

    // Verify: KNN k=3 returns only 2
    let result = run(
        &storage,
        "SELECT id FROM items WHERE embedding <|3|> [0.5, 0.5, 0.5]",
    );
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 2, "Should find only 2 documents after delete");

    // Verify: deleted document ID is not in results
    let ids: Vec<&str> = arr.iter().map(|v| v["id"].as_str().unwrap()).collect();
    assert!(
        !ids.contains(&"items:v2"),
        "Deleted document should not be in KNN results"
    );
    assert!(ids.contains(&"items:v1"));
    assert!(ids.contains(&"items:v3"));
}

/// Edge case: delete the only document, KNN should return empty.
#[test]
fn test_hnsw_delete_single_document_knn() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION items");
    run(
        &storage,
        "CREATE INDEX ON items(embedding) HNSW DIMENSION 3 DIST COSINE",
    );

    run(
        &storage,
        r#"INSERT INTO items {id: "v1", embedding: [1.0, 0.0, 0.0]}"#,
    );

    // Verify exists
    let result = run(
        &storage,
        "SELECT id FROM items WHERE embedding <|1|> [1.0, 0.0, 0.0]",
    );
    assert_eq!(result.as_array().unwrap().len(), 1);

    // Delete
    run(&storage, "DELETE items:v1");

    // KNN should return empty, not error
    let result = run(
        &storage,
        "SELECT id FROM items WHERE embedding <|1|> [1.0, 0.0, 0.0]",
    );
    let arr = result.as_array().unwrap();
    assert_eq!(
        arr.len(),
        0,
        "KNN should return empty after deleting only document"
    );
}
