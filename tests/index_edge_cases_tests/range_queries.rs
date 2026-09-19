//! Tests for range query efficiency with delimiter-based index format.
//!
//! Verifies that range queries (>, >=, <, <=, BETWEEN) correctly use IndexScan
//! rather than performing full table scans.

use super::{run, setup};

// =============================================================================
// Greater than (>) queries
// =============================================================================

#[test]
fn test_range_query_gt_uses_index() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION users");
    run(&s, "CREATE INDEX ON users(age)");

    // Insert docs with various ages
    for i in 0..100 {
        run(
            &s,
            &format!("INSERT INTO users {{id: 'u{}', age: {}}}", i, i),
        );
    }

    // Query age > 50 should use index
    let result = run(&s, "SELECT * FROM users WHERE age > 50");
    assert_eq!(result.as_array().unwrap().len(), 49); // 51-99

    // Verify index was used
    let explain = run(&s, "EXPLAIN SELECT * FROM users WHERE age > 50");
    let plan = explain.as_array().unwrap()[0].as_str().unwrap();
    assert!(
        plan.contains("IndexScan"),
        "Expected IndexScan in plan: {}",
        plan
    );
}

// =============================================================================
// Greater than or equal (>=) queries
// =============================================================================

#[test]
fn test_range_query_gte_uses_index() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION items");
    run(&s, "CREATE INDEX ON items(price)");

    for i in 0..50 {
        run(
            &s,
            &format!("INSERT INTO items {{id: 'i{}', price: {}}}", i, i * 10),
        );
    }

    let result = run(&s, "SELECT * FROM items WHERE price >= 200");
    assert_eq!(result.as_array().unwrap().len(), 30); // 200, 210, ..., 490

    let explain = run(&s, "EXPLAIN SELECT * FROM items WHERE price >= 200");
    let plan = explain.as_array().unwrap()[0].as_str().unwrap();
    assert!(plan.contains("IndexScan"), "Expected IndexScan: {}", plan);
}

// =============================================================================
// Less than (<) queries
// =============================================================================

#[test]
fn test_range_query_lt_uses_index() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION products");
    run(&s, "CREATE INDEX ON products(stock)");

    for i in 0..100 {
        run(
            &s,
            &format!("INSERT INTO products {{id: 'p{}', stock: {}}}", i, i),
        );
    }

    let result = run(&s, "SELECT * FROM products WHERE stock < 25");
    assert_eq!(result.as_array().unwrap().len(), 25); // 0-24

    let explain = run(&s, "EXPLAIN SELECT * FROM products WHERE stock < 25");
    let plan = explain.as_array().unwrap()[0].as_str().unwrap();
    assert!(plan.contains("IndexScan"), "Expected IndexScan: {}", plan);
}

// =============================================================================
// Less than or equal (<=) queries
// =============================================================================

#[test]
fn test_range_query_lte_uses_index() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION inventory");
    run(&s, "CREATE INDEX ON inventory(quantity)");

    for i in 0..100 {
        run(
            &s,
            &format!("INSERT INTO inventory {{id: 'inv{}', quantity: {}}}", i, i),
        );
    }

    let result = run(&s, "SELECT * FROM inventory WHERE quantity <= 30");
    assert_eq!(result.as_array().unwrap().len(), 31); // 0-30 inclusive

    let explain = run(&s, "EXPLAIN SELECT * FROM inventory WHERE quantity <= 30");
    let plan = explain.as_array().unwrap()[0].as_str().unwrap();
    assert!(plan.contains("IndexScan"), "Expected IndexScan: {}", plan);
}

// =============================================================================
// BETWEEN / range queries (combined >= and <=)
// =============================================================================

#[test]
fn test_range_query_between_uses_index() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION orders");
    run(&s, "CREATE INDEX ON orders(amount)");

    for i in 0..100 {
        run(
            &s,
            &format!("INSERT INTO orders {{id: 'o{}', amount: {}}}", i, i * 5),
        );
    }

    // BETWEEN 100 AND 200 inclusive
    // Values: 100, 105, 110, ..., 200 = (200-100)/5 + 1 = 21 values
    // i=20 gives 100, i=40 gives 200, so 40-20+1 = 21 values
    let result = run(
        &s,
        "SELECT * FROM orders WHERE amount >= 100 AND amount <= 200",
    );
    assert_eq!(result.as_array().unwrap().len(), 21);

    let explain = run(
        &s,
        "EXPLAIN SELECT * FROM orders WHERE amount >= 100 AND amount <= 200",
    );
    let plan = explain.as_array().unwrap()[0].as_str().unwrap();
    assert!(plan.contains("IndexScan"), "Expected IndexScan: {}", plan);
}

