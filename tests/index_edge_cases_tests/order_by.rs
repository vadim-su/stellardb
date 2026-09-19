//! ORDER BY correctness tests - verifying Sort happens before Project.

use super::{run, setup};

#[test]
fn test_order_by_field_not_in_projection() {
    // Tests that ORDER BY works correctly when sorting by a field not in SELECT.
    // This validates that Sort operator runs before Project operator.
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION products");

    run(
        &s,
        "INSERT INTO products {id: 'p1', name: 'Apple', price: 150, category: 'fruit'}",
    );
    run(
        &s,
        "INSERT INTO products {id: 'p2', name: 'Banana', price: 50, category: 'fruit'}",
    );
    run(
        &s,
        "INSERT INTO products {id: 'p3', name: 'Cherry', price: 200, category: 'fruit'}",
    );
    run(
        &s,
        "INSERT INTO products {id: 'p4', name: 'Date', price: 100, category: 'fruit'}",
    );

    // Select only name, but order by price (not in projection)
    let result = run(&s, "SELECT name FROM products ORDER price ASC");
    let names: Vec<&str> = result
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["Banana", "Date", "Apple", "Cherry"],
        "Should be ordered by price ASC even though price not in SELECT"
    );

    // Descending order
    let result = run(&s, "SELECT name FROM products ORDER price DESC");
    let names: Vec<&str> = result
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["Cherry", "Apple", "Date", "Banana"],
        "Should be ordered by price DESC"
    );
}

#[test]
fn test_order_by_with_expression() {
    // Tests ORDER BY with computed expressions.
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION items");

    run(
        &s,
        "INSERT INTO items {id: 'i1', quantity: 10, unit_price: 5}",
    );
    run(
        &s,
        "INSERT INTO items {id: 'i2', quantity: 3, unit_price: 20}",
    );
    run(
        &s,
        "INSERT INTO items {id: 'i3', quantity: 7, unit_price: 8}",
    );

    // Order by computed total (quantity * unit_price)
    let result = run(&s, "SELECT id FROM items ORDER quantity * unit_price ASC");
    let ids: Vec<&str> = result
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    // i1: 10*5=50, i2: 3*20=60, i3: 7*8=56
    assert_eq!(
        ids,
        vec!["items:i1", "items:i3", "items:i2"],
        "Should be ordered by computed total ASC"
    );
}

#[test]
fn test_order_by_multiple_fields() {
    // Tests ORDER BY with multiple fields.
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION employees");

    run(
        &s,
        "INSERT INTO employees {id: 'e1', dept: 'Sales', salary: 50000}",
    );
    run(
        &s,
        "INSERT INTO employees {id: 'e2', dept: 'Engineering', salary: 70000}",
    );
    run(
        &s,
        "INSERT INTO employees {id: 'e3', dept: 'Sales', salary: 60000}",
    );
    run(
        &s,
        "INSERT INTO employees {id: 'e4', dept: 'Engineering', salary: 65000}",
    );

    // Order by dept ASC, then salary DESC
    let result = run(&s, "SELECT id FROM employees ORDER dept ASC, salary DESC");
    let ids: Vec<&str> = result
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![
            "employees:e2",
            "employees:e4",
            "employees:e3",
            "employees:e1"
        ],
        "Should be ordered by dept ASC, then salary DESC"
    );
}

#[test]
fn test_order_by_with_limit() {
    // Tests that ORDER BY is applied before LIMIT.
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION numbers");

    for i in 1..=10 {
        run(
            &s,
            &format!("INSERT INTO numbers {{id: 'n{}', value: {}}}", i, i * 10),
        );
    }

    // Get top 3 by value descending
    let result = run(&s, "SELECT id FROM numbers ORDER value DESC LIMIT 3");
    let ids: Vec<&str> = result
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 3, "Should return exactly 3 results");
    assert_eq!(
        ids,
        vec!["numbers:n10", "numbers:n9", "numbers:n8"],
        "Should return top 3 highest values"
    );
}

#[test]
fn test_order_by_with_null_values() {
    // Tests that NULL values are handled correctly in ORDER BY.
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");

    run(&s, "INSERT INTO data {id: 'd1', score: 30}");
    run(&s, "INSERT INTO data {id: 'd2'}"); // no score field = NULL
    run(&s, "INSERT INTO data {id: 'd3', score: 10}");
    run(&s, "INSERT INTO data {id: 'd4', score: null}");
    run(&s, "INSERT INTO data {id: 'd5', score: 20}");

    // NULLs should sort first (lowest) in ASC order
    let result = run(&s, "SELECT id FROM data ORDER score ASC");
    let ids: Vec<&str> = result
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    // NULL values (d2, d4) should come first
    assert!(
        ids[0] == "data:d2" || ids[0] == "data:d4",
        "NULL should be first"
    );
    assert!(
        ids[1] == "data:d2" || ids[1] == "data:d4",
        "NULL should be first"
    );
    assert_eq!(ids[2], "data:d3", "10 should be third");
    assert_eq!(ids[3], "data:d5", "20 should be fourth");
    assert_eq!(ids[4], "data:d1", "30 should be fifth");
}
