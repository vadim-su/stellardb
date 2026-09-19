//! Key encoding functions for storage layer
//!
//! Document keys: просто `{key}` (collection в имени keyspace)
//!
//! Edge keys (keyspace per label):
//!   OUT:  0x01 | from | 0x00 | to [| 0x00 | edge_id]  → EdgeData
//!   IN:   0x02 | to | 0x00 | from [| 0x00 | edge_id]  → edge_id (ref)
//!   ID:   0x03 | edge_id                              → from | 0x00 | to
//!   IDX:  0x04 | field | 0x00 | value | 0x00 | edge_id → ()
//!
//! Secondary index keys (delimiter based):
//!   `[value₁][delim]...[0xFF][doc_key]`

use crate::document::Value;
use decimal_bytes::Decimal as DecimalBytes;
use std::str::FromStr;

// =============================================================================
// Edge Key Prefixes (v2 architecture)
// =============================================================================

/// Prefix for outgoing edge index: from → to
pub const EDGE_PREFIX_OUT: u8 = 0x01;

/// Prefix for incoming edge index: to ← from
pub const EDGE_PREFIX_IN: u8 = 0x02;

/// Prefix for edge ID lookup: id → (from, to)
pub const EDGE_PREFIX_ID: u8 = 0x03;

/// Prefix for field index: field/value → edge_id
pub const EDGE_PREFIX_IDX: u8 = 0x04;

// =============================================================================
// Edge Key Encoding (v2 - keyspace per label)
// =============================================================================

/// Encode OUT key for Single cardinality: 0x01 | from | 0x00 | to
pub fn encode_edge_out_key_single(from: &str, to: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + from.len() + 1 + to.len());
    buf.push(EDGE_PREFIX_OUT);
    buf.extend(from.as_bytes());
    buf.push(0x00);
    buf.extend(to.as_bytes());
    buf
}

/// Encode OUT key for Multiple cardinality: 0x01 | from | 0x00 | to | 0x00 | edge_id
pub fn encode_edge_out_key_multi(from: &str, to: &str, edge_id: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + from.len() + 1 + to.len() + 1 + edge_id.len());
    buf.push(EDGE_PREFIX_OUT);
    buf.extend(from.as_bytes());
    buf.push(0x00);
    buf.extend(to.as_bytes());
    buf.push(0x00);
    buf.extend(edge_id.as_bytes());
    buf
}

/// Encode IN key for Single cardinality: 0x02 | to | 0x00 | from
pub fn encode_edge_in_key_single(to: &str, from: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + to.len() + 1 + from.len());
    buf.push(EDGE_PREFIX_IN);
    buf.extend(to.as_bytes());
    buf.push(0x00);
    buf.extend(from.as_bytes());
    buf
}

/// Encode IN key for Multiple cardinality: 0x02 | to | 0x00 | from | 0x00 | edge_id
pub fn encode_edge_in_key_multi(to: &str, from: &str, edge_id: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + to.len() + 1 + from.len() + 1 + edge_id.len());
    buf.push(EDGE_PREFIX_IN);
    buf.extend(to.as_bytes());
    buf.push(0x00);
    buf.extend(from.as_bytes());
    buf.push(0x00);
    buf.extend(edge_id.as_bytes());
    buf
}

/// Encode ID lookup key: 0x03 | edge_id
pub fn encode_edge_id_key(edge_id: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + edge_id.len());
    buf.push(EDGE_PREFIX_ID);
    buf.extend(edge_id.as_bytes());
    buf
}

/// Encode ID lookup value: from | 0x00 | to
pub fn encode_edge_id_value(from: &str, to: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(from.len() + 1 + to.len());
    buf.extend(from.as_bytes());
    buf.push(0x00);
    buf.extend(to.as_bytes());
    buf
}

/// Decode ID lookup value into (from, to)
pub fn decode_edge_id_value(bytes: &[u8]) -> Option<(String, String)> {
    let sep = bytes.iter().position(|&b| b == 0x00)?;
    let from = String::from_utf8_lossy(&bytes[..sep]).to_string();
    let to = String::from_utf8_lossy(&bytes[sep + 1..]).to_string();
    Some((from, to))
}

/// Encode OUT prefix for scanning all outgoing edges from a node: 0x01 | from | 0x00
pub fn encode_edge_out_prefix_v2(from: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + from.len() + 1);
    buf.push(EDGE_PREFIX_OUT);
    buf.extend(from.as_bytes());
    buf.push(0x00);
    buf
}

/// Encode OUT prefix end: 0x01 | from | 0x01
pub fn encode_edge_out_prefix_end_v2(from: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + from.len() + 1);
    buf.push(EDGE_PREFIX_OUT);
    buf.extend(from.as_bytes());
    buf.push(0x01);
    buf
}

/// Encode OUT prefix for a specific target: 0x01 | from | 0x00 | to | 0x00
pub fn encode_edge_out_target_prefix(from: &str, to: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + from.len() + 1 + to.len() + 1);
    buf.push(EDGE_PREFIX_OUT);
    buf.extend(from.as_bytes());
    buf.push(0x00);
    buf.extend(to.as_bytes());
    buf.push(0x00);
    buf
}

/// Encode OUT prefix end for a specific target: 0x01 | from | 0x00 | to | 0x01
pub fn encode_edge_out_target_prefix_end(from: &str, to: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + from.len() + 1 + to.len() + 1);
    buf.push(EDGE_PREFIX_OUT);
    buf.extend(from.as_bytes());
    buf.push(0x00);
    buf.extend(to.as_bytes());
    buf.push(0x01);
    buf
}

/// Encode IN prefix for scanning all incoming edges to a node: 0x02 | to | 0x00
pub fn encode_edge_in_prefix_v2(to: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + to.len() + 1);
    buf.push(EDGE_PREFIX_IN);
    buf.extend(to.as_bytes());
    buf.push(0x00);
    buf
}

/// Encode IN prefix end: 0x02 | to | 0x01
pub fn encode_edge_in_prefix_end_v2(to: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + to.len() + 1);
    buf.push(EDGE_PREFIX_IN);
    buf.extend(to.as_bytes());
    buf.push(0x01);
    buf
}

/// Encode field index key: 0x04 | field | 0x00 | encoded_value | 0x00 | edge_id
pub fn encode_edge_field_index_key(field: &str, value: &Value, edge_id: &str) -> Vec<u8> {
    let encoded_value = encode_value_ordered(value);
    let mut buf = Vec::with_capacity(1 + field.len() + 1 + encoded_value.len() + 1 + edge_id.len());
    buf.push(EDGE_PREFIX_IDX);
    buf.extend(field.as_bytes());
    buf.push(0x00);
    buf.extend(encoded_value);
    buf.push(0x00);
    buf.extend(edge_id.as_bytes());
    buf
}

/// Encode field index prefix for scanning: 0x04 | field | 0x00
pub fn encode_edge_field_index_prefix(field: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + field.len() + 1);
    buf.push(EDGE_PREFIX_IDX);
    buf.extend(field.as_bytes());
    buf.push(0x00);
    buf
}

/// Encode field index prefix end: 0x04 | field | 0x01
pub fn encode_edge_field_index_prefix_end(field: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + field.len() + 1);
    buf.push(EDGE_PREFIX_IDX);
    buf.extend(field.as_bytes());
    buf.push(0x01);
    buf
}

/// Encode field index value prefix: 0x04 | field | 0x00 | encoded_value | 0x00
pub fn encode_edge_field_index_value_prefix(field: &str, value: &Value) -> Vec<u8> {
    let encoded_value = encode_value_ordered(value);
    let mut buf = Vec::with_capacity(1 + field.len() + 1 + encoded_value.len() + 1);
    buf.push(EDGE_PREFIX_IDX);
    buf.extend(field.as_bytes());
    buf.push(0x00);
    buf.extend(encoded_value);
    buf.push(0x00);
    buf
}

// =============================================================================
// Value Type Tags and Secondary Index Encoding
// =============================================================================

// Type tags for ordered value encoding (sorted by natural type precedence)
const TAG_NULL: u8 = 0x00;
const TAG_BOOL: u8 = 0x01;
const TAG_NUMBER: u8 = 0x02; // Unified tag for Int, Float, Decimal
const TAG_STRING: u8 = 0x04;
const TAG_ARRAY: u8 = 0x05;
const TAG_OBJECT: u8 = 0x06;
const TAG_DATETIME: u8 = 0x07;
const TAG_DURATION: u8 = 0x08;
const TAG_BYTES: u8 = 0x09;
const TAG_RANGE: u8 = 0x0A;

/// Fixed width for numeric value encoding (excluding type tag)
/// Format: [sign: 1][exponent: 2][mantissa: 13] = 16 bytes
const NUMERIC_FIXED_WIDTH: usize = 16;

/// Sign bytes for fixed-width numeric encoding
const NUMERIC_SIGN_NEGATIVE: u8 = 0x00;
const NUMERIC_SIGN_ZERO: u8 = 0x80;
const NUMERIC_SIGN_POSITIVE: u8 = 0xFF;

/// Special value markers (fit in 16 bytes)
const NUMERIC_NEG_INFINITY: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];
const NUMERIC_POS_INFINITY: [u8; 16] = [
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
];
const NUMERIC_NAN: [u8; 16] = [
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
];

