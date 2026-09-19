//! Integration tests for secondary indexes

use std::sync::Arc;
use stellardb::Database;
use stellardb::schema::{HnswParams, IndexDef, IndexType};
use tempfile::TempDir;

fn setup() -> (TempDir, Arc<Database>) {
    let tmp = TempDir::new().unwrap();
    let storage = Arc::new(Database::open(tmp.path()).unwrap());
    (tmp, storage)
}

fn run_query(storage: &Arc<Database>, sql: &str) -> serde_json::Value {
    stellardb::query_json(storage.clone(), sql)
}

fn run_query_err(storage: &Arc<Database>, sql: &str) -> String {
    stellardb::try_run_sql!(storage.clone(), sql)
        .unwrap_err()
        .to_string()
}

#[test]
fn test_create_and_show_index() {
    let (_tmp, storage) = setup();

    // Define collection first
    run_query(&storage, "DEFINE COLLECTION users");

    // Create collection with data
    run_query(
        &storage,
        "INSERT INTO users {id: 'alice', email: 'alice@test.com'}",
    );

    // Create index
    let result = run_query(&storage, "CREATE INDEX ON users(email) UNIQUE");
    let arr = result.as_array().unwrap();
    assert_eq!(arr[0]["status"], "created");

    // Check indexes via DESCRIBE COLLECTION
    let result = run_query(&storage, "DESCRIBE COLLECTION users");
    let schema = &result.as_array().unwrap()[0];
    let arr = schema["indexes"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["fields"], serde_json::json!(["email"]));
    assert!(arr[0]["unique"].as_bool().unwrap());
}

#[test]
fn test_unique_constraint_violation() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION users");
    run_query(
        &storage,
        "INSERT INTO users {id: 'alice', email: 'alice@test.com'}",
    );
    run_query(&storage, "CREATE INDEX ON users(email) UNIQUE");

    // Try to insert duplicate
    let err = run_query_err(
        &storage,
        "INSERT INTO users {id: 'bob', email: 'alice@test.com'}",
    );
    assert!(
        err.contains("Conflict") || err.contains("Unique") || err.contains("constraint"),
        "Expected unique constraint error, got: {}",
        err
    );
}

#[test]
fn test_drop_index() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION users");
    run_query(
        &storage,
        "INSERT INTO users {id: 'alice', email: 'alice@test.com'}",
    );
    run_query(&storage, "CREATE INDEX ON users(email)");

    let result = run_query(&storage, "DROP INDEX idx_users_btree_email ON users");
    let arr = result.as_array().unwrap();
    assert_eq!(arr[0]["status"], "dropped");

    // Verify index is gone via DESCRIBE COLLECTION
    let result = run_query(&storage, "DESCRIBE COLLECTION users");
    let schema = &result.as_array().unwrap()[0];
    assert!(schema["indexes"].as_array().unwrap().is_empty());
}

#[test]
fn test_compound_index() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION users");
    run_query(
        &storage,
        "INSERT INTO users {id: 'alice', status: 'active', score: 100}",
    );
    run_query(&storage, "CREATE INDEX ON users(status, score)");

    // Check index via DESCRIBE COLLECTION
    let result = run_query(&storage, "DESCRIBE COLLECTION users");
    let schema = &result.as_array().unwrap()[0];
    let arr = schema["indexes"].as_array().unwrap();
    assert_eq!(arr[0]["fields"], serde_json::json!(["status", "score"]));
}

#[test]
fn test_missing_fields_not_indexed() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION users");
    run_query(&storage, "INSERT INTO users {id: 'alice'}"); // no email field
    run_query(&storage, "INSERT INTO users {id: 'bob'}"); // no email field
    run_query(&storage, "CREATE INDEX ON users(email) UNIQUE");

    // These should succeed - missing fields are not indexed
    run_query(&storage, "INSERT INTO users {id: 'carol'}");
    run_query(&storage, "INSERT INTO users {id: 'dave'}");

    // Verify all documents exist
    let result = run_query(&storage, "SELECT * FROM users");
    assert_eq!(result.as_array().unwrap().len(), 4);
}

#[test]
fn test_null_values_are_indexed() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION users");
    run_query(&storage, "INSERT INTO users {id: 'alice', email: null}");
    run_query(&storage, "CREATE INDEX ON users(email) UNIQUE");

    // This should fail - null values ARE indexed and unique constraint applies
    let err = run_query_err(&storage, "INSERT INTO users {id: 'bob', email: null}");
    assert!(
        err.contains("Conflict") || err.contains("Unique") || err.contains("constraint"),
        "Expected unique constraint error, got: {}",
        err
    );
}

#[test]
fn test_index_maintained_on_update() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION users");
    run_query(
        &storage,
        "INSERT INTO users {id: 'alice', email: 'alice@test.com'}",
    );
    run_query(&storage, "CREATE INDEX ON users(email) UNIQUE");

    // Update email
    run_query(
        &storage,
        "UPDATE users:alice SET email = 'newalice@test.com'",
    );

    // Now we can insert the old email
    run_query(
        &storage,
        "INSERT INTO users {id: 'bob', email: 'alice@test.com'}",
    );

    // Verify both documents exist with correct emails
    let result = run_query(&storage, "SELECT * FROM users");
    assert_eq!(result.as_array().unwrap().len(), 2);
}

#[test]
fn test_index_maintained_on_delete() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION users");
    run_query(
        &storage,
        "INSERT INTO users {id: 'alice', email: 'alice@test.com'}",
    );
    run_query(&storage, "CREATE INDEX ON users(email) UNIQUE");

    // Delete document
    run_query(&storage, "DELETE users:alice");

    // Now we can insert with same email
    run_query(
        &storage,
        "INSERT INTO users {id: 'bob', email: 'alice@test.com'}",
    );

    let result = run_query(&storage, "SELECT * FROM users");
    assert_eq!(result.as_array().unwrap().len(), 1);
}

