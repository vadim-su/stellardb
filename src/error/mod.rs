pub mod code;
pub use code::{ErrorCode, ErrorResponse, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    InvalidArgument,
    NotFound,
    AlreadyExists,
    Aborted,
    Timeout,
    Unauthorized,
    Forbidden,
    FailedPrecondition,
    Unimplemented,
    Internal,
}

#[cfg(feature = "server")]
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
#[cfg(feature = "server")]
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("storage error: {0}")]
    Fjall(#[from] fjall::Error),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("index error: {0}")]
    Index(String),

    #[error("io error: {0}")]
    Io(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("already exists: {0}")]
    AlreadyExists(String),

    #[error("invalid operation: {0}")]
    InvalidOperation(String),

    #[error("collection not loaded: {0}")]
    CollectionNotLoaded(String),

    #[error("index lookup failed: {0}")]
    IndexLookupFailed(String),

    #[error("backend not found: {0}")]
    BackendNotFound(String),

    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("transaction failed: {0}")]
    TransactionFailed(String),

    #[error("{0}")]
    Other(String),
}

#[derive(Debug, thiserror::Error)]
#[cfg(feature = "server")]
pub enum AppError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("task join error: {0}")]
    Join(#[from] tokio::task::JoinError),

    #[error("admission error: {0}")]
    Admission(#[from] crate::server::admission::AdmissionError),

    #[error("authentication management error: {0}")]
    Auth(#[from] crate::auth::AuthError),

    #[error("not found")]
    NotFound,

    #[error("conflict: {0} already exists")]
    Conflict(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("{0}")]
    QueryFailed(crate::query::QueryError),
}

#[cfg(feature = "server")]
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        if let AppError::Admission(crate::server::admission::AdmissionError::OperationRunning(
            operation_id,
        )) = &self
        {
            return (
                StatusCode::ACCEPTED,
                Json(json!({
                    "operation_id": operation_id,
                    "status": "running",
                    "status_url": format!("/v1/operations/{operation_id}")
                })),
            )
                .into_response();
        }

        let (status, code, message, hint) = match &self {
            AppError::QueryFailed(e) => {
                let status = class_to_http_status(query_error_class(e));
                (status, e.code(), safe_query_error_message(e), e.hint())
            }
            AppError::NotFound => (
                StatusCode::NOT_FOUND,
                "SDB-ST006",
                "Not found".to_string(),
                None,
            ),
            AppError::Conflict(id) => (
                StatusCode::CONFLICT,
                "SDB-ST007",
                format!("{} already exists", id),
                None,
            ),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "SDB-ST008", msg.clone(), None),
            AppError::Unauthorized(msg) => {
                (StatusCode::UNAUTHORIZED, "SDB-AC001", msg.clone(), None)
            }
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, "SDB-AC002", msg.clone(), None),
            AppError::Storage(e) => {
                let status = class_to_http_status(storage_error_class(e));
                (status, e.code(), safe_storage_error_message(e), None)
            }
            AppError::Join(e) => {
                tracing::error!("internal error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "SDB-ST001",
                    "Internal error".to_string(),
                    None,
                )
            }
            AppError::Admission(error) => match error {
                crate::server::admission::AdmissionError::RateLimited => (
                    StatusCode::TOO_MANY_REQUESTS,
                    "SDB-SV001",
                    error.to_string(),
                    None,
                ),
                crate::server::admission::AdmissionError::Overloaded => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "SDB-SV002",
                    error.to_string(),
                    None,
                ),
                crate::server::admission::AdmissionError::OperationRunning(_) => {
                    unreachable!("handled before error-envelope mapping")
                }
                crate::server::admission::AdmissionError::Join(join) => {
                    tracing::error!("internal blocking task error: {}", join);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "SDB-ST001",
                        "Internal error".to_string(),
                        None,
                    )
                }
            },
            AppError::Auth(error) => (
                class_to_http_status(error.class()),
                error.code(),
                error.safe_message(),
                None,
            ),
        };

        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!(error.code = code, error.source = %self, "request failed");
        }

        let mut error_obj = json!({
            "code": code,
            "message": message,
            "severity": "ERROR"
        });

        if let Some(h) = hint {
            error_obj["hint"] = serde_json::Value::String(h);
        }

        (status, Json(json!({ "error": error_obj }))).into_response()
    }
}

pub fn storage_error_class(error: &StorageError) -> ErrorClass {
    match error {
        StorageError::NotFound(_) => ErrorClass::NotFound,
        StorageError::AlreadyExists(_) => ErrorClass::AlreadyExists,
        StorageError::Conflict(_) => ErrorClass::Aborted,
        StorageError::InvalidOperation(_) | StorageError::InvalidConfig(_) => {
            ErrorClass::InvalidArgument
        }
        _ => ErrorClass::Internal,
    }
}

