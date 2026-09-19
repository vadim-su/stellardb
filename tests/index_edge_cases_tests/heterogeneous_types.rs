//! Tests for heterogeneous data types in flexible collections.
//!
//! Covers scenarios where the same field has different types across documents:
//! - Strings vs numbers with same field name
//! - Objects vs scalars
//! - Arrays vs scalars
//! - Index behavior with mixed types
//! - Unique constraints with type mismatches

use super::{run, run_err, setup};

// =============================================================================
// AGGRESSIVE: Stress tests for heterogeneous data
// =============================================================================

/// Rapid type switching on same document - index must update correctly each time
#[test]
fn test_aggressive_rapid_type_switching() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    run(&s, "INSERT INTO data {id: 'd1', value: 'initial'}");

    // Rapidly switch types 50 times (only scalar types - arrays/objects not updatable inline)
    let types = [
        "100",      // int (index 0)
        "'string'", // string (index 1)
        "3.14",     // float (index 2)
        "true",     // bool (index 3)
        "null",     // null (index 4)
        "99dec",    // decimal (index 5)
        "-999",     // negative int (index 6)
        "'emoji'",  // simple string (index 7)
        "0",        // zero (index 8)
        "false",    // bool false (index 9)
    ];

    for i in 0..50 {
        let val = types[i % types.len()];
        run(&s, &format!("UPDATE data:d1 SET value = {}", val));
    }

    // Final iteration is i=49, 49 % 10 = 9, which is "false"
    // Document should exist and have value = false
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 1, "Document should exist");

    let result = run(&s, "SELECT * FROM data WHERE value = false");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "Final value should be false (49 % 10 = 9)"
    );

    // All other values should NOT be found
    let result = run(&s, "SELECT * FROM data WHERE value = 'string'");
    assert_eq!(
        result.as_array().unwrap().len(),
        0,
        "Old string value should be gone"
    );

    let result = run(&s, "SELECT * FROM data WHERE value = 3.14");
    assert_eq!(
        result.as_array().unwrap().len(),
        0,
        "Old float value should be gone"
    );
}

/// Mass insert with alternating types
#[test]
fn test_aggressive_mass_alternating_types() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(val)");

    // Insert 500 documents with alternating types
    for i in 0..500 {
        let val = match i % 7 {
            0 => format!("{}", i),        // int
            1 => format!("'{}'", i),      // string
            2 => format!("{}.5", i),      // float
            3 => "true".to_string(),      // bool
            4 => "null".to_string(),      // null
            5 => format!("{}dec", i),     // decimal
            _ => format!("{{x: {}}}", i), // object
        };
        run(
            &s,
            &format!("INSERT INTO data {{id: 'd{}', val: {}}}", i, val),
        );
    }

    // Verify all documents exist
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 500);

    // Count by type - ints (0, 7, 14, 21...)
    let result = run(&s, "SELECT * FROM data WHERE val = 0");
    assert_eq!(result.as_array().unwrap().len(), 1);

    // Count bools (all true at positions 3, 10, 17... -> i % 7 == 3)
    // 500/7 = 71.4, so 71 or 72 depending on exact distribution
    let result = run(&s, "SELECT * FROM data WHERE val = true");
    let true_count = result.as_array().unwrap().len();
    assert!(
        true_count == 71 || true_count == 72,
        "~71-72 docs with val=true, got {}",
        true_count
    );

    // Count nulls (i % 7 == 4)
    let result = run(&s, "SELECT * FROM data WHERE val = null");
    let null_count = result.as_array().unwrap().len();
    assert!(
        null_count == 71 || null_count == 72,
        "~71-72 docs with val=null, got {}",
        null_count
    );
}

/// Unique constraint under type-switching stress
#[test]
fn test_aggressive_unique_type_switching() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(code) UNIQUE");

    // Insert initial values of different types
    run(&s, "INSERT INTO data {id: 'd1', code: 100}");
    run(&s, "INSERT INTO data {id: 'd2', code: '100'}"); // string, different from int
    run(&s, "INSERT INTO data {id: 'd3', code: true}");

    // Update d1 to string - should work (100 int is freed)
    run(&s, "UPDATE data:d1 SET code = 'freed'");

    // Now we can insert 100 as int again
    run(&s, "INSERT INTO data {id: 'd4', code: 100}");

    // But 100.0 should conflict with 100 (unified numeric)
    let err = run_err(&s, "INSERT INTO data {id: 'd5', code: 100.0}");
    assert!(
        err.contains("Conflict") || err.contains("Unique"),
        "100.0 should conflict with 100"
    );

    // Verify state
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 4);
}

