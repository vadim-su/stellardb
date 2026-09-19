//! gRPC interceptor for extracting the `x-database` metadata key and authenticating requests.

use std::sync::Arc;

use tonic::{Request, Status};

use crate::auth::{Action, ResolvedSubject};

use crate::server::AppState;
use crate::storage::Database;

/// Extract the database name from gRPC metadata.
pub fn extract_database_name(metadata: &tonic::metadata::MetadataMap) -> Result<&str, Status> {
    metadata
        .get("x-database")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| Status::invalid_argument("x-database metadata required"))
}

/// Extract the database name from gRPC metadata `x-database` key
/// and resolve it to a Database handle.
pub async fn extract_database(
    state: &AppState,
    request: &Request<impl std::fmt::Debug>,
) -> Result<Arc<Database>, Status> {
    let database = extract_database_name(request.metadata())?.to_string();
    let namespace = state.namespace.clone();
    state
        .admission
        .run_db(move || namespace.get_database(&database))
        .await
        .map_err(admission_error_to_status)?
        .map_err(storage_error_to_status)
}

/// Extract and authenticate the subject from gRPC metadata.
/// Returns `Ok(None)` if auth is not configured (i.e. `state.auth` is `None`).
/// A missing token resolves to the configured anonymous user; otherwise
/// returns `Err(Status::unauthenticated)` when the token is missing or invalid.
pub(crate) async fn extract_subject(
    state: &AppState,
    metadata: &tonic::metadata::MetadataMap,
) -> Result<Option<ResolvedSubject>, Status> {
    let auth = match &state.auth {
        Some(a) => a.clone(),
        None => return Ok(None), // Auth not configured
    };

    let token = metadata
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);
    if token.is_none() && auth.anonymous_user().is_none() {
        return Err(Status::unauthenticated("Authorization metadata required"));
    }

    state
        .admission
        .run_db(move || auth.authenticate_request(token.as_deref()))
        .await
        .map_err(admission_error_to_status)?
        .map(Some)
        .map_err(|error| {
            tracing::debug!(error, "request authentication failed");
            Status::unauthenticated("invalid credentials")
        })
}

/// Authorize a gRPC request using the shared transport-independent policy boundary.
pub(crate) async fn authorize_request(
    state: &AppState,
    subject: Option<&ResolvedSubject>,
    action: Action,
    database: &str,
    collection: Option<&str>,
) -> Result<(), Status> {
    let auth = state.auth.clone();
    let subject = subject.map(|resolved| resolved.subject().clone());
    let database = database.to_string();
    let collection = collection.map(str::to_string);
    state
        .admission
        .run_db(move || {
            crate::auth::authorize(
                auth.as_deref()
                    .map(|service| service as &dyn crate::auth::AuthorizationProvider),
                subject.as_ref(),
                action,
                &database,
                collection.as_deref(),
            )
        })
        .await
        .map_err(admission_error_to_status)?
        .map_err(|error| Status::permission_denied(error.to_string()))
}

/// Authorize a database-scoped gRPC request from its metadata.
pub(crate) async fn authorize_database_request(
    state: &AppState,
    subject: Option<&ResolvedSubject>,
    action: Action,
    metadata: &tonic::metadata::MetadataMap,
    collection: Option<&str>,
) -> Result<(), Status> {
    authorize_request(
        state,
        subject,
        action,
        extract_database_name(metadata)?,
        collection,
    )
    .await
}

/// Convert shared admission failures to stable gRPC status codes.
pub fn admission_error_to_status(error: crate::server::admission::AdmissionError) -> Status {
    match error {
        crate::server::admission::AdmissionError::RateLimited
        | crate::server::admission::AdmissionError::Overloaded => {
            Status::resource_exhausted(error.to_string())
        }
        crate::server::admission::AdmissionError::OperationRunning(operation_id) => {
            Status::unavailable(format!(
                "operation continues in the background; operation_id={operation_id}"
            ))
        }
        crate::server::admission::AdmissionError::Join(join) => {
            tracing::error!("internal blocking task error: {}", join);
            Status::internal("Internal error")
        }
    }
}

fn classified_status(class: crate::error::ErrorClass, message: String) -> Status {
    use crate::error::ErrorClass;
    match class {
        ErrorClass::InvalidArgument => Status::invalid_argument(message),
        ErrorClass::NotFound => Status::not_found(message),
        ErrorClass::AlreadyExists => Status::already_exists(message),
        ErrorClass::Aborted => Status::aborted(message),
        ErrorClass::Timeout => Status::deadline_exceeded(message),
        ErrorClass::Unauthorized => Status::unauthenticated(message),
        ErrorClass::Forbidden => Status::permission_denied(message),
        ErrorClass::FailedPrecondition => Status::failed_precondition(message),
        ErrorClass::Unimplemented => Status::unimplemented(message),
        ErrorClass::Internal => Status::internal(message),
    }
}

