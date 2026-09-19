//! Edge case tests for indexes on nested (dotted) fields.
//!
//! Nested field paths like `address.city` or `a.b.c` are supported by
//! filtering, projection, and indexes.

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

fn run_err(storage: &Arc<Database>, sql: &str) -> String {
    stellardb::try_run_sql!(storage.clone(), sql)
        .unwrap_err()
        .to_string()
}

// =============================================================================
// CREATE INDEX on nested fields
// =============================================================================

#[test]
fn test_create_index_on_nested_field_succeeds() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(
        &s,
        "INSERT INTO users {id: 'alice', address: {city: 'Moscow'}}",
    );

    // Parser accepts dotted field paths — CREATE INDEX should succeed
    let result = run(&s, "CREATE INDEX ON users(address.city)");
    let arr = result.as_array().unwrap();
    assert!(arr[0]["status"].is_string());
}

#[test]
fn test_describe_shows_nested_field_index() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(address.city)");

    let result = run(&s, "DESCRIBE COLLECTION users");
    let schema = &result.as_array().unwrap()[0];
    let indexes = schema["indexes"].as_array().unwrap();
    assert_eq!(indexes.len(), 1);
    assert_eq!(indexes[0]["fields"], serde_json::json!(["address.city"]));
}

#[test]
fn test_drop_index_on_nested_field() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(address.city)");

    let result = run(&s, "DROP INDEX idx_users_btree_address_city ON users");
    let arr = result.as_array().unwrap();
    assert!(arr[0]["status"].is_string());

    let result = run(&s, "DESCRIBE COLLECTION users");
    let schema = &result.as_array().unwrap()[0];
    assert!(schema["indexes"].as_array().unwrap().is_empty());
}

// =============================================================================
// Deeply nested fields (3+ levels)
// =============================================================================

#[test]
fn test_create_index_on_deeply_nested_field() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "INSERT INTO data {id: '1', a: {b: {c: {d: 'deep'}}}}");

    let result = run(&s, "CREATE INDEX ON data(a.b.c.d)");
    let arr = result.as_array().unwrap();
    assert!(arr[0]["status"].is_string());

    let result = run(&s, "DESCRIBE COLLECTION data");
    let schema = &result.as_array().unwrap()[0];
    let indexes = schema["indexes"].as_array().unwrap();
    assert_eq!(indexes[0]["fields"], serde_json::json!(["a.b.c.d"]));
}

// =============================================================================
// Filtering on nested fields

#[test]
fn test_filter_nested_field_without_index() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(
        &s,
        "INSERT INTO users {id: 'alice', address: {city: 'Moscow', zip: '101000'}}",
    );
    run(
        &s,
        "INSERT INTO users {id: 'bob', address: {city: 'Berlin', zip: '10115'}}",
    );

    let result = run(&s, "SELECT * FROM users WHERE address.city = 'Moscow'");
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1, "Expected 1 result, got {:?}", arr);
    assert_eq!(arr[0]["id"], "users:alice");
}

#[test]
fn test_filter_deeply_nested_field() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "INSERT INTO data {id: '1', a: {b: {c: 'target'}}}");
    run(&s, "INSERT INTO data {id: '2', a: {b: {c: 'other'}}}");

    let result = run(&s, "SELECT * FROM data WHERE a.b.c = 'target'");
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "data:1");
}

// =============================================================================
// Index-accelerated queries on nested fields

#[test]
fn test_index_scan_on_nested_field() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(address.city)");
    run(
        &s,
        "INSERT INTO users {id: 'alice', address: {city: 'Moscow'}}",
    );
    run(
        &s,
        "INSERT INTO users {id: 'bob', address: {city: 'Berlin'}}",
    );
    run(
        &s,
        "INSERT INTO users {id: 'carol', address: {city: 'Moscow'}}",
    );

    let result = run(&s, "SELECT * FROM users WHERE address.city = 'Moscow'");
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 2, "Expected 2 results, got {:?}", arr);

    // Verify index is used
    let explain = run(
        &s,
        "EXPLAIN SELECT * FROM users WHERE address.city = 'Moscow'",
    );
    let plan = explain.as_array().unwrap()[0].as_str().unwrap();
    assert!(
        plan.contains("IndexScan"),
        "Expected IndexScan in plan: {}",
        plan
    );
}

