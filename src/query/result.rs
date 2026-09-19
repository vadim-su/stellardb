use crate::query::execute::{ExecuteResult, Row};

/// Result of executing a single statement
#[derive(Debug, Clone)]
pub struct StatementResult {
    pub statement_index: usize,
    pub statement_type: String,
    pub rows: Vec<Row>,
    pub rows_affected: Option<u64>,
    pub session_id: Option<String>,
}

/// Safe, structured error that occurred during batch execution.
#[derive(Debug, Clone)]
pub struct BatchError {
    pub statement_index: usize,
    pub code: &'static str,
    pub class: crate::error::ErrorClass,
    pub message: String,
}

impl BatchError {
    pub fn from_query_error(statement_index: usize, error: &crate::query::QueryError) -> Self {
        use crate::error::ErrorCode;

        if let crate::query::QueryError::Execute(error) = error {
            return Self::from_execute_error(statement_index, error);
        }

        let class = crate::error::query_error_class(error);
        if class == crate::error::ErrorClass::Internal {
            tracing::error!(
                error.code = error.code(),
                error.source = %error,
                statement.index = statement_index,
                "batch statement failed"
            );
        }
        Self {
            statement_index,
            code: error.code(),
            class,
            message: crate::error::safe_query_error_message(error),
        }
    }

    pub fn from_execute_error(statement_index: usize, error: &crate::query::ExecuteError) -> Self {
        use crate::error::ErrorCode;

        if let crate::query::ExecuteError::Storage(error) = error {
            return Self::from_storage_error(statement_index, error);
        }

        let class = crate::error::execute_error_class(error);
        if class == crate::error::ErrorClass::Internal {
            tracing::error!(
                error.code = error.code(),
                error.source = %error,
                statement.index = statement_index,
                "batch statement failed"
            );
        }
        Self {
            statement_index,
            code: error.code(),
            class,
            message: crate::error::safe_execute_error_message(error),
        }
    }

    pub fn from_storage_error(statement_index: usize, error: &crate::error::StorageError) -> Self {
        use crate::error::ErrorCode;

        let class = crate::error::storage_error_class(error);
        if class == crate::error::ErrorClass::Internal {
            tracing::error!(
                error.code = error.code(),
                error.source = %error,
                statement.index = statement_index,
                "batch statement failed"
            );
        }
        Self {
            statement_index,
            code: error.code(),
            class,
            message: crate::error::safe_storage_error_message(error),
        }
    }

    #[cfg(feature = "auth")]
    pub fn from_auth_error(statement_index: usize, error: &crate::auth::AuthError) -> Self {
        if error.class() == crate::error::ErrorClass::Internal {
            tracing::error!(
                error.code = error.code(),
                error.source = %error,
                statement.index = statement_index,
                "auth management statement failed"
            );
        }
        Self {
            statement_index,
            code: error.code(),
            class: error.class(),
            message: error.safe_message(),
        }
    }

    pub fn public(
        statement_index: usize,
        code: &'static str,
        class: crate::error::ErrorClass,
        message: impl Into<String>,
    ) -> Self {
        Self {
            statement_index,
            code,
            class,
            message: message.into(),
        }
    }

    pub fn internal(
        statement_index: usize,
        code: &'static str,
        source: impl std::fmt::Display,
    ) -> Self {
        tracing::error!(
            error.code = code,
            error.source = %source,
            statement.index = statement_index,
            "batch statement failed"
        );
        Self::public(
            statement_index,
            code,
            crate::error::ErrorClass::Internal,
            "Internal error",
        )
    }
}

/// Result of executing a batch of statements
#[derive(Debug, Clone)]
pub struct BatchResult {
    pub results: Vec<StatementResult>,
    pub completed: usize,
    pub error: Option<BatchError>,
}

impl StatementResult {
    pub fn data(index: usize, stmt_type: &str, rows: Vec<Row>) -> Self {
        Self {
            statement_index: index,
            statement_type: stmt_type.to_string(),
            rows,
            rows_affected: None,
            session_id: None,
        }
    }

