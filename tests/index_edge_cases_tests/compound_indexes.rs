//! Important tests for compound index behavior.
//!
//! Tests correctness issues with compound indexes:
//! - Partial null handling
//! - Field order in compound indexes
//! - Prefix queries
//! - Unique compound indexes

use super::{run, run_err, setup};

#[test]
fn test_compound_index_partial_null_first_field() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(country, city)");

    // First field null, second non-null
    run(
        &s,
        "INSERT INTO users {id: 'alice', country: null, city: 'Moscow'}",
    );
    run(
        &s,
        "INSERT INTO users {id: 'bob', country: 'RU', city: 'Moscow'}",
    );

    // Query by second field only should work (full scan or partial index use)
    let result = run(&s, "SELECT * FROM users WHERE city = 'Moscow'");
    assert_eq!(result.as_array().unwrap().len(), 2);
}

#[test]
fn test_compound_index_partial_null_second_field() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(country, city)");

    // First field non-null, second null
    run(
        &s,
        "INSERT INTO users {id: 'alice', country: 'RU', city: null}",
    );
    run(
        &s,
        "INSERT INTO users {id: 'bob', country: 'RU', city: 'Moscow'}",
    );

    // Query by first field should return both
    let result = run(&s, "SELECT * FROM users WHERE country = 'RU'");
    assert_eq!(result.as_array().unwrap().len(), 2);
}

#[test]
fn test_compound_index_field_order_matters() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(a, b)");

    run(&s, "INSERT INTO users {id: '1', a: 1, b: 2}");
    run(&s, "INSERT INTO users {id: '2', a: 1, b: 3}");
    run(&s, "INSERT INTO users {id: '3', a: 2, b: 2}");

    // Query on first field should use index
    let explain = run(&s, "EXPLAIN SELECT * FROM users WHERE a = 1");
    let plan = explain.as_array().unwrap()[0].as_str().unwrap();
    assert!(
        plan.contains("IndexScan") || plan.contains("Index"),
        "Expected index usage for first field query: {}",
        plan
    );
}

#[test]
fn test_compound_index_prefix_query() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(country, city, district)");

    run(
        &s,
        "INSERT INTO users {id: '1', country: 'RU', city: 'Moscow', district: 'Central'}",
    );
    run(
        &s,
        "INSERT INTO users {id: '2', country: 'RU', city: 'Moscow', district: 'North'}",
    );
    run(
        &s,
        "INSERT INTO users {id: '3', country: 'RU', city: 'SPb', district: 'Central'}",
    );

    // Query on all three fields - should use compound index
    let result = run(
        &s,
        "SELECT * FROM users WHERE country = 'RU' AND city = 'Moscow' AND district = 'Central'",
    );
    assert_eq!(result.as_array().unwrap().len(), 1);

    // Query on first field only - may fall back to scan but should work
    let result = run(&s, "SELECT * FROM users WHERE country = 'RU'");
    assert_eq!(result.as_array().unwrap().len(), 3);
}

#[test]
fn test_compound_unique_allows_partial_duplicates() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(country, email) UNIQUE");

    // Same country, different emails - OK
    run(
        &s,
        "INSERT INTO users {id: 'alice', country: 'RU', email: 'alice@test.com'}",
    );
    run(
        &s,
        "INSERT INTO users {id: 'bob', country: 'RU', email: 'bob@test.com'}",
    );

    // Same email, different countries - OK
    run(
        &s,
        "INSERT INTO users {id: 'carol', country: 'US', email: 'alice@test.com'}",
    );

    let result = run(&s, "SELECT * FROM users");
    assert_eq!(result.as_array().unwrap().len(), 3);
}

#[test]
fn test_compound_unique_rejects_full_duplicates() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(country, email) UNIQUE");

    run(
        &s,
        "INSERT INTO users {id: 'alice', country: 'RU', email: 'alice@test.com'}",
    );

    // Same country AND email - should fail
    let err = run_err(
        &s,
        "INSERT INTO users {id: 'bob', country: 'RU', email: 'alice@test.com'}",
    );
    assert!(
        err.contains("Conflict") || err.contains("Unique") || err.contains("constraint"),
        "Expected unique constraint error, got: {}",
        err
    );
}

#[test]
fn test_partial_compound_index_scan() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(status, age, name)");

    // Insert test data
    run(
        &s,
        "INSERT INTO users {id: 'u1', status: 'active', age: 25, name: 'Alice'}",
    );
    run(
        &s,
        "INSERT INTO users {id: 'u2', status: 'active', age: 30, name: 'Bob'}",
    );
    run(
        &s,
        "INSERT INTO users {id: 'u3', status: 'inactive', age: 25, name: 'Charlie'}",
    );

    // Query by first field only
    let result = run(&s, "SELECT * FROM users WHERE status = 'active'");
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 2, "Should find 2 active users");

    // Verify index is used
    let explain = run(&s, "EXPLAIN SELECT * FROM users WHERE status = 'active'");
    let plan = explain.as_array().unwrap()[0].as_str().unwrap();
    assert!(
        plan.contains("IndexScan"),
        "Should use IndexScan for partial compound query: {}",
        plan
    );
}
