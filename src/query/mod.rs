pub mod ast;
pub mod bind;
pub mod error;
pub mod execute;
pub mod function;
pub mod optimize;
pub mod parse;
pub mod plan;
pub mod result;

pub mod builder;

#[cfg(test)]
mod batch_test;

mod test_helpers;

pub use ast::{Expr, Projection, Statement};
pub use bind::bind_statement;
pub use error::{ExecuteError, ParseError, QueryError};
pub use execute::{ExecuteResult, execute, execute_ddl};
pub use function::{AggregateFunction, Registry as FnRegistry};
pub use parse::parse;
pub use plan::LogicalPlan;
pub use result::{BatchError, BatchResult, StatementResult};

pub use builder::{ExecuteContext, Query};

/// Runs one query and converts its result to JSON for in-crate tests.
#[cfg(test)]
pub fn query_json(
    storage: std::sync::Arc<crate::Database>,
    sql: &str,
) -> Result<serde_json::Value, QueryError> {
    use crate::query::execute::operators::operator::rows_to_json;
    let batch_result = Query::parse(sql)?
        .bind(storage)?
        .execute(&ExecuteContext::none())?;

    // Check for batch error and convert to QueryError
    if let Some(err) = batch_result.error {
        return Err(QueryError::Execute(ExecuteError::Internal(err.message)));
    }

    // Extract single result
    if batch_result.results.len() != 1 {
        return Err(QueryError::Execute(ExecuteError::Internal(format!(
            "expected single statement, got {}",
            batch_result.results.len()
        ))));
    }

    let stmt_result = batch_result
        .results
        .into_iter()
        .next()
        .expect("length checked to be 1 above");
    Ok(rows_to_json(&stmt_result.rows))
}