pub fn execute_error_class(error: &crate::query::ExecuteError) -> ErrorClass {
    use crate::query::ExecuteError;
    match error {
        ExecuteError::NotFound(_)
        | ExecuteError::CollectionNotFound(_)
        | ExecuteError::IndexNotFound(..)
        | ExecuteError::AnalyzerNotFound(_) => ErrorClass::NotFound,
        ExecuteError::AlreadyExists(_)
        | ExecuteError::IndexAlreadyExists(..)
        | ExecuteError::UniqueViolation { .. } => ErrorClass::AlreadyExists,
        ExecuteError::TransactionConflict => ErrorClass::Aborted,
        ExecuteError::QueryTimeout(_) | ExecuteError::ServerDeadlineExceeded => ErrorClass::Timeout,
        ExecuteError::SessionAccessDenied => ErrorClass::Forbidden,
        ExecuteError::SessionDatabaseMismatch | ExecuteError::SessionExpired => {
            ErrorClass::FailedPrecondition
        }
        ExecuteError::NotYetImplemented(_) => ErrorClass::Unimplemented,
        ExecuteError::Storage(error) => storage_error_class(error),
        ExecuteError::Internal(_)
        | ExecuteError::OperatorNotInitialized(_)
        | ExecuteError::UnexpectedExpression(_) => ErrorClass::Internal,
        _ => ErrorClass::InvalidArgument,
    }
}

pub fn query_error_class(error: &crate::query::QueryError) -> ErrorClass {
    use crate::query::error::QueryError;
    match error {
        QueryError::Parse(_) | QueryError::Bind(_) => ErrorClass::InvalidArgument,
        QueryError::Execute(error) => execute_error_class(error),
    }
}

pub fn safe_storage_error_message(error: &StorageError) -> String {
    match storage_error_class(error) {
        ErrorClass::Internal => "Internal error".to_string(),
        _ => error.to_string(),
    }
}

pub fn safe_execute_error_message(error: &crate::query::ExecuteError) -> String {
    match execute_error_class(error) {
        ErrorClass::Internal => "Internal error".to_string(),
        _ => error.to_string(),
    }
}

pub fn safe_query_error_message(error: &crate::query::QueryError) -> String {
    match error {
        crate::query::QueryError::Execute(error) => safe_execute_error_message(error),
        _ => error.to_string(),
    }
}

