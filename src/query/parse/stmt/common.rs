//! Common parsing helpers used across statement types.

use pest::iterators::Pair;

use super::super::expr::parse_expr;
use super::super::value::Rule;
use super::super::{parse_collection, parse_field_path_segments};
use crate::query::ast::{Assignment, Target};
use crate::query::error::ParseError;

/// Parse a target (collection:key).
pub(super) fn parse_target(pair: Pair<Rule>) -> Target {
    let mut collection = String::new();
    let mut key = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::collection => collection = parse_collection(inner),
            Rule::key => key = Some(parse_key(inner)),
            _ => {}
        }
    }

    Target { collection, key }
}

/// Parse a key, handling quoted keys by stripping quotes.
pub(super) fn parse_key(pair: Pair<Rule>) -> String {
    // key = { quoted_key | unquoted_key }
    let inner = pair.into_inner().next().expect("grammar");
    match inner.as_rule() {
        Rule::quoted_key => {
            // Strip surrounding quotes
            let s = inner.as_str();
            s[1..s.len() - 1].to_string()
        }
        Rule::unquoted_key => inner.as_str().to_string(),
        _ => inner.as_str().to_string(),
    }
}

/// Parse assignments list.
pub(super) fn parse_assignments(pair: Pair<Rule>) -> Result<Vec<Assignment>, ParseError> {
    let mut assignments = Vec::new();

    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::assignment {
            assignments.push(parse_assignment(inner)?);
        }
    }

    Ok(assignments)
}

/// Parse a single assignment.
pub(super) fn parse_assignment(pair: Pair<Rule>) -> Result<Assignment, ParseError> {
    let mut path = Vec::new();
    let mut expr = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::field_path => {
                path = parse_field_path_segments(inner);
            }
            Rule::expr => expr = Some(parse_expr(inner)?),
            _ => {}
        }
    }

    Ok(Assignment {
        path,
        expr: expr.ok_or(ParseError::MissingExpression)?,
    })
}
