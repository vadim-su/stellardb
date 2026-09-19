//! Tests for atomic_write_batch ensuring it correctly updates BTree indexes,
//! enforces unique constraints, and creates existence markers.

use std::sync::Arc;
use stellardb::Database;
use stellardb::Document;
use stellardb::storage::WriteOp;
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

fn make_doc(id: &str, fields: Vec<(&str, stellardb::Value)>) -> Document {
    Document {
        id: id.to_string(),
        fields: fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    }
}

// =============================================================================
// BTree Index Tests
// =============================================================================

/// atomic_write_batch inserts should be findable via BTree index scan.
#[test]
fn test_batch_insert_updates_btree_index() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION items");
    run_query(&storage, "CREATE INDEX ON items(category)");

    // Insert via atomic_write_batch
    let doc = make_doc(
        "items:a1",
        vec![("category", stellardb::Value::String("electronics".into()))],
    );
    storage
        .atomic_write_batch(vec![WriteOp::InsertDoc(doc)])
        .unwrap();

    // Query via BTree index should find the document
    let result = run_query(
        &storage,
        r#"SELECT id FROM items WHERE category = "electronics""#,
    );
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1, "BTree index should find batch-inserted doc");
    assert_eq!(arr[0]["id"], "items:a1");
}

/// atomic_write_batch deletes should remove entries from BTree index.
#[test]
fn test_batch_delete_updates_btree_index() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION items");
    run_query(&storage, "CREATE INDEX ON items(category)");
    run_query(
        &storage,
        r#"INSERT INTO items {id: "x1", category: "books"}"#,
    );

    // Verify it's in the index
    let result = run_query(&storage, r#"SELECT id FROM items WHERE category = "books""#);
    assert_eq!(result.as_array().unwrap().len(), 1);

    // Delete via batch
    storage
        .atomic_write_batch(vec![WriteOp::DeleteDoc {
            collection: "items".into(),
            key: "x1".into(),
        }])
        .unwrap();

    // BTree index should no longer find it
    let result = run_query(&storage, r#"SELECT id FROM items WHERE category = "books""#);
    assert_eq!(
        result.as_array().unwrap().len(),
        0,
        "BTree index should not find batch-deleted doc"
    );
}

/// atomic_write_batch with update (overwrite) should update BTree index entries.
#[test]
fn test_batch_update_moves_btree_index_entry() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION items");
    run_query(&storage, "CREATE INDEX ON items(category)");
    run_query(
        &storage,
        r#"INSERT INTO items {id: "u1", category: "old_cat"}"#,
    );

    // Update via batch (InsertDoc overwrites)
    let updated = make_doc(
        "items:u1",
        vec![("category", stellardb::Value::String("new_cat".into()))],
    );
    storage
        .atomic_write_batch(vec![WriteOp::InsertDoc(updated)])
        .unwrap();

    // Old value should not be found
    let result = run_query(
        &storage,
        r#"SELECT id FROM items WHERE category = "old_cat""#,
    );
    assert_eq!(
        result.as_array().unwrap().len(),
        0,
        "Old BTree entry should be removed"
    );

    // New value should be found
    let result = run_query(
        &storage,
        r#"SELECT id FROM items WHERE category = "new_cat""#,
    );
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "New BTree entry should exist"
    );
}

// =============================================================================
// Unique Constraint Tests
// =============================================================================

/// atomic_write_batch should enforce unique constraints on inserts.
#[test]
fn test_batch_insert_enforces_unique_constraint() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION items");
    run_query(&storage, "CREATE INDEX ON items(code) UNIQUE");
    run_query(
        &storage,
        r#"INSERT INTO items {id: "existing", code: "ABC"}"#,
    );

    // Batch insert with duplicate unique value should fail
    let doc = make_doc(
        "items:new_item",
        vec![("code", stellardb::Value::String("ABC".into()))],
    );
    let result = storage.atomic_write_batch(vec![WriteOp::InsertDoc(doc)]);
    assert!(
        result.is_err(),
        "Batch insert with duplicate unique value should fail"
    );

    // Original document should still exist unchanged
    let result = run_query(&storage, r#"SELECT id FROM items WHERE code = "ABC""#);
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "items:existing");
}