/// Delete and reinsert with different type (non-unique index)
#[test]
fn test_aggressive_delete_reinsert_different_type() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(val)"); // No UNIQUE - allows duplicate values

    // Cycle: insert type A -> delete -> insert type B with same id
    for i in 0..100 {
        let id = format!("d{}", i % 10); // reuse 10 ids

        // Delete if exists
        run(&s, &format!("DELETE data:{}", id));

        // Insert with type based on iteration (unique values per iteration)
        let val = match i % 5 {
            0 => format!("{}", i * 100),
            1 => format!("'str{}'", i),
            2 => format!("{}.99", i),
            3 => format!("{}", i % 2 == 0), // alternating true/false
            _ => format!("{}dec", i),       // decimals instead of null
        };
        run(
            &s,
            &format!("INSERT INTO data {{id: '{}', val: {}}}", id, val),
        );
    }

    // Should have exactly 10 documents
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 10);
}

/// Nested field index with type changes at intermediate levels
#[test]
fn test_aggressive_nested_field_type_mutation() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(a.b.c)");

    // Insert with proper nested structure
    run(&s, "INSERT INTO data {id: 'd1', a: {b: {c: 'deep'}}}");

    let result = run(&s, "SELECT * FROM data WHERE a.b.c = 'deep'");
    assert_eq!(result.as_array().unwrap().len(), 1);

    // Break the chain at different levels
    run(&s, "UPDATE data:d1 SET a = 'not_object'");
    let result = run(&s, "SELECT * FROM data WHERE a.b.c = 'deep'");
    assert_eq!(
        result.as_array().unwrap().len(),
        0,
        "After a becomes string"
    );

    // Restore and break at b
    run(&s, "UPDATE data:d1 SET a = {b: 'not_object'}");
    let result = run(&s, "SELECT * FROM data WHERE a.b.c = 'deep'");
    assert_eq!(
        result.as_array().unwrap().len(),
        0,
        "After a.b becomes string"
    );

    // Restore fully
    run(&s, "UPDATE data:d1 SET a = {b: {c: 'restored'}}");
    let result = run(&s, "SELECT * FROM data WHERE a.b.c = 'restored'");
    assert_eq!(result.as_array().unwrap().len(), 1, "After full restore");

    // Change leaf type
    run(&s, "UPDATE data:d1 SET a = {b: {c: 12345}}");
    let result = run(&s, "SELECT * FROM data WHERE a.b.c = 12345");
    assert_eq!(result.as_array().unwrap().len(), 1, "After c becomes int");

    let result = run(&s, "SELECT * FROM data WHERE a.b.c = 'restored'");
    assert_eq!(result.as_array().unwrap().len(), 0, "Old string value gone");
}

/// Extreme values of different types in same index
#[test]
fn test_aggressive_extreme_values() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(val)");

    // Insert extreme values
    run(&s, "INSERT INTO data {id: 'd1', val: 9223372036854775807}"); // i64::MAX
    run(&s, "INSERT INTO data {id: 'd2', val: -9223372036854775808}"); // i64::MIN
    run(&s, "INSERT INTO data {id: 'd3', val: 0.0000000000001}"); // tiny float
    run(&s, "INSERT INTO data {id: 'd4', val: 999999999999.999999}"); // large float

    // Very long string
    let long_str = "x".repeat(10000);
    run(
        &s,
        &format!("INSERT INTO data {{id: 'd5', val: '{}'}}", long_str),
    );

    // Empty string
    run(&s, "INSERT INTO data {id: 'd6', val: ''}");

    // Unicode extremes
    run(&s, "INSERT INTO data {id: 'd7', val: '\\u0000'}"); // null char (escaped)
    run(&s, "INSERT INTO data {id: 'd8', val: '🎉🚀💻🔥'}");

    // All should be stored
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 8);

    // Range query should work across types
    let result = run(&s, "SELECT * FROM data WHERE val > 0");
    // Should find: i64::MAX, tiny float, large float (3 numbers)
    // Strings are > numbers in encoding, so also finds them (5 strings)
    assert!(
        result.as_array().unwrap().len() >= 3,
        "At least 3 positive numbers"
    );
}