    pub fn empty(index: usize, stmt_type: &str) -> Self {
        Self {
            statement_index: index,
            statement_type: stmt_type.to_string(),
            rows: Vec::new(),
            rows_affected: None,
            session_id: None,
        }
    }

    pub fn affected(index: usize, stmt_type: &str, count: u64) -> Self {
        Self {
            statement_index: index,
            statement_type: stmt_type.to_string(),
            rows: Vec::new(),
            rows_affected: Some(count),
            session_id: None,
        }
    }

    pub fn begin(index: usize, session_id: String) -> Self {
        Self {
            statement_index: index,
            statement_type: "BEGIN".to_string(),
            rows: Vec::new(),
            rows_affected: None,
            session_id: Some(session_id),
        }
    }

    pub fn commit(index: usize) -> Self {
        Self {
            statement_index: index,
            statement_type: "COMMIT".to_string(),
            rows: Vec::new(),
            rows_affected: None,
            session_id: None,
        }
    }

    pub fn rollback(index: usize) -> Self {
        Self {
            statement_index: index,
            statement_type: "ROLLBACK".to_string(),
            rows: Vec::new(),
            rows_affected: None,
            session_id: None,
        }
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
}

impl BatchResult {
    /// Extract single result, returning an error if the batch failed or
    /// contains != 1 statement.
    pub fn single(self) -> Result<ExecuteResult, String> {
        if let Some(err) = &self.error {
            return Err(format!(
                "batch error at statement {}: {}",
                err.statement_index, err.message
            ));
        }
        if self.results.len() != 1 {
            return Err(format!(
                "expected single statement, got {}",
                self.results.len()
            ));
        }
        let stmt_result = self
            .results
            .into_iter()
            .next()
            .expect("length checked to be 1 above");
        Ok(ExecuteResult::new(stmt_result.rows))
    }

