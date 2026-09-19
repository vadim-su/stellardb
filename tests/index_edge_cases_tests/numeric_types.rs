//! Tests for numeric type handling in indexes.
//!
//! Tests cross-type range queries (Int vs Float) and numeric boundaries.

use super::{run, setup};

// =============================================================================
// Numeric boundaries - integers
// =============================================================================

#[test]
fn test_index_numeric_boundaries_int() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    // Use large but parseable values (parser may have limits)
    run(&s, "INSERT INTO data {id: 'min', value: -1000000000000}");
    run(&s, "INSERT INTO data {id: 'max', value: 1000000000000}");
    run(&s, "INSERT INTO data {id: 'zero', value: 0}");

    // Range query should order correctly
    let result = run(&s, "SELECT * FROM data WHERE value >= -1000000000000");
    assert_eq!(result.as_array().unwrap().len(), 3);

    let result = run(&s, "SELECT * FROM data WHERE value < 0");
    assert_eq!(result.as_array().unwrap().len(), 1);
}

// Test: Cross-type numeric range queries work correctly with unified numeric encoding.
// Previously broken: Int and Float had separate type tags (0x02 vs 0x03),
// causing `value > 0` (Int) to not find Float values. Fixed with TAG_NUMBER.
#[test]
fn test_index_numeric_boundaries_float() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    run(&s, "INSERT INTO data {id: 'neg', value: -1.5}");
    run(&s, "INSERT INTO data {id: 'zero', value: 0.0}");
    run(&s, "INSERT INTO data {id: 'pos', value: 1.5}");
    run(&s, "INSERT INTO data {id: 'small', value: 0.0000001}");

    // Test equality first (should work)
    let result = run(&s, "SELECT * FROM data WHERE value = 1.5");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "Equality on float should work"
    );

    // Range query on floats: value > 0 should return 2 (1.5 and 0.0000001)
    // Fixed: unified numeric encoding ensures correct cross-type comparisons
    let result = run(&s, "SELECT * FROM data WHERE value > 0");
    let count = result.as_array().unwrap().len();
    assert_eq!(
        count, 2,
        "value > 0 should return exactly 2 docs (1.5 and 0.0000001), got {}: {:?}",
        count, result
    );
}

// =============================================================================
// Cross-type numeric range queries (Int vs Float)
// These tests verify that unified numeric encoding (TAG_NUMBER) works correctly.
// Previously broken: Int used TAG_INT64 (0x02), Float used TAG_FLOAT64 (0x03),
// causing all Floats to sort AFTER all Ints in lexicographic byte ordering.
// =============================================================================

#[test]
fn test_index_cross_type_range_gt() {
    // Tests that Int query values work correctly with Float index values
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    run(&s, "INSERT INTO data {id: 'd1', value: -10.5}");
    run(&s, "INSERT INTO data {id: 'd2', value: -5.0}");
    run(&s, "INSERT INTO data {id: 'd3', value: 0.0}");
    run(&s, "INSERT INTO data {id: 'd4', value: 5.5}");
    run(&s, "INSERT INTO data {id: 'd5', value: 10.0}");

    // value > 0 (Int) should match 5.5 and 10.0 (Floats)
    // Fixed: unified numeric encoding ensures correct cross-type comparisons
    let result = run(&s, "SELECT * FROM data WHERE value > 0");
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "value > 0 (Int) should find 2 floats"
    );

    // value > -6 (Int) should match -5.0, 0.0, 5.5, 10.0
    let result = run(&s, "SELECT * FROM data WHERE value > -6");
    assert_eq!(
        result.as_array().unwrap().len(),
        4,
        "value > -6 (Int) should find 4 floats"
    );
}

#[test]
fn test_index_cross_type_range_lt() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    run(&s, "INSERT INTO data {id: 'd1', value: -10.5}");
    run(&s, "INSERT INTO data {id: 'd2', value: -5.0}");
    run(&s, "INSERT INTO data {id: 'd3', value: 0.0}");
    run(&s, "INSERT INTO data {id: 'd4', value: 5.5}");
    run(&s, "INSERT INTO data {id: 'd5', value: 10.0}");

    // value < 0 (Int) should match -10.5 and -5.0 (Floats)
    // Fixed: unified numeric encoding ensures correct cross-type comparisons
    let result = run(&s, "SELECT * FROM data WHERE value < 0");
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "value < 0 (Int) should find 2 floats"
    );

    // value < 6 (Int) should match -10.5, -5.0, 0.0, 5.5
    let result = run(&s, "SELECT * FROM data WHERE value < 6");
    assert_eq!(
        result.as_array().unwrap().len(),
        4,
        "value < 6 (Int) should find 4 floats"
    );
}