/// Encode a number to fixed-width 16-byte format for correct bytewise comparison.
/// This is the fallback for Decimal types that go through string parsing.
fn encode_number_fixed_width(value: &str) -> [u8; NUMERIC_FIXED_WIDTH] {
    let dec = DecimalBytes::from_str(value)
        .unwrap_or_else(|_| DecimalBytes::from_str("0").expect("\"0\" is a valid decimal"));

    let bytes = dec.as_bytes();

    if dec.is_nan() {
        return NUMERIC_NAN;
    }
    if dec.is_pos_infinity() {
        return NUMERIC_POS_INFINITY;
    }
    if dec.is_neg_infinity() {
        return NUMERIC_NEG_INFINITY;
    }
    if dec.is_zero() {
        let mut result = [0u8; NUMERIC_FIXED_WIDTH];
        result[0] = NUMERIC_SIGN_ZERO;
        return result;
    }

    let mut result = [0u8; NUMERIC_FIXED_WIDTH];
    result[0] = bytes[0];
    if bytes.len() >= 3 {
        result[1] = bytes[1];
        result[2] = bytes[2];
    }
    let mantissa_len = bytes.len().saturating_sub(3).min(13);
    if mantissa_len > 0 {
        result[3..3 + mantissa_len].copy_from_slice(&bytes[3..3 + mantissa_len]);
    }
    let pad_byte = if bytes[0] == NUMERIC_SIGN_NEGATIVE {
        0xFF
    } else {
        0x00
    };
    for item in result
        .iter_mut()
        .take(NUMERIC_FIXED_WIDTH)
        .skip(3 + mantissa_len)
    {
        *item = pad_byte;
    }
    result
}

/// Optimized encoder for i64 - avoids string parsing.
/// Produces the same bytes as DecimalBytes would.
fn encode_i64_fixed_width(n: i64) -> [u8; NUMERIC_FIXED_WIDTH] {
    // Zero is special
    if n == 0 {
        let mut result = [0u8; NUMERIC_FIXED_WIDTH];
        result[0] = NUMERIC_SIGN_ZERO;
        return result;
    }

    let is_negative = n < 0;
    // Handle i64::MIN specially since abs() would overflow
    let abs_n = if n == i64::MIN {
        9223372036854775808u64
    } else {
        n.unsigned_abs()
    };

    // Count decimal digits
    let digit_count = count_digits_u64(abs_n);

    // Convert to BCD mantissa
    let mut mantissa = [0u8; 13];
    let mantissa_len = int_to_bcd(abs_n, digit_count, &mut mantissa);

    // Build result
    let mut result = [0u8; NUMERIC_FIXED_WIDTH];

    if is_negative {
        result[0] = NUMERIC_SIGN_NEGATIVE;
        result[1] = 0xFF - 0x40;
        result[2] = 0xFF - digit_count;
        for (i, m) in mantissa.iter().enumerate().take(mantissa_len) {
            result[3 + i] = 0xFF - m;
        }
        // Pad with 0xFF for negative numbers
        for item in result
            .iter_mut()
            .take(NUMERIC_FIXED_WIDTH)
            .skip(3 + mantissa_len)
        {
            *item = 0xFF;
        }
    } else {
        result[0] = NUMERIC_SIGN_POSITIVE;
        result[1] = 0x40;
        result[2] = digit_count;
        result[3..3 + mantissa_len].copy_from_slice(&mantissa[..mantissa_len]);
        // Pad with 0x00 for positive numbers (already zero-initialized)
    }

    result
}

/// Count decimal digits in a u64.
#[inline]
fn count_digits_u64(n: u64) -> u8 {
    // Lookup table approach - faster than log10
    if n < 10 {
        1
    } else if n < 100 {
        2
    } else if n < 1_000 {
        3
    } else if n < 10_000 {
        4
    } else if n < 100_000 {
        5
    } else if n < 1_000_000 {
        6
    } else if n < 10_000_000 {
        7
    } else if n < 100_000_000 {
        8
    } else if n < 1_000_000_000 {
        9
    } else if n < 10_000_000_000 {
        10
    } else if n < 100_000_000_000 {
        11
    } else if n < 1_000_000_000_000 {
        12
    } else if n < 10_000_000_000_000 {
        13
    } else if n < 100_000_000_000_000 {
        14
    } else if n < 1_000_000_000_000_000 {
        15
    } else if n < 10_000_000_000_000_000 {
        16
    } else if n < 100_000_000_000_000_000 {
        17
    } else if n < 1_000_000_000_000_000_000 {
        18
    } else if n < 10_000_000_000_000_000_000 {
        19
    } else {
        20
    }
}

/// Convert integer to BCD format.
/// Returns the number of BCD bytes written.
#[inline]
fn int_to_bcd(mut n: u64, digit_count: u8, out: &mut [u8; 13]) -> usize {
    // We need to produce BCD pairs, padding to even digit count
    let padded_digits = if digit_count.is_multiple_of(2) {
        digit_count
    } else {
        digit_count + 1
    };
    let bcd_len = (padded_digits / 2) as usize;

    // Extract digits from right to left
    let mut digits = [0u8; 20];
    for i in 0..digit_count as usize {
        digits[digit_count as usize - 1 - i] = (n % 10) as u8;
        n /= 10;
    }

    // Pack into BCD pairs (with trailing zero padding if odd digit count)
    for (i, out_byte) in out.iter_mut().enumerate().take(bcd_len) {
        let high = digits.get(i * 2).copied().unwrap_or(0);
        let low = if i * 2 + 1 < digit_count as usize {
            digits[i * 2 + 1]
        } else {
            0 // trailing zero padding
        };
        *out_byte = (high << 4) | low;
    }

    bcd_len
}

/// Optimized encoder for f64 - parses float string directly without DecimalBytes.
fn encode_f64_fixed_width(f: f64) -> [u8; NUMERIC_FIXED_WIDTH] {
    // Handle special cases
    if f.is_nan() {
        return NUMERIC_NAN;
    }
    if f == f64::INFINITY {
        return NUMERIC_POS_INFINITY;
    }
    if f == f64::NEG_INFINITY {
        return NUMERIC_NEG_INFINITY;
    }
    if f == 0.0 {
        let mut result = [0u8; NUMERIC_FIXED_WIDTH];
        result[0] = NUMERIC_SIGN_ZERO;
        return result;
    }

    let is_negative = f < 0.0;
    let abs_f = f.abs();

    // Check if it's actually an integer (no fractional part)
    if abs_f == abs_f.trunc() && abs_f <= i64::MAX as f64 {
        let n = if is_negative {
            -(abs_f as i64)
        } else {
            abs_f as i64
        };
        return encode_i64_fixed_width(n);
    }

    // For floats with fractional parts, we need to parse the string representation
    // This is necessary for correct decimal representation
    let s = abs_f.to_string();
    encode_float_string(&s, is_negative)
}

/// Parse a float string and encode to fixed-width format.
/// The string should represent a positive number (sign handled separately).
fn encode_float_string(s: &str, is_negative: bool) -> [u8; NUMERIC_FIXED_WIDTH] {
    // Parse the string to extract digits and decimal position
    // Handles formats: "123.456", "0.001", "1e10", "1.5e-5"

    let mut digits = Vec::with_capacity(20);
    let mut decimal_pos: i16 = 0; // Position of decimal point from left
    let mut found_decimal = false;
    let mut in_exponent = false;
    let mut exp_negative = false;
    let mut exponent: i16 = 0;
    let mut leading_zeros_after_decimal = 0i16;
    let mut found_significant = false;

    for c in s.chars() {
        match c {
            '0'..='9' => {
                if in_exponent {
                    exponent = exponent * 10 + (c as i16 - '0' as i16);
                } else {
                    let digit = c as u8 - b'0';
                    if digit == 0 && !found_significant && found_decimal {
                        leading_zeros_after_decimal += 1;
                    } else if digit != 0 || found_significant {
                        found_significant = true;
                        digits.push(digit);
                        if !found_decimal {
                            decimal_pos += 1;
                        }
                    } else if !found_decimal {
                        // Leading zeros before decimal - ignore but don't count
                    }
                }
            }
            '.' => {
                found_decimal = true;
            }
            'e' | 'E' => {
                in_exponent = true;
            }
            '-' if in_exponent => {
                exp_negative = true;
            }
            '+' => {} // Ignore
            _ => {}
        }
    }

    if exp_negative {
        exponent = -exponent;
    }

    // Adjust decimal position for exponent
    decimal_pos += exponent;

    // For numbers < 1 (like 0.001), decimal_pos will be 0 or negative
    // We need to account for leading zeros after decimal
    if !found_significant || digits.is_empty() {
        // Edge case: all zeros
        let mut result = [0u8; NUMERIC_FIXED_WIDTH];
        result[0] = NUMERIC_SIGN_ZERO;
        return result;
    }

    // If no integer part was found (e.g., "0.123"), adjust for leading zeros
    if decimal_pos <= 0 {
        decimal_pos -= leading_zeros_after_decimal;
    }

    // Exponent encoding: 0x4000 + decimal_pos
    let exp_value = 0x4000i16 + decimal_pos;
    let exp_bytes = (exp_value as u16).to_be_bytes();

    // Convert digits to BCD
    let digit_count = digits.len();
    let padded_count = if digit_count.is_multiple_of(2) {
        digit_count
    } else {
        digit_count + 1
    };
    let bcd_len = padded_count / 2;

    let mut mantissa = [0u8; 13];
    for (i, m) in mantissa.iter_mut().enumerate().take(bcd_len.min(13)) {
        let high = digits.get(i * 2).copied().unwrap_or(0);
        let low = digits.get(i * 2 + 1).copied().unwrap_or(0);
        *m = (high << 4) | low;
    }

    // Build result
    let mut result = [0u8; NUMERIC_FIXED_WIDTH];

    if is_negative {
        result[0] = NUMERIC_SIGN_NEGATIVE;
        result[1] = 0xFF - exp_bytes[0];
        result[2] = 0xFF - exp_bytes[1];
        for (i, m) in mantissa.iter().enumerate().take(bcd_len.min(13)) {
            result[3 + i] = 0xFF - m;
        }
        // Pad with 0xFF for negative numbers
        for item in result
            .iter_mut()
            .take(NUMERIC_FIXED_WIDTH)
            .skip(3 + bcd_len.min(13))
        {
            *item = 0xFF;
        }
    } else {
        result[0] = NUMERIC_SIGN_POSITIVE;
        result[1] = exp_bytes[0];
        result[2] = exp_bytes[1];
        result[3..3 + bcd_len.min(13)].copy_from_slice(&mantissa[..bcd_len.min(13)]);
        // Pad with 0x00 for positive (already zero-initialized)
    }

    result
}