#[test]
fn test_range_query_open_range() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION scores");
    run(&s, "CREATE INDEX ON scores(value)");

    for i in 0..50 {
        run(
            &s,
            &format!("INSERT INTO scores {{id: 's{}', value: {}}}", i, i * 2),
        );
    }

    // Open range: > 20 AND < 60 (exclusive bounds)
    let result = run(&s, "SELECT * FROM scores WHERE value > 20 AND value < 60");
    // Values: 22, 24, 26, ..., 58 (values divisible by 2 between 21-59)
    // That's 22, 24, 26, 28, 30, 32, 34, 36, 38, 40, 42, 44, 46, 48, 50, 52, 54, 56, 58 = 19 values
    assert_eq!(result.as_array().unwrap().len(), 19);

    let explain = run(
        &s,
        "EXPLAIN SELECT * FROM scores WHERE value > 20 AND value < 60",
    );
    let plan = explain.as_array().unwrap()[0].as_str().unwrap();
    assert!(plan.contains("IndexScan"), "Expected IndexScan: {}", plan);
}

// =============================================================================
// Compound index with equality + range
// =============================================================================

#[test]
fn test_compound_index_eq_plus_range() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION events");
    run(&s, "CREATE INDEX ON events(category, timestamp)");

    for i in 0..50 {
        let cat = if i % 2 == 0 { "A" } else { "B" };
        run(
            &s,
            &format!(
                "INSERT INTO events {{id: 'e{}', category: '{}', timestamp: {}}}",
                i,
                cat,
                i * 100
            ),
        );
    }

    // Equality on first field, range on second
    let result = run(
        &s,
        "SELECT * FROM events WHERE category = 'A' AND timestamp > 2000",
    );
    let arr = result.as_array().unwrap();

    // Verify all results have category=A and timestamp > 2000
    for doc in arr {
        assert_eq!(doc["category"].as_str().unwrap(), "A");
        assert!(doc["timestamp"].as_i64().unwrap() > 2000);
    }

    // Category A has timestamps: 0, 200, 400, ..., 4800 (even indices * 100)
    // Those > 2000: 2200, 2400, ..., 4800 = 14 values (indices 22, 24, ..., 48)
    assert_eq!(arr.len(), 14);
}

// =============================================================================
// Range queries with negative numbers
// =============================================================================

#[test]
fn test_range_query_with_negative_values() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION temps");
    run(&s, "CREATE INDEX ON temps(celsius)");

    // Insert temperatures from -50 to 49
    for i in -50..50 {
        run(
            &s,
            &format!("INSERT INTO temps {{id: 't{}', celsius: {}}}", i + 50, i),
        );
    }

    // Query celsius >= -10 AND celsius <= 10
    let result = run(
        &s,
        "SELECT * FROM temps WHERE celsius >= -10 AND celsius <= 10",
    );
    assert_eq!(result.as_array().unwrap().len(), 21); // -10, -9, ..., 10

    let explain = run(
        &s,
        "EXPLAIN SELECT * FROM temps WHERE celsius >= -10 AND celsius <= 10",
    );
    let plan = explain.as_array().unwrap()[0].as_str().unwrap();
    assert!(plan.contains("IndexScan"), "Expected IndexScan: {}", plan);
}

// =============================================================================
// Range queries with floats
// =============================================================================

#[test]
fn test_range_query_with_floats() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION measurements");
    run(&s, "CREATE INDEX ON measurements(reading)");

    for i in 0..100 {
        run(
            &s,
            &format!(
                "INSERT INTO measurements {{id: 'm{}', reading: {}}}",
                i,
                i as f64 * 0.5
            ),
        );
    }

    // Query reading >= 10.0 AND reading < 20.0
    let result = run(
        &s,
        "SELECT * FROM measurements WHERE reading >= 10.0 AND reading < 20.0",
    );
    // Values: 10.0, 10.5, 11.0, ..., 19.5
    // i=20 gives 10.0, i=39 gives 19.5 (i*0.5), so 39-20+1 = 20 values
    // i=40 would give 20.0 which is excluded by < 20.0
    assert_eq!(result.as_array().unwrap().len(), 20);

    let explain = run(
        &s,
        "EXPLAIN SELECT * FROM measurements WHERE reading >= 10.0 AND reading < 20.0",
    );
    let plan = explain.as_array().unwrap()[0].as_str().unwrap();
    assert!(plan.contains("IndexScan"), "Expected IndexScan: {}", plan);
}

// =============================================================================
// Range queries with strings (lexicographic ordering)
// =============================================================================

#[test]
fn test_range_query_with_strings() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION names");
    run(&s, "CREATE INDEX ON names(name)");

    run(&s, "INSERT INTO names {id: '1', name: 'Alice'}");
    run(&s, "INSERT INTO names {id: '2', name: 'Bob'}");
    run(&s, "INSERT INTO names {id: '3', name: 'Charlie'}");
    run(&s, "INSERT INTO names {id: '4', name: 'David'}");
    run(&s, "INSERT INTO names {id: '5', name: 'Eve'}");

    // Query names >= 'Bob' AND names <= 'David' (lexicographic)
    let result = run(
        &s,
        "SELECT * FROM names WHERE name >= 'Bob' AND name <= 'David'",
    );
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 3); // Bob, Charlie, David

    let explain = run(
        &s,
        "EXPLAIN SELECT * FROM names WHERE name >= 'Bob' AND name <= 'David'",
    );
    let plan = explain.as_array().unwrap()[0].as_str().unwrap();
    assert!(plan.contains("IndexScan"), "Expected IndexScan: {}", plan);
}