pub fn query_error_to_status(error: &crate::query::QueryError) -> Status {
    use crate::error::ErrorCode;
    let class = crate::error::query_error_class(error);
    if class == crate::error::ErrorClass::Internal {
        tracing::error!(error.code = error.code(), error.source = %error, "gRPC query failed");
    }
    classified_status(
        class,
        format!(
            "[{}] {}",
            error.code(),
            crate::error::safe_query_error_message(error)
        ),
    )
}

/// Convert AppError to tonic::Status using the shared error classification.
pub fn app_error_to_status(err: &crate::error::AppError) -> Status {
    use crate::error::{AppError, ErrorCode};

    match err {
        AppError::QueryFailed(error) => query_error_to_status(error),
        AppError::NotFound => Status::not_found("[SDB-ST006] Not found"),
        AppError::BadRequest(msg) => Status::invalid_argument(format!("[SDB-ST008] {}", msg)),
        AppError::Unauthorized(msg) => Status::unauthenticated(format!("[SDB-AC001] {}", msg)),
        AppError::Forbidden(msg) => Status::permission_denied(format!("[SDB-AC002] {}", msg)),
        AppError::Conflict(id) => {
            Status::already_exists(format!("[SDB-ST007] {} already exists", id))
        }
        AppError::Storage(error) => {
            let class = crate::error::storage_error_class(error);
            if class == crate::error::ErrorClass::Internal {
                tracing::error!(error.code = error.code(), error.source = %error, "gRPC storage failed");
            }
            classified_status(
                class,
                format!(
                    "[{}] {}",
                    error.code(),
                    crate::error::safe_storage_error_message(error)
                ),
            )
        }
        other => {
            tracing::error!("internal gRPC error: {}", other);
            Status::internal("Internal error")
        }
    }
}