/// Encode a Value for ordered byte comparison in secondary indexes.
///
/// The encoding preserves lexicographic ordering:
/// - Null < Bool < Int < Float < String (by type tag)
/// - Within type: values compare correctly
///
/// For integers: XOR with i64::MIN to convert signed to unsigned ordering
/// For floats: IEEE 754 trick to make byte comparison match numeric comparison
/// For strings: escape 0x00 bytes to allow null-termination
pub fn encode_value_ordered(value: &Value) -> Vec<u8> {
    match value {
        Value::Null => vec![TAG_NULL],

        Value::Bool(b) => vec![TAG_BOOL, if *b { 1 } else { 0 }],

        Value::Int(n) => {
            let mut buf = vec![TAG_NUMBER];
            buf.extend(encode_i64_fixed_width(*n));
            buf
        }

        Value::Float(f) => {
            let mut buf = vec![TAG_NUMBER];
            buf.extend(encode_f64_fixed_width(*f));
            buf
        }

        Value::Decimal(d) => {
            let mut buf = vec![TAG_NUMBER];
            buf.extend(encode_number_fixed_width(&d.as_decimal().to_string()));
            buf
        }

        Value::String(s) => {
            // Escape 0x00 bytes as 0x00 0xFF, then terminate with 0x00 0x00
            // This allows null-terminated strings while preserving ordering
            let mut buf = Vec::with_capacity(1 + s.len() + 2);
            buf.push(TAG_STRING);
            for byte in s.as_bytes() {
                buf.push(*byte);
                if *byte == 0x00 {
                    buf.push(0xFF); // Escape null bytes
                }
            }
            buf.push(0x00);
            buf.push(0x00); // Double null terminator
            buf
        }

        // Arrays and Objects are not directly indexable for range scans
        // They should be handled at a higher level (index specific fields)
        Value::Array(_) | Value::Object(_) => {
            // Return a marker that sorts after strings but is not searchable
            vec![0xFF]
        }

        // References encode the same as strings — indexes are per-field so no ambiguity
        Value::Reference(id) => {
            let mut buf = Vec::with_capacity(1 + id.len() + 2);
            buf.push(TAG_STRING);
            for byte in id.as_bytes() {
                buf.push(*byte);
                if *byte == 0x00 {
                    buf.push(0xFF); // Escape null bytes
                }
            }
            buf.push(0x00);
            buf.push(0x00); // Double null terminator
            buf
        }

        // Datetime: tag + 8 bytes (i64 timestamp millis, big endian for ordering)
        Value::Datetime(ms) => {
            let mut buf = vec![TAG_DATETIME];
            // XOR with i64::MIN to convert signed to unsigned for correct byte ordering
            buf.extend_from_slice(&((*ms as u64) ^ 0x8000_0000_0000_0000).to_be_bytes());
            buf
        }

        // Duration: tag + 8 bytes (u64 nanoseconds, big endian for ordering)
        Value::Duration(ns) => {
            let mut buf = vec![TAG_DURATION];
            buf.extend_from_slice(&ns.to_be_bytes());
            buf
        }

        // Bytes: tag + length + bytes
        Value::Bytes(b) => {
            let mut buf = vec![TAG_BYTES];
            buf.extend_from_slice(&(b.len() as u32).to_be_bytes());
            buf.extend_from_slice(b);
            buf
        }

        // Range: tag + encoded start + encoded end
        Value::Range { start, end } => {
            let mut buf = vec![TAG_RANGE];
            let start_encoded = encode_value_ordered(start);
            let end_encoded = encode_value_ordered(end);
            buf.extend_from_slice(&(start_encoded.len() as u32).to_be_bytes());
            buf.extend(start_encoded);
            buf.extend(end_encoded);
            buf
        }
    }
}

/// Encode a value for use in unique constraint hashing.
/// Unlike `encode_value_ordered`, this properly handles arrays and objects
/// by recursively encoding their contents. The encoding is not suitable for
/// range scans but produces unique byte sequences for unique values.
///
/// IMPORTANT: All numeric types (Int, Float, Decimal) are encoded uniformly
/// so that semantically equal values (100, 100.0, 100dec) produce the same hash.
/// This ensures unique constraints work correctly across numeric types.
pub fn encode_value_for_hash(value: &Value) -> Vec<u8> {
    match value {
        Value::Null => vec![TAG_NULL],
        Value::Bool(b) => vec![TAG_BOOL, if *b { 1 } else { 0 }],
        // Unified numeric encoding: convert all numbers to fixed-width for consistent hashing
        Value::Int(n) => {
            let mut buf = vec![TAG_NUMBER];
            buf.extend(encode_i64_fixed_width(*n));
            buf
        }
        Value::Float(f) => {
            let mut buf = vec![TAG_NUMBER];
            buf.extend(encode_f64_fixed_width(*f));
            buf
        }
        Value::Decimal(d) => {
            let mut buf = vec![TAG_NUMBER];
            buf.extend(encode_number_fixed_width(&d.as_decimal().to_string()));
            buf
        }
        Value::String(s) => {
            let mut buf = vec![TAG_STRING];
            buf.extend_from_slice(s.as_bytes());
            buf.push(0x00); // Null terminator
            buf
        }
        Value::Array(arr) => {
            let mut buf = vec![TAG_ARRAY];
            // Encode length as 4 bytes
            buf.extend_from_slice(&(arr.len() as u32).to_le_bytes());
            // Recursively encode each element
            for elem in arr {
                let encoded = encode_value_for_hash(elem);
                buf.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
                buf.extend(encoded);
            }
            buf
        }
        Value::Object(obj) => {
            let mut buf = vec![TAG_OBJECT];
            // Sort keys for deterministic encoding
            let mut pairs: Vec<_> = obj.iter().collect();
            pairs.sort_by_key(|(k, _)| *k);
            // Encode count
            buf.extend_from_slice(&(pairs.len() as u32).to_le_bytes());
            for (key, val) in pairs {
                // Encode key
                buf.extend_from_slice(&(key.len() as u32).to_le_bytes());
                buf.extend_from_slice(key.as_bytes());
                // Encode value
                let encoded = encode_value_for_hash(val);
                buf.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
                buf.extend(encoded);
            }
            buf
        }
        Value::Reference(id) => {
            let mut buf = vec![TAG_STRING];
            buf.extend_from_slice(id.as_bytes());
            buf.push(0x00); // Null terminator
            buf
        }
        Value::Datetime(ms) => {
            let mut buf = vec![TAG_DATETIME];
            buf.extend_from_slice(&ms.to_le_bytes());
            buf
        }
        Value::Duration(ns) => {
            let mut buf = vec![TAG_DURATION];
            buf.extend_from_slice(&ns.to_le_bytes());
            buf
        }
        Value::Bytes(b) => {
            let mut buf = vec![TAG_BYTES];
            buf.extend_from_slice(&(b.len() as u32).to_le_bytes());
            buf.extend_from_slice(b);
            buf
        }
        Value::Range { start, end } => {
            let mut buf = vec![TAG_RANGE];
            let start_encoded = encode_value_for_hash(start);
            let end_encoded = encode_value_for_hash(end);
            buf.extend_from_slice(&(start_encoded.len() as u32).to_le_bytes());
            buf.extend(start_encoded);
            buf.extend_from_slice(&(end_encoded.len() as u32).to_le_bytes());
            buf.extend(end_encoded);
            buf
        }
    }
}

// =============================================================================
// Delimiter-based Index Encoding
// =============================================================================

/// Encode value with delimiter for index keys.
///
/// With fixed-width numeric encoding, no delimiter is needed for Null/Bool/Number
/// since their lengths are now known. Strings retain their 0x00 0x00 terminator.
pub fn encode_value_with_delimiter(value: &Value) -> Vec<u8> {
    encode_value_ordered(value)
}

/// Encode complete index key: `[value1][delim]...[0xFF][doc_key]`
///
/// This format allows:
/// - Efficient prefix scans for equality queries
/// - Correct range query behavior with bytewise comparison
/// - Unambiguous parsing due to type-specific delimiters
pub fn encode_index_key(values: &[&Value], doc_key: &str) -> Vec<u8> {
    let mut buf = Vec::new();

    for value in values {
        buf.extend(encode_value_with_delimiter(value));
    }

    buf.push(0xFF);
    buf.extend(doc_key.as_bytes());

    buf
}

