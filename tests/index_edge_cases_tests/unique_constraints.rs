//! Critical tests for unique constraint behavior.
//!
//! Tests that would catch data corruption or silent failures:
//! - Unique index creation on existing duplicates
//! - Unique constraint violation rollback
//! - Concurrent unique inserts

use super::{run, run_err, setup};
use stellardb::{Document, Value};

// =============================================================================
// CRITICAL: Unique index creation on existing duplicates
// =============================================================================

#[test]
fn test_unique_index_fails_on_existing_duplicates() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");

    // Insert documents with duplicate email values FIRST
    run(
        &s,
        "INSERT INTO users {id: 'alice', email: 'same@test.com'}",
    );
    run(&s, "INSERT INTO users {id: 'bob', email: 'same@test.com'}");

    // Now try to create unique index - should fail
    let err = run_err(&s, "CREATE INDEX ON users(email) UNIQUE");
    assert!(
        err.contains("Conflict") || err.contains("Duplicate") || err.contains("unique"),
        "Expected duplicate value error when creating unique index on non-unique data, got: {}",
        err
    );
}

#[test]
fn test_unique_index_fails_on_existing_null_duplicates() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");

    // Insert documents with multiple null values
    run(&s, "INSERT INTO users {id: 'alice', email: null}");
    run(&s, "INSERT INTO users {id: 'bob', email: null}");

    // Creating unique index should fail - null values are indexed
    let err = run_err(&s, "CREATE INDEX ON users(email) UNIQUE");
    assert!(
        err.contains("Conflict") || err.contains("Duplicate") || err.contains("unique"),
        "Expected duplicate null error when creating unique index, got: {}",
        err
    );
}

#[test]
fn test_compound_unique_index_fails_on_existing_duplicates() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");

    // Insert documents with duplicate (country, email) combinations
    run(
        &s,
        "INSERT INTO users {id: 'alice', country: 'RU', email: 'test@test.com'}",
    );
    run(
        &s,
        "INSERT INTO users {id: 'bob', country: 'RU', email: 'test@test.com'}",
    );

    // Creating compound unique index should fail
    let err = run_err(&s, "CREATE INDEX ON users(country, email) UNIQUE");
    assert!(
        err.contains("Conflict") || err.contains("Duplicate") || err.contains("unique"),
        "Expected duplicate error for compound unique index, got: {}",
        err
    );
}

// =============================================================================
// CRITICAL: Unique constraint violation rollback
// =============================================================================

/// Test that when a unique constraint violation occurs, the document is NOT created.
/// This ensures atomicity - either the insert succeeds completely or fails completely.
#[test]
fn test_unique_violation_does_not_create_document() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(
        &s,
        "INSERT INTO users {id: 'alice', email: 'alice@test.com'}",
    );
    run(&s, "CREATE INDEX ON users(email) UNIQUE");

    // Try to insert duplicate - should fail
    let _err = run_err(&s, "INSERT INTO users {id: 'bob', email: 'alice@test.com'}");

    // Verify 'bob' document was NOT created
    let result = run(&s, "SELECT * FROM users WHERE id = users:bob");
    let arr = result.as_array().unwrap();
    assert!(
        arr.is_empty(),
        "Document should not exist after unique violation, but found: {:?}",
        arr
    );

    // Verify only alice exists
    let result = run(&s, "SELECT * FROM users");
    assert_eq!(result.as_array().unwrap().len(), 1);
}

#[test]
fn test_index_not_polluted_after_failed_insert() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(
        &s,
        "INSERT INTO users {id: 'alice', email: 'alice@test.com'}",
    );
    run(&s, "CREATE INDEX ON users(email) UNIQUE");

    // Try to insert duplicate
    let _err = run_err(&s, "INSERT INTO users {id: 'bob', email: 'alice@test.com'}");

    // Now insert with a different email - should work
    run(
        &s,
        "INSERT INTO users {id: 'carol', email: 'carol@test.com'}",
    );

    // Verify carol was inserted
    let result = run(&s, "SELECT * FROM users WHERE email = 'carol@test.com'");
    assert_eq!(result.as_array().unwrap().len(), 1);
}

#[test]
fn test_update_document_enforces_unique_constraint() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(email) UNIQUE");
    run(
        &s,
        "INSERT INTO users {id: 'alice', email: 'alice@test.com'}",
    );
    run(&s, "INSERT INTO users {id: 'bob', email: 'bob@test.com'}");

    let mut fields = std::collections::HashMap::new();
    fields.insert(
        "email".to_string(),
        Value::String("alice@test.com".to_string()),
    );
    let updated_bob = Document {
        id: "users:bob".to_string(),
        fields,
    };

    let err = s.update_document(&updated_bob).unwrap_err().to_string();
    let err_lower = err.to_lowercase();
    assert!(
        err_lower.contains("unique")
            || err_lower.contains("conflict")
            || err_lower.contains("duplicate"),
        "Expected unique violation through update_document, got: {}",
        err
    );

    let result = run(&s, "SELECT id FROM users WHERE email = 'alice@test.com'");
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "users:alice");

    let result = run(&s, "SELECT id, email FROM users WHERE id = users:bob");
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["email"], "bob@test.com");
}

// =============================================================================
// CRITICAL: Concurrent unique inserts (simulated via sequential rapid inserts)
// =============================================================================

#[test]
fn test_rapid_unique_inserts_no_duplicates() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(email) UNIQUE");

    // Insert many documents rapidly - all should have unique emails
    for i in 0..100 {
        run(
            &s,
            &format!(
                "INSERT INTO users {{id: 'user{}', email: 'user{}@test.com'}}",
                i, i
            ),
        );
    }

    let result = run(&s, "SELECT * FROM users");
    assert_eq!(result.as_array().unwrap().len(), 100);
}

#[test]
fn test_unique_index_integrity_after_many_operations() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(email) UNIQUE");

    // Insert, update, delete cycle
    for i in 0..50 {
        run(
            &s,
            &format!(
                "INSERT INTO users {{id: 'user{}', email: 'email{}@test.com'}}",
                i, i
            ),
        );
    }

    // Update half of them
    for i in 0..25 {
        run(
            &s,
            &format!("UPDATE users:user{} SET email = 'updated{}@test.com'", i, i),
        );
    }

    // Delete some
    for i in 25..35 {
        run(&s, &format!("DELETE users:user{}", i));
    }

    // Now old deleted emails should be available
    for i in 25..35 {
        run(
            &s,
            &format!(
                "INSERT INTO users {{id: 'newuser{}', email: 'email{}@test.com'}}",
                i, i
            ),
        );
    }

    // Verify count: 50 - 10 deleted + 10 new = 50
    let result = run(&s, "SELECT * FROM users");
    assert_eq!(result.as_array().unwrap().len(), 50);
}
