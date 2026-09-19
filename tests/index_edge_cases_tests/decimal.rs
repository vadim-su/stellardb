//! Tests for Decimal type integration with indexes.
//!
//! Covers:
//! - Basic decimal storage and retrieval
//! - Cross-type numeric operations
//! - Decimal in unique constraints
//! - Decimal aggregates and ORDER BY

use super::{run, run_err, setup};

// =============================================================================
// Basic decimal range queries
// =============================================================================

#[test]
fn test_index_decimal_range_queries() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    run(&s, "INSERT INTO data {id: 'd1', value: 99dec}");
    run(&s, "INSERT INTO data {id: 'd2', value: 99.99dec}");
    run(&s, "INSERT INTO data {id: 'd3', value: 100}"); // Int
    run(&s, "INSERT INTO data {id: 'd4', value: 100.5}"); // Float

    // Decimal < Int
    let result = run(&s, "SELECT * FROM data WHERE value < 100");
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "99dec and 99.99dec should be < 100"
    );

    // Mixed comparison (using float literal since expression parser doesn't support dec suffix yet)
    let result = run(&s, "SELECT * FROM data WHERE value >= 99.99");
    assert_eq!(
        result.as_array().unwrap().len(),
        3,
        "99.99dec, 100, 100.5 should be >= 99.99"
    );
}

#[test]
fn test_decimal_equality() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    run(&s, "INSERT INTO data {id: 'd1', value: 100dec}");
    run(&s, "INSERT INTO data {id: 'd2', value: 100}");
    run(&s, "INSERT INTO data {id: 'd3', value: 100.0}");

    // All three should be found with = 100
    let result = run(&s, "SELECT * FROM data WHERE value = 100");
    assert_eq!(
        result.as_array().unwrap().len(),
        3,
        "100dec, 100, 100.0 should all equal 100"
    );
}

// =============================================================================
// Decimal precision
// =============================================================================

#[test]
fn test_decimal_precision() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    // High precision decimal values
    run(&s, "INSERT INTO data {id: 'd1', value: 0.123456789dec}");
    run(&s, "INSERT INTO data {id: 'd2', value: 99.999999dec}");
    run(&s, "INSERT INTO data {id: 'd3', value: 0.000000001dec}");

    // Verify all values are stored
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 3);

    // Range query on high-precision decimals
    let result = run(&s, "SELECT * FROM data WHERE value > 0.1");
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "0.123456789dec and 99.999999dec should be > 0.1"
    );

    let result = run(&s, "SELECT * FROM data WHERE value < 1.0");
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "0.123456789dec and 0.000000001dec should be < 1"
    );
}

#[test]
fn test_very_large_decimal() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    // Large decimal values
    run(&s, "INSERT INTO data {id: 'd1', value: 999999999999dec}");
    run(&s, "INSERT INTO data {id: 'd2', value: 1000000000000dec}");
    run(&s, "INSERT INTO data {id: 'd3', value: 500000000000dec}");

    // Verify all values stored
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 3);

    // Equality query on large decimal value
    let result = run(&s, "SELECT * FROM data WHERE value = 999999999999dec");
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result.as_array().unwrap()[0]["id"], "data:d1");

    // Another equality query on large decimal
    let result = run(&s, "SELECT * FROM data WHERE value = 1000000000000dec");
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result.as_array().unwrap()[0]["id"], "data:d2");
}

#[test]
fn test_very_small_decimal() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    // Very small decimal values
    run(&s, "INSERT INTO data {id: 'd1', value: 0.000001dec}");
    run(&s, "INSERT INTO data {id: 'd2', value: 0.0000001dec}");
    run(&s, "INSERT INTO data {id: 'd3', value: 0.00000001dec}");
    run(&s, "INSERT INTO data {id: 'd4', value: 0dec}");

    // Verify all values stored
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 4);

    // Equality query on zero decimal
    let result = run(&s, "SELECT * FROM data WHERE value = 0dec");
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result.as_array().unwrap()[0]["id"], "data:d4");

    // Equality query on small decimal
    let result = run(&s, "SELECT * FROM data WHERE value = 0.000001dec");
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result.as_array().unwrap()[0]["id"], "data:d1");

    // Verify distinct small values are stored separately
    let result = run(&s, "SELECT * FROM data WHERE value = 0.0000001dec");
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result.as_array().unwrap()[0]["id"], "data:d2");
}

