//! Statement parsing module.
//!
//! This module parses statement AST nodes from pest parse trees.

mod analyzer;
mod auth;
mod common;
mod ddl;
mod dml;
mod relate;
mod schema;
mod select;

use pest::iterators::Pair;

use super::value::Rule;
use crate::query::ast::{BeginAst, CommitAst, RollbackAst, Statement};
use crate::query::error::ParseError;

// Re-exports
pub use ddl::{parse_describe, parse_drop_collection};
pub use schema::parse_define_collection;
pub use select::parse_select_from_pair;

/// Parse a list of statements.
pub fn parse_statement_list(pair: Pair<Rule>) -> Result<Vec<Statement>, ParseError> {
    let mut statements = Vec::new();

    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::statement {
            statements.push(parse_statement(inner)?);
        }
    }

    Ok(statements)
}

/// Parse a single statement.
pub fn parse_statement(pair: Pair<Rule>) -> Result<Statement, ParseError> {
    let inner = pair.into_inner().next().expect("grammar");
    match inner.as_rule() {
        Rule::select_stmt => Ok(Statement::Select(select::parse_select(inner)?)),
        Rule::insert_stmt => Ok(Statement::Insert(dml::parse_insert(inner)?)),
        Rule::create_stmt => Ok(Statement::Create(dml::parse_create(inner)?)),
        Rule::update_stmt => Ok(Statement::Update(dml::parse_update(inner)?)),
        Rule::upsert_stmt => Ok(Statement::Upsert(dml::parse_upsert(inner)?)),
        Rule::delete_stmt => Ok(Statement::Delete(dml::parse_delete(inner)?)),
        Rule::begin_stmt => Ok(Statement::Begin(BeginAst)),
        Rule::commit_stmt => Ok(Statement::Commit(CommitAst)),
        Rule::rollback_stmt => Ok(Statement::Rollback(RollbackAst)),
        Rule::define_collection_stmt => schema::parse_define_collection(inner),
        Rule::drop_collection_stmt => ddl::parse_drop_collection(inner),
        Rule::describe_stmt => ddl::parse_describe(inner),
        Rule::create_index_stmt => Ok(Statement::CreateIndex(ddl::parse_create_index(inner)?)),
        Rule::drop_index_stmt => Ok(Statement::DropIndex(ddl::parse_drop_index(inner)?)),
        Rule::reindex_stmt => ddl::parse_reindex(inner),
        Rule::explain_stmt => Ok(Statement::Explain(ddl::parse_explain(inner)?)),
        Rule::let_stmt => Ok(Statement::Let(select::parse_let(inner)?)),
        Rule::define_analyzer_stmt => Ok(Statement::DefineAnalyzer(
            analyzer::parse_define_analyzer(inner)?,
        )),
        Rule::drop_analyzer_stmt => Ok(Statement::DropAnalyzer(analyzer::parse_drop_analyzer(
            inner,
        )?)),
        Rule::relate_stmt => Ok(Statement::Relate(relate::parse_relate(inner)?)),
        Rule::set_stmt => Ok(Statement::Set(dml::parse_set(inner)?)),
        Rule::create_user_stmt => Ok(Statement::CreateUser(auth::parse_create_user(inner)?)),
        Rule::drop_user_stmt => Ok(Statement::DropUser(auth::parse_drop_user(inner)?)),
        Rule::alter_user_stmt => auth::parse_alter_user(inner),
        Rule::create_api_key_stmt => {
            Ok(Statement::CreateApiKey(auth::parse_create_api_key(inner)?))
        }
        Rule::drop_api_key_stmt => Ok(Statement::DropApiKey(auth::parse_drop_api_key(inner)?)),
        Rule::create_policy_stmt => Ok(Statement::CreatePolicy(auth::parse_create_policy(inner)?)),
        Rule::drop_policy_stmt => Ok(Statement::DropPolicy(auth::parse_drop_policy(inner)?)),
        Rule::alter_collection_attrs_stmt => auth::parse_alter_collection_attrs(inner),
        Rule::expr_stmt => {
            let expr_pair = inner.into_inner().next().expect("grammar");
            Ok(Statement::Expr(super::expr::parse_expr(expr_pair)?))
        }
        _ => unreachable!("unknown statement rule"),
    }
}

#[cfg(test)]
mod tests;
