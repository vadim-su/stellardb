//! DDL statement parsing: DROP COLLECTION, DESCRIBE, CREATE/DROP INDEX, REINDEX, EXPLAIN.

use pest::iterators::Pair;

use super::super::value::Rule;
use super::super::{parse_collection, parse_field_path};
use super::dml::{parse_delete, parse_update};
use super::schema::parse_hnsw_params;
use super::select::parse_select;
use crate::query::ast::{
    CreateIndexAst, DescribeAst, DropCollectionAst, DropIndexAst, ExplainAst, ReindexAst, Statement,
};
use crate::query::error::ParseError;

/// Parse DROP COLLECTION statement.
pub fn parse_drop_collection(pair: Pair<Rule>) -> Result<Statement, ParseError> {
    let mut inner = super::super::semantic_children(pair).into_iter();
    let name = super::super::parse_ident(inner.next().expect("grammar"));
    let cascade = inner.next().is_some(); // cascade_mod present?

    Ok(Statement::DropCollection(DropCollectionAst {
        name,
        cascade,
    }))
}

/// Parse DESCRIBE statement.
pub fn parse_describe(pair: Pair<Rule>) -> Result<Statement, ParseError> {
    let inner = super::super::semantic_children(pair)
        .into_iter()
        .next()
        .expect("grammar");
    match inner.as_rule() {
        Rule::describe_collection => {
            // describe_collection = { kw_COLLECTION ~ ident }
            let ident = super::super::semantic_children(inner)
                .into_iter()
                .next()
                .expect("grammar");
            Ok(Statement::Describe(DescribeAst::Collection(
                super::super::parse_ident(ident),
            )))
        }
        Rule::describe_collections => Ok(Statement::Describe(DescribeAst::Collections)),
        _ => unreachable!("grammar guarantees describe_collection or describe_collections"),
    }
}

/// Parse CREATE INDEX statement.
pub(super) fn parse_create_index(pair: Pair<Rule>) -> Result<CreateIndexAst, ParseError> {
    let mut collection = String::new();
    let mut fields = Vec::new();
    let mut unique = false;
    let mut index_type = crate::schema::IndexType::BTree;
    let mut hnsw_params: Option<crate::schema::HnswParams> = None;
    let mut analyzer: Option<String> = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::collection => collection = parse_collection(inner),
            Rule::index_fields => {
                for field in inner.into_inner() {
                    if field.as_rule() == Rule::field_path {
                        fields.push(parse_field_path(field));
                    }
                }
            }
            Rule::index_options => {
                // index_options contains unique_mod, fulltext_mod, or hnsw_mod
                if let Some(opt) = inner.into_inner().next() {
                    match opt.as_rule() {
                        Rule::unique_mod => unique = true,
                        Rule::fulltext_mod => {
                            index_type = crate::schema::IndexType::FullText;
                            for child in opt.into_inner() {
                                if child.as_rule() == Rule::analyzer_clause {
                                    // analyzer_clause = { kw_ANALYZER ~ ident }
                                    // Find the ident child (skip keyword)
                                    if let Some(name) =
                                        child.into_inner().find(|p| p.as_rule() == Rule::ident)
                                    {
                                        analyzer = Some(super::super::parse_ident(name));
                                    }
                                }
                            }
                        }
                        Rule::hnsw_mod => {
                            index_type = crate::schema::IndexType::Hnsw;
                            // Find the hnsw_params child within hnsw_mod
                            if let Some(params_pair) =
                                opt.into_inner().find(|p| p.as_rule() == Rule::hnsw_params)
                            {
                                hnsw_params = Some(parse_hnsw_params(params_pair));
                            }
                        }
                        _ => {}
                    }
                }
            }
            Rule::unique_mod => unique = true,
            Rule::fulltext_mod => {
                index_type = crate::schema::IndexType::FullText;
                for child in inner.into_inner() {
                    if child.as_rule() == Rule::analyzer_clause {
                        // analyzer_clause = { kw_ANALYZER ~ ident }
                        // Find the ident child (skip keyword)
                        if let Some(name) = child.into_inner().find(|p| p.as_rule() == Rule::ident)
                        {
                            analyzer = Some(super::super::parse_ident(name));
                        }
                    }
                }
            }
            Rule::hnsw_mod => {
                index_type = crate::schema::IndexType::Hnsw;
                // Find the hnsw_params child within hnsw_mod
                if let Some(params_pair) = inner
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::hnsw_params)
                {
                    hnsw_params = Some(parse_hnsw_params(params_pair));
                }
            }
            _ => {}
        }
    }

    Ok(CreateIndexAst {
        collection,
        fields,
        unique,
        index_type,
        hnsw_params,
        analyzer,
    })
}

/// Parse DROP INDEX statement.
pub(super) fn parse_drop_index(pair: Pair<Rule>) -> Result<DropIndexAst, ParseError> {
    // Grammar: DROP INDEX name ON collection
    let mut name = String::new();
    let mut collection = String::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::ident => name = super::super::parse_ident(inner),
            Rule::collection => collection = parse_collection(inner),
            _ => {}
        }
    }

    Ok(DropIndexAst { name, collection })
}

/// Parse REINDEX statement.
pub(super) fn parse_reindex(pair: Pair<Rule>) -> Result<Statement, ParseError> {
    // Grammar: REINDEX name ON collection
    let mut name = String::new();
    let mut collection = String::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::ident => name = super::super::parse_ident(inner),
            Rule::collection => collection = parse_collection(inner),
            _ => {}
        }
    }

    Ok(Statement::Reindex(ReindexAst { name, collection }))
}

/// Parse EXPLAIN statement.
pub(super) fn parse_explain(pair: Pair<Rule>) -> Result<ExplainAst, ParseError> {
    let mut analyze = false;
    let mut inner_stmt = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::analyze_flag => analyze = true,
            Rule::explainable_stmt => {
                // explainable_stmt contains one of: select_stmt | update_stmt | delete_stmt
                let stmt_pair = inner.into_inner().next().expect("grammar");
                let stmt = match stmt_pair.as_rule() {
                    Rule::select_stmt => Statement::Select(parse_select(stmt_pair)?),
                    Rule::update_stmt => Statement::Update(parse_update(stmt_pair)?),
                    Rule::delete_stmt => Statement::Delete(parse_delete(stmt_pair)?),
                    _ => unreachable!("grammar guarantees explainable statement"),
                };
                inner_stmt = Some(stmt);
            }
            _ => {}
        }
    }

    Ok(ExplainAst {
        analyze,
        statement: Box::new(inner_stmt.expect("grammar guarantees explainable statement")),
    })
}