/// atomic_write_batch delete should release unique constraint slot.
#[test]
fn test_batch_delete_releases_unique_constraint() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION items");
    run_query(&storage, "CREATE INDEX ON items(code) UNIQUE");
    run_query(
        &storage,
        r#"INSERT INTO items {id: "holder", code: "TAKEN"}"#,
    );

    // Delete the holder via batch
    storage
        .atomic_write_batch(vec![WriteOp::DeleteDoc {
            collection: "items".into(),
            key: "holder".into(),
        }])
        .unwrap();

    // Now inserting with the same unique value should succeed
    run_query(
        &storage,
        r#"INSERT INTO items {id: "new_holder", code: "TAKEN"}"#,
    );

    let result = run_query(&storage, r#"SELECT id FROM items WHERE code = "TAKEN""#);
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "items:new_holder");
}

/// If a batch fails after releasing a unique lock but before commit, the lock
/// must be restored because the primary document was not deleted.
#[test]
fn test_batch_failure_rolls_back_unique_release_for_delete() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION users");
    run_query(&storage, "CREATE INDEX ON users(email) UNIQUE");
    run_query(
        &storage,
        r#"INSERT INTO users {id: "alice", email: "alice@test.com"}"#,
    );
    run_query(&storage, "DEFINE COLLECTION vectors");
    run_query(
        &storage,
        "CREATE INDEX ON vectors(embedding) HNSW DIMENSION 3 DIST COSINE",
    );

    let invalid_vector_doc = make_doc(
        "vectors:bad",
        vec![(
            "embedding",
            stellardb::Value::Array(vec![
                stellardb::Value::Float(1.0),
                stellardb::Value::Float(0.0),
                stellardb::Value::Float(0.0),
                stellardb::Value::Float(0.0),
            ]),
        )],
    );

    let result = storage.atomic_write_batch(vec![
        WriteOp::DeleteDoc {
            collection: "users".into(),
            key: "alice".into(),
        },
        WriteOp::InsertDoc(invalid_vector_doc),
    ]);
    assert!(
        result.is_err(),
        "invalid vector dimension should fail batch before commit"
    );

    let existing = run_query(&storage, "SELECT id FROM users WHERE id = users:alice");
    assert_eq!(
        existing.as_array().unwrap().len(),
        1,
        "failed batch must not delete the primary document"
    );

    let err = run_query_err(
        &storage,
        r#"INSERT INTO users {id: "carol", email: "alice@test.com"}"#,
    );
    assert!(
        err.to_lowercase().contains("unique") || err.to_lowercase().contains("conflict"),
        "unique lock should still belong to alice after failed batch, got: {err}"
    );
}

/// A preparation failure after an earlier INSERT must roll back that INSERT's
/// claimed unique lock.
#[test]
fn test_batch_preparation_failure_rolls_back_prior_unique_claim() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION items");
    run_query(&storage, "CREATE INDEX ON items(code) UNIQUE");

    let claimed = make_doc(
        "items:claimed_before_prepare_failure",
        vec![("code", stellardb::Value::String("LEAKED".into()))],
    );
    let invalid = make_doc(
        "../bad:oops",
        vec![("code", stellardb::Value::String("OTHER".into()))],
    );

    let result = storage.atomic_write_batch(vec![
        WriteOp::InsertDoc(claimed),
        WriteOp::InsertDoc(invalid),
    ]);
    assert!(
        result.is_err(),
        "invalid collection name should fail batch preparation"
    );

    run_query(
        &storage,
        r#"INSERT INTO items {id: "after_failure", code: "LEAKED"}"#,
    );

    let result = run_query(&storage, r#"SELECT id FROM items WHERE code = "LEAKED""#);
    let rows = result.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "items:after_failure");
}

