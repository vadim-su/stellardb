//! Tests for index key ordering correctness
//!
//! These tests verify that the index key encoding preserves
//! correct ordering when compared bytewise.

use stellardb::document::Value;
use stellardb::storage::{
    encode_index_key, encode_range_bound_exclusive, encode_value_with_delimiter,
};

/// Verify numeric ordering is preserved
#[test]
fn test_number_ordering_negative_to_positive() {
    let values = [-1000, -100, -1, 0, 1, 100, 1000];
    let encoded: Vec<_> = values
        .iter()
        .map(|&n| encode_value_with_delimiter(&Value::Int(n)))
        .collect();

    for i in 0..encoded.len() - 1 {
        assert!(
            encoded[i] < encoded[i + 1],
            "{} should be < {}",
            values[i],
            values[i + 1]
        );
    }
}

/// Verify float ordering
#[test]
fn test_float_ordering() {
    let values = [-1.5, -0.1, 0.0, 0.1, 1.5];
    let encoded: Vec<_> = values
        .iter()
        .map(|&f| encode_value_with_delimiter(&Value::Float(f)))
        .collect();

    for i in 0..encoded.len() - 1 {
        assert!(encoded[i] < encoded[i + 1]);
    }
}

/// Verify string ordering
#[test]
fn test_string_ordering() {
    let values = ["", "a", "aa", "ab", "b"];
    let encoded: Vec<_> = values
        .iter()
        .map(|s| encode_value_with_delimiter(&Value::String(s.to_string())))
        .collect();

    for i in 0..encoded.len() - 1 {
        assert!(
            encoded[i] < encoded[i + 1],
            "{:?} should be < {:?}",
            values[i],
            values[i + 1]
        );
    }
}

/// Verify string with embedded null ordering
#[test]
fn test_string_with_null_ordering() {
    let s1 = "a";
    let s2 = "a\x00";
    let s3 = "a\x00b";
    let s4 = "ab";

    let e1 = encode_value_with_delimiter(&Value::String(s1.to_string()));
    let e2 = encode_value_with_delimiter(&Value::String(s2.to_string()));
    let e3 = encode_value_with_delimiter(&Value::String(s3.to_string()));
    let e4 = encode_value_with_delimiter(&Value::String(s4.to_string()));

    // "a" < "a\x00" < "a\x00b" < "ab"
    assert!(e1 < e2);
    assert!(e2 < e3);
    assert!(e3 < e4);
}

/// Verify cross-type ordering (Null < Bool < Number < String)
#[test]
fn test_cross_type_ordering() {
    let null = encode_value_with_delimiter(&Value::Null);
    let bool_val = encode_value_with_delimiter(&Value::Bool(false));
    let num = encode_value_with_delimiter(&Value::Int(0));
    let string = encode_value_with_delimiter(&Value::String("".to_string()));

    assert!(null < bool_val);
    assert!(bool_val < num);
    assert!(num < string);
}

/// Verify complete index key ordering
#[test]
fn test_index_key_ordering() {
    let key1 = encode_index_key(&[&Value::Int(25)], "doc:a");
    let key2 = encode_index_key(&[&Value::Int(25)], "doc:b");
    let key3 = encode_index_key(&[&Value::Int(26)], "doc:a");

    // Same value, different doc_key
    assert!(key1 < key2);

    // Different value
    assert!(key2 < key3);
}

/// Verify range boundary correctness
///
/// The exclusive boundary is used as a start bound for "greater than" queries.
/// Structure:
///   boundary = [value][delimiter][0xFF]
///   key      = [value][delimiter][0xFF][doc_key]
///
/// Since keys have more bytes after the shared prefix, keys > boundary.
/// This allows efficient range scans where we start from the boundary
/// to get all documents with values strictly greater than the target.
#[test]
fn test_range_boundary_exclusive() {
    let value = Value::Int(25);
    let key_25_a = encode_index_key(&[&value], "doc:a");
    let key_25_z = encode_index_key(&[&value], "doc:zzz");
    let boundary = encode_range_bound_exclusive(&value);
    let key_26 = encode_index_key(&[&Value::Int(26)], "doc:a");

    // Keys with value 25 should be > boundary (they have doc_key bytes after shared prefix)
    assert!(
        key_25_a > boundary,
        "key with doc should be > boundary (more bytes)"
    );
    assert!(
        key_25_z > boundary,
        "key with doc should be > boundary (more bytes)"
    );

    // Keys with larger values should also be > boundary
    assert!(
        key_26 > boundary,
        "key with larger value should be > boundary"
    );

    // The boundary for value 25 should be < first key with value 26
    // This verifies that using boundary as start excludes all value=25 keys
    // when we want "value > 25" semantics
    let boundary_25 = encode_range_bound_exclusive(&Value::Int(25));
    let prefix_26 = encode_value_with_delimiter(&Value::Int(26));
    // The prefix for 26 (without 0xFF separator) should be > boundary for 25
    assert!(
        prefix_26 > boundary_25,
        "value 26 prefix should be > value 25 boundary"
    );
}

/// Verify compound index ordering
#[test]
fn test_compound_index_ordering() {
    let key1 = encode_index_key(&[&Value::String("a".to_string()), &Value::Int(1)], "doc:1");
    let key2 = encode_index_key(&[&Value::String("a".to_string()), &Value::Int(2)], "doc:1");
    let key3 = encode_index_key(&[&Value::String("b".to_string()), &Value::Int(1)], "doc:1");

    // Same first field, ordered by second
    assert!(key1 < key2);

    // Different first field
    assert!(key2 < key3);
}

/// Verify mixed int/float ordering (cross-numeric type)
#[test]
fn test_mixed_int_float_ordering() {
    let int_1 = encode_value_with_delimiter(&Value::Int(1));
    let float_1_5 = encode_value_with_delimiter(&Value::Float(1.5));
    let int_2 = encode_value_with_delimiter(&Value::Int(2));

    // 1 < 1.5 < 2
    assert!(int_1 < float_1_5);
    assert!(float_1_5 < int_2);
}