#[test]
fn test_index_cross_type_range_gte_lte() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    run(&s, "INSERT INTO data {id: 'd1', value: -5.0}");
    run(&s, "INSERT INTO data {id: 'd2', value: 0.0}");
    run(&s, "INSERT INTO data {id: 'd3', value: 5.0}");
    run(&s, "INSERT INTO data {id: 'd4', value: 10.0}");

    // value >= 0 (Int) should match 0.0, 5.0, 10.0
    let result = run(&s, "SELECT * FROM data WHERE value >= 0");
    assert_eq!(
        result.as_array().unwrap().len(),
        3,
        "value >= 0 should find 3"
    );

    // value <= 5 (Int) should match -5.0, 0.0, 5.0
    let result = run(&s, "SELECT * FROM data WHERE value <= 5");
    assert_eq!(
        result.as_array().unwrap().len(),
        3,
        "value <= 5 should find 3"
    );
}

#[test]
fn test_index_cross_type_int_values_float_query() {
    // Reverse case: Int values in index, Float in query
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    run(&s, "INSERT INTO data {id: 'd1', value: -10}");
    run(&s, "INSERT INTO data {id: 'd2', value: 0}");
    run(&s, "INSERT INTO data {id: 'd3', value: 10}");
    run(&s, "INSERT INTO data {id: 'd4', value: 20}");

    // value > 0.5 (Float) should match 10 and 20 (Ints)
    // Fixed: unified numeric encoding ensures correct cross-type comparisons
    let result = run(&s, "SELECT * FROM data WHERE value > 0.5");
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "value > 0.5 should find 2 ints"
    );

    // value < 15.5 (Float) should match -10, 0, 10
    let result = run(&s, "SELECT * FROM data WHERE value < 15.5");
    assert_eq!(
        result.as_array().unwrap().len(),
        3,
        "value < 15.5 should find 3 ints"
    );
}

#[test]
fn test_index_mixed_int_float_values() {
    // Mix of Int and Float values in same index
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    run(&s, "INSERT INTO data {id: 'd1', value: -5}"); // Int
    run(&s, "INSERT INTO data {id: 'd2', value: -2.5}"); // Float
    run(&s, "INSERT INTO data {id: 'd3', value: 0}"); // Int
    run(&s, "INSERT INTO data {id: 'd4', value: 2.5}"); // Float
    run(&s, "INSERT INTO data {id: 'd5', value: 5}"); // Int

    // value > 0 should find 2.5 (Float) and 5 (Int)
    let result = run(&s, "SELECT * FROM data WHERE value > 0");
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "value > 0 should find 2 mixed"
    );

    // value >= -2.5 should find -2.5, 0, 2.5, 5
    let result = run(&s, "SELECT * FROM data WHERE value >= -2.5");
    assert_eq!(
        result.as_array().unwrap().len(),
        4,
        "value >= -2.5 should find 4 mixed"
    );

    // value < 2.5 should find -5, -2.5, 0
    let result = run(&s, "SELECT * FROM data WHERE value < 2.5");
    assert_eq!(
        result.as_array().unwrap().len(),
        3,
        "value < 2.5 should find 3 mixed"
    );
}

#[test]
fn test_index_float_between_integers() {
    // Float values that fall between integer boundaries
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    run(&s, "INSERT INTO data {id: 'd1', value: 0.1}");
    run(&s, "INSERT INTO data {id: 'd2', value: 0.5}");
    run(&s, "INSERT INTO data {id: 'd3', value: 0.9}");
    run(&s, "INSERT INTO data {id: 'd4', value: 1.1}");

    // All should be > 0 (Int)
    let result = run(&s, "SELECT * FROM data WHERE value > 0");
    assert_eq!(
        result.as_array().unwrap().len(),
        4,
        "all floats should be > 0"
    );

    // All should be < 2 (Int)
    let result = run(&s, "SELECT * FROM data WHERE value < 2");
    assert_eq!(
        result.as_array().unwrap().len(),
        4,
        "all floats should be < 2"
    );

    // Only 0.1, 0.5, 0.9 should be < 1 (Int)
    let result = run(&s, "SELECT * FROM data WHERE value < 1");
    assert_eq!(
        result.as_array().unwrap().len(),
        3,
        "3 floats should be < 1"
    );

    // value >= 1 (Int) should find only 1.1
    let result = run(&s, "SELECT * FROM data WHERE value >= 1");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "only 1.1 should be >= 1"
    );
}

