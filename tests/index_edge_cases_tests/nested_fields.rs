//! Important tests for nested field index edge cases.
//!
//! Tests how indexes handle:
//! - Intermediate fields becoming non-objects
//! - Intermediate fields becoming null
//! - Deeply nested paths

use super::{run, setup};

#[test]
fn test_nested_index_intermediate_becomes_non_object() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(address.city)");

    // Start with valid nested structure
    run(
        &s,
        "INSERT INTO users {id: 'alice', address: {city: 'Moscow'}}",
    );

    // Update address to be a string instead of object
    run(&s, "UPDATE users:alice SET address = 'invalid'");

    // Query should not find alice anymore
    let result = run(&s, "SELECT * FROM users WHERE address.city = 'Moscow'");
    assert_eq!(result.as_array().unwrap().len(), 0);
}

#[test]
fn test_nested_index_intermediate_becomes_null() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(address.city)");

    run(
        &s,
        "INSERT INTO users {id: 'alice', address: {city: 'Moscow'}}",
    );

    // Update address to null
    run(&s, "UPDATE users:alice SET address = null");

    // Query should not find alice
    let result = run(&s, "SELECT * FROM users WHERE address.city = 'Moscow'");
    assert_eq!(result.as_array().unwrap().len(), 0);
}

#[test]
fn test_deeply_nested_index_five_levels() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(a.b.c.d.e)");

    run(
        &s,
        "INSERT INTO data {id: '1', a: {b: {c: {d: {e: 'deep'}}}}}",
    );
    run(
        &s,
        "INSERT INTO data {id: '2', a: {b: {c: {d: {e: 'other'}}}}}",
    );

    let result = run(&s, "SELECT * FROM data WHERE a.b.c.d.e = 'deep'");
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result.as_array().unwrap()[0]["id"], "data:1");
}
