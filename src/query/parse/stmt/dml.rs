//! DML statement parsing: INSERT, CREATE, UPDATE, DELETE.

use pest::iterators::Pair;

use super::super::expr::parse_where;
use super::super::value::{Rule, parse_object_list};
use super::common::{parse_assignments, parse_target};
use crate::query::ast::{
    CreateAst, DeleteAst, DeleteTarget, InsertAst, SetAst, Target, UpdateAst, UpsertAst,
};
use crate::query::error::ParseError;
use crate::query::parse::{parse_collection, parse_ident};

/// Parse INSERT statement.
pub(super) fn parse_insert(pair: Pair<Rule>) -> Result<InsertAst, ParseError> {
    let mut collection = String::new();
    let mut objects = Vec::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::collection => collection = parse_collection(inner),
            Rule::object_list => objects = parse_object_list(inner)?,
            _ => {}
        }
    }

    Ok(InsertAst {
        collection,
        objects,
    })
}

/// Parse CREATE statement.
pub(super) fn parse_create(pair: Pair<Rule>) -> Result<CreateAst, ParseError> {
    let mut target = Target {
        collection: String::new(),
        key: None,
    };
    let mut assignments = Vec::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::target => target = parse_target(inner),
            Rule::assignments => assignments = parse_assignments(inner)?,
            _ => {}
        }
    }

    Ok(CreateAst {
        target,
        assignments,
    })
}

/// Parse UPDATE statement.
pub(super) fn parse_update(pair: Pair<Rule>) -> Result<UpdateAst, ParseError> {
    let mut target = Target {
        collection: String::new(),
        key: None,
    };
    let mut assignments = Vec::new();
    let mut filter = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::target => target = parse_target(inner),
            Rule::assignments => assignments = parse_assignments(inner)?,
            Rule::where_clause => filter = Some(parse_where(inner)?),
            _ => {}
        }
    }

    Ok(UpdateAst {
        target,
        filter,
        assignments,
    })
}

/// Parse UPSERT statement.
pub(super) fn parse_upsert(pair: Pair<Rule>) -> Result<UpsertAst, ParseError> {
    let mut target = Target {
        collection: String::new(),
        key: None,
    };
    let mut assignments = Vec::new();
    let mut replace = false;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::target => target = parse_target(inner),
            Rule::kw_REPLACE => replace = true,
            Rule::assignments => assignments = parse_assignments(inner)?,
            _ => {}
        }
    }

    if target.key.is_none() {
        return Err(ParseError::UnexpectedToken(
            "UPSERT requires an explicit key (e.g., UPSERT user:alice SET ...)".to_string(),
        ));
    }

    Ok(UpsertAst {
        target,
        assignments,
        replace,
    })
}

/// Parse DELETE statement.
pub(super) fn parse_delete(pair: Pair<Rule>) -> Result<DeleteAst, ParseError> {
    let mut target = None;
    let mut filter = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::delete_edge_target => {
                target = Some(parse_delete_edge_target(inner)?);
            }
            Rule::target => {
                target = Some(DeleteTarget::Document(parse_target(inner)));
            }
            Rule::where_clause => filter = Some(parse_where(inner)?),
            _ => {}
        }
    }

    Ok(DeleteAst {
        target: target.unwrap_or(DeleteTarget::Document(Target {
            collection: String::new(),
            key: None,
        })),
        filter,
    })
}

/// Parse edge deletion target: user:alice->follows->user:bob or user:alice->follows
fn parse_delete_edge_target(pair: Pair<Rule>) -> Result<DeleteTarget, ParseError> {
    let mut inner_iter = pair.into_inner();

    // Parse the 'from' target (relate_target)
    let from = parse_relate_target(inner_iter.next().expect("grammar"))?;

    // Parse the edge label (ident)
    let label = parse_ident(inner_iter.next().expect("grammar"));

    // Parse the optional 'to' target (relate_target)
    let to = inner_iter.next().map(parse_relate_target).transpose()?;

    Ok(DeleteTarget::Edge { from, label, to })
}

/// Parse a relate_target: collection:key (uses relate_key which doesn't consume ->)
fn parse_relate_target(pair: Pair<Rule>) -> Result<Target, ParseError> {
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

/// Parse relate_key: quoted_key | relate_unquoted_key
fn parse_relate_key(pair: Pair<Rule>) -> String {
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

/// Parse SET statement (e.g. SET QUERY_TIMEOUT = 5s).
pub(super) fn parse_set(pair: Pair<Rule>) -> Result<SetAst, ParseError> {
    let mut inner = pair.into_inner();
    inner.next(); // kw_SET
    inner.next(); // kw_QUERY_TIMEOUT
    let value_pair = inner.next().expect("grammar");

    let value_nanos = match value_pair.as_rule() {
        Rule::duration_literal => super::super::expr::parse_duration_nanos(value_pair.as_str())?,
        Rule::number => {
            let n: i64 = value_pair
                .as_str()
                .parse()
                .map_err(|_| ParseError::InvalidDuration(value_pair.as_str().to_string()))?;
            if n != 0 {
                return Err(ParseError::InvalidDuration(
                    "SET QUERY_TIMEOUT value must be a duration (e.g. 5s) or 0 to disable"
                        .to_string(),
                ));
            }
            0
        }
        _ => unreachable!("grammar ensures duration_literal or number"),
    };

    Ok(SetAst {
        key: "QUERY_TIMEOUT".to_string(),
        value_nanos,
    })
}