#[test]
fn test_index_already_exists_error() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION users");
    run_query(
        &storage,
        "INSERT INTO users {id: 'alice', email: 'alice@test.com'}",
    );
    run_query(&storage, "CREATE INDEX ON users(email)");

    let err = run_query_err(&storage, "CREATE INDEX ON users(email)");
    assert!(
        err.contains("already exists") || err.contains("Already"),
        "Expected already exists error, got: {}",
        err
    );
}

#[test]
fn test_index_not_found_error() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION users");
    run_query(&storage, "INSERT INTO users {id: 'alice'}");

    let err = run_query_err(&storage, "DROP INDEX idx_users_nonexistent ON users");
    assert!(
        err.contains("not found") || err.contains("Not found"),
        "Expected not found error, got: {}",
        err
    );
}

// =============================================================================
// IndexDef and IndexType Tests
// =============================================================================

#[test]
fn test_index_def_constructors() {
    // Test new() constructor - creates non-unique BTree index with auto-generated name
    let idx = IndexDef::new("test_collection", vec!["field1".to_string()]);
    assert_eq!(idx.name, "idx_test_collection_btree_field1");
    assert_eq!(idx.fields, vec!["field1"]);
    assert!(!idx.unique);
    assert_eq!(idx.index_type, IndexType::BTree);

    // Test unique() constructor - creates unique BTree index
    let idx = IndexDef::unique("users", vec!["email".to_string()]);
    assert_eq!(idx.name, "idx_users_btree_email");
    assert_eq!(idx.fields, vec!["email"]);
    assert!(idx.unique);
    assert_eq!(idx.index_type, IndexType::BTree);

    // Test fulltext() constructor - creates non-unique FullText index
    // FTS uses {sorted_fields}_fts format (sorted alphabetically)
    let idx = IndexDef::fulltext("posts", vec!["title".to_string(), "body".to_string()]);
    assert_eq!(idx.name, "body_title_fts");
    assert_eq!(idx.fields, vec!["title", "body"]);
    assert!(!idx.unique);
    assert_eq!(idx.index_type, IndexType::FullText);

    // Test hnsw() constructor - creates non-unique Hnsw index with params
    let params = HnswParams::new(128);
    let idx = IndexDef::hnsw("products", vec!["embedding".to_string()], params);
    assert_eq!(idx.name, "idx_products_hnsw_embedding");
    assert_eq!(idx.fields, vec!["embedding"]);
    assert!(!idx.unique);
    assert_eq!(idx.index_type, IndexType::Hnsw);
    assert!(idx.hnsw_params.is_some());
    let hnsw_params = idx.hnsw_params.unwrap();
    assert_eq!(hnsw_params.dimension, 128);
    assert_eq!(hnsw_params.m, 16);
    assert_eq!(hnsw_params.ef_construction, 200);
}

#[test]
fn test_index_type_default() {
    // IndexType::BTree should be the default
    let default_type: IndexType = Default::default();
    assert_eq!(default_type, IndexType::BTree);
}

#[test]
fn test_create_fulltext_index() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION posts");
    run_query(
        &storage,
        "INSERT INTO posts {id: 'p1', title: 'Hello World', body: 'This is a test'}",
    );

    // Create a full-text search index
    let result = run_query(&storage, "CREATE INDEX ON posts(title, body) FULLTEXT");
    let arr = result.as_array().unwrap();
    assert_eq!(arr[0]["status"], "created");

    // Check index via DESCRIBE COLLECTION
    let result = run_query(&storage, "DESCRIBE COLLECTION posts");
    let schema = &result.as_array().unwrap()[0];
    let arr = schema["indexes"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["fields"], serde_json::json!(["title", "body"]));
    assert!(!arr[0]["unique"].as_bool().unwrap());
    assert_eq!(arr[0]["index_type"], "FullText");
}

#[test]
fn test_describe_shows_index_type() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION users");
    run_query(
        &storage,
        "INSERT INTO users {id: 'alice', email: 'alice@test.com'}",
    );
    run_query(&storage, "CREATE INDEX ON users(email) UNIQUE");

    let result = run_query(&storage, "DESCRIBE COLLECTION users");
    let schema = &result.as_array().unwrap()[0];
    let arr = schema["indexes"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["index_type"], "BTree");
    assert!(arr[0]["unique"].as_bool().unwrap());
}

/// Test that BTree index entries are visible immediately after INSERT without sync().
///
/// This verifies the transactional BTree index update fix - index entries are now
/// written in the same transaction as the document, ensuring immediate visibility.
#[test]
fn test_btree_index_visible_immediately_after_insert() {
    let (_tmp, storage) = setup();
    run_query(&storage, "DEFINE COLLECTION users");
    run_query(&storage, "CREATE INDEX ON users(status)");

    // Insert and immediately read - should work WITHOUT sync()
    run_query(&storage, "INSERT INTO users {id: 'u1', status: 'active'}");

    let result = run_query(&storage, "SELECT * FROM users WHERE status = 'active'");
    assert_eq!(result.as_array().unwrap().len(), 1);

    // Insert more documents and verify they're all immediately visible
    run_query(&storage, "INSERT INTO users {id: 'u2', status: 'active'}");
    run_query(&storage, "INSERT INTO users {id: 'u3', status: 'inactive'}");

    let result = run_query(&storage, "SELECT * FROM users WHERE status = 'active'");
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "Both active users should be visible immediately after insert"
    );

    let result = run_query(&storage, "SELECT * FROM users WHERE status = 'inactive'");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "Inactive user should be visible immediately after insert"
    );
}