/// Convert a storage error directly to tonic::Status.
pub fn storage_error_to_status(err: crate::error::StorageError) -> Status {
    app_error_to_status(&crate::error::AppError::Storage(err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{MetricsCollector, MetricsConfig};
    use crate::namespace::Namespace;
    use crate::session::{SessionConfig, SessionManager};

    fn make_state_without_auth() -> Arc<AppState> {
        let tmp = tempfile::tempdir().unwrap();
        let namespace = Arc::new(Namespace::open(tmp.path()).unwrap());
        let admission = Arc::new(crate::server::admission::AdmissionController::new(
            crate::server::admission::AdmissionConfig::default(),
        ));
        let ddl_jobs =
            crate::server::ddl_jobs::DdlJobManager::open(namespace.clone(), admission.clone())
                .unwrap();
        // Leak to keep db alive
        std::mem::forget(tmp);
        Arc::new(AppState {
            namespace,
            sessions: Arc::new(SessionManager::new(SessionConfig::default())),
            metrics: Arc::new(MetricsCollector::new(MetricsConfig::default())),
            admission,
            ddl_jobs,
            query_timeout_default: None,
            trusted_proxy_hops: 0,
            auth: None,
        })
    }

    fn make_state_with_auth() -> (Arc<AppState>, String) {
        let tmp = tempfile::tempdir().unwrap();
        let namespace = Arc::new(Namespace::open(tmp.path()).unwrap());
        let system_db = namespace.ensure_system_database().unwrap();
        let (auth_service, root_pw) =
            crate::auth::AuthService::init(system_db, b"test-secret-32-bytes-minimum!!", 3600)
                .unwrap();
        let admission = Arc::new(crate::server::admission::AdmissionController::new(
            crate::server::admission::AdmissionConfig::default(),
        ));
        let ddl_jobs =
            crate::server::ddl_jobs::DdlJobManager::open(namespace.clone(), admission.clone())
                .unwrap();
        std::mem::forget(tmp);
        let state = Arc::new(AppState {
            namespace,
            sessions: Arc::new(SessionManager::new(SessionConfig::default())),
            metrics: Arc::new(MetricsCollector::new(MetricsConfig::default())),
            admission,
            ddl_jobs,
            query_timeout_default: None,
            trusted_proxy_hops: 0,
            auth: Some(Arc::new(auth_service)),
        });
        (state, root_pw.unwrap())
    }

    #[tokio::test]
    async fn extract_subject_no_auth_returns_none() {
        let state = make_state_without_auth();
        let metadata = tonic::metadata::MetadataMap::new();
        let result = extract_subject(&state, &metadata).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn extract_subject_missing_authorization_header() {
        let (state, _) = make_state_with_auth();
        let metadata = tonic::metadata::MetadataMap::new();
        let err = extract_subject(&state, &metadata).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
        assert!(err.message().contains("Authorization"));
    }

    #[tokio::test]
    async fn extract_subject_missing_bearer_prefix() {
        let (state, _) = make_state_with_auth();
        let mut metadata = tonic::metadata::MetadataMap::new();
        metadata.insert("authorization", "Token abc123".parse().unwrap());
        let err = extract_subject(&state, &metadata).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[tokio::test]
    async fn extract_subject_invalid_token() {
        let (state, _) = make_state_with_auth();
        let mut metadata = tonic::metadata::MetadataMap::new();
        metadata.insert("authorization", "Bearer invalid.jwt.token".parse().unwrap());
        let err = extract_subject(&state, &metadata).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
        assert!(err.message().contains("invalid credentials"));
    }

    #[tokio::test]
    async fn extract_subject_valid_jwt() {
        let (state, root_pw) = make_state_with_auth();

        // Login to get a valid JWT
        let auth = state.auth.as_ref().unwrap();
        let token = auth.login("root", &root_pw).unwrap();

        let mut metadata = tonic::metadata::MetadataMap::new();
        metadata.insert(
            "authorization",
            format!("Bearer {}", token).parse().unwrap(),
        );

        let subject = extract_subject(&state, &metadata).await.unwrap().unwrap();
        assert_eq!(subject.subject().user_id, "root");
    }

    #[tokio::test]
    async fn extract_subject_valid_api_key() {
        let (state, _) = make_state_with_auth();

        let auth = state.auth.as_ref().unwrap();
        let api_key = auth.create_api_key("test-key", "root").unwrap();

        let mut metadata = tonic::metadata::MetadataMap::new();
        metadata.insert(
            "authorization",
            format!("Bearer {}", api_key).parse().unwrap(),
        );

        let subject = extract_subject(&state, &metadata).await.unwrap().unwrap();
        assert_eq!(subject.subject().user_id, "root");
    }

    // ─── app_error_to_status mapping tests ───

    #[test]
    fn query_error_mapping_has_transport_parity_and_redacts_internal_details() {
        use crate::query::{ExecuteError, ParseError, QueryError};

        let parse = query_error_to_status(&QueryError::Parse(ParseError::EmptyInput));
        assert_eq!(parse.code(), tonic::Code::InvalidArgument);

        let timeout = query_error_to_status(&QueryError::Execute(ExecuteError::QueryTimeout(
            "deadline".into(),
        )));
        assert_eq!(timeout.code(), tonic::Code::DeadlineExceeded);

        let denied = query_error_to_status(&QueryError::Execute(ExecuteError::SessionAccessDenied));
        assert_eq!(denied.code(), tonic::Code::PermissionDenied);

        let wrong_db =
            query_error_to_status(&QueryError::Execute(ExecuteError::SessionDatabaseMismatch));
        assert_eq!(wrong_db.code(), tonic::Code::FailedPrecondition);

        let already_exists = query_error_to_status(&QueryError::Execute(
            ExecuteError::AlreadyExists("x".into()),
        ));
        assert_eq!(already_exists.code(), tonic::Code::AlreadyExists);

        let aborted =
            query_error_to_status(&QueryError::Execute(ExecuteError::TransactionConflict));
        assert_eq!(aborted.code(), tonic::Code::Aborted);

        let unimplemented = query_error_to_status(&QueryError::Execute(
            ExecuteError::NotYetImplemented("feature"),
        ));
        assert_eq!(unimplemented.code(), tonic::Code::Unimplemented);

        let marker = "/private/storage/backend-secret";
        let internal =
            query_error_to_status(&QueryError::Execute(ExecuteError::Internal(marker.into())));
        assert_eq!(internal.code(), tonic::Code::Internal);
        assert!(!internal.message().contains(marker));
    }

    #[test]
    fn unauthorized_maps_to_unauthenticated() {
        let status = app_error_to_status(&crate::error::AppError::Unauthorized("no token".into()));
        assert_eq!(status.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn forbidden_maps_to_permission_denied() {
        let status = app_error_to_status(&crate::error::AppError::Forbidden("denied".into()));
        assert_eq!(status.code(), tonic::Code::PermissionDenied);
    }

    #[test]
    fn not_found_maps_to_not_found() {
        let status = app_error_to_status(&crate::error::AppError::NotFound);
        assert_eq!(status.code(), tonic::Code::NotFound);
    }

    #[test]
    fn bad_request_maps_to_invalid_argument() {
        let status = app_error_to_status(&crate::error::AppError::BadRequest("bad input".into()));
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
    }
}