/// Compound index with all fields changing types (without null)
#[test]
fn test_aggressive_compound_all_fields_change_type() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(a, b, c)");

    run(&s, "INSERT INTO data {id: 'd1', a: 1, b: 2, c: 3}");

    // Verify initial state
    let result = run(&s, "SELECT * FROM data WHERE a = 1 AND b = 2 AND c = 3");
    assert_eq!(result.as_array().unwrap().len(), 1);

    // Change first field to string
    run(&s, "UPDATE data:d1 SET a = 'one'");
    let result = run(&s, "SELECT * FROM data WHERE a = 1");
    assert_eq!(
        result.as_array().unwrap().len(),
        0,
        "Old a=1 should be gone"
    );
    let result = run(&s, "SELECT * FROM data WHERE a = 'one'");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "New a='one' should be found"
    );

    // Change second field to bool
    run(&s, "UPDATE data:d1 SET b = true");
    let result = run(&s, "SELECT * FROM data WHERE a = 'one' AND b = true");
    assert_eq!(result.as_array().unwrap().len(), 1, "a='one' AND b=true");

    // Change third field to a different number
    run(&s, "UPDATE data:d1 SET c = 999");
    let result = run(
        &s,
        "SELECT * FROM data WHERE a = 'one' AND b = true AND c = 999",
    );
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "all three fields updated"
    );

    // Revert all at once
    run(&s, "UPDATE data:d1 SET a = 100, b = 200, c = 300");
    let result = run(
        &s,
        "SELECT * FROM data WHERE a = 100 AND b = 200 AND c = 300",
    );
    assert_eq!(result.as_array().unwrap().len(), 1, "all reverted to ints");

    // Old values should be gone
    let result = run(&s, "SELECT * FROM data WHERE a = 'one'");
    assert_eq!(result.as_array().unwrap().len(), 0, "old string value gone");
}

// =============================================================================
// Test: Compound index with null after UPDATE (previously a bug, now fixed)
// =============================================================================

/// Test: Compound index correctly handles field updated to null.
/// Previously broken: After UPDATE set c = null, the compound index query
/// with c = null returned 0 results. Fixed with proper null value handling.
#[test]
fn test_bug_compound_index_null_after_update() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(a, b, c)");

    run(&s, "INSERT INTO data {id: 'd1', a: 'one', b: true, c: 3}");

    // Verify document exists and is indexed
    let result = run(
        &s,
        "SELECT * FROM data WHERE a = 'one' AND b = true AND c = 3",
    );
    assert_eq!(result.as_array().unwrap().len(), 1, "Initial state OK");

    // Update c to null
    run(&s, "UPDATE data:d1 SET c = null");

    // Document should still exist
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 1, "Document exists");

    // Fixed: compound index now properly handles null values after UPDATE
    let result = run(
        &s,
        "SELECT * FROM data WHERE a = 'one' AND b = true AND c = null",
    );
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "compound index with c=null after UPDATE should find the document"
    );
}

/// Test: Compound index properly handles null in non-first field on INSERT.
/// Previously broken, now fixed with proper null value handling.
#[test]
fn test_bug_compound_index_null_on_insert() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(a, b, c)");

    // Insert with c = null from the start
    run(
        &s,
        "INSERT INTO data {id: 'd1', a: 'one', b: true, c: null}",
    );

    // Fixed: compound index now finds doc with c=null on INSERT
    let result = run(
        &s,
        "SELECT * FROM data WHERE a = 'one' AND b = true AND c = null",
    );
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "Compound index should find doc with c=null on INSERT"
    );
}

/// Single-field index works when updated to null
#[test]
fn test_single_index_null_after_update() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(val)");

    run(&s, "INSERT INTO data {id: 'd1', val: 'initial'}");

    // Update to null
    run(&s, "UPDATE data:d1 SET val = null");

    // Query should find it
    let result = run(&s, "SELECT * FROM data WHERE val = null");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "Single index with null after UPDATE should work"
    );

    // Old value should be gone
    let result = run(&s, "SELECT * FROM data WHERE val = 'initial'");
    assert_eq!(
        result.as_array().unwrap().len(),
        0,
        "Old value should be gone"
    );
}

/// Compound index partial query works after field is updated to null
#[test]
fn test_compound_index_partial_query_after_null() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(a, b, c)");

    run(&s, "INSERT INTO data {id: 'd1', a: 'one', b: true, c: 3}");
    run(&s, "UPDATE data:d1 SET c = null");

    // Try partial query - just first two fields
    let result = run(&s, "SELECT * FROM data WHERE a = 'one' AND b = true");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "Partial compound query (a, b) should still work"
    );

    // Try just first field
    let result = run(&s, "SELECT * FROM data WHERE a = 'one'");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "Partial compound query (a) should still work"
    );
}

/// Test: Compound index properly handles null in middle field.
/// Previously broken, now fixed with proper null value handling.
#[test]
fn test_bug_compound_index_middle_field_null() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(a, b, c)");

    run(&s, "INSERT INTO data {id: 'd1', a: 'one', b: true, c: 3}");

    // Update middle field to null
    run(&s, "UPDATE data:d1 SET b = null");

    // Query with b = null
    let result = run(
        &s,
        "SELECT * FROM data WHERE a = 'one' AND b = null AND c = 3",
    );
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "Compound index with middle field null should work"
    );
}