/// Encode prefix for equality queries.
///
/// Used for scanning all documents matching specific field values.
pub fn encode_index_prefix(values: &[&Value]) -> Vec<u8> {
    let mut buf = Vec::new();

    for value in values {
        buf.extend(encode_value_with_delimiter(value));
    }

    buf
}

/// Encode exclusive start boundary (for > queries).
///
/// Returns a key that sorts after all index keys with the given value,
/// allowing efficient "greater than" range queries.
pub fn encode_range_bound_exclusive(value: &Value) -> Vec<u8> {
    let mut buf = encode_value_with_delimiter(value);
    buf.push(0xFF);
    buf
}

/// Compute the byte-level successor of a byte sequence.
///
/// Returns the smallest byte sequence that is strictly greater than the input.
/// Used for computing exclusive upper bounds in range scans: all keys starting
/// with the input prefix sort before the successor.
pub fn byte_successor(bytes: &[u8]) -> Vec<u8> {
    let mut result = bytes.to_vec();
    // Increment from the rightmost byte, carrying as needed
    for i in (0..result.len()).rev() {
        if result[i] < 0xFF {
            result[i] += 1;
            result.truncate(i + 1);
            return result;
        }
    }
    // All bytes are 0xFF — append a 0x00 byte to make it longer
    result.push(0x00);
    result
}

/// Extract doc_key from index key by finding 0xFF separator
pub fn extract_doc_key(key: &[u8]) -> Option<String> {
    // Find last 0xFF (separator before doc_key)
    let separator_pos = key.iter().rposition(|&b| b == 0xFF)?;

    std::str::from_utf8(&key[separator_pos + 1..])
        .ok()
        .map(|s| s.to_string())
}