// =============================================================================
// Negative decimal values
// =============================================================================

#[test]
fn test_negative_decimal_storage() {
    // Tests that negative decimal values can be stored and queried.
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    // Negative decimal values
    run(&s, "INSERT INTO data {id: 'd1', value: -99.5dec}");
    run(&s, "INSERT INTO data {id: 'd2', value: -50.25dec}");
    run(&s, "INSERT INTO data {id: 'd3', value: -0.01dec}");
    run(&s, "INSERT INTO data {id: 'd4', value: 0dec}");
    run(&s, "INSERT INTO data {id: 'd5', value: 10.5dec}");

    // Verify all values stored
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 5);

    // Verify positive decimal query works
    let result = run(&s, "SELECT * FROM data WHERE value = 0dec");
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result.as_array().unwrap()[0]["id"], "data:d4");

    let result = run(&s, "SELECT * FROM data WHERE value = 10.5dec");
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result.as_array().unwrap()[0]["id"], "data:d5");

    // Negative decimal queries now work (unary minus applied to decimal)
    let result = run(&s, "SELECT * FROM data WHERE value = -99.5dec");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "-99.5dec should find one doc"
    );
    assert_eq!(result.as_array().unwrap()[0]["id"], "data:d1");

    let result = run(&s, "SELECT * FROM data WHERE value = -50.25dec");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "-50.25dec should find one doc"
    );
    assert_eq!(result.as_array().unwrap()[0]["id"], "data:d2");

    // Range query with negative decimals
    let result = run(&s, "SELECT * FROM data WHERE value < 0dec");
    assert_eq!(
        result.as_array().unwrap().len(),
        3,
        "3 values should be < 0"
    );
}

// =============================================================================
// Decimal storage and retrieval
// =============================================================================

#[test]
fn test_decimal_storage_and_retrieval() {
    // Tests that decimal values are stored and can be retrieved.
    // ORDER and range queries on decimals now work correctly with unified numeric encoding.
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    // All decimal values
    run(&s, "INSERT INTO data {id: 'd1', value: 30dec}");
    run(&s, "INSERT INTO data {id: 'd2', value: 10dec}");
    run(&s, "INSERT INTO data {id: 'd3', value: 50dec}");
    run(&s, "INSERT INTO data {id: 'd4', value: 20dec}");
    run(&s, "INSERT INTO data {id: 'd5', value: 40dec}");

    // Verify all documents are stored
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 5);

    // Verify equality queries work correctly
    let result = run(&s, "SELECT * FROM data WHERE value = 30dec");
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result.as_array().unwrap()[0]["id"], "data:d1");

    let result = run(&s, "SELECT * FROM data WHERE value = 10dec");
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result.as_array().unwrap()[0]["id"], "data:d2");
}

#[test]
fn test_decimal_equality_various() {
    // Tests decimal equality queries - these should work correctly.
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    run(&s, "INSERT INTO data {id: 'd1', value: 10.5dec}");
    run(&s, "INSERT INTO data {id: 'd2', value: 20dec}");
    run(&s, "INSERT INTO data {id: 'd3', value: 30.75dec}");
    run(&s, "INSERT INTO data {id: 'd4', value: 40dec}");
    run(&s, "INSERT INTO data {id: 'd5', value: 50.25dec}");

    // Verify all data is inserted
    let all = run(&s, "SELECT * FROM data");
    assert_eq!(
        all.as_array().unwrap().len(),
        5,
        "5 docs should be inserted"
    );

    // Equality queries on decimal values
    let result = run(&s, "SELECT * FROM data WHERE value = 20dec");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "value = 20dec should find one doc"
    );

    let result = run(&s, "SELECT * FROM data WHERE value = 30.75dec");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "value = 30.75dec should find one doc"
    );

    let result = run(&s, "SELECT * FROM data WHERE value = 10.5dec");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "value = 10.5dec should find one doc"
    );
}