/// A failed INSERT must not leave a claimed unique lock behind.
#[test]
fn test_failed_insert_rolls_back_unique_claim() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION items");
    run_query(&storage, "CREATE INDEX ON items(email) UNIQUE");
    run_query(
        &storage,
        "CREATE INDEX ON items(embedding) HNSW DIMENSION 3 DIST COSINE",
    );

    let err = run_query_err(
        &storage,
        r#"INSERT INTO items {id: "bad", email: "claim@test.com", embedding: [1.0, 0.0, 0.0, 0.0]}"#,
    );
    assert!(
        err.to_lowercase().contains("dimension") || err.to_lowercase().contains("vector"),
        "wrong vector dimension should fail insert, got: {err}"
    );

    run_query(
        &storage,
        r#"INSERT INTO items {id: "good", email: "claim@test.com", embedding: [1.0, 0.0, 0.0]}"#,
    );

    let result = run_query(
        &storage,
        r#"SELECT id FROM items WHERE email = "claim@test.com""#,
    );
    let rows = result.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "items:good");
}

/// A failed set_document must not leave a claimed unique lock behind.
#[test]
fn test_failed_set_document_rolls_back_unique_claim() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION items");
    run_query(&storage, "CREATE INDEX ON items(email) UNIQUE");
    run_query(
        &storage,
        "CREATE INDEX ON items(embedding) HNSW DIMENSION 3 DIST COSINE",
    );

    let bad = make_doc(
        "items:bad_set",
        vec![
            ("email", stellardb::Value::String("set@test.com".into())),
            (
                "embedding",
                stellardb::Value::Array(vec![
                    stellardb::Value::Float(1.0),
                    stellardb::Value::Float(0.0),
                    stellardb::Value::Float(0.0),
                    stellardb::Value::Float(0.0),
                ]),
            ),
        ],
    );
    let err = storage.set_document(&bad).unwrap_err().to_string();
    assert!(
        err.to_lowercase().contains("dimension") || err.to_lowercase().contains("vector"),
        "wrong vector dimension should fail set_document, got: {err}"
    );

    run_query(
        &storage,
        r#"INSERT INTO items {id: "good_set", email: "set@test.com", embedding: [1.0, 0.0, 0.0]}"#,
    );

    let result = run_query(
        &storage,
        r#"SELECT id FROM items WHERE email = "set@test.com""#,
    );
    let rows = result.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "items:good_set");
}

/// A failed UPDATE must restore the old unique claim and release the new one.
#[test]
fn test_failed_update_rolls_back_unique_change() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION items");
    run_query(&storage, "CREATE INDEX ON items(email) UNIQUE");
    run_query(
        &storage,
        "CREATE INDEX ON items(embedding) HNSW DIMENSION 3 DIST COSINE",
    );
    run_query(
        &storage,
        r#"INSERT INTO items {id: "holder", email: "old@test.com", embedding: [1.0, 0.0, 0.0]}"#,
    );

    let err = run_query_err(
        &storage,
        r#"UPDATE items:holder SET email = "new@test.com", embedding = [1.0, 0.0, 0.0, 0.0]"#,
    );
    assert!(
        err.to_lowercase().contains("dimension") || err.to_lowercase().contains("vector"),
        "wrong vector dimension should fail update, got: {err}"
    );

    let old_claim_err = run_query_err(
        &storage,
        r#"INSERT INTO items {id: "old_dupe", email: "old@test.com", embedding: [0.0, 1.0, 0.0]}"#,
    );
    assert!(
        old_claim_err.to_lowercase().contains("unique")
            || old_claim_err.to_lowercase().contains("conflict"),
        "failed update must restore the old unique claim, got: {old_claim_err}"
    );

    run_query(
        &storage,
        r#"INSERT INTO items {id: "new_holder", email: "new@test.com", embedding: [0.0, 1.0, 0.0]}"#,
    );
}

// =============================================================================
// Existence Marker Tests
// =============================================================================