/// Compound index works when first field is updated to null
#[test]
fn test_compound_index_first_field_null() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(a, b, c)");

    run(&s, "INSERT INTO data {id: 'd1', a: 'one', b: true, c: 3}");

    // Update first field to null
    run(&s, "UPDATE data:d1 SET a = null");

    // Query with a = null
    let result = run(
        &s,
        "SELECT * FROM data WHERE a = null AND b = true AND c = 3",
    );
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "Compound index with first field null should work"
    );
}

// =============================================================================
// Verified working: UPDATE with complex types
// =============================================================================

/// UNIQUE constraint works with null values after delete/reinsert cycle
#[test]
fn test_unique_with_null_delete_reinsert() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(val) UNIQUE");

    // Insert null
    run(&s, "INSERT INTO data {id: 'd1', val: null}");

    // Delete
    run(&s, "DELETE data:d1");

    // Reinsert null - should work (old null was deleted)
    run(&s, "INSERT INTO data {id: 'd2', val: null}");

    let result = run(&s, "SELECT * FROM data");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "Should have 1 doc with null"
    );
}

/// UPDATE can change field to array value
#[test]
fn test_update_field_to_array() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(val)");

    run(&s, "INSERT INTO data {id: 'd1', val: 'initial'}");

    // Update to array
    run(&s, "UPDATE data:d1 SET val = [1, 2, 3]");

    // Check the value changed
    let result = run(&s, "SELECT * FROM data");
    let doc = &result.as_array().unwrap()[0];
    assert!(doc["val"].is_array(), "val should be an array after UPDATE");
}

/// UPDATE can change field to object value
#[test]
fn test_update_field_to_object() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(val)");

    run(&s, "INSERT INTO data {id: 'd1', val: 'initial'}");

    // Update to object
    run(&s, "UPDATE data:d1 SET val = {nested: 'value'}");

    // Check the value changed
    let result = run(&s, "SELECT * FROM data");
    let doc = &result.as_array().unwrap()[0];
    assert!(
        doc["val"].is_object(),
        "val should be an object after UPDATE"
    );
}