// =============================================================================
// Decimal in compound indexes
// =============================================================================

#[test]
fn test_decimal_in_compound_index() {
    // Tests compound index with decimal amounts using equality queries.
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION orders");
    run(&s, "CREATE INDEX ON orders(status, amount)");

    run(
        &s,
        "INSERT INTO orders {id: 'o1', status: 'pending', amount: 99.99dec}",
    );
    run(
        &s,
        "INSERT INTO orders {id: 'o2', status: 'pending', amount: 150dec}",
    );
    run(
        &s,
        "INSERT INTO orders {id: 'o3', status: 'completed', amount: 99.99dec}",
    );
    run(
        &s,
        "INSERT INTO orders {id: 'o4', status: 'completed', amount: 200.50dec}",
    );

    // Query by status prefix should use index
    let result = run(&s, "SELECT * FROM orders WHERE status = 'pending'");
    assert_eq!(result.as_array().unwrap().len(), 2);

    let result = run(&s, "SELECT * FROM orders WHERE status = 'completed'");
    assert_eq!(result.as_array().unwrap().len(), 2);

    // Query with first field equality only - should work
    let result = run(&s, "SELECT * FROM orders WHERE status = 'completed'");
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 2);

    // Verify we can retrieve the documents correctly
    let result = run(&s, "SELECT * FROM orders");
    assert_eq!(result.as_array().unwrap().len(), 4);
}

// =============================================================================
// Decimal unique constraints
// =============================================================================

#[test]
fn test_decimal_unique_constraint() {
    // Tests unique constraint behavior with decimal values.
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION accounts");
    run(&s, "CREATE INDEX ON accounts(balance) UNIQUE");

    // Insert decimal value
    run(&s, "INSERT INTO accounts {id: 'a1', balance: 100dec}");

    // Insert same decimal value - should fail
    let err = run_err(&s, "INSERT INTO accounts {id: 'a2', balance: 100dec}");
    assert!(
        err.contains("Conflict") || err.contains("Unique") || err.contains("constraint"),
        "Duplicate decimal should violate unique constraint, got: {}",
        err
    );

    // Different value should succeed
    run(&s, "INSERT INTO accounts {id: 'a3', balance: 100.01dec}");

    let result = run(&s, "SELECT * FROM accounts");
    assert_eq!(result.as_array().unwrap().len(), 2);
}

#[test]
fn test_cross_type_unique_constraint() {
    // Tests that equivalent numeric values across Int/Float/Decimal types
    // DO conflict in unique constraint (unified numeric encoding).
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value) UNIQUE");

    // Insert Int value
    run(&s, "INSERT INTO data {id: 'd1', value: 100}");

    // Insert equivalent Decimal - SHOULD conflict because 100 == 100dec
    let err = run_err(&s, "INSERT INTO data {id: 'd2', value: 100dec}");
    assert!(
        err.contains("Conflict") || err.contains("Unique") || err.contains("constraint"),
        "100dec should conflict with 100 in unique index, got: {}",
        err
    );

    // Insert equivalent Float - SHOULD also conflict
    let err = run_err(&s, "INSERT INTO data {id: 'd3', value: 100.0}");
    assert!(
        err.contains("Conflict") || err.contains("Unique") || err.contains("constraint"),
        "100.0 should conflict with 100 in unique index, got: {}",
        err
    );

    // Different value should succeed
    run(&s, "INSERT INTO data {id: 'd4', value: 101}");

    let result = run(&s, "SELECT * FROM data");
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "Only 2 records (100 and 101)"
    );
}

// =============================================================================
// Decimal updates
// =============================================================================

