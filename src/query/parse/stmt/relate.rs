//! RELATE statement parser.

use pest::iterators::Pair;

use crate::query::ast::{ObjectLiteral, RelateAst, RelateData, RelateSource, ReturnClause, Target};
use crate::query::error::ParseError;
use crate::query::parse::value::Rule;
use crate::query::parse::{parse_collection, parse_ident, semantic_children};

use super::common::parse_assignments;
use super::select::{parse_projection_item_list, parse_select};

/// Parse RELATE statement
pub fn parse_relate(pair: Pair<Rule>) -> Result<RelateAst, ParseError> {
    // Use semantic_children to filter out keyword rules (like kw_RELATE)
    let children = semantic_children(pair);
    let mut iter = children.into_iter();

    // Parse: relate_source -> label -> relate_source [relate_data] [return_clause]
    let from = parse_relate_source(iter.next().expect("grammar"))?;

    // The label is an ident between the arrows
    let label = parse_ident(iter.next().expect("grammar"));

    let to = parse_relate_source(iter.next().expect("grammar"))?;

    let mut data = None;
    let mut return_clause = ReturnClause::After;

    for part in iter {
        match part.as_rule() {
            Rule::relate_data => {
                data = Some(parse_relate_data(part)?);
            }
            Rule::return_clause => {
                return_clause = parse_return_clause(part)?;
            }
            _ => {}
        }
    }

    Ok(RelateAst {
        from,
        label,
        to,
        data,
        return_clause,
    })
}

fn parse_relate_source(pair: Pair<Rule>) -> Result<RelateSource, ParseError> {
    let inner = pair.into_inner().next().expect("grammar");
    match inner.as_rule() {
        Rule::relate_target => Ok(RelateSource::Record(parse_relate_target(inner)?)),
        Rule::relate_array => {
            let targets: Result<Vec<Target>, _> = inner
                .into_inner()
                .filter(|p| p.as_rule() == Rule::relate_target)
                .map(parse_relate_target)
                .collect();
            Ok(RelateSource::Array(targets?))
        }
        Rule::relate_subquery => {
            let select_pair = inner.into_inner().next().expect("grammar");
            Ok(RelateSource::Subquery(Box::new(parse_select(select_pair)?)))
        }
        _ => Err(ParseError::UnexpectedToken(format!(
            "unexpected relate_source rule: {:?}",
            inner.as_rule()
        ))),
    }
}

fn parse_relate_target(pair: Pair<Rule>) -> Result<Target, ParseError> {
    // relate_target = { collection ~ (":" ~ relate_key)? }
    let mut collection = String::new();
    let mut key = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::collection => collection = parse_collection(inner),
            Rule::relate_key => key = Some(parse_relate_key(inner)),
            _ => {}
        }
    }

    Ok(Target { collection, key })
}

fn parse_relate_key(pair: Pair<Rule>) -> String {
    // relate_key = { quoted_key | relate_unquoted_key }
    let inner = pair.into_inner().next().expect("grammar");
    match inner.as_rule() {
        Rule::quoted_key => {
            // Strip surrounding quotes
            let s = inner.as_str();
            s[1..s.len() - 1].to_string()
        }
        Rule::relate_unquoted_key => inner.as_str().to_string(),
        _ => inner.as_str().to_string(),
    }
}

fn parse_relate_data(pair: Pair<Rule>) -> Result<RelateData, ParseError> {
    let inner = pair.into_inner().next().expect("grammar");
    match inner.as_rule() {
        Rule::relate_set => {
            let assignments = parse_assignments(
                inner
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::assignments)
                    .expect("grammar"),
            )?;
            Ok(RelateData::Set(assignments))
        }
        Rule::relate_content => {
            let obj_pair = inner
                .into_inner()
                .find(|p| p.as_rule() == Rule::object)
                .expect("grammar");
            let obj = parse_object_literal(obj_pair)?;
            Ok(RelateData::Content(obj))
        }
        _ => Err(ParseError::UnexpectedToken(
            "unexpected relate_data rule".into(),
        )),
    }
}

fn parse_object_literal(pair: Pair<Rule>) -> Result<ObjectLiteral, ParseError> {
    // Delegate to value::parse_object
    crate::query::parse::value::parse_object(pair)
}

fn parse_return_clause(pair: Pair<Rule>) -> Result<ReturnClause, ParseError> {
    let inner = pair
        .into_inner()
        .find(|p| p.as_rule() == Rule::return_value)
        .expect("grammar");
    let value = inner.into_inner().next().expect("grammar");

    match value.as_rule() {
        Rule::return_none => Ok(ReturnClause::None),
        Rule::return_before => Ok(ReturnClause::Before),
        Rule::return_after => Ok(ReturnClause::After),
        Rule::projection_item_list => {
            let items = parse_projection_item_list(value)?;
            Ok(ReturnClause::Fields(items))
        }
        _ => Err(ParseError::UnexpectedToken(
            "unexpected return_value rule".into(),
        )),
    }
}