/// String that looks like JSON shouldn't be parsed
#[test]
fn test_aggressive_json_like_strings() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(val)");

    // Strings that look like JSON but should remain strings
    run(
        &s,
        r#"INSERT INTO data {id: 'd1', val: '{"key": "value"}'}"#,
    );
    run(&s, r#"INSERT INTO data {id: 'd2', val: '[1, 2, 3]'}"#);
    run(&s, r#"INSERT INTO data {id: 'd3', val: 'null'}"#); // string "null"
    run(&s, r#"INSERT INTO data {id: 'd4', val: 'true'}"#); // string "true"
    run(&s, r#"INSERT INTO data {id: 'd5', val: '123'}"#); // string "123"

    // Also insert actual types
    run(&s, "INSERT INTO data {id: 'd6', val: null}"); // actual null
    run(&s, "INSERT INTO data {id: 'd7', val: true}"); // actual bool
    run(&s, "INSERT INTO data {id: 'd8', val: 123}"); // actual int

    // String queries should find strings
    let result = run(&s, r#"SELECT * FROM data WHERE val = 'null'"#);
    assert_eq!(result.as_array().unwrap().len(), 1, "String 'null'");

    let result = run(&s, r#"SELECT * FROM data WHERE val = 'true'"#);
    assert_eq!(result.as_array().unwrap().len(), 1, "String 'true'");

    let result = run(&s, r#"SELECT * FROM data WHERE val = '123'"#);
    assert_eq!(result.as_array().unwrap().len(), 1, "String '123'");

    // Actual type queries should find actual types
    let result = run(&s, "SELECT * FROM data WHERE val = null");
    assert_eq!(result.as_array().unwrap().len(), 1, "Actual null");

    let result = run(&s, "SELECT * FROM data WHERE val = true");
    assert_eq!(result.as_array().unwrap().len(), 1, "Actual bool");

    let result = run(&s, "SELECT * FROM data WHERE val = 123");
    assert_eq!(result.as_array().unwrap().len(), 1, "Actual int");
}

/// Concurrent-like rapid operations with mixed types
#[test]
fn test_aggressive_concurrent_simulation() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(val)");

    // Simulate concurrent workload: rapid insert/update/delete
    for round in 0..10 {
        // Insert batch
        for i in 0..20 {
            let id = round * 20 + i;
            let val = match i % 4 {
                0 => format!("{}", id),
                1 => format!("'s{}'", id),
                2 => format!("{}.5", id),
                _ => "null".to_string(),
            };
            run(
                &s,
                &format!("INSERT INTO data {{id: 'd{}', val: {}}}", id, val),
            );
        }

        // Update some from previous rounds (change types)
        if round > 0 {
            for i in 0..5 {
                let id = (round - 1) * 20 + i;
                run(
                    &s,
                    &format!("UPDATE data:d{} SET val = 'updated{}'", id, id),
                );
            }
        }

        // Delete some from 2 rounds ago
        if round > 1 {
            for i in 10..15 {
                let id = (round - 2) * 20 + i;
                run(&s, &format!("DELETE data:d{}", id));
            }
        }
    }

    // Verify index integrity - count should be:
    // 10 rounds * 20 inserts = 200
    // minus 8 rounds * 5 deletes = 40 deleted
    // = 160 documents
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(
        result.as_array().unwrap().len(),
        160,
        "160 docs after simulation"
    );

    // Verify we can query different types
    let result = run(&s, "SELECT * FROM data WHERE val = null");
    assert!(!result.as_array().unwrap().is_empty(), "Should find nulls");
}

/// Same field, all possible type transitions
#[test]
fn test_aggressive_all_type_transitions() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(val)");

    run(&s, "INSERT INTO data {id: 'd1', val: 'start'}");

    // All type transitions: string -> each type -> string
    let transitions = [
        ("42", "int"),
        ("3.14", "float"),
        ("true", "bool true"),
        ("false", "bool false"),
        ("null", "null"),
        ("99dec", "decimal"),
        ("[]", "array"),
        ("{}", "object"),
        ("{a: 1}", "object with field"),
        ("[1, 2, 3]", "array with elements"),
        ("'back'", "string"),
    ];

    for (val, desc) in transitions {
        run(&s, &format!("UPDATE data:d1 SET val = {}", val));

        // Verify document still exists
        let result = run(&s, "SELECT * FROM data");
        assert_eq!(
            result.as_array().unwrap().len(),
            1,
            "Doc exists after {}",
            desc
        );
    }

    // Final state
    let result = run(&s, "SELECT * FROM data WHERE val = 'back'");
    assert_eq!(result.as_array().unwrap().len(), 1, "Final value is 'back'");
}

/// Multiple indexes on same collection with heterogeneous data
#[test]
fn test_aggressive_multiple_indexes_heterogeneous() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(a)");
    run(&s, "CREATE INDEX ON data(b)");
    run(&s, "CREATE INDEX ON data(c)");

    // Insert with different types in each field
    for i in 0..100 {
        let a = match i % 3 {
            0 => format!("{}", i),
            1 => format!("'a{}'", i),
            _ => "null".to_string(),
        };
        let b = match i % 4 {
            0 => "true".to_string(),
            1 => "false".to_string(),
            2 => format!("{}.5", i),
            _ => format!("'b{}'", i),
        };
        let c = match i % 2 {
            0 => format!("{}dec", i),
            _ => format!("{{x: {}}}", i),
        };

        run(
            &s,
            &format!(
                "INSERT INTO data {{id: 'd{}', a: {}, b: {}, c: {}}}",
                i, a, b, c
            ),
        );
    }

    // Query each index
    // a=null when i % 3 == 2: positions 2,5,8,...,98 -> 33 values
    let result = run(&s, "SELECT * FROM data WHERE a = null");
    assert_eq!(
        result.as_array().unwrap().len(),
        33,
        "33 docs with a=null (i%3==2)"
    );

    // b=true when i % 4 == 0: positions 0,4,8,...,96 -> 25 values
    let result = run(&s, "SELECT * FROM data WHERE b = true");
    assert_eq!(
        result.as_array().unwrap().len(),
        25,
        "25 docs with b=true (i%4==0)"
    );

    // Verify total
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 100);
}

/// Test index rebuild with heterogeneous data
#[test]
fn test_aggressive_reindex_heterogeneous() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");

    // Insert heterogeneous data WITHOUT index
    for i in 0..50 {
        let val = match i % 5 {
            0 => format!("{}", i),
            1 => format!("'s{}'", i),
            2 => format!("{}.5", i),
            3 => "true".to_string(),
            _ => "null".to_string(),
        };
        run(
            &s,
            &format!("INSERT INTO data {{id: 'd{}', val: {}}}", i, val),
        );
    }

    // Now create index - must handle all existing types
    run(&s, "CREATE INDEX ON data(val)");

    // Verify queries work
    let result = run(&s, "SELECT * FROM data WHERE val = true");
    assert_eq!(
        result.as_array().unwrap().len(),
        10,
        "10 docs with val=true"
    );

    let result = run(&s, "SELECT * FROM data WHERE val = null");
    assert_eq!(
        result.as_array().unwrap().len(),
        10,
        "10 docs with val=null"
    );

    // Range query
    let result = run(&s, "SELECT * FROM data WHERE val > 20");
    // Numbers > 20: 25, 30, 35, 40, 45 (5 ints) + all floats > 20 + all strings + all bools (in encoding)
    assert!(
        result.as_array().unwrap().len() > 5,
        "More than 5 results for val > 20"
    );
}

// =============================================================================
// Basic heterogeneous data - same field, different types
// =============================================================================

#[test]
fn test_heterogeneous_string_vs_int() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    // Insert same field with different types
    run(&s, "INSERT INTO data {id: 'd1', value: 'hello'}");
    run(&s, "INSERT INTO data {id: 'd2', value: 42}");
    run(&s, "INSERT INTO data {id: 'd3', value: 'world'}");
    run(&s, "INSERT INTO data {id: 'd4', value: 100}");

    // All documents should be stored
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 4);

    // Query for string values
    let result = run(&s, "SELECT * FROM data WHERE value = 'hello'");
    assert_eq!(result.as_array().unwrap().len(), 1);

    // Query for int values
    let result = run(&s, "SELECT * FROM data WHERE value = 42");
    assert_eq!(result.as_array().unwrap().len(), 1);
}