#[cfg(feature = "server")]
pub fn class_to_http_status(class: ErrorClass) -> StatusCode {
    match class {
        ErrorClass::InvalidArgument => StatusCode::BAD_REQUEST,
        ErrorClass::NotFound => StatusCode::NOT_FOUND,
        ErrorClass::AlreadyExists | ErrorClass::Aborted | ErrorClass::FailedPrecondition => {
            StatusCode::CONFLICT
        }
        ErrorClass::Timeout => StatusCode::GATEWAY_TIMEOUT,
        ErrorClass::Unauthorized => StatusCode::UNAUTHORIZED,
        ErrorClass::Forbidden => StatusCode::FORBIDDEN,
        ErrorClass::Unimplemented => StatusCode::NOT_IMPLEMENTED,
        ErrorClass::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

impl ErrorCode for StorageError {
    fn code(&self) -> &'static str {
        match self {
            Self::Fjall(_) => "SDB-ST001",
            Self::Serialization(_) => "SDB-ST002",
            Self::Conflict(_) => "SDB-ST003",
            Self::Index(_) => "SDB-ST004",
            Self::Io(_) => "SDB-ST005",
            Self::NotFound(_) => "SDB-ST006",
            Self::AlreadyExists(_) => "SDB-ST007",
            Self::InvalidOperation(_) => "SDB-ST008",
            Self::CollectionNotLoaded(_) => "SDB-ST010",
            Self::IndexLookupFailed(_) => "SDB-ST011",
            Self::BackendNotFound(_) => "SDB-ST012",
            Self::InvalidConfig(_) => "SDB-ST013",
            Self::TransactionFailed(_) => "SDB-ST014",
            Self::Other(_) => "SDB-ST009",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_classification_and_redaction_contract() {
        use crate::query::{ExecuteError, ParseError, QueryError};

        let syntax = QueryError::Parse(ParseError::EmptyInput);
        assert_eq!(query_error_class(&syntax), ErrorClass::InvalidArgument);
        let timeout = QueryError::Execute(ExecuteError::QueryTimeout("30s".into()));
        assert_eq!(query_error_class(&timeout), ErrorClass::Timeout);
        let denied = QueryError::Execute(ExecuteError::SessionAccessDenied);
        assert_eq!(query_error_class(&denied), ErrorClass::Forbidden);
        let wrong_db = QueryError::Execute(ExecuteError::SessionDatabaseMismatch);
        assert_eq!(query_error_class(&wrong_db), ErrorClass::FailedPrecondition);
        let expired = QueryError::Execute(ExecuteError::SessionExpired);
        assert_eq!(query_error_class(&expired), ErrorClass::FailedPrecondition);
        let exists = QueryError::Execute(ExecuteError::AlreadyExists("x".into()));
        assert_eq!(query_error_class(&exists), ErrorClass::AlreadyExists);
        let aborted = QueryError::Execute(ExecuteError::TransactionConflict);
        assert_eq!(query_error_class(&aborted), ErrorClass::Aborted);
        let storage_conflict = QueryError::Execute(ExecuteError::Storage(StorageError::Conflict(
            "unique constraint violation".into(),
        )));
        assert_eq!(query_error_class(&storage_conflict), ErrorClass::Aborted);
        assert!(safe_query_error_message(&storage_conflict).contains("unique constraint"));
        let invalid_vector = QueryError::Execute(ExecuteError::Storage(
            StorageError::InvalidConfig("vector dimension must be 3".into()),
        ));
        assert_eq!(
            query_error_class(&invalid_vector),
            ErrorClass::InvalidArgument
        );
        assert!(safe_query_error_message(&invalid_vector).contains("vector dimension"));
        let missing_endpoint = QueryError::Execute(ExecuteError::Storage(StorageError::NotFound(
            "Target document does not exist".into(),
        )));
        assert_eq!(query_error_class(&missing_endpoint), ErrorClass::NotFound);
        assert!(safe_query_error_message(&missing_endpoint).contains("does not exist"));
        let unimplemented = QueryError::Execute(ExecuteError::NotYetImplemented("feature"));
        assert_eq!(query_error_class(&unimplemented), ErrorClass::Unimplemented);

        let marker = "/private/data/backend-secret";
        let internal = QueryError::Execute(ExecuteError::Internal(marker.into()));
        assert_eq!(query_error_class(&internal), ErrorClass::Internal);
        assert_eq!(safe_query_error_message(&internal), "Internal error");
        assert!(!safe_query_error_message(&internal).contains(marker));

        let storage = StorageError::Io(marker.into());
        assert_eq!(storage_error_class(&storage), ErrorClass::Internal);
        assert_eq!(safe_storage_error_message(&storage), "Internal error");
        let storage_query = QueryError::Execute(ExecuteError::Storage(storage));
        assert_eq!(query_error_class(&storage_query), ErrorClass::Internal);
        assert_eq!(safe_query_error_message(&storage_query), "Internal error");
        assert!(!safe_query_error_message(&storage_query).contains(marker));
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn http_internal_response_does_not_leak_source() {
        let marker = "/private/storage/backend-secret";
        let response = AppError::QueryFailed(crate::query::QueryError::Execute(
            crate::query::ExecuteError::Internal(marker.into()),
        ))
        .into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("Internal error"));
        assert!(!text.contains(marker));
    }

    #[cfg(feature = "server")]
    #[test]
    fn http_status_contract_has_expected_parity() {
        assert_eq!(
            class_to_http_status(ErrorClass::InvalidArgument),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            class_to_http_status(ErrorClass::NotFound),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            class_to_http_status(ErrorClass::AlreadyExists),
            StatusCode::CONFLICT
        );
        assert_eq!(
            class_to_http_status(ErrorClass::Aborted),
            StatusCode::CONFLICT
        );
        assert_eq!(
            class_to_http_status(ErrorClass::Timeout),
            StatusCode::GATEWAY_TIMEOUT
        );
        assert_eq!(
            class_to_http_status(ErrorClass::Forbidden),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            class_to_http_status(ErrorClass::FailedPrecondition),
            StatusCode::CONFLICT
        );
        assert_eq!(
            class_to_http_status(ErrorClass::Unimplemented),
            StatusCode::NOT_IMPLEMENTED
        );
        assert_eq!(
            class_to_http_status(ErrorClass::Internal),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[cfg(feature = "server")]
    #[test]
    fn running_operation_uses_accepted_instead_of_gateway_timeout() {
        use axum::response::IntoResponse;

        let response = AppError::Admission(
            crate::server::admission::AdmissionError::OperationRunning("op_test".to_string()),
        )
        .into_response();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }

    #[test]
    fn test_storage_error_codes() {
        assert_eq!(
            StorageError::Serialization("test".into()).code(),
            "SDB-ST002"
        );
        assert_eq!(StorageError::Conflict("test".into()).code(), "SDB-ST003");
        assert_eq!(StorageError::NotFound("test".into()).code(), "SDB-ST006");
        assert_eq!(
            StorageError::CollectionNotLoaded("test".into()).code(),
            "SDB-ST010"
        );
        assert_eq!(
            StorageError::IndexLookupFailed("test".into()).code(),
            "SDB-ST011"
        );
        assert_eq!(
            StorageError::BackendNotFound("test".into()).code(),
            "SDB-ST012"
        );
        assert_eq!(
            StorageError::InvalidConfig("test".into()).code(),
            "SDB-ST013"
        );
        assert_eq!(
            StorageError::TransactionFailed("test".into()).code(),
            "SDB-ST014"
        );
        assert_eq!(StorageError::Other("test".into()).code(), "SDB-ST009");
    }
}
