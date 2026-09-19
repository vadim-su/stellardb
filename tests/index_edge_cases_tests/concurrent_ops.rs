//! Concurrent-like operations (sequential simulation).
//!
//! Note: True concurrency tests would require threads/async, these test
//! rapid sequential operations which can reveal some race conditions.

use super::{run, setup};

#[test]
fn test_insert_then_create_index_rapid() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");

    // Rapid insert-create-insert cycle
    run(
        &s,
        "INSERT INTO users {id: 'user1', email: 'user1@test.com'}",
    );
    run(&s, "CREATE INDEX ON users(email) UNIQUE");
    run(
        &s,
        "INSERT INTO users {id: 'user2', email: 'user2@test.com'}",
    );

    // Both should be indexed
    let result = run(&s, "SELECT * FROM users WHERE email = 'user1@test.com'");
    assert_eq!(result.as_array().unwrap().len(), 1);

    let result = run(&s, "SELECT * FROM users WHERE email = 'user2@test.com'");
    assert_eq!(result.as_array().unwrap().len(), 1);
}

#[test]
fn test_update_during_index_existence() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "INSERT INTO users {id: 'alice', email: 'old@test.com'}");
    run(&s, "CREATE INDEX ON users(email) UNIQUE");

    // Rapid updates
    run(&s, "UPDATE users:alice SET email = 'new1@test.com'");
    run(&s, "UPDATE users:alice SET email = 'new2@test.com'");
    run(&s, "UPDATE users:alice SET email = 'new3@test.com'");

    // Index should reflect final state
    let result = run(&s, "SELECT * FROM users WHERE email = 'new3@test.com'");
    assert_eq!(result.as_array().unwrap().len(), 1);

    // Old values should not be found
    let result = run(&s, "SELECT * FROM users WHERE email = 'old@test.com'");
    assert_eq!(result.as_array().unwrap().len(), 0);

    let result = run(&s, "SELECT * FROM users WHERE email = 'new1@test.com'");
    assert_eq!(result.as_array().unwrap().len(), 0);
}

#[test]
fn test_delete_during_index_existence() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(email) UNIQUE");

    // Insert, delete, reinsert cycle
    run(
        &s,
        "INSERT INTO users {id: 'alice', email: 'alice@test.com'}",
    );
    run(&s, "DELETE users:alice");
    run(&s, "INSERT INTO users {id: 'bob', email: 'alice@test.com'}");

    // Should find bob with alice's old email
    let result = run(&s, "SELECT * FROM users WHERE email = 'alice@test.com'");
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "users:bob");
}