#[test]
fn test_heterogeneous_string_vs_float() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(value)");

    run(&s, "INSERT INTO data {id: 'd1', value: 'price'}");
    run(&s, "INSERT INTO data {id: 'd2', value: 19.99}");
    run(&s, "INSERT INTO data {id: 'd3', value: '19.99'}"); // String that looks like number

    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 3);

    // Numeric query should only find numeric value
    let result = run(&s, "SELECT * FROM data WHERE value = 19.99");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "Should find only numeric 19.99, not string '19.99'"
    );

    // String query should only find string value
    let result = run(&s, "SELECT * FROM data WHERE value = '19.99'");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "Should find only string '19.99', not numeric 19.99"
    );
}

#[test]
fn test_heterogeneous_bool_vs_string() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(active)");

    run(&s, "INSERT INTO data {id: 'd1', active: true}");
    run(&s, "INSERT INTO data {id: 'd2', active: 'true'}"); // String 'true'
    run(&s, "INSERT INTO data {id: 'd3', active: false}");
    run(&s, "INSERT INTO data {id: 'd4', active: 'yes'}");

    // Boolean query
    let result = run(&s, "SELECT * FROM data WHERE active = true");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "Should find only boolean true"
    );

    // String query
    let result = run(&s, "SELECT * FROM data WHERE active = 'true'");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "Should find only string 'true'"
    );
}

// =============================================================================
// Objects and arrays vs scalars
// =============================================================================

#[test]
fn test_heterogeneous_object_vs_scalar() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(info)");

    // Some documents have object, some have scalar
    run(
        &s,
        "INSERT INTO data {id: 'd1', info: {name: 'Alice', age: 30}}",
    );
    run(&s, "INSERT INTO data {id: 'd2', info: 'simple string'}");
    run(&s, "INSERT INTO data {id: 'd3', info: 42}");
    run(&s, "INSERT INTO data {id: 'd4', info: {name: 'Bob'}}");

    // All documents should be stored
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 4);

    // Scalar query should find scalar values
    let result = run(&s, "SELECT * FROM data WHERE info = 'simple string'");
    assert_eq!(result.as_array().unwrap().len(), 1);

    let result = run(&s, "SELECT * FROM data WHERE info = 42");
    assert_eq!(result.as_array().unwrap().len(), 1);
}

#[test]
fn test_heterogeneous_array_vs_scalar() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(tags)");

    // Some documents have array, some have scalar
    run(&s, "INSERT INTO data {id: 'd1', tags: ['a', 'b', 'c']}");
    run(&s, "INSERT INTO data {id: 'd2', tags: 'single-tag'}");
    run(&s, "INSERT INTO data {id: 'd3', tags: 123}");
    run(&s, "INSERT INTO data {id: 'd4', tags: [1, 2, 3]}");

    // All documents should be stored
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 4);

    // Scalar query should find scalar values only
    let result = run(&s, "SELECT * FROM data WHERE tags = 'single-tag'");
    assert_eq!(result.as_array().unwrap().len(), 1);
}

// =============================================================================
// Range queries with heterogeneous types
// =============================================================================