#[test]
fn test_index_range_scan_on_nested_field() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(address.zip)");
    run(&s, "INSERT INTO users {id: '1', address: {zip: 100}}");
    run(&s, "INSERT INTO users {id: '2', address: {zip: 200}}");
    run(&s, "INSERT INTO users {id: '3', address: {zip: 300}}");

    let result = run(&s, "SELECT * FROM users WHERE address.zip > 150");
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 2, "Expected zip=200 and zip=300, got {:?}", arr);
}

#[test]
fn test_unique_index_on_nested_field() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(address.email) UNIQUE");
    run(
        &s,
        "INSERT INTO users {id: 'alice', address: {email: 'alice@test.com'}}",
    );

    let err = run_err(
        &s,
        "INSERT INTO users {id: 'bob', address: {email: 'alice@test.com'}}",
    );
    assert!(
        err.contains("Conflict") || err.contains("Unique") || err.contains("constraint"),
        "Expected unique constraint error, got: {}",
        err
    );
}

#[test]
fn test_index_maintained_on_nested_field_update() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(address.city)");
    run(
        &s,
        "INSERT INTO users {id: 'alice', address: {city: 'Moscow'}}",
    );

    // Update nested field — index should reflect new value
    // Note: this requires UPDATE to support nested field assignment
    run(&s, "UPDATE users:alice SET address = {city: 'Berlin'}");

    let result = run(&s, "SELECT * FROM users WHERE address.city = 'Berlin'");
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "users:alice");

    // Old value should return nothing
    let result = run(&s, "SELECT * FROM users WHERE address.city = 'Moscow'");
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 0);
}

// =============================================================================
// Compound index with nested fields
// =============================================================================

#[test]
fn test_compound_index_with_nested_field() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(address.city, status)");
    run(
        &s,
        "INSERT INTO users {id: '1', address: {city: 'Moscow'}, status: 'active'}",
    );
    run(
        &s,
        "INSERT INTO users {id: '2', address: {city: 'Moscow'}, status: 'inactive'}",
    );
    run(
        &s,
        "INSERT INTO users {id: '3', address: {city: 'Berlin'}, status: 'active'}",
    );

    let result = run(
        &s,
        "SELECT * FROM users WHERE address.city = 'Moscow' AND status = 'active'",
    );
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "users:1");
}

#[test]
fn test_compound_index_both_nested() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(address.city, address.zip)");
    run(
        &s,
        "INSERT INTO users {id: '1', address: {city: 'Moscow', zip: '101'}}",
    );
    run(
        &s,
        "INSERT INTO users {id: '2', address: {city: 'Moscow', zip: '102'}}",
    );

    let result = run(
        &s,
        "SELECT * FROM users WHERE address.city = 'Moscow' AND address.zip = '101'",
    );
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "users:1");
}

// =============================================================================
// Edge cases: missing nested paths, null values
// =============================================================================

#[test]
fn test_index_nested_field_missing_parent() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(address.city)");

    // Document has no 'address' field at all — should not be indexed
    run(&s, "INSERT INTO users {id: 'alice', name: 'Alice'}");
    run(
        &s,
        "INSERT INTO users {id: 'bob', address: {city: 'Berlin'}}",
    );

    let result = run(&s, "SELECT * FROM users WHERE address.city = 'Berlin'");
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "users:bob");
}

#[test]
fn test_index_nested_field_null_intermediate() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(address.city)");

    // address is null, not an object
    run(&s, "INSERT INTO users {id: 'alice', address: null}");
    run(
        &s,
        "INSERT INTO users {id: 'bob', address: {city: 'Berlin'}}",
    );

    let result = run(&s, "SELECT * FROM users WHERE address.city = 'Berlin'");
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "users:bob");
}

#[test]
fn test_index_nested_field_wrong_type_intermediate() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(address.city)");

    // address is a string, not an object — city subfield doesn't exist
    run(
        &s,
        "INSERT INTO users {id: 'alice', address: 'not an object'}",
    );
    run(
        &s,
        "INSERT INTO users {id: 'bob', address: {city: 'Berlin'}}",
    );

    let result = run(&s, "SELECT * FROM users WHERE address.city = 'Berlin'");
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "users:bob");
}

// =============================================================================
// Projection of nested fields
// =============================================================================

#[test]
fn test_select_nested_field_in_projection() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(
        &s,
        "INSERT INTO users {id: 'alice', address: {city: 'Moscow', zip: '101000'}}",
    );

    let result = run(&s, "SELECT address.city FROM users");
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["address.city"], "Moscow");
}