#[test]
fn test_index_very_small_floats() {
    // Very small positive floats should be > 0
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    run(&s, "INSERT INTO data {id: 'd1', value: 0.0}");
    run(&s, "INSERT INTO data {id: 'd2', value: 0.0000001}");
    run(&s, "INSERT INTO data {id: 'd3', value: 0.000000001}");
    run(&s, "INSERT INTO data {id: 'd4', value: -0.0000001}");

    // value > 0 should find the two positive tiny floats
    let result = run(&s, "SELECT * FROM data WHERE value > 0");
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "tiny positive floats should be > 0"
    );

    // value < 0 should find the negative tiny float
    let result = run(&s, "SELECT * FROM data WHERE value < 0");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "tiny negative float should be < 0"
    );

    // value >= 0 should find 0.0 and the two positive
    let result = run(&s, "SELECT * FROM data WHERE value >= 0");
    assert_eq!(
        result.as_array().unwrap().len(),
        3,
        "0 and tiny positives should be >= 0"
    );
}

#[test]
fn test_index_negative_float_range() {
    // Range queries on negative floats with Int boundaries
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    run(&s, "INSERT INTO data {id: 'd1', value: -100.5}");
    run(&s, "INSERT INTO data {id: 'd2', value: -50.0}");
    run(&s, "INSERT INTO data {id: 'd3', value: -10.5}");
    run(&s, "INSERT INTO data {id: 'd4', value: -5.0}");

    // value > -50 (Int) should find -10.5 and -5.0
    let result = run(&s, "SELECT * FROM data WHERE value > -50");
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "value > -50 should find 2"
    );

    // value >= -50 (Int) should find -50.0, -10.5, -5.0
    let result = run(&s, "SELECT * FROM data WHERE value >= -50");
    assert_eq!(
        result.as_array().unwrap().len(),
        3,
        "value >= -50 should find 3"
    );

    // value < -10 (Int) should find -100.5 and -50.0 and -10.5
    let result = run(&s, "SELECT * FROM data WHERE value < -10");
    assert_eq!(
        result.as_array().unwrap().len(),
        3,
        "value < -10 should find 3"
    );
}

// =============================================================================
// Mixed type indexing and ordering
// =============================================================================

#[test]
fn test_index_type_ordering() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    // Insert different types
    run(&s, "INSERT INTO data {id: 'null', value: null}");
    run(&s, "INSERT INTO data {id: 'bool', value: true}");
    run(&s, "INSERT INTO data {id: 'int', value: 42}");
    run(&s, "INSERT INTO data {id: 'float', value: 3.14}");
    run(&s, "INSERT INTO data {id: 'string', value: 'hello'}");

    // All documents should be stored
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 5);

    // Equality queries should work for each type
    let result = run(&s, "SELECT * FROM data WHERE value = null");
    assert_eq!(result.as_array().unwrap().len(), 1);

    let result = run(&s, "SELECT * FROM data WHERE value = true");
    assert_eq!(result.as_array().unwrap().len(), 1);

    let result = run(&s, "SELECT * FROM data WHERE value = 42");
    assert_eq!(result.as_array().unwrap().len(), 1);

    let result = run(&s, "SELECT * FROM data WHERE value = 'hello'");
    assert_eq!(result.as_array().unwrap().len(), 1);
}

// =============================================================================
// Range queries with nulls
// =============================================================================

#[test]
fn test_range_query_excludes_null() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    run(&s, "INSERT INTO data {id: '1', value: 10}");
    run(&s, "INSERT INTO data {id: '2', value: 20}");
    run(&s, "INSERT INTO data {id: '3', value: null}");

    // Range query should not include null
    let result = run(&s, "SELECT * FROM data WHERE value > 5");
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "Null should not be included in > 5"
    );

    let result = run(&s, "SELECT * FROM data WHERE value >= 0");
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "Null should not be included in >= 0"
    );
}
