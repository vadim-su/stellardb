//! Tests for DROP INDEX behavior.
//!
//! Verifies:
//! - Queries work after dropping index
//! - Indexes can be recreated
//! - Unique constraints are removed on drop

use super::{run, run_err, setup};

#[test]
fn test_query_works_after_drop_index() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(email)");

    run(
        &s,
        "INSERT INTO users {id: 'alice', email: 'alice@test.com'}",
    );
    run(&s, "INSERT INTO users {id: 'bob', email: 'bob@test.com'}");

    run(&s, "DROP INDEX idx_users_btree_email ON users");

    // Query should still work via full scan
    let result = run(&s, "SELECT * FROM users WHERE email = 'alice@test.com'");
    assert_eq!(result.as_array().unwrap().len(), 1);
}

#[test]
fn test_recreate_dropped_index() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(email) UNIQUE");

    run(
        &s,
        "INSERT INTO users {id: 'alice', email: 'alice@test.com'}",
    );

    run(&s, "DROP INDEX idx_users_btree_email ON users");

    // After drop, can insert duplicate
    run(&s, "INSERT INTO users {id: 'bob', email: 'alice@test.com'}");

    // Cannot recreate unique index now - duplicates exist
    let err = run_err(&s, "CREATE INDEX ON users(email) UNIQUE");
    assert!(
        err.contains("Conflict") || err.contains("Duplicate"),
        "Expected duplicate error, got: {}",
        err
    );
}

#[test]
fn test_recreate_dropped_index_non_unique() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(status)");

    run(&s, "INSERT INTO users {id: 'alice', status: 'active'}");
    run(&s, "INSERT INTO users {id: 'bob', status: 'active'}");

    run(&s, "DROP INDEX idx_users_btree_status ON users");

    // Can recreate non-unique index
    let result = run(&s, "CREATE INDEX ON users(status)");
    let arr = result.as_array().unwrap();
    // After refactoring, DDL returns {status: "created"} instead of {ok: true}
    assert!(arr[0]["status"].as_str().unwrap() == "created");

    // Index should work
    let explain = run(&s, "EXPLAIN SELECT * FROM users WHERE status = 'active'");
    let plan = explain.as_array().unwrap()[0].as_str().unwrap();
    assert!(
        plan.contains("IndexScan"),
        "Expected IndexScan after recreate: {}",
        plan
    );
}

#[test]
fn test_drop_index_then_insert_duplicate() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(email) UNIQUE");

    run(
        &s,
        "INSERT INTO users {id: 'alice', email: 'alice@test.com'}",
    );

    // Drop index
    run(&s, "DROP INDEX idx_users_btree_email ON users");

    // Now can insert duplicate
    run(&s, "INSERT INTO users {id: 'bob', email: 'alice@test.com'}");

    let result = run(&s, "SELECT * FROM users WHERE email = 'alice@test.com'");
    assert_eq!(result.as_array().unwrap().len(), 2);
}