/// Range queries on heterogeneous data follow byte ordering.
/// Strings (TAG_STRING=0x04) are encoded after numbers (TAG_NUMBER=0x02),
/// so `score > 20` finds both numeric values > 20 AND all strings.
/// This is expected behavior for index-based range scans.
#[test]
fn test_heterogeneous_range_query_numbers_vs_strings() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(score)");

    // Mix of numbers and strings
    run(&s, "INSERT INTO data {id: 'd1', score: 10}");
    run(&s, "INSERT INTO data {id: 'd2', score: 50}");
    run(&s, "INSERT INTO data {id: 'd3', score: 'high'}");
    run(&s, "INSERT INTO data {id: 'd4', score: 'low'}");
    run(&s, "INSERT INTO data {id: 'd5', score: 100}");

    // Range query `score > 20` finds:
    // - numeric values > 20: 50, 100
    // - all strings (they have higher type tag): 'high', 'low'
    let result = run(&s, "SELECT * FROM data WHERE score > 20");
    assert_eq!(
        result.as_array().unwrap().len(),
        4,
        "Range query finds 50, 100 (numbers > 20) + 'high', 'low' (strings have higher tag)"
    );

    // score < 60 finds only numbers < 60 (strings are > all numbers in encoding)
    let result = run(&s, "SELECT * FROM data WHERE score < 60");
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "score < 60 should find only 10 and 50 (numbers, strings excluded)"
    );
}

#[test]
fn test_heterogeneous_range_query_strings() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(name)");

    // Mix of strings and numbers
    run(&s, "INSERT INTO data {id: 'd1', name: 'alice'}");
    run(&s, "INSERT INTO data {id: 'd2', name: 'bob'}");
    run(&s, "INSERT INTO data {id: 'd3', name: 42}");
    run(&s, "INSERT INTO data {id: 'd4', name: 'charlie'}");

    // String range query
    let result = run(&s, "SELECT * FROM data WHERE name > 'b'");
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "String range should find bob and charlie"
    );
}

// =============================================================================
// Unique constraints with heterogeneous types
// =============================================================================

#[test]
fn test_unique_constraint_different_types_same_literal() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(code) UNIQUE");

    // String "100" and Int 100 should be treated as different values
    run(&s, "INSERT INTO data {id: 'd1', code: '100'}");
    run(&s, "INSERT INTO data {id: 'd2', code: 100}");

    // Both should exist - they are different types
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "String '100' and Int 100 should be distinct unique values"
    );
}

#[test]
fn test_unique_constraint_bool_vs_string() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(flag) UNIQUE");

    // Boolean true and String "true" should be different
    run(&s, "INSERT INTO data {id: 'd1', flag: true}");
    run(&s, "INSERT INTO data {id: 'd2', flag: 'true'}");

    let result = run(&s, "SELECT * FROM data");
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "Boolean true and String 'true' should be distinct"
    );
}

#[test]
fn test_unique_constraint_null_vs_missing() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(optional) UNIQUE");

    // Explicit null vs missing field
    run(&s, "INSERT INTO data {id: 'd1', optional: null}");
    run(&s, "INSERT INTO data {id: 'd2'}"); // Field is missing

    // Both should exist - null is indexed, missing is not
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 2);

    // Can insert another document with missing field
    run(&s, "INSERT INTO data {id: 'd3'}");
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(
        result.as_array().unwrap().len(),
        3,
        "Multiple docs with missing field should be allowed"
    );

    // But cannot insert another null
    let err = run_err(&s, "INSERT INTO data {id: 'd4', optional: null}");
    assert!(
        err.contains("Conflict") || err.contains("Unique"),
        "Second null should violate unique constraint: {}",
        err
    );
}

// =============================================================================
// Compound indexes with heterogeneous types
// =============================================================================

#[test]
fn test_compound_index_heterogeneous_types() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(category, value)");

    // Category is always string, but value varies
    run(&s, "INSERT INTO data {id: 'd1', category: 'A', value: 100}");
    run(
        &s,
        "INSERT INTO data {id: 'd2', category: 'A', value: 'hundred'}",
    );
    run(&s, "INSERT INTO data {id: 'd3', category: 'B', value: 200}");
    run(
        &s,
        "INSERT INTO data {id: 'd4', category: 'B', value: true}",
    );

    // Query by category should find all regardless of value type
    let result = run(&s, "SELECT * FROM data WHERE category = 'A'");
    assert_eq!(result.as_array().unwrap().len(), 2);

    // Query by both should work for specific types
    let result = run(
        &s,
        "SELECT * FROM data WHERE category = 'A' AND value = 100",
    );
    assert_eq!(result.as_array().unwrap().len(), 1);

    let result = run(
        &s,
        "SELECT * FROM data WHERE category = 'A' AND value = 'hundred'",
    );
    assert_eq!(result.as_array().unwrap().len(), 1);
}

// =============================================================================
// Type ordering in indexes
// =============================================================================