/// Skip one encoded value, returning the number of bytes consumed.
/// Used for parsing compound index keys.
pub fn skip_encoded_value(data: &[u8]) -> Option<usize> {
    if data.is_empty() {
        return None;
    }

    match data[0] {
        TAG_NULL => Some(1),                         // [tag] only
        TAG_BOOL => Some(2),                         // [tag][val]
        TAG_NUMBER => Some(1 + NUMERIC_FIXED_WIDTH), // [tag][16 bytes] = 17 bytes
        TAG_STRING => {
            // Find 0x00 0x00 terminator (0x00 escaped as 0x00 0xFF)
            let mut i = 1; // Skip tag
            while i + 1 < data.len() {
                if data[i] == 0x00 && data[i + 1] == 0x00 {
                    return Some(i + 2);
                }
                if data[i] == 0x00 && data[i + 1] == 0xFF {
                    i += 2; // Skip escaped null
                } else {
                    i += 1;
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_compound_null_encoding() {
        // Test null encoding
        let null_encoded = encode_value_ordered(&Value::Null);
        eprintln!("Null encoded: {:02x?}", null_encoded);
        assert_eq!(null_encoded, vec![TAG_NULL]); // TAG_NULL = 0x00

        // Build compound key for (a='one', b=true, c=null)
        let mut key = Vec::new();

        // field a = 'one'
        key.extend(b"a");
        key.push(0x00);
        key.extend(encode_value_ordered(&Value::String("one".to_string())));
        key.push(0x00);

        // field b = true
        key.extend(b"b");
        key.push(0x00);
        key.extend(encode_value_ordered(&Value::Bool(true)));
        key.push(0x00);

        // field c = null
        key.extend(b"c");
        key.push(0x00);
        key.extend(encode_value_ordered(&Value::Null));
        key.push(0x00);

        // doc_key
        key.extend(b"d1");

        eprintln!("Full key: {:02x?}", key);

        // Build prefix for query (same as key but without doc_key)
        let mut prefix = Vec::new();
        prefix.extend(b"a");
        prefix.push(0x00);
        prefix.extend(encode_value_ordered(&Value::String("one".to_string())));
        prefix.push(0x00);
        prefix.extend(b"b");
        prefix.push(0x00);
        prefix.extend(encode_value_ordered(&Value::Bool(true)));
        prefix.push(0x00);
        prefix.extend(b"c");
        prefix.push(0x00);
        prefix.extend(encode_value_ordered(&Value::Null));
        prefix.push(0x00);

        eprintln!("Query prefix: {:02x?}", prefix);
        eprintln!("Key starts with prefix: {}", key.starts_with(&prefix));

        // The key must start with prefix
        assert!(key.starts_with(&prefix), "Key should start with prefix");
    }

    // =========================================================================
    // Secondary Index Encoding Tests
    // =========================================================================

    #[test]
    fn test_int_ordering() {
        let neg = encode_value_ordered(&Value::Int(-100));
        let zero = encode_value_ordered(&Value::Int(0));
        let pos = encode_value_ordered(&Value::Int(100));

        assert!(neg < zero, "negative should be less than zero");
        assert!(zero < pos, "zero should be less than positive");

        // Test extremes
        let min = encode_value_ordered(&Value::Int(i64::MIN));
        let max = encode_value_ordered(&Value::Int(i64::MAX));
        assert!(min < neg, "MIN should be less than -100");
        assert!(pos < max, "100 should be less than MAX");

        // Test close values
        let neg1 = encode_value_ordered(&Value::Int(-1));
        let pos1 = encode_value_ordered(&Value::Int(1));
        assert!(neg1 < zero, "-1 should be less than 0");
        assert!(zero < pos1, "0 should be less than 1");
    }

    #[test]
    fn test_float_ordering() {
        let neg = encode_value_ordered(&Value::Float(-100.5));
        let zero = encode_value_ordered(&Value::Float(0.0));
        let pos = encode_value_ordered(&Value::Float(100.5));

        assert!(neg < zero, "negative float should be less than zero");
        assert!(zero < pos, "zero should be less than positive float");

        // Test small differences
        let small_neg = encode_value_ordered(&Value::Float(-0.001));
        let small_pos = encode_value_ordered(&Value::Float(0.001));
        assert!(small_neg < zero, "-0.001 should be less than 0");
        assert!(zero < small_pos, "0 should be less than 0.001");

        // Test infinity
        let neg_inf = encode_value_ordered(&Value::Float(f64::NEG_INFINITY));
        let pos_inf = encode_value_ordered(&Value::Float(f64::INFINITY));
        assert!(neg_inf < neg, "-inf should be less than -100.5");
        assert!(pos < pos_inf, "100.5 should be less than +inf");

        // Test negative zero equals positive zero in ordering
        let neg_zero = encode_value_ordered(&Value::Float(-0.0));
        // -0.0 should sort before +0.0 due to bit representation, but both are valid
        assert!(neg_zero <= zero, "-0.0 should be <= +0.0");
    }

    #[test]
    fn test_string_ordering() {
        let a = encode_value_ordered(&Value::String("a".to_string()));
        let b = encode_value_ordered(&Value::String("b".to_string()));
        let aa = encode_value_ordered(&Value::String("aa".to_string()));
        let empty = encode_value_ordered(&Value::String("".to_string()));

        assert!(empty < a, "empty string should be less than 'a'");
        assert!(a < aa, "'a' should be less than 'aa'");
        assert!(a < b, "'a' should be less than 'b'");
        assert!(aa < b, "'aa' should be less than 'b'");

        // Test strings with null bytes (edge case)
        let with_null = encode_value_ordered(&Value::String("a\x00b".to_string()));
        let without_null = encode_value_ordered(&Value::String("ab".to_string()));
        // The string "a\x00b" should sort before "ab" because \x00 < 'b'
        assert!(
            with_null < without_null,
            "'a\\x00b' should be less than 'ab'"
        );
    }

    #[test]
    fn test_type_ordering() {
        // Types should sort: Null < Bool < Number < String
        // With unified number encoding, Int(0) == Float(0.0) == Decimal(0)
        let null = encode_value_ordered(&Value::Null);
        let bool_false = encode_value_ordered(&Value::Bool(false));
        let bool_true = encode_value_ordered(&Value::Bool(true));
        let int_zero = encode_value_ordered(&Value::Int(0));
        let float_zero = encode_value_ordered(&Value::Float(0.0));
        let string = encode_value_ordered(&Value::String("".to_string()));

        assert!(null < bool_false, "null should be less than bool");
        assert!(bool_false < bool_true, "false should be less than true");
        assert!(bool_true < int_zero, "bool should be less than number");
        // Int and Float with same value encode identically (unified number space)
        assert_eq!(
            int_zero, float_zero,
            "Int(0) and Float(0.0) should encode identically"
        );
        assert!(float_zero < string, "number should be less than string");
    }

    // =========================================================================
    // Edge v2 Encoding Tests
    // =========================================================================

    #[test]
    fn test_edge_v2_out_key_single() {
        let key = encode_edge_out_key_single("user:alice", "user:bob");
        assert_eq!(key[0], EDGE_PREFIX_OUT);
        assert_eq!(&key[1..], b"user:alice\x00user:bob");
    }

    #[test]
    fn test_edge_v2_out_key_multi() {
        let key = encode_edge_out_key_multi("user:alice", "user:bob", "abc123");
        assert_eq!(key[0], EDGE_PREFIX_OUT);
        assert_eq!(&key[1..], b"user:alice\x00user:bob\x00abc123");
    }

    #[test]
    fn test_edge_v2_in_key_single() {
        let key = encode_edge_in_key_single("user:bob", "user:alice");
        assert_eq!(key[0], EDGE_PREFIX_IN);
        assert_eq!(&key[1..], b"user:bob\x00user:alice");
    }

    #[test]
    fn test_edge_v2_in_key_multi() {
        let key = encode_edge_in_key_multi("user:bob", "user:alice", "abc123");
        assert_eq!(key[0], EDGE_PREFIX_IN);
        assert_eq!(&key[1..], b"user:bob\x00user:alice\x00abc123");
    }

    #[test]
    fn test_edge_v2_id_key() {
        let key = encode_edge_id_key("abc123");
        assert_eq!(key[0], EDGE_PREFIX_ID);
        assert_eq!(&key[1..], b"abc123");
    }

    #[test]
    fn test_edge_v2_id_value_roundtrip() {
        let value = encode_edge_id_value("user:alice", "user:bob");
        let (from, to) = decode_edge_id_value(&value).unwrap();
        assert_eq!(from, "user:alice");
        assert_eq!(to, "user:bob");
    }

    #[test]
    fn test_edge_v2_out_prefix_range() {
        let prefix = encode_edge_out_prefix_v2("user:alice");
        let prefix_end = encode_edge_out_prefix_end_v2("user:alice");

        let key1 = encode_edge_out_key_single("user:alice", "user:bob");
        let key2 = encode_edge_out_key_single("user:alice", "user:carol");

        // Both keys should be in range
        assert!(key1.as_slice() >= prefix.as_slice());
        assert!(key1.as_slice() < prefix_end.as_slice());
        assert!(key2.as_slice() >= prefix.as_slice());
        assert!(key2.as_slice() < prefix_end.as_slice());

        // Keys should be ordered
        assert!(key1 < key2, "bob should sort before carol");
    }

    #[test]
    fn test_edge_v2_multi_prefix_range() {
        let prefix = encode_edge_out_target_prefix("user:alice", "user:bob");
        let prefix_end = encode_edge_out_target_prefix_end("user:alice", "user:bob");

        let key1 = encode_edge_out_key_multi("user:alice", "user:bob", "id1");
        let key2 = encode_edge_out_key_multi("user:alice", "user:bob", "id2");

        // Both keys should be in range
        assert!(key1.as_slice() >= prefix.as_slice());
        assert!(key1.as_slice() < prefix_end.as_slice());
        assert!(key2.as_slice() >= prefix.as_slice());
        assert!(key2.as_slice() < prefix_end.as_slice());

        // Keys should be ordered by edge_id
        assert!(key1 < key2);
    }

    #[test]
    fn test_edge_v2_field_index_key() {
        let key = encode_edge_field_index_key("amount", &Value::Int(100), "abc123");
        assert_eq!(key[0], EDGE_PREFIX_IDX);
        assert!(key[1..].starts_with(b"amount\x00"));
    }

    #[test]
    fn test_edge_v2_prefix_isolation() {
        // Ensure different prefixes don't overlap
        let out_key = encode_edge_out_key_single("user:alice", "user:bob");
        let in_key = encode_edge_in_key_single("user:alice", "user:bob");
        let id_key = encode_edge_id_key("user:alice");

        // First bytes should be different
        assert_ne!(out_key[0], in_key[0]);
        assert_ne!(out_key[0], id_key[0]);
        assert_ne!(in_key[0], id_key[0]);

        // No key should be a prefix of another
        assert!(!out_key.starts_with(&in_key));
        assert!(!in_key.starts_with(&out_key));
    }
}

#[cfg(test)]
mod delimiter_tests {
    use super::*;

    #[test]
    fn test_encode_value_with_delimiter_null() {
        let encoded = encode_value_with_delimiter(&Value::Null);
        assert_eq!(encoded[0], TAG_NULL);
        // With fixed-width encoding, null is just the tag (1 byte)
        assert_eq!(encoded.len(), 1);
    }

    #[test]
    fn test_encode_value_with_delimiter_bool() {
        let encoded = encode_value_with_delimiter(&Value::Bool(true));
        assert_eq!(encoded[0], TAG_BOOL);
        // Bool is tag + value (2 bytes)
        assert_eq!(encoded.len(), 2);
        assert_eq!(encoded[1], 1); // true
    }

    #[test]
    fn test_encode_value_with_delimiter_number() {
        let encoded = encode_value_with_delimiter(&Value::Int(42));
        assert_eq!(encoded[0], TAG_NUMBER);
        // Number is tag + 16 fixed-width bytes (17 total)
        assert_eq!(encoded.len(), 1 + NUMERIC_FIXED_WIDTH);
    }

    #[test]
    fn test_encode_value_with_delimiter_string() {
        let encoded = encode_value_with_delimiter(&Value::String("hello".to_string()));
        assert_eq!(encoded[0], TAG_STRING);
        // Ends with 0x00 0x00 (existing terminator)
        assert_eq!(encoded[encoded.len() - 2..], [0x00, 0x00]);
    }

    #[test]
    fn test_encode_index_key_single() {
        let value = Value::Int(25);
        let key = encode_index_key(&[&value], "user:1");

        // Should contain 0xFF before doc_key
        assert!(key.windows(2).any(|w| w[0] == 0xFF && w[1] == b'u'));
        assert!(key.ends_with(b"user:1"));
    }

    #[test]
    fn test_encode_index_key_compound() {
        let status = Value::String("active".to_string());
        let age = Value::Int(25);
        let key = encode_index_key(&[&status, &age], "user:1");

        assert!(key.ends_with(b"user:1"));
        // Contains 0xFF separator
        assert!(key.contains(&0xFF));
    }

    #[test]
    fn test_encode_range_bound_exclusive() {
        let value = Value::Int(25);
        let bound = encode_range_bound_exclusive(&value);
        let key_with_doc = encode_index_key(&[&value], "doc:a");

        // bound = [value][0xFF]
        // key   = [value][0xFF][doc:a]
        // The bound serves as exclusive end for range scans:
        // - Any key with same value will be > bound (key has more bytes after shared prefix)
        // - This allows range scan with end < bound to exclude all keys with this value
        assert!(
            key_with_doc > bound,
            "key with doc should be > bound (more bytes)"
        );

        // Verify bound can be used to exclude keys with same value in range scan
        // A key with a larger value should be >= bound
        let larger_value = Value::Int(26);
        let larger_key = encode_index_key(&[&larger_value], "doc:a");
        assert!(
            larger_key > bound,
            "key with larger value should be > bound"
        );
    }

    #[test]
    fn test_extract_doc_key_from_index_key() {
        let value = Value::Int(25);
        let key = encode_index_key(&[&value], "user:alice");

        let doc_key = extract_doc_key(&key).unwrap();
        assert_eq!(doc_key, "user:alice");
    }

    #[test]
    fn test_extract_doc_key_compound() {
        let v1 = Value::String("active".to_string());
        let v2 = Value::Int(25);
        let key = encode_index_key(&[&v1, &v2], "doc:123");

        let doc_key = extract_doc_key(&key).unwrap();
        assert_eq!(doc_key, "doc:123");
    }

    #[test]
    fn test_extract_doc_key_string_with_null() {
        let value = Value::String("hello\x00world".to_string());
        let key = encode_index_key(&[&value], "test:1");

        let doc_key = extract_doc_key(&key).unwrap();
        assert_eq!(doc_key, "test:1");
    }

    #[test]
    fn test_skip_value_null() {
        let encoded = encode_value_with_delimiter(&Value::Null);
        assert_eq!(skip_encoded_value(&encoded), Some(encoded.len()));
    }

    #[test]
    fn test_skip_value_bool() {
        let encoded = encode_value_with_delimiter(&Value::Bool(true));
        assert_eq!(skip_encoded_value(&encoded), Some(encoded.len()));
    }

    #[test]
    fn test_skip_value_number() {
        let encoded = encode_value_with_delimiter(&Value::Int(12345));
        assert_eq!(skip_encoded_value(&encoded), Some(encoded.len()));
    }

    #[test]
    fn test_skip_value_string() {
        let encoded = encode_value_with_delimiter(&Value::String("test".to_string()));
        assert_eq!(skip_encoded_value(&encoded), Some(encoded.len()));
    }

    #[test]
    fn test_skip_value_string_with_embedded_null() {
        let encoded = encode_value_with_delimiter(&Value::String("a\x00b".to_string()));
        let len = skip_encoded_value(&encoded).unwrap();
        assert_eq!(len, encoded.len());
    }

    #[test]
    fn test_skip_multiple_values() {
        let v1 = Value::String("hello".to_string());
        let v2 = Value::Int(42);

        let mut buf = encode_value_with_delimiter(&v1);
        buf.extend(encode_value_with_delimiter(&v2));

        let skip1 = skip_encoded_value(&buf).unwrap();
        let skip2 = skip_encoded_value(&buf[skip1..]).unwrap();

        assert_eq!(skip1 + skip2, buf.len());
    }

    /// Test that verifies the critical bug fix: 100 < 100.5 with fixed-width encoding
    /// This was previously broken due to the 0xAA delimiter interfering with comparison.
    #[test]
    fn test_int_vs_float_ordering_fixed() {
        let v100 = encode_value_ordered(&Value::Int(100));
        let v100_5 = encode_value_ordered(&Value::Float(100.5));
        assert!(v100 < v100_5, "Int(100) should be less than Float(100.5)");

        // Also test via encode_value_with_delimiter to ensure no delimiter interference
        let v100_delim = encode_value_with_delimiter(&Value::Int(100));
        let v100_5_delim = encode_value_with_delimiter(&Value::Float(100.5));
        assert!(
            v100_delim < v100_5_delim,
            "Int(100) with delimiter should be less than Float(100.5) with delimiter"
        );

        // Test index keys to ensure correct range query behavior
        let key100 = encode_index_key(&[&Value::Int(100)], "doc:a");
        let key100_5 = encode_index_key(&[&Value::Float(100.5)], "doc:a");
        assert!(
            key100 < key100_5,
            "Index key for 100 should be less than index key for 100.5"
        );
    }

    #[test]
    fn test_numeric_fixed_width_size() {
        // All numeric values should have same encoded size (1 tag + 16 bytes)
        let expected_size = 1 + NUMERIC_FIXED_WIDTH;

        let small_int = encode_value_ordered(&Value::Int(1));
        let large_int = encode_value_ordered(&Value::Int(999999999999i64));
        let small_float = encode_value_ordered(&Value::Float(0.1));
        let large_float = encode_value_ordered(&Value::Float(123_456_789.123_456_79));

        assert_eq!(
            small_int.len(),
            expected_size,
            "small int should be {} bytes",
            expected_size
        );
        assert_eq!(
            large_int.len(),
            expected_size,
            "large int should be {} bytes",
            expected_size
        );
        assert_eq!(
            small_float.len(),
            expected_size,
            "small float should be {} bytes",
            expected_size
        );
        assert_eq!(
            large_float.len(),
            expected_size,
            "large float should be {} bytes",
            expected_size
        );
    }

    #[test]
    fn test_optimized_i64_encoder_matches_original() {
        // Verify the optimized encoder produces identical bytes to string-based encoder
        let test_values: &[i64] = &[
            0,
            1,
            2,
            9,
            10,
            11,
            99,
            100,
            123,
            1000,
            12345,
            123456,
            -1,
            -10,
            -100,
            -12345,
            i64::MAX,
            i64::MIN,
            999999999999999999, // 18 digits
        ];

        for &n in test_values {
            let optimized = encode_i64_fixed_width(n);
            let original = encode_number_fixed_width(&n.to_string());
            assert_eq!(
                optimized,
                original,
                "Mismatch for {}: optimized={:02x?}, original={:02x?}",
                n,
                &optimized[..8],
                &original[..8]
            );
        }
    }

    #[test]
    fn test_optimized_f64_encoder_matches_original() {
        let test_values: &[f64] = &[
            0.0,
            0.5,
            0.1,
            0.01,
            0.001,
            1.0,
            1.5,
            1.23,
            1.234,
            10.0,
            10.5,
            12.34,
            100.0,
            100.5,
            123.456,
            -0.5,
            -1.5,
            -100.5,
            -12.34,
            1e10,
            1e-10,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ];

        for &f in test_values {
            let optimized = encode_f64_fixed_width(f);
            let original_str = if f.is_finite() {
                f.to_string()
            } else if f == f64::INFINITY {
                "Infinity".to_string()
            } else if f == f64::NEG_INFINITY {
                "-Infinity".to_string()
            } else {
                "NaN".to_string()
            };
            let original = encode_number_fixed_width(&original_str);

            // For NaN, just check it's the same special marker
            if f.is_nan() {
                assert_eq!(optimized, NUMERIC_NAN, "NaN should encode to NUMERIC_NAN");
                continue;
            }

            assert_eq!(
                optimized,
                original,
                "Mismatch for {}: optimized={:02x?}, original={:02x?}",
                f,
                &optimized[..8],
                &original[..8]
            );
        }
    }

    #[test]
    fn bench_i64_encoding_comparison() {
        use std::hint::black_box;
        use std::time::Instant;

        let test_values: &[i64] = &[
            0,
            1,
            10,
            100,
            12345,
            123456789,
            -1,
            -100,
            -12345,
            i64::MAX,
            i64::MIN,
        ];
        const ITERATIONS: usize = 10_000;

        // Warm up
        for _ in 0..1000 {
            for &n in test_values {
                black_box(encode_i64_fixed_width(n));
                black_box(encode_number_fixed_width(&n.to_string()));
            }
        }

        // Benchmark optimized
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            for &n in test_values {
                black_box(encode_i64_fixed_width(n));
            }
        }
        let optimized_time = start.elapsed();

        // Benchmark original (string-based)
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            for &n in test_values {
                black_box(encode_number_fixed_width(&n.to_string()));
            }
        }
        let original_time = start.elapsed();

        let total_ops = ITERATIONS * test_values.len();
        let speedup = original_time.as_nanos() as f64 / optimized_time.as_nanos() as f64;

        eprintln!("\n=== i64 Encoding Benchmark ===");
        eprintln!(
            "Optimized: {:?} ({:.1} ns/op)",
            optimized_time,
            optimized_time.as_nanos() as f64 / total_ops as f64
        );
        eprintln!(
            "Original:  {:?} ({:.1} ns/op)",
            original_time,
            original_time.as_nanos() as f64 / total_ops as f64
        );
        eprintln!("Speedup:   {:.2}x", speedup);
    }

    #[test]
    fn bench_f64_encoding_comparison() {
        use std::hint::black_box;
        use std::time::Instant;

        let test_values: &[f64] = &[
            0.0, 0.5, 0.1, 0.001, 1.0, 1.5, 12.34, 123.456, -0.5, -12.34, 1e10, 1e-10,
        ];
        const ITERATIONS: usize = 10_000;

        // Warm up
        for _ in 0..1000 {
            for &f in test_values {
                black_box(encode_f64_fixed_width(f));
                let s = f.to_string();
                black_box(encode_number_fixed_width(&s));
            }
        }

        // Benchmark optimized
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            for &f in test_values {
                black_box(encode_f64_fixed_width(f));
            }
        }
        let optimized_time = start.elapsed();

        // Benchmark original (string-based + DecimalBytes)
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            for &f in test_values {
                let s = f.to_string();
                black_box(encode_number_fixed_width(&s));
            }
        }
        let original_time = start.elapsed();

        let total_ops = ITERATIONS * test_values.len();
        let speedup = original_time.as_nanos() as f64 / optimized_time.as_nanos() as f64;

        eprintln!("\n=== f64 Encoding Benchmark ===");
        eprintln!(
            "Optimized: {:?} ({:.1} ns/op)",
            optimized_time,
            optimized_time.as_nanos() as f64 / total_ops as f64
        );
        eprintln!(
            "Original:  {:?} ({:.1} ns/op)",
            original_time,
            original_time.as_nanos() as f64 / total_ops as f64
        );
        eprintln!("Speedup:   {:.2}x", speedup);
    }

    // =========================================================================
    // Comprehensive Numeric Ordering Tests
    // =========================================================================

    #[test]
    fn test_numeric_ordering_property() {
        // Property: if a < b mathematically, then encode(a) < encode(b)
        let test_pairs: &[(f64, f64)] = &[
            // Basic comparisons
            (-100.0, -10.0),
            (-10.0, -1.0),
            (-1.0, -0.5),
            (-0.5, 0.0),
            (0.0, 0.5),
            (0.5, 1.0),
            (1.0, 10.0),
            (10.0, 100.0),
            // Close values
            (0.0, 0.000001),
            (0.999999, 1.0),
            (99.9, 100.0),
            (100.0, 100.1),
            (100.0, 100.5),
            (100.5, 101.0),
            // Powers of 10
            (1.0, 10.0),
            (10.0, 100.0),
            (100.0, 1000.0),
            (0.1, 1.0),
            (0.01, 0.1),
            // Large numbers
            (1e10, 1e11),
            (1e15, 1e16),
            (-1e16, -1e15),
            // Small decimals
            (0.001, 0.01),
            (0.0001, 0.001),
            (-0.001, -0.0001),
            // Safe large values (within f64 exact integer range: 2^53)
            (9007199254740990.0, 9007199254740991.0), // 2^53 - 2 and 2^53 - 1
            (-9007199254740991.0, -9007199254740990.0),
        ];

        for &(a, b) in test_pairs {
            let enc_a = encode_value_ordered(&Value::Float(a));
            let enc_b = encode_value_ordered(&Value::Float(b));
            assert!(
                enc_a < enc_b,
                "Expected encode({}) < encode({}), but got {:02x?} >= {:02x?}",
                a,
                b,
                &enc_a[..8],
                &enc_b[..8]
            );
        }
    }

    #[test]
    fn test_numeric_ordering_int_float_mixed() {
        // Property: Int and Float should be in same ordering space
        let test_cases: &[(i64, f64)] = &[
            (0, 0.5),
            (1, 1.5),
            (99, 99.5),
            (100, 100.5),
            (100, 100.01),
            (100, 100.001),
            (-1, -0.5),
            (-100, -99.5),
            (-101, -100.5),
        ];

        for &(i, f) in test_cases {
            let enc_i = encode_value_ordered(&Value::Int(i));
            let enc_f = encode_value_ordered(&Value::Float(f));

            if (i as f64) < f {
                assert!(
                    enc_i < enc_f,
                    "Expected encode(Int({})) < encode(Float({})), but got {:02x?} >= {:02x?}",
                    i,
                    f,
                    &enc_i[..8],
                    &enc_f[..8]
                );
            } else {
                assert!(
                    enc_i > enc_f,
                    "Expected encode(Int({})) > encode(Float({})), but got {:02x?} <= {:02x?}",
                    i,
                    f,
                    &enc_i[..8],
                    &enc_f[..8]
                );
            }
        }

        // Same value should encode identically
        for i in [0i64, 1, 10, 100, -1, -10, -100] {
            let enc_i = encode_value_ordered(&Value::Int(i));
            let enc_f = encode_value_ordered(&Value::Float(i as f64));
            assert_eq!(
                enc_i, enc_f,
                "Int({}) and Float({}.0) should encode identically",
                i, i
            );
        }
    }

    #[test]
    fn test_numeric_ordering_transitivity() {
        // Property: if a < b && b < c, then a < c
        let values: Vec<f64> = vec![
            f64::NEG_INFINITY,
            -1e100,
            -1e10,
            -1000.0,
            -100.5,
            -100.0,
            -10.0,
            -1.0,
            -0.5,
            -0.001,
            0.0,
            0.001,
            0.5,
            1.0,
            10.0,
            100.0,
            100.5,
            1000.0,
            1e10,
            1e100,
            f64::INFINITY,
        ];

        let encoded: Vec<Vec<u8>> = values
            .iter()
            .map(|&v| encode_value_ordered(&Value::Float(v)))
            .collect();

        // Check all pairs are correctly ordered
        for i in 0..values.len() {
            for j in (i + 1)..values.len() {
                assert!(
                    encoded[i] < encoded[j],
                    "Transitivity failed: encode({}) should be < encode({}), indices {} and {}",
                    values[i],
                    values[j],
                    i,
                    j
                );
            }
        }
    }

    #[test]
    fn test_numeric_array_sort() {
        // Test that sorting encoded values gives correct mathematical order
        let input: Vec<f64> = vec![
            100.5,
            -10.0,
            0.0,
            1000.0,
            -1000.0,
            0.001,
            -0.001,
            100.0,
            99.9,
            100.1,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];

        let mut encoded: Vec<(f64, Vec<u8>)> = input
            .iter()
            .map(|&v| (v, encode_value_ordered(&Value::Float(v))))
            .collect();

        // Sort by encoded bytes
        encoded.sort_by(|a, b| a.1.cmp(&b.1));

        // Extract sorted values
        let sorted_values: Vec<f64> = encoded.iter().map(|(v, _)| *v).collect();

        // Expected mathematical order
        let mut expected = input.clone();
        expected.sort_by(|a, b| a.partial_cmp(b).unwrap());

        assert_eq!(
            sorted_values, expected,
            "Byte-sorted values should match mathematically sorted values"
        );
    }

    #[test]
    fn test_numeric_boundary_cases() {
        // Test boundary conditions

        // i64::MIN and i64::MIN + 1
        let min = encode_value_ordered(&Value::Int(i64::MIN));
        let min_plus_1 = encode_value_ordered(&Value::Int(i64::MIN + 1));
        assert!(min < min_plus_1, "i64::MIN should be < i64::MIN + 1");

        // i64::MAX and i64::MAX - 1
        let max = encode_value_ordered(&Value::Int(i64::MAX));
        let max_minus_1 = encode_value_ordered(&Value::Int(i64::MAX - 1));
        assert!(max_minus_1 < max, "i64::MAX - 1 should be < i64::MAX");

        // Around zero
        let neg_1 = encode_value_ordered(&Value::Int(-1));
        let zero = encode_value_ordered(&Value::Int(0));
        let pos_1 = encode_value_ordered(&Value::Int(1));
        assert!(neg_1 < zero, "-1 should be < 0");
        assert!(zero < pos_1, "0 should be < 1");

        // Powers of 10 transitions
        let v9 = encode_value_ordered(&Value::Int(9));
        let v10 = encode_value_ordered(&Value::Int(10));
        let v11 = encode_value_ordered(&Value::Int(11));
        assert!(v9 < v10, "9 should be < 10");
        assert!(v10 < v11, "10 should be < 11");

        let v99 = encode_value_ordered(&Value::Int(99));
        let v100 = encode_value_ordered(&Value::Int(100));
        let v101 = encode_value_ordered(&Value::Int(101));
        assert!(v99 < v100, "99 should be < 100");
        assert!(v100 < v101, "100 should be < 101");

        // Negative powers of 10
        let neg_9 = encode_value_ordered(&Value::Int(-9));
        let neg_10 = encode_value_ordered(&Value::Int(-10));
        let neg_11 = encode_value_ordered(&Value::Int(-11));
        assert!(neg_11 < neg_10, "-11 should be < -10");
        assert!(neg_10 < neg_9, "-10 should be < -9");
    }

    #[test]
    fn test_numeric_decimal_precision() {
        // Test that decimal precision is preserved in ordering
        let close_values: &[f64] = &[
            100.0, 100.001, 100.01, 100.1, 100.5, 100.9, 100.99, 100.999, 101.0,
        ];

        let encoded: Vec<Vec<u8>> = close_values
            .iter()
            .map(|&v| encode_value_ordered(&Value::Float(v)))
            .collect();

        for i in 0..(close_values.len() - 1) {
            assert!(
                encoded[i] < encoded[i + 1],
                "encode({}) should be < encode({})",
                close_values[i],
                close_values[i + 1]
            );
        }
    }

    #[test]
    fn test_numeric_negative_decimal_precision() {
        // Test decimal precision for negative numbers (reverse order)
        let close_values: &[f64] = &[
            -101.0, -100.999, -100.99, -100.9, -100.5, -100.1, -100.01, -100.001, -100.0,
        ];

        let encoded: Vec<Vec<u8>> = close_values
            .iter()
            .map(|&v| encode_value_ordered(&Value::Float(v)))
            .collect();

        for i in 0..(close_values.len() - 1) {
            assert!(
                encoded[i] < encoded[i + 1],
                "encode({}) should be < encode({})",
                close_values[i],
                close_values[i + 1]
            );
        }
    }

    #[test]
    fn test_numeric_special_floats() {
        // Test special float values
        let nan = encode_value_ordered(&Value::Float(f64::NAN));
        let pos_inf = encode_value_ordered(&Value::Float(f64::INFINITY));
        let neg_inf = encode_value_ordered(&Value::Float(f64::NEG_INFINITY));
        let max = encode_value_ordered(&Value::Float(f64::MAX));
        let min = encode_value_ordered(&Value::Float(f64::MIN));
        let zero = encode_value_ordered(&Value::Float(0.0));

        // Order: NEG_INFINITY < MIN < 0 < MAX < INFINITY < NAN
        assert!(neg_inf < min, "NEG_INFINITY should be < MIN");
        assert!(min < zero, "MIN should be < 0");
        assert!(zero < max, "0 should be < MAX");
        assert!(max < pos_inf, "MAX should be < INFINITY");
        assert!(pos_inf < nan, "INFINITY should be < NAN");
    }

    #[test]
    fn test_int_all_digit_lengths() {
        // Test integers of all digit lengths (1-19 digits)
        let test_values: Vec<i64> = vec![
            0, // special case
            1,
            9, // 1 digit
            10,
            99, // 2 digits
            100,
            999, // 3 digits
            1000,
            9999, // 4 digits
            10000,
            99999, // 5 digits
            100000,
            999999, // 6 digits
            1000000,
            9999999, // 7 digits
            10000000,
            99999999, // 8 digits
            100000000,
            999999999, // 9 digits
            1000000000,
            9999999999, // 10 digits
            10000000000,
            99999999999,
            100000000000,
            999999999999,
            1000000000000,
            9999999999999,
            10000000000000,
            99999999999999,
            100000000000000,
            999999999999999,
            1000000000000000,
            9999999999999999,
            10000000000000000,
            99999999999999999,
            100000000000000000,
            999999999999999999,
            1000000000000000000,
            i64::MAX, // 19 digits
        ];

        let encoded: Vec<Vec<u8>> = test_values
            .iter()
            .map(|&v| encode_value_ordered(&Value::Int(v)))
            .collect();

        // All should be in ascending order
        for i in 0..(test_values.len() - 1) {
            assert!(
                encoded[i] < encoded[i + 1],
                "encode({}) should be < encode({})",
                test_values[i],
                test_values[i + 1]
            );
        }

        // Test negative counterparts
        let neg_values: Vec<i64> = test_values
            .iter()
            .filter(|&&v| v > 0)
            .map(|&v| -v)
            .collect();

        let neg_encoded: Vec<Vec<u8>> = neg_values
            .iter()
            .map(|&v| encode_value_ordered(&Value::Int(v)))
            .collect();

        // Negative values should be in descending absolute value order
        // (more negative = smaller encoded value)
        for i in 0..(neg_values.len() - 1) {
            // -larger should be < -smaller
            if neg_values[i].abs() > neg_values[i + 1].abs() {
                assert!(
                    neg_encoded[i] < neg_encoded[i + 1],
                    "encode({}) should be < encode({}) (more negative is smaller)",
                    neg_values[i],
                    neg_values[i + 1]
                );
            }
        }
    }

    // =========================================================================
    // Type Comparison Tests (for hash encoding / equality)
    // =========================================================================

    #[test]
    fn test_different_types_not_equal() {
        // Different types should never produce equal hash encodings
        use std::collections::HashMap;

        let null = Value::Null;
        let bool_val = Value::Bool(false);
        let int_val = Value::Int(0);
        let float_val = Value::Float(0.1); // Not equal to Int(0)
        let string_val = Value::String("".to_string());
        let array_val = Value::Array(vec![]);
        let object_val = Value::Object(HashMap::new());

        let values = [
            ("Null", &null),
            ("Bool", &bool_val),
            ("Int", &int_val),
            ("Float", &float_val),
            ("String", &string_val),
            ("Array", &array_val),
            ("Object", &object_val),
        ];

        // All pairs of different types should have different encodings
        for i in 0..values.len() {
            for j in (i + 1)..values.len() {
                let enc_i = encode_value_for_hash(values[i].1);
                let enc_j = encode_value_for_hash(values[j].1);

                // Skip Int(0) vs Float(0.0) comparison - they're semantically equal
                if (values[i].0 == "Int" && values[j].0 == "Float")
                    || (values[i].0 == "Float" && values[j].0 == "Int")
                {
                    continue;
                }

                assert_ne!(
                    enc_i, enc_j,
                    "{} and {} should have different hash encodings",
                    values[i].0, values[j].0
                );
            }
        }
    }

    #[test]
    fn test_empty_array_not_equal_empty_object() {
        use std::collections::HashMap;

        let empty_array = Value::Array(vec![]);
        let empty_object = Value::Object(HashMap::new());

        let enc_array = encode_value_for_hash(&empty_array);
        let enc_object = encode_value_for_hash(&empty_object);

        assert_ne!(
            enc_array, enc_object,
            "Empty array and empty object should have different hash encodings"
        );

        // Verify they start with different tags
        assert_eq!(enc_array[0], TAG_ARRAY, "Array should have TAG_ARRAY");
        assert_eq!(enc_object[0], TAG_OBJECT, "Object should have TAG_OBJECT");
    }

    #[test]
    fn test_array_equality() {
        // Same arrays should encode identically
        let arr1 = Value::Array(vec![
            Value::Int(1),
            Value::String("hello".to_string()),
            Value::Bool(true),
        ]);
        let arr2 = Value::Array(vec![
            Value::Int(1),
            Value::String("hello".to_string()),
            Value::Bool(true),
        ]);

        assert_eq!(
            encode_value_for_hash(&arr1),
            encode_value_for_hash(&arr2),
            "Identical arrays should have same hash encoding"
        );

        // Different arrays should encode differently
        let arr3 = Value::Array(vec![
            Value::Int(1),
            Value::String("world".to_string()),
            Value::Bool(true),
        ]);

        assert_ne!(
            encode_value_for_hash(&arr1),
            encode_value_for_hash(&arr3),
            "Different arrays should have different hash encodings"
        );
    }

    #[test]
    fn test_array_order_matters() {
        // Array element order should affect encoding
        let arr1 = Value::Array(vec![Value::Int(1), Value::Int(2)]);
        let arr2 = Value::Array(vec![Value::Int(2), Value::Int(1)]);

        assert_ne!(
            encode_value_for_hash(&arr1),
            encode_value_for_hash(&arr2),
            "Arrays with different element order should have different encodings"
        );
    }

    #[test]
    fn test_array_length_matters() {
        let arr1 = Value::Array(vec![Value::Int(1)]);
        let arr2 = Value::Array(vec![Value::Int(1), Value::Int(1)]);

        assert_ne!(
            encode_value_for_hash(&arr1),
            encode_value_for_hash(&arr2),
            "Arrays with different lengths should have different encodings"
        );
    }

    #[test]
    fn test_nested_arrays() {
        let nested1 = Value::Array(vec![
            Value::Array(vec![Value::Int(1), Value::Int(2)]),
            Value::Array(vec![Value::Int(3)]),
        ]);
        let nested2 = Value::Array(vec![
            Value::Array(vec![Value::Int(1), Value::Int(2)]),
            Value::Array(vec![Value::Int(3)]),
        ]);
        let nested3 = Value::Array(vec![
            Value::Array(vec![Value::Int(1), Value::Int(2)]),
            Value::Array(vec![Value::Int(4)]), // Different!
        ]);

        assert_eq!(
            encode_value_for_hash(&nested1),
            encode_value_for_hash(&nested2),
            "Identical nested arrays should have same encoding"
        );

        assert_ne!(
            encode_value_for_hash(&nested1),
            encode_value_for_hash(&nested3),
            "Different nested arrays should have different encodings"
        );
    }

    #[test]
    fn test_object_equality() {
        use std::collections::HashMap;

        let mut obj1 = HashMap::new();
        obj1.insert("name".to_string(), Value::String("Alice".to_string()));
        obj1.insert("age".to_string(), Value::Int(30));

        let mut obj2 = HashMap::new();
        obj2.insert("age".to_string(), Value::Int(30));
        obj2.insert("name".to_string(), Value::String("Alice".to_string()));

        // Order of insertion shouldn't matter (keys are sorted)
        assert_eq!(
            encode_value_for_hash(&Value::Object(obj1.clone())),
            encode_value_for_hash(&Value::Object(obj2)),
            "Objects with same keys/values should have same encoding regardless of insertion order"
        );

        // Different values should differ
        let mut obj3 = HashMap::new();
        obj3.insert("name".to_string(), Value::String("Bob".to_string()));
        obj3.insert("age".to_string(), Value::Int(30));

        assert_ne!(
            encode_value_for_hash(&Value::Object(obj1)),
            encode_value_for_hash(&Value::Object(obj3)),
            "Objects with different values should have different encodings"
        );
    }

    #[test]
    fn test_object_keys_matter() {
        use std::collections::HashMap;

        let mut obj1 = HashMap::new();
        obj1.insert("a".to_string(), Value::Int(1));

        let mut obj2 = HashMap::new();
        obj2.insert("b".to_string(), Value::Int(1));

        assert_ne!(
            encode_value_for_hash(&Value::Object(obj1)),
            encode_value_for_hash(&Value::Object(obj2)),
            "Objects with different keys should have different encodings"
        );
    }

    #[test]
    fn test_nested_objects() {
        use std::collections::HashMap;

        let mut inner = HashMap::new();
        inner.insert("x".to_string(), Value::Int(10));

        let mut outer1 = HashMap::new();
        outer1.insert("inner".to_string(), Value::Object(inner.clone()));

        let mut outer2 = HashMap::new();
        outer2.insert("inner".to_string(), Value::Object(inner.clone()));

        assert_eq!(
            encode_value_for_hash(&Value::Object(outer1.clone())),
            encode_value_for_hash(&Value::Object(outer2)),
            "Identical nested objects should have same encoding"
        );

        // Different nested value
        let mut inner2 = HashMap::new();
        inner2.insert("x".to_string(), Value::Int(20));

        let mut outer3 = HashMap::new();
        outer3.insert("inner".to_string(), Value::Object(inner2));

        assert_ne!(
            encode_value_for_hash(&Value::Object(outer1)),
            encode_value_for_hash(&Value::Object(outer3)),
            "Objects with different nested values should have different encodings"
        );
    }

    #[test]
    fn test_mixed_array_object_nesting() {
        use std::collections::HashMap;

        // Array containing objects
        let mut obj = HashMap::new();
        obj.insert("id".to_string(), Value::Int(1));

        let arr_of_obj = Value::Array(vec![Value::Object(obj.clone())]);

        // Object containing array
        let mut obj_with_arr = HashMap::new();
        obj_with_arr.insert("items".to_string(), Value::Array(vec![Value::Int(1)]));

        assert_ne!(
            encode_value_for_hash(&arr_of_obj),
            encode_value_for_hash(&Value::Object(obj_with_arr)),
            "Array of objects and object with array should be different"
        );
    }

    #[test]
    fn test_numeric_equality_across_types() {
        // Int and Float with same mathematical value should be equal
        let int_100 = Value::Int(100);
        let float_100 = Value::Float(100.0);

        assert_eq!(
            encode_value_for_hash(&int_100),
            encode_value_for_hash(&float_100),
            "Int(100) and Float(100.0) should have same hash encoding"
        );

        // But Float(100.5) should differ
        let float_100_5 = Value::Float(100.5);
        assert_ne!(
            encode_value_for_hash(&int_100),
            encode_value_for_hash(&float_100_5),
            "Int(100) and Float(100.5) should have different hash encodings"
        );
    }

    #[test]
    fn test_deep_nesting() {
        use std::collections::HashMap;

        // Build deeply nested structure: {a: {b: {c: {d: {e: [1, 2, 3]}}}}}
        fn build_nested(depth: usize, leaf: Value) -> Value {
            if depth == 0 {
                leaf
            } else {
                let key = format!("level_{}", depth);
                let mut obj = HashMap::new();
                obj.insert(key, build_nested(depth - 1, leaf));
                Value::Object(obj)
            }
        }

        let deep1 = build_nested(
            7,
            Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        );

        let deep2 = build_nested(
            7,
            Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        );

        let deep3 = build_nested(
            7,
            Value::Array(vec![
                Value::Int(1),
                Value::Int(2),
                Value::Int(4), // Different leaf value
            ]),
        );

        // Same deep structure should be equal
        assert_eq!(
            encode_value_for_hash(&deep1),
            encode_value_for_hash(&deep2),
            "Identical deeply nested structures should have same encoding"
        );

        // Different leaf should make entire structure different
        assert_ne!(
            encode_value_for_hash(&deep1),
            encode_value_for_hash(&deep3),
            "Deeply nested structures with different leaves should differ"
        );

        // Different depth should differ
        let deep4 = build_nested(
            6,
            Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        );

        assert_ne!(
            encode_value_for_hash(&deep1),
            encode_value_for_hash(&deep4),
            "Structures with different nesting depth should differ"
        );
    }
}