#[test]
fn test_decimal_update() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION accounts");
    run(&s, "CREATE INDEX ON accounts(balance)");

    run(&s, "INSERT INTO accounts {id: 'a1', balance: 100.50dec}");

    // Verify initial value
    let result = run(&s, "SELECT * FROM accounts WHERE balance = 100.50");
    assert_eq!(result.as_array().unwrap().len(), 1);

    // Update to new decimal value
    run(&s, "UPDATE accounts:a1 SET balance = 200.75dec");

    // Old value should not be found
    let result = run(&s, "SELECT * FROM accounts WHERE balance = 100.50");
    assert_eq!(
        result.as_array().unwrap().len(),
        0,
        "old value should be gone"
    );

    // New value should be found
    let result = run(&s, "SELECT * FROM accounts WHERE balance = 200.75");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "new value should be indexed"
    );

    // Update to Int value
    run(&s, "UPDATE accounts:a1 SET balance = 300");

    let result = run(&s, "SELECT * FROM accounts WHERE balance = 300");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "Int value should be found"
    );

    // Update back to decimal
    run(&s, "UPDATE accounts:a1 SET balance = 350.99dec");

    let result = run(&s, "SELECT * FROM accounts WHERE balance = 350.99");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "decimal value should be found"
    );
}

// =============================================================================
// Decimal aggregates
// =============================================================================

#[test]
fn test_mixed_numeric_aggregates() {
    // Tests aggregate functions on mixed Int/Float values.
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION transactions");

    run(&s, "INSERT INTO transactions {id: 't1', amount: 10.5}"); // Float
    run(&s, "INSERT INTO transactions {id: 't2', amount: 20}"); // Int
    run(&s, "INSERT INTO transactions {id: 't3', amount: 30.25}"); // Float
    run(&s, "INSERT INTO transactions {id: 't4', amount: 40}"); // Int

    // SUM
    let result = run(&s, "SELECT SUM(amount) AS total FROM transactions");
    let total = result.as_array().unwrap()[0]["total"].as_f64().unwrap();
    assert!(
        (total - 100.75).abs() < 0.0001,
        "SUM should be 100.75, got {}",
        total
    );

    // AVG
    let result = run(&s, "SELECT AVG(amount) AS avg FROM transactions");
    let avg = result.as_array().unwrap()[0]["avg"].as_f64().unwrap();
    assert!(
        (avg - 25.1875).abs() < 0.0001,
        "AVG should be 25.1875, got {}",
        avg
    );

    // MIN
    let result = run(&s, "SELECT MIN(amount) AS min FROM transactions");
    let min = result.as_array().unwrap()[0]["min"].as_f64().unwrap();
    assert!(
        (min - 10.5).abs() < 0.0001,
        "MIN should be 10.5, got {}",
        min
    );

    // MAX
    let result = run(&s, "SELECT MAX(amount) AS max FROM transactions");
    let max = result.as_array().unwrap()[0]["max"].as_i64().unwrap();
    assert_eq!(max, 40, "MAX should be 40");
}

#[test]
fn test_decimal_aggregates() {
    // Tests that aggregate functions work with Decimal values.
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION transactions");

    run(&s, "INSERT INTO transactions {id: 't1', amount: 10.5dec}"); // Decimal
    run(&s, "INSERT INTO transactions {id: 't2', amount: 20}"); // Int
    run(&s, "INSERT INTO transactions {id: 't3', amount: 30.25dec}"); // Decimal
    run(&s, "INSERT INTO transactions {id: 't4', amount: 40}"); // Int

    // SUM - should include decimals
    let result = run(&s, "SELECT SUM(amount) AS total FROM transactions");
    let total = result.as_array().unwrap()[0]["total"].as_f64().unwrap();
    assert!(
        (total - 100.75).abs() < 0.0001,
        "SUM should be 100.75, got {}",
        total
    );

    // AVG - should include decimals
    let result = run(&s, "SELECT AVG(amount) AS avg FROM transactions");
    let avg = result.as_array().unwrap()[0]["avg"].as_f64().unwrap();
    assert!(
        (avg - 25.1875).abs() < 0.0001,
        "AVG should be ~25.1875, got {}",
        avg
    );

    // MIN - should find decimal
    let result = run(&s, "SELECT MIN(amount) AS min FROM transactions");
    let min = result.as_array().unwrap()[0]["min"].as_f64().unwrap();
    assert!(
        (min - 10.5).abs() < 0.0001,
        "MIN should be 10.5 (decimal), got {}",
        min
    );

    // MAX - should find int
    let result = run(&s, "SELECT MAX(amount) AS max FROM transactions");
    let max = result.as_array().unwrap()[0]["max"].as_i64().unwrap();
    assert_eq!(max, 40, "MAX should be 40 (int)");
}

