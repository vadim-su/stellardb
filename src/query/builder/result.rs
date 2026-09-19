use crate::query::ast::{DeleteTarget, Statement};
use crate::query::execute::ExecuteResult;
use crate::query::result::{BatchError, BatchResult, StatementResult};

pub(super) fn statement_type(stmt: &Statement) -> &'static str {
    match stmt {
        Statement::Select(_) => "SELECT",
        Statement::Insert(_) => "INSERT",
        Statement::Create(_) => "CREATE",
        Statement::Update(_) => "UPDATE",
        Statement::Upsert(_) => "UPSERT",
        Statement::Delete(_) => "DELETE",
        Statement::Begin(_) => "BEGIN",
        Statement::Commit(_) => "COMMIT",
        Statement::Rollback(_) => "ROLLBACK",
        Statement::DefineCollection(_) => "DEFINE COLLECTION",
        Statement::DropCollection(_) => "DROP COLLECTION",
        Statement::Describe(_) => "DESCRIBE",
        Statement::CreateIndex(_) => "CREATE INDEX",
        Statement::DropIndex(_) => "DROP INDEX",
        Statement::Explain(_) => "EXPLAIN",
        Statement::Let(_) => "LET",
        Statement::Reindex(_) => "REINDEX",
        Statement::DefineAnalyzer(_) => "DEFINE ANALYZER",
        Statement::DropAnalyzer(_) => "DROP ANALYZER",
        Statement::Relate(_) => "RELATE",
        Statement::Set(_) => "SET",
        Statement::Expr(_) => "EXPR",
        Statement::CreateUser(_) => "CREATE USER",
        Statement::DropUser(_) => "DROP USER",
        Statement::AlterUser(_) => "ALTER USER",
        Statement::CreateApiKey(_) => "CREATE API KEY",
        Statement::DropApiKey(_) => "DROP API KEY",
        Statement::CreatePolicy(_) => "CREATE POLICY",
        Statement::DropPolicy(_) => "DROP POLICY",
        Statement::AlterCollectionAttrs(_) => "ALTER COLLECTION",
    }
}

pub(super) fn make_statement_result(
    index: usize,
    stmt: &Statement,
    result: ExecuteResult,
) -> StatementResult {
    let stmt_type = statement_type(stmt);
    match stmt {
        Statement::Delete(d) if matches!(d.target, DeleteTarget::Document(_)) => {
            let count = result.rows.len() as u64;
            StatementResult::affected(index, stmt_type, count)
        }
        Statement::Delete(_) => StatementResult::data(index, stmt_type, result.rows),
        _ => StatementResult::data(index, stmt_type, result.rows),
    }
}

pub(super) fn error_result(
    results: Vec<StatementResult>,
    statement_index: usize,
    error: BatchError,
) -> BatchResult {
    BatchResult {
        results,
        completed: statement_index,
        error: Some(error),
    }
}

pub(super) fn query_error_result(
    results: Vec<StatementResult>,
    statement_index: usize,
    error: crate::query::QueryError,
) -> BatchResult {
    let error = BatchError::from_query_error(statement_index, &error);
    error_result(results, statement_index, error)
}