/// atomic_write_batch should create existence markers for new documents.
/// Verified by: RELATE requiring existence markers to validate endpoints.
#[test]
fn test_batch_insert_creates_existence_marker() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION users");

    // Insert via batch
    let doc = make_doc(
        "users:alice",
        vec![("name", stellardb::Value::String("Alice".into()))],
    );
    let doc2 = make_doc(
        "users:bob",
        vec![("name", stellardb::Value::String("Bob".into()))],
    );
    storage
        .atomic_write_batch(vec![WriteOp::InsertDoc(doc), WriteOp::InsertDoc(doc2)])
        .unwrap();

    // RELATE uses existence markers to validate endpoints.
    // If markers are missing, RELATE would fail with "does not exist".
    let result = run_query(&storage, "RELATE users:alice -> follows -> users:bob");
    let arr = result.as_array().unwrap();
    assert_eq!(
        arr.len(),
        1,
        "RELATE should succeed when existence markers are present"
    );
}

/// atomic_write_batch delete should remove existence markers.
#[test]
fn test_batch_delete_removes_existence_marker() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION users");
    run_query(
        &storage,
        r#"INSERT INTO users {id: "target", name: "Target"}"#,
    );
    run_query(
        &storage,
        r#"INSERT INTO users {id: "source", name: "Source"}"#,
    );

    // Delete target via batch
    storage
        .atomic_write_batch(vec![WriteOp::DeleteDoc {
            collection: "users".into(),
            key: "target".into(),
        }])
        .unwrap();

    // RELATE to deleted doc should fail (no existence marker)
    let err = run_query_err(&storage, "RELATE users:source -> follows -> users:target");
    assert!(
        err.contains("does not exist") || err.contains("not exist") || err.contains("NotFound"),
        "RELATE to batch-deleted doc should fail, got: {}",
        err
    );
}

#[test]
fn test_batch_delete_edge_removes_edge_in_same_batch() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION users");
    run_query(&storage, "INSERT INTO users {id: 'alice'}");
    run_query(&storage, "INSERT INTO users {id: 'bob'}");
    run_query(&storage, "RELATE users:alice -> follows -> users:bob");

    storage
        .atomic_write_batch(vec![WriteOp::DeleteEdge {
            from: "users:alice".into(),
            label: "follows".into(),
            to: "users:bob".into(),
        }])
        .unwrap();

    let result = run_query(
        &storage,
        "SELECT ->follows->users.* AS following FROM users:alice",
    );
    let rows = result.as_array().unwrap();
    let following = rows[0]["following"].as_array().unwrap();
    assert_eq!(
        following.len(),
        0,
        "Batch edge delete should remove the edge"
    );
}

// =============================================================================
// Mixed Batch Operations
// =============================================================================

/// A batch with both inserts and deletes should atomically update all indexes.
#[test]
fn test_batch_mixed_inserts_and_deletes() {
    let (_tmp, storage) = setup();

    run_query(&storage, "DEFINE COLLECTION items");
    run_query(&storage, "CREATE INDEX ON items(status)");

    // Insert initial docs
    run_query(
        &storage,
        r#"INSERT INTO items {id: "d1", status: "active"}"#,
    );
    run_query(
        &storage,
        r#"INSERT INTO items {id: "d2", status: "active"}"#,
    );

    // Batch: delete d1, insert d3
    let new_doc = make_doc(
        "items:d3",
        vec![("status", stellardb::Value::String("active".into()))],
    );
    storage
        .atomic_write_batch(vec![
            WriteOp::DeleteDoc {
                collection: "items".into(),
                key: "d1".into(),
            },
            WriteOp::InsertDoc(new_doc),
        ])
        .unwrap();

    // Should find d2 and d3 but not d1
    let result = run_query(&storage, r#"SELECT id FROM items WHERE status = "active""#);
    let arr = result.as_array().unwrap();
    let ids: Vec<&str> = arr.iter().filter_map(|v| v["id"].as_str()).collect();
    assert_eq!(ids.len(), 2);
    assert!(!ids.contains(&"items:d1"), "Deleted doc should not appear");
    assert!(ids.contains(&"items:d2"), "Existing doc should remain");
    assert!(ids.contains(&"items:d3"), "New doc should appear");
}
