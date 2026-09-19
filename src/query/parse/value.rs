use pest::iterators::Pair;
use std::collections::HashMap;

use crate::document::{Decimal, DecimalValue, Value};
use crate::query::ast::ObjectLiteral;
use crate::query::error::ParseError;

pub use super::Rule;

/// Decode JSON string escapes plus `\'`, which is supported by StellarQL's
/// single-quoted string literals.
pub(super) fn unescape_string(s: &str) -> Result<String, ParseError> {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some('\\') => result.push('\\'),
                Some('"') => result.push('"'),
                Some('\'') => result.push('\''),
                Some('/') => result.push('/'),
                Some('b') => result.push('\x08'),
                Some('f') => result.push('\x0C'),
                Some('u') => decode_unicode_escape(&mut chars, &mut result)?,
                Some(other) => {
                    return Err(ParseError::InvalidStringEscape(format!("\\{other}")));
                }
                None => return Err(ParseError::InvalidStringEscape("trailing backslash".into())),
            }
        } else {
            result.push(c);
        }
    }

    Ok(result)
}

fn decode_unicode_escape<I>(
    chars: &mut std::iter::Peekable<I>,
    result: &mut String,
) -> Result<(), ParseError>
where
    I: Iterator<Item = char>,
{
    let first = read_hex_quad(chars)?;

    let codepoint = if (0xD800..=0xDBFF).contains(&first) {
        match (chars.next(), chars.next()) {
            (Some('\\'), Some('u')) => {}
            _ => {
                return Err(ParseError::InvalidStringEscape(
                    "high surrogate must be followed by a low surrogate".into(),
                ));
            }
        }
        let second = read_hex_quad(chars)?;
        if !(0xDC00..=0xDFFF).contains(&second) {
            return Err(ParseError::InvalidStringEscape(
                "high surrogate must be followed by a low surrogate".into(),
            ));
        }
        0x10000 + (((first as u32 - 0xD800) << 10) | (second as u32 - 0xDC00))
    } else if (0xDC00..=0xDFFF).contains(&first) {
        return Err(ParseError::InvalidStringEscape(
            "unexpected low surrogate".into(),
        ));
    } else {
        first as u32
    };

    let decoded = char::from_u32(codepoint).ok_or_else(|| {
        ParseError::InvalidStringEscape(format!("invalid Unicode scalar U+{codepoint:04X}"))
    })?;
    result.push(decoded);
    Ok(())
}

fn read_hex_quad<I>(chars: &mut std::iter::Peekable<I>) -> Result<u16, ParseError>
where
    I: Iterator<Item = char>,
{
    let mut value = 0u16;
    for _ in 0..4 {
        let digit = chars.next().and_then(|c| c.to_digit(16)).ok_or_else(|| {
            ParseError::InvalidStringEscape("expected four hexadecimal digits after \\u".into())
        })?;
        value = (value << 4) | digit as u16;
    }
    Ok(value)
}

pub(super) fn parse_string_literal(pair: Pair<Rule>) -> Result<String, ParseError> {
    let s = pair.as_str();
    unescape_string(&s[1..s.len() - 1])
}

pub fn parse_value(pair: Pair<Rule>) -> Result<Value, ParseError> {
    let inner = pair.into_inner().next().expect("grammar");

    match inner.as_rule() {
        Rule::string => Ok(Value::String(parse_string_literal(inner)?)),
        Rule::number => {
            let inner = inner.into_inner().next().expect("grammar");
            match inner.as_rule() {
                Rule::decimal_number => {
                    let s = inner.as_str();
                    // Strip "dec" suffix (case insensitive)
                    let num_str = s
                        .strip_suffix("dec")
                        .or_else(|| s.strip_suffix("DEC"))
                        .or_else(|| s.strip_suffix("Dec"))
                        .unwrap_or(s);
                    let d: Decimal = num_str
                        .parse()
                        .map_err(|_| ParseError::InvalidNumber(s.to_string()))?;
                    Ok(Value::Decimal(DecimalValue::new(d)))
                }
                Rule::float_number => {
                    let s = inner.as_str();
                    // Strip "f" suffix if present
                    let num_str = s
                        .strip_suffix('f')
                        .or_else(|| s.strip_suffix('F'))
                        .unwrap_or(s);
                    let f: f64 = num_str
                        .parse()
                        .map_err(|_| ParseError::InvalidNumber(s.to_string()))?;
                    Ok(Value::Float(f))
                }
                Rule::int_number => {
                    let s = inner.as_str();
                    let i: i64 = s
                        .parse()
                        .map_err(|_| ParseError::InvalidNumber(s.to_string()))?;
                    Ok(Value::Int(i))
                }
                _ => unreachable!("Unexpected number rule: {:?}", inner.as_rule()),
            }
        }
        Rule::boolean => Ok(Value::Bool(inner.as_str().to_lowercase() == "true")),
        Rule::null => Ok(Value::Null),
        Rule::array => {
            let mut items = Vec::new();
            for item in inner.into_inner() {
                if item.as_rule() == Rule::value {
                    items.push(parse_value(item)?);
                }
            }
            Ok(Value::Array(items))
        }
        Rule::object => {
            let obj = parse_object(inner)?;
            let map: HashMap<String, Value> = obj.fields.into_iter().collect();
            Ok(Value::Object(map))
        }
        Rule::reference_literal => {
            // Format: collection:key (e.g., "user:alice")
            let s = inner.as_str();
            Ok(Value::Reference(s.to_string()))
        }
        _ => unreachable!(),
    }
}

pub fn parse_object(pair: Pair<Rule>) -> Result<ObjectLiteral, ParseError> {
    let mut fields = Vec::new();

    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::object_pairs {
            for pair_inner in inner.into_inner() {
                if pair_inner.as_rule() == Rule::object_pair {
                    fields.push(parse_object_pair(pair_inner)?);
                }
            }
        }
    }

    Ok(ObjectLiteral { fields })
}

pub fn parse_object_pair(pair: Pair<Rule>) -> Result<(String, Value), ParseError> {
    let mut key = String::new();
    let mut value = Value::Null;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::ident => key = super::parse_ident(inner),
            Rule::string => {
                key = parse_string_literal(inner)?;
            }
            Rule::value => value = parse_value(inner)?,
            _ => {}
        }
    }

    Ok((key, value))
}

pub fn parse_object_list(pair: Pair<Rule>) -> Result<Vec<ObjectLiteral>, ParseError> {
    let mut objects = Vec::new();

    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::object {
            objects.push(parse_object(inner)?);
        }
    }

    Ok(objects)
}

pub fn parse_limit(pair: Pair<Rule>) -> Result<usize, ParseError> {
    let integer = pair
        .into_inner()
        .find(|p| p.as_rule() == Rule::integer)
        .expect("grammar");
    integer
        .as_str()
        .parse()
        .map_err(|_| ParseError::InvalidNumber(integer.as_str().to_string()))
}
