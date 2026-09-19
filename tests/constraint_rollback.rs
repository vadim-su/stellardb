//! Tests for unique constraint rollback on transaction failure.
//!
//! Verifies that when an OCC transaction fails (conflict), any unique constraint
//! claims made pre-commit are correctly rolled back, leaving the constraint state
//! consistent.

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

fn try_run(storage: &Arc<Database>, sql: &str) -> Result<serde_json::Value, String> {
    use stellardb::query::execute::operators::operator::rows_to_json;
    stellardb::try_run_sql!(storage.clone(), sql)
        .map(|result| rows_to_json(&result.rows))
        .map_err(|e| e.to_string())
}

// =============================================================================
// Constraint Rollback After Failed create_document
// =============================================================================

/// If create_document finds the doc already exists (within the tx),
/// the constraint claim should be rolled back, allowing a subsequent insert
/// with that unique value to succeed.
#[test]
fn test_create_existing_doc_rolls_back_constraint() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION items");
    run(&storage, "CREATE INDEX ON items(code) UNIQUE");

    // Insert initial document
    run(&storage, "INSERT INTO items {id: 'doc1', code: 'ALPHA'}");

    // Try to create the same doc again (should fail silently - returns false)
    // This exercises the early-return path in create_document
    let result = storage.create_document(&stellardb::Document {
        id: "items:doc1".to_string(),
        fields: [(
            "code".to_string(),
            stellardb::Value::String("BETA".to_string()),
        )]
        .into_iter()
        .collect(),
    });
    // create_document returns Ok(false) when doc exists
    assert!(!result.unwrap());

    // Now a different doc with code=BETA should succeed
    // (constraint was rolled back, not stuck claiming BETA)
    let result = try_run(&storage, "INSERT INTO items {id: 'doc2', code: 'BETA'}");
    assert!(
        result.is_ok(),
        "Insert should succeed after failed create rolled back constraint, got: {:?}",
        result
    );
}

// =============================================================================
// Constraint State After Delete + Reinsert Sequence
// =============================================================================

/// After deleting a document, the unique value should be available for reuse.
/// This tests the constraint release path in delete_document.
#[test]
fn test_delete_releases_constraint_for_reuse() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION items");
    run(&storage, "CREATE INDEX ON items(code) UNIQUE");
    run(
        &storage,
        "INSERT INTO items {id: 'holder', code: 'PRECIOUS'}",
    );

    // Delete the holder
    run(&storage, "DELETE items:holder");

    // Insert with same unique value should succeed
    let result = try_run(
        &storage,
        "INSERT INTO items {id: 'new_holder', code: 'PRECIOUS'}",
    );
    assert!(
        result.is_ok(),
        "Reuse of freed unique value should succeed, got: {:?}",
        result
    );
}

// =============================================================================
// Constraint Consistency Under Repeated Operations
// =============================================================================

/// Repeatedly insert/delete/reinsert with the same unique value.
/// The constraint should always be consistent.
#[test]
fn test_repeated_insert_delete_constraint_consistency() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION items");
    run(&storage, "CREATE INDEX ON items(code) UNIQUE");

    for i in 0..20 {
        // Insert
        run(
            &storage,
            &format!("INSERT INTO items {{id: 'cycle{}', code: 'CYCLE_VAL'}}", i),
        );

        // Verify only one exists
        let result = run(&storage, "SELECT id FROM items WHERE code = 'CYCLE_VAL'");
        assert_eq!(
            result.as_array().unwrap().len(),
            1,
            "Iteration {}: expected exactly 1 doc with CYCLE_VAL",
            i
        );

        // Delete it
        run(&storage, &format!("DELETE items:cycle{}", i));

        // Verify constraint is released
        let result = run(&storage, "SELECT id FROM items WHERE code = 'CYCLE_VAL'");
        assert_eq!(
            result.as_array().unwrap().len(),
            0,
            "Iteration {}: expected 0 docs after delete",
            i
        );
    }
}

/// Update a document's unique field, then verify the old value is freed
/// and the new value is claimed.
#[test]
fn test_update_unique_field_frees_old_claims_new() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION items");
    run(&storage, "CREATE INDEX ON items(code) UNIQUE");
    run(&storage, "INSERT INTO items {id: 'doc1', code: 'OLD_VAL'}");

    // Update to new value
    run(&storage, "UPDATE items:doc1 SET code = 'NEW_VAL'");

    // Old value should be free
    let result = try_run(&storage, "INSERT INTO items {id: 'doc2', code: 'OLD_VAL'}");
    assert!(
        result.is_ok(),
        "Old unique value should be available after update, got: {:?}",
        result
    );

    // New value should be claimed
    let err = try_run(&storage, "INSERT INTO items {id: 'doc3', code: 'NEW_VAL'}");
    assert!(
        err.is_err(),
        "New unique value should be claimed after update"
    );
}

/// Multiple unique indexes: constraint rollback should handle all of them.
#[test]
fn test_multiple_unique_indexes_constraint_consistency() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION users");
    run(&storage, "CREATE INDEX ON users(email) UNIQUE");
    run(&storage, "CREATE INDEX ON users(username) UNIQUE");

    run(
        &storage,
        "INSERT INTO users {id: 'u1', email: 'a@test.com', username: 'alice'}",
    );

    // Try to insert with duplicate email but unique username
    let err = try_run(
        &storage,
        "INSERT INTO users {id: 'u2', email: 'a@test.com', username: 'bob'}",
    );
    assert!(err.is_err(), "Duplicate email should fail");

    // The username 'bob' should NOT be stuck in the constraint
    // (it should have been rolled back when email constraint failed)
    let result = try_run(
        &storage,
        "INSERT INTO users {id: 'u3', email: 'b@test.com', username: 'bob'}",
    );
    assert!(
        result.is_ok(),
        "Username should be available after failed insert rolled back, got: {:?}",
        result
    );
}