#[test]
fn test_type_ordering_in_sort() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(val)");

    // Insert various types
    run(&s, "INSERT INTO data {id: 'd1', val: null}");
    run(&s, "INSERT INTO data {id: 'd2', val: false}");
    run(&s, "INSERT INTO data {id: 'd3', val: true}");
    run(&s, "INSERT INTO data {id: 'd4', val: -10}");
    run(&s, "INSERT INTO data {id: 'd5', val: 0}");
    run(&s, "INSERT INTO data {id: 'd6', val: 10}");
    run(&s, "INSERT INTO data {id: 'd7', val: 'aaa'}");
    run(&s, "INSERT INTO data {id: 'd8', val: 'zzz'}");

    // ORDER should respect type ordering: null < bool < number < string
    let result = run(&s, "SELECT id FROM data ORDER val ASC");
    let ids: Vec<&str> = result
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["id"].as_str().unwrap())
        .collect();

    // Expected order: null, false, true, -10, 0, 10, aaa, zzz
    assert_eq!(ids[0], "data:d1", "null should be first");
    assert_eq!(ids[1], "data:d2", "false should be second");
    assert_eq!(ids[2], "data:d3", "true should be third");
    // Numbers next (already unified)
    assert!(
        ids[3..6].contains(&"data:d4")
            && ids[3..6].contains(&"data:d5")
            && ids[3..6].contains(&"data:d6"),
        "numbers should be in middle"
    );
    // Strings last
    assert!(
        ids[6] == "data:d7" || ids[6] == "data:d8",
        "strings should be last"
    );
    assert!(
        ids[7] == "data:d7" || ids[7] == "data:d8",
        "strings should be last"
    );
}

// =============================================================================
// Edge cases
// =============================================================================

#[test]
fn test_heterogeneous_numeric_string_that_looks_like_number() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(amount)");

    // String representations of numbers
    run(&s, "INSERT INTO data {id: 'd1', amount: '42'}");
    run(&s, "INSERT INTO data {id: 'd2', amount: 42}");
    run(&s, "INSERT INTO data {id: 'd3', amount: '42.0'}");
    run(&s, "INSERT INTO data {id: 'd4', amount: 42.0}");

    // Each should be retrievable by its own type
    let result = run(&s, "SELECT * FROM data WHERE amount = '42'");
    assert_eq!(result.as_array().unwrap().len(), 1, "String '42'");

    let result = run(&s, "SELECT * FROM data WHERE amount = 42");
    // 42 (int) and 42.0 (float) should be equal due to unified numeric encoding
    assert_eq!(
        result.as_array().unwrap().len(),
        2,
        "Int 42 and Float 42.0 should match"
    );
}

#[test]
fn test_heterogeneous_empty_values() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(content)");

    // Various "empty" values
    run(&s, "INSERT INTO data {id: 'd1', content: ''}");
    run(&s, "INSERT INTO data {id: 'd2', content: null}");
    run(&s, "INSERT INTO data {id: 'd3', content: 0}");
    run(&s, "INSERT INTO data {id: 'd4', content: false}");
    run(&s, "INSERT INTO data {id: 'd5', content: []}");
    run(&s, "INSERT INTO data {id: 'd6', content: {}}");

    // All should be stored
    let result = run(&s, "SELECT * FROM data");
    assert_eq!(result.as_array().unwrap().len(), 6);

    // Each empty value is distinct
    let result = run(&s, "SELECT * FROM data WHERE content = ''");
    assert_eq!(result.as_array().unwrap().len(), 1, "Empty string");

    let result = run(&s, "SELECT * FROM data WHERE content = null");
    assert_eq!(result.as_array().unwrap().len(), 1, "Null");

    let result = run(&s, "SELECT * FROM data WHERE content = 0");
    assert_eq!(result.as_array().unwrap().len(), 1, "Zero");

    let result = run(&s, "SELECT * FROM data WHERE content = false");
    assert_eq!(result.as_array().unwrap().len(), 1, "False");
}

#[test]
fn test_heterogeneous_update_changes_type() {
    let (_tmp, s) = setup();
    run(&s, "DEFINE COLLECTION data");
    run(&s, "CREATE INDEX ON data(status)");

    // Insert with one type
    run(&s, "INSERT INTO data {id: 'd1', status: 'active'}");

    let result = run(&s, "SELECT * FROM data WHERE status = 'active'");
    assert_eq!(result.as_array().unwrap().len(), 1);

    // Update to different type (use full id format collection:key)
    run(&s, "UPDATE data:d1 SET status = 1");

    // Old query should not find it
    let result = run(&s, "SELECT * FROM data WHERE status = 'active'");
    assert_eq!(
        result.as_array().unwrap().len(),
        0,
        "After update, string query should find nothing"
    );

    // New query should find it
    let result = run(&s, "SELECT * FROM data WHERE status = 1");
    assert_eq!(
        result.as_array().unwrap().len(),
        1,
        "After update, int query should find it"
    );
}