    /// Try to extract single result. Returns None if batch has error or != 1 statement.
    pub fn try_single(self) -> Option<ExecuteResult> {
        if self.error.is_some() || self.results.len() != 1 {
            return None;
        }
        let stmt_result = self
            .results
            .into_iter()
            .next()
            .expect("length checked to be 1 above");
        Some(ExecuteResult::new(stmt_result.rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_batch(results: Vec<StatementResult>, error: Option<BatchError>) -> BatchResult {
        let completed = results.len();
        BatchResult {
            results,
            completed,
            error,
        }
    }

    #[cfg(feature = "auth")]
    #[test]
    fn auth_error_mapping_preserves_public_classes_and_redacts_internal_sources() {
        let duplicate = BatchError::from_auth_error(
            1,
            &crate::auth::AuthError::already_exists("API key", "deploy"),
        );
        assert_eq!(duplicate.code, "SDB-AC005");
        assert_eq!(duplicate.class, crate::error::ErrorClass::AlreadyExists);
        assert!(duplicate.message.contains("already exists"));

        let missing =
            BatchError::from_auth_error(2, &crate::auth::AuthError::not_found("user", "missing"));
        assert_eq!(missing.code, "SDB-AC004");
        assert_eq!(missing.class, crate::error::ErrorClass::NotFound);
        assert_eq!(missing.message, "user 'missing' not found");

        let invalid = BatchError::from_auth_error(
            3,
            &crate::auth::AuthError::invalid("cannot drop the root user"),
        );
        assert_eq!(invalid.code, "SDB-AC003");
        assert_eq!(invalid.class, crate::error::ErrorClass::InvalidArgument);
        assert_eq!(invalid.message, "cannot drop the root user");

        let marker = "/private/system/auth.db";
        let internal = BatchError::from_auth_error(4, &crate::auth::AuthError::internal(marker));
        assert_eq!(internal.code, "SDB-AC006");
        assert_eq!(internal.class, crate::error::ErrorClass::Internal);
        assert_eq!(internal.message, "Internal error");
        assert!(!internal.message.contains(marker));
    }

    #[test]
    fn batch_error_redacts_internal_source_and_keeps_stable_code() {
        let marker = "/private/storage/backend-secret";
        let error = BatchError::from_query_error(
            2,
            &crate::query::QueryError::Execute(crate::query::ExecuteError::Internal(
                marker.to_string(),
            )),
        );
        assert_eq!(error.statement_index, 2);
        assert_eq!(error.code, "SDB-QE016");
        assert_eq!(error.class, crate::error::ErrorClass::Internal);
        assert_eq!(error.message, "Internal error");
        assert!(!error.message.contains(marker));

        let storage_error =
            BatchError::from_storage_error(3, &crate::error::StorageError::Io(marker.to_string()));
        assert_eq!(storage_error.code, "SDB-ST005");
        assert_eq!(storage_error.class, crate::error::ErrorClass::Internal);
        assert_eq!(storage_error.message, "Internal error");
        assert!(!storage_error.message.contains(marker));
    }

    #[test]
    fn batch_error_keeps_actionable_storage_error_mappings() {
        let unique = BatchError::from_execute_error(
            0,
            &crate::query::ExecuteError::UniqueViolation {
                field: "email".into(),
                value: "taken@example.com".into(),
            },
        );
        assert_eq!(unique.code, "SDB-QE013");
        assert_eq!(unique.class, crate::error::ErrorClass::AlreadyExists);
        assert!(unique.message.contains("Unique constraint violation"));

        let conflict = BatchError::from_storage_error(
            0,
            &crate::error::StorageError::Conflict("unique constraint violation".into()),
        );
        assert_eq!(conflict.code, "SDB-ST003");
        assert_eq!(conflict.class, crate::error::ErrorClass::Aborted);
        assert!(conflict.message.contains("unique constraint"));

        let vector = BatchError::from_execute_error(
            1,
            &crate::query::ExecuteError::Storage(crate::error::StorageError::InvalidConfig(
                "vector dimension must be 3".into(),
            )),
        );
        assert_eq!(vector.code, "SDB-ST013");
        assert_eq!(vector.class, crate::error::ErrorClass::InvalidArgument);
        assert!(vector.message.contains("vector dimension"));

        let endpoint = BatchError::from_execute_error(
            2,
            &crate::query::ExecuteError::Storage(crate::error::StorageError::NotFound(
                "Target document does not exist".into(),
            )),
        );
        assert_eq!(endpoint.code, "SDB-ST006");
        assert_eq!(endpoint.class, crate::error::ErrorClass::NotFound);
        assert!(endpoint.message.contains("does not exist"));
    }

    #[test]
    fn test_single_returns_ok_for_one_result() {
        let batch = make_batch(vec![StatementResult::empty(0, "SELECT")], None);
        assert!(batch.single().is_ok());
    }

    #[test]
    fn test_single_returns_err_for_empty_results() {
        let batch = make_batch(vec![], None);
        let err = batch.single().unwrap_err();
        assert!(err.contains("expected single statement, got 0"));
    }

    #[test]
    fn test_single_returns_err_for_multiple_results() {
        let batch = make_batch(
            vec![
                StatementResult::empty(0, "SELECT"),
                StatementResult::empty(1, "SELECT"),
            ],
            None,
        );
        let err = batch.single().unwrap_err();
        assert!(err.contains("expected single statement, got 2"));
    }

    #[test]
    fn test_single_returns_err_for_batch_error() {
        let batch = make_batch(
            vec![],
            Some(BatchError::public(
                0,
                "SDB-QP001",
                crate::error::ErrorClass::InvalidArgument,
                "syntax error",
            )),
        );
        let err = batch.single().unwrap_err();
        assert!(err.contains("batch error"));
        assert!(err.contains("syntax error"));
    }
}
