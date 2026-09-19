use crate::error::StorageError;

#[derive(Debug, thiserror::Error)]
pub enum TransactionError {
    #[error("Transaction conflict: another transaction modified the data")]
    Conflict,

    #[error("Transaction timed out")]
    Timeout,

    #[error("Transaction already completed")]
    AlreadyCompleted,

    #[error("No active transaction")]
    NoActiveTransaction,

    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<TransactionError> for StorageError {
    fn from(error: TransactionError) -> Self {
        match error {
            TransactionError::Storage(error) => error,
            TransactionError::Conflict => {
                StorageError::Conflict("transaction write conflict".to_string())
            }
            other => StorageError::TransactionFailed(other.to_string()),
        }
    }
}

use crate::error::ErrorCode;

impl ErrorCode for TransactionError {
    fn code(&self) -> &'static str {
        match self {
            Self::Conflict => "SDB-TX001",
            Self::Timeout => "SDB-TX002",
            Self::AlreadyCompleted => "SDB-TX003",
            Self::NoActiveTransaction => "SDB-TX004",
            Self::Storage(_) => "SDB-TX005",
            Self::Internal(_) => "SDB-TX006",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    #[test]
    fn test_transaction_error_codes() {
        assert_eq!(TransactionError::Conflict.code(), "SDB-TX001");
        assert_eq!(TransactionError::Timeout.code(), "SDB-TX002");
        assert_eq!(TransactionError::AlreadyCompleted.code(), "SDB-TX003");
        assert_eq!(TransactionError::NoActiveTransaction.code(), "SDB-TX004");
        assert_eq!(
            TransactionError::Storage(StorageError::Io("test".into())).code(),
            "SDB-TX005"
        );
        assert_eq!(
            TransactionError::Internal("test".into()).code(),
            "SDB-TX006"
        );
    }
}