// =============================================================================
// Decimal ORDER BY
// =============================================================================

#[test]
fn test_decimal_order_by() {
    // Tests that ORDER correctly sorts mixed Int/Float/Decimal values numerically.
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");

    run(&s, "INSERT INTO data {id: 'd1', amount: 30dec}");
    run(&s, "INSERT INTO data {id: 'd2', amount: 10}"); // Int
    run(&s, "INSERT INTO data {id: 'd3', amount: 50.5}"); // Float
    run(&s, "INSERT INTO data {id: 'd4', amount: 20dec}");
    run(&s, "INSERT INTO data {id: 'd5', amount: 40.25}"); // Float

    // ORDER ASC - should sort numerically: 10, 20, 30, 40.25, 50.5
    let result = run(&s, "SELECT id FROM data ORDER amount ASC");
    let ids: Vec<&str> = result
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec!["data:d2", "data:d4", "data:d1", "data:d5", "data:d3"],
        "ORDER ASC should sort numerically across Int/Float/Decimal"
    );

    // ORDER DESC
    let result = run(&s, "SELECT id FROM data ORDER amount DESC");
    let ids: Vec<&str> = result
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec!["data:d3", "data:d5", "data:d1", "data:d4", "data:d2"],
        "ORDER DESC should sort numerically across Int/Float/Decimal"
    );
}

// =============================================================================
// Decimal unary minus and expressions
// =============================================================================

#[test]
fn test_decimal_unary_minus_in_expressions() {
    // Tests that unary minus works correctly with decimal values in expressions.
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    run(&s, "INSERT INTO data {id: 'd1', value: 100dec}");
    run(&s, "INSERT INTO data {id: 'd2', value: -50dec}");
    run(&s, "INSERT INTO data {id: 'd3', value: 25.5dec}");

    // Query with negation of decimal literal
    let result = run(&s, "SELECT * FROM data WHERE value = -50dec");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "-50dec query should work"
    );
    assert_eq!(result.as_array().unwrap()[0]["id"], "data:d2");

    // Query with negative decimal in comparison
    let result = run(&s, "SELECT * FROM data WHERE value > -100dec");
    assert_eq!(result.as_array().unwrap().len(), 3, "all values > -100dec");

    // Query with computed negative
    let result = run(&s, "SELECT * FROM data WHERE value < -25dec");
    assert_eq!(result.as_array().unwrap().len(), 1, "only -50dec < -25dec");
}

#[test]
fn test_decimal_range_queries_comprehensive() {
    // Comprehensive test for decimal range queries with mixed numeric types.
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(amount)");

    // Insert mixed types
    run(&s, "INSERT INTO data {id: 'd1', amount: 10}"); // Int
    run(&s, "INSERT INTO data {id: 'd2', amount: 20.5}"); // Float
    run(&s, "INSERT INTO data {id: 'd3', amount: 30dec}"); // Decimal
    run(&s, "INSERT INTO data {id: 'd4', amount: 40.75dec}"); // Decimal
    run(&s, "INSERT INTO data {id: 'd5', amount: 50}"); // Int

    // Range with Int boundary
    let result = run(&s, "SELECT * FROM data WHERE amount > 20");
    assert_eq!(result.as_array().unwrap().len(), 4, "4 values > 20");

    // Range with Float boundary
    let result = run(&s, "SELECT * FROM data WHERE amount >= 20.5");
    assert_eq!(result.as_array().unwrap().len(), 4, "4 values >= 20.5");

    // Range with Decimal boundary
    let result = run(&s, "SELECT * FROM data WHERE amount < 30dec");
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "2 values < 30dec (10 and 20.5)"
    );

    // Range query with mixed boundaries (BETWEEN not supported, use >= AND <=)
    let result = run(&s, "SELECT * FROM data WHERE amount >= 20 AND amount <= 40");
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "2 values between 20 and 40: 20.5 and 30"
    );
}
