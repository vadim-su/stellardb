// src/query/test_helpers.rs

/// Helper macro for running single SQL statements in tests.
/// Usage: run_sql!(storage, "SELECT * FROM users")
/// `storage` should be an `Arc<Database>` — the macro calls `.clone()` on it.
#[macro_export]
macro_rules! run_sql {
    ($storage:expr, $sql:expr) => {{
        use $crate::query::{ExecuteContext, Query};
        Query::parse($sql)
            .expect("parse failed")
            .bind($storage.clone())
            .expect("bind failed")
            .execute(&ExecuteContext::none())
            .expect("execute failed")
            .single()
            .expect("expected single result")
    }};
}

/// Helper for tests that need Result instead of panic.
/// `storage` should be an `Arc<Database>` — the macro calls `.clone()` on it.
#[macro_export]
macro_rules! try_run_sql {
    ($storage:expr, $sql:expr) => {{
        use $crate::query::error::ExecuteError;
        use $crate::query::{ExecuteContext, Query, QueryError};
        (|| -> Result<$crate::query::execute::ExecuteResult, QueryError> {
            let batch_result = Query::parse($sql)?
                .bind($storage.clone())?
                .execute(&ExecuteContext::none())?;

            // Check for batch error
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

            let stmt_result = batch_result.results.into_iter().next().unwrap();
            Ok($crate::query::execute::ExecuteResult::new(stmt_result.rows))
        })()
    }};
}
