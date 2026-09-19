//! Useful tests for boundary values and special strings.
//!
//! Tests robustness with:
//! - Empty strings
//! - Unicode strings
//! - Very long strings
//! - Boolean values
//! - Strings with quotes, backslashes, newlines

use super::{run, run_err, setup};

// =============================================================================
// Empty string and null handling
// =============================================================================

#[test]
fn test_index_with_empty_string() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(name) UNIQUE");

    run(&s, "INSERT INTO users {id: 'alice', name: ''}");

    // Empty string is a valid unique value
    let err = run_err(&s, "INSERT INTO users {id: 'bob', name: ''}");
    assert!(
        err.contains("Conflict") || err.contains("Unique"),
        "Empty string should be subject to unique constraint, got: {}",
        err
    );
}

#[test]
fn test_index_empty_string_vs_null() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(name) UNIQUE");

    // Empty string and null are different values
    run(&s, "INSERT INTO users {id: 'alice', name: ''}");
    run(&s, "INSERT INTO users {id: 'bob', name: null}");

    let result = run(&s, "SELECT * FROM users");
    assert_eq!(result.as_array().unwrap().len(), 2);
}

// =============================================================================
// Unicode strings
// =============================================================================

#[test]
fn test_index_with_unicode_strings() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(name) UNIQUE");

    run(&s, "INSERT INTO users {id: 'alice', name: 'Алиса'}");
    run(&s, "INSERT INTO users {id: 'bob', name: '日本語'}");
    run(&s, "INSERT INTO users {id: 'carol', name: '🎉🚀'}");

    let result = run(&s, "SELECT * FROM users WHERE name = 'Алиса'");
    assert_eq!(result.as_array().unwrap().len(), 1);

    let result = run(&s, "SELECT * FROM users WHERE name = '🎉🚀'");
    assert_eq!(result.as_array().unwrap().len(), 1);
}

// =============================================================================
// Very long strings
// =============================================================================

#[test]
fn test_index_with_very_long_string() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(data)");

    let long_string = "x".repeat(10000);
    run(
        &s,
        &format!("INSERT INTO users {{id: 'alice', data: '{}'}}", long_string),
    );

    let result = run(
        &s,
        &format!("SELECT * FROM users WHERE data = '{}'", long_string),
    );
    assert_eq!(result.as_array().unwrap().len(), 1);
}

// =============================================================================
// Boolean values
// =============================================================================

#[test]
fn test_index_bool_values() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(flag)");

    run(&s, "INSERT INTO data {id: 'true1', flag: true}");
    run(&s, "INSERT INTO data {id: 'true2', flag: true}");
    run(&s, "INSERT INTO data {id: 'false1', flag: false}");

    let result = run(&s, "SELECT * FROM data WHERE flag = true");
    assert_eq!(result.as_array().unwrap().len(), 2);

    let result = run(&s, "SELECT * FROM data WHERE flag = false");
    assert_eq!(result.as_array().unwrap().len(), 1);
}

// =============================================================================
// Special characters in strings
// =============================================================================

#[test]
fn test_index_with_string_containing_quotes() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value) UNIQUE");

    // Strings with escaped quotes
    run(&s, r#"INSERT INTO data {id: '1', value: 'hello "world"'}"#);
    run(&s, r#"INSERT INTO data {id: '2', value: "it's fine"}"#);

    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 2);
}

#[test]
fn test_index_with_string_containing_backslash() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(path) UNIQUE");

    run(&s, r#"INSERT INTO data {id: '1', path: 'C:\\Users\\test'}"#);
    run(&s, r#"INSERT INTO data {id: '2', path: '/home/user'}"#);

    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 2);
}

#[test]
fn test_index_with_string_containing_newlines() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(text)");

    run(&s, r#"INSERT INTO data {id: '1', text: 'line1\nline2'}"#);
    run(&s, r#"INSERT INTO data {id: '2', text: 'single line'}"#);

    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 2);
}
