//! Tests for collection and index interaction.
//!
//! Verifies:
//! - Same index on different collections
//! - DROP COLLECTION cleans up indexes

use super::{run, setup};

#[test]
fn test_same_index_different_collections() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "DEFINE COLLECTION admins");

    run(&s, "CREATE INDEX ON users(email) UNIQUE");
    run(&s, "CREATE INDEX ON admins(email) UNIQUE");

    // Same email in different collections is OK
    run(
        &s,
        "INSERT INTO users {id: 'alice', email: 'alice@test.com'}",
    );
    run(
        &s,
        "INSERT INTO admins {id: 'alice', email: 'alice@test.com'}",
    );

    let result = run(&s, "SELECT * FROM users");
    assert_eq!(result.as_array().unwrap().len(), 1);

    let result = run(&s, "SELECT * FROM admins");
    assert_eq!(result.as_array().unwrap().len(), 1);
}

#[test]
fn test_drop_collection_cleans_up_indexes() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(email) UNIQUE");
    run(
        &s,
        "INSERT INTO users {id: 'alice', email: 'alice@test.com'}",
    );

    // Must delete documents before dropping collection
    run(&s, "DELETE users:alice");

    // Drop collection
    run(&s, "DROP COLLECTION users");

    // Recreate collection - should be able to use same index name
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(email) UNIQUE");

    // Should be able to insert same email (old data gone)
    run(&s, "INSERT INTO users {id: 'bob', email: 'alice@test.com'}");

    let result = run(&s, "SELECT * FROM users");
    assert_eq!(result.as_array().unwrap().len(), 1);
}

#[test]
fn test_index_scan_with_many_results() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(status)");

    // Insert many documents with same status
    for i in 0..100 {
        run(
            &s,
            &format!("INSERT INTO data {{id: '{}', status: 'active'}}", i),
        );
    }

    // Query should return all
    let result = run(&s, "SELECT * FROM data WHERE status = 'active'");
    assert_eq!(result.as_array().unwrap().len(), 100);
}

#[test]
fn test_index_scan_near_threshold() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(category)");

    // Insert documents in two categories
    for i in 0..500 {
        run(
            &s,
            &format!("INSERT INTO data {{id: 'a{}', category: 'A'}}", i),
        );
    }
    for i in 0..500 {
        run(
            &s,
            &format!("INSERT INTO data {{id: 'b{}', category: 'B'}}", i),
        );
    }

    // Query each category
    let result = run(&s, "SELECT * FROM data WHERE category = 'A'");
    assert_eq!(result.as_array().unwrap().len(), 500);

    let result = run(&s, "SELECT * FROM data WHERE category = 'B'");
    assert_eq!(result.as_array().unwrap().len(), 500);
}
