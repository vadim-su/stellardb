pub mod admission;
pub mod capabilities;
pub mod ddl_jobs;
#[cfg(feature = "grpc")]
pub mod grpc;
pub mod metrics;
pub mod serialize;
#[cfg(feature = "tls")]
pub mod tls;
#[cfg(feature = "ui")]
pub mod ui;

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Json, Router,
    extract::{ConnectInfo, DefaultBodyLimit, Extension, Path, Query, State},
    http::{
        HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri,
        header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, PRAGMA},
    },
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use tower_http::cors::{Any, CorsLayer};

use crate::auth::{Action, Subject};
use crate::document::{Document, extract_user_fields};
use crate::edge::Edge;
use crate::error::{AppError, StorageError};
use crate::metrics::{MetricsCollector, MetricsConfig};
use crate::namespace::Namespace;
use crate::query::{self, ExecuteContext, Query as QueryBuilder};
use crate::schema::{CollectionSchema, DefaultValue, FieldDef, FieldType, IndexDef, SchemaMode};
use crate::server::admission::{AdmissionConfig, AdmissionController, OperationStatus};
use crate::server::ddl_jobs::{CancelError, DdlJobManager};
use crate::server::serialize::{BatchResultJson, TypedBatchResultJson};
use crate::session::{SessionConfig, SessionManager};
use crate::storage::Database;

/// Application state shared across all handlers
pub struct AppState {
    pub namespace: Arc<Namespace>,
    pub sessions: Arc<SessionManager>,
    pub metrics: Arc<MetricsCollector>,
    pub admission: Arc<AdmissionController>,
    pub ddl_jobs: Arc<DdlJobManager>,
    pub query_timeout_default: Option<Duration>,
    pub trusted_proxy_hops: usize,
    pub auth: Option<Arc<crate::auth::AuthService>>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ListQuery {
    #[serde(default = "default_limit")]
    pub limit: usize,
    pub after: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct V1CollectionsQuery {
    pub include: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct OperationsQuery {
    #[serde(default)]
    active: bool,
    limit: Option<usize>,
}

fn default_limit() -> usize {
    50
}

fn schema_mode_json(mode: &SchemaMode) -> &'static str {
    match mode {
        SchemaMode::Strict => "strict",
        SchemaMode::Flexible => "flexible",
    }
}

fn collection_schema_to_v1_json(schema: &CollectionSchema) -> serde_json::Value {
    json!({
        "name": schema.name,
        "mode": schema_mode_json(&schema.mode),
        "fields": schema.fields.iter().map(field_def_to_v1_json).collect::<Vec<_>>(),
        "indexes": schema.indexes.iter().map(index_def_to_v1_json).collect::<Vec<_>>(),
    })
}

fn field_def_to_v1_json(field: &FieldDef) -> serde_json::Value {
    let mut value = json!({
        "name": field.name,
        "type": field_type_to_v1_string(&field.field_type),
        "required": field.required,
    });

    if let Some(default) = &field.default {
        value["default"] = default_value_to_v1_json(default);
    }

    match &field.field_type {
        FieldType::Object { fields, .. } => {
            value["fields"] = json!(fields.iter().map(field_def_to_v1_json).collect::<Vec<_>>());
        }
        FieldType::Array(inner) => {
            if let FieldType::Object { fields, .. } = inner.as_ref() {
                value["fields"] =
                    json!(fields.iter().map(field_def_to_v1_json).collect::<Vec<_>>());
            }
        }
        _ => {}
    }

    value
}

fn field_type_to_v1_string(field_type: &FieldType) -> String {
    match field_type {
        FieldType::String => "string".to_string(),
        FieldType::Int => "int".to_string(),
        FieldType::Float => "float".to_string(),
        FieldType::Decimal => "decimal".to_string(),
        FieldType::Bool => "bool".to_string(),
        FieldType::Datetime => "datetime".to_string(),
        FieldType::Duration => "duration".to_string(),
        FieldType::Bytes => "bytes".to_string(),
        FieldType::Object { .. } => "object".to_string(),
        FieldType::Array(inner) => format!("[{}]", field_type_to_v1_string(inner)),
        FieldType::AnyArray => "array".to_string(),
        FieldType::Reference(Some(collection)) => format!("ref<{}>", collection),
        FieldType::Reference(None) => "ref".to_string(),
        FieldType::Any => "any".to_string(),
        FieldType::Range(inner) => format!("range<{}>", field_type_to_v1_string(inner)),
        FieldType::Union(types) => types
            .iter()
            .map(field_type_to_v1_string)
            .collect::<Vec<_>>()
            .join(" | "),
    }
}

fn default_value_to_v1_json(default: &DefaultValue) -> serde_json::Value {
    match default {
        DefaultValue::Null => serde_json::Value::Null,
        DefaultValue::Bool(value) => json!(value),
        DefaultValue::Int(value) => json!(value),
        DefaultValue::Float(value) => json!(value),
        DefaultValue::String(value) => json!(value),
        DefaultValue::Now => json!("now()"),
    }
}

fn index_def_to_v1_json(index: &IndexDef) -> serde_json::Value {
    json!({
        "name": index.name,
        "fields": index.fields,
        "unique": index.unique,
        "index_type": format!("{:?}", index.index_type),
    })
}

#[derive(Debug, Clone, Default)]
pub enum CorsConfig {
    #[default]
    Disabled,
    Wildcard,
    Origins(Vec<HeaderValue>),
}

impl CorsConfig {
    pub fn from_value(value: Option<&str>) -> Result<Self, String> {
        match value {
            None => Ok(Self::Disabled),
            Some(value) => match parse_cors_allow_origins(value)? {
                None => Ok(Self::Wildcard),
                Some(origins) => Ok(Self::Origins(origins)),
            },
        }
    }

    pub fn is_disabled(&self) -> bool {
        matches!(self, Self::Disabled)
    }

    fn layer(&self) -> Option<CorsLayer> {
        let cors = CorsLayer::new()
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
            ])
            .allow_headers([
                CONTENT_TYPE,
                AUTHORIZATION,
                HeaderName::from_static("x-database"),
                HeaderName::from_static("x-session-id"),
                HeaderName::from_static("x-query-timeout"),
            ]);
        match self {
            Self::Disabled => None,
            Self::Wildcard => Some(cors.allow_origin(Any)),
            Self::Origins(origins) => Some(cors.allow_origin(origins.clone())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpServerConfig {
    pub admission: AdmissionConfig,
    pub cors: CorsConfig,
    pub metrics: MetricsConfig,
    pub trusted_proxy_hops: usize,
    pub login_body_limit: usize,
    pub http_body_limit: usize,
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            admission: AdmissionConfig::default(),
            cors: CorsConfig::Disabled,
            metrics: MetricsConfig::default(),
            trusted_proxy_hops: 0,
            login_body_limit: 16 * 1024,
            http_body_limit: 10 * 1024 * 1024,
        }
    }
}

fn parse_cors_allow_origins(value: &str) -> Result<Option<Vec<HeaderValue>>, String> {
    let value = value.trim();
    if value == "*" {
        return Ok(None);
    }
    if value.is_empty() {
        return Err("STELLARDB_CORS_ALLOW_ORIGINS cannot be empty".to_string());
    }

    let mut origins = Vec::new();
    for origin in value.split(',').map(str::trim) {
        if origin.is_empty() {
            return Err("CORS origin list contains an empty entry".to_string());
        }
        validate_cors_origin(origin)?;
        origins.push(
            HeaderValue::from_str(origin)
                .map_err(|e| format!("Invalid CORS origin '{}': {}", origin, e))?,
        );
    }

    Ok(Some(origins))
}

fn validate_cors_origin(origin: &str) -> Result<(), String> {
    let uri = origin
        .parse::<Uri>()
        .map_err(|e| format!("Invalid CORS origin '{}': {}", origin, e))?;

    let scheme = uri
        .scheme_str()
        .ok_or_else(|| format!("CORS origin '{}' is missing a scheme", origin))?;
    if scheme != "http" && scheme != "https" {
        return Err(format!("CORS origin '{}' must use http or https", origin));
    }
    if uri.authority().is_none() {
        return Err(format!("CORS origin '{}' is missing a host", origin));
    }
    if uri.path() != "/" || uri.query().is_some() {
        return Err(format!(
            "CORS origin '{}' must not include a path or query",
            origin
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cors_allow_origins_parser_accepts_wildcard_and_origin_list() {
        assert!(parse_cors_allow_origins("*").unwrap().is_none());

        let origins =
            parse_cors_allow_origins("https://station.example.com, http://localhost:3000")
                .unwrap()
                .expect("origin list should produce explicit origins");
        assert_eq!(origins.len(), 2);
        assert_eq!(
            origins[0],
            HeaderValue::from_static("https://station.example.com")
        );
        assert_eq!(
            origins[1],
            HeaderValue::from_static("http://localhost:3000")
        );
    }

    #[test]
    fn forwarded_client_ip_requires_explicit_trusted_hops() {
        let peer: SocketAddr = "127.0.0.1:4000".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("198.51.100.10, 203.0.113.20"),
        );
        assert_eq!(resolve_client_ip(Some(peer), &headers, 0), Some(peer.ip()));
        assert_eq!(
            resolve_client_ip(Some(peer), &headers, 1),
            Some("203.0.113.20".parse().unwrap())
        );
        assert_eq!(
            resolve_client_ip(Some(peer), &headers, 2),
            Some("198.51.100.10".parse().unwrap())
        );
        assert_eq!(resolve_client_ip(Some(peer), &headers, 3), Some(peer.ip()));
        headers.insert("x-forwarded-for", HeaderValue::from_static("spoofed"));
        assert_eq!(resolve_client_ip(Some(peer), &headers, 1), Some(peer.ip()));
    }

    #[test]
    fn cors_allow_origins_parser_rejects_invalid_or_empty_config() {
        assert!(parse_cors_allow_origins("").is_err());
        assert!(parse_cors_allow_origins("not-a-url").is_err());
        assert!(parse_cors_allow_origins("https://ok.example, not-a-url").is_err());
        assert!(parse_cors_allow_origins("https://ok.example/path").is_err());
    }

    #[test]
    fn timeout_matrix_uses_shortest_explicit_absolute_deadline() {
        let start = std::time::Instant::now();
        let db = Duration::from_secs(60);
        assert_eq!(
            effective_absolute_deadline(start, Some(Duration::from_secs(5)), db),
            start + Duration::from_secs(5)
        );
        assert_eq!(
            effective_absolute_deadline(start, Some(Duration::from_secs(90)), db),
            start + db
        );
        assert_eq!(effective_absolute_deadline(start, None, db), start + db);
    }

    #[test]
    fn query_timeout_header_is_explicit_and_invalid_values_are_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Query-Timeout", HeaderValue::from_static("5s"));
        assert_eq!(
            query_timeout_from_headers(&headers, Some(Duration::from_secs(30))).unwrap(),
            (Some(Duration::from_secs(5)), true)
        );

        headers.insert("X-Query-Timeout", HeaderValue::from_static("0"));
        assert_eq!(
            query_timeout_from_headers(&headers, Some(Duration::from_secs(30))).unwrap(),
            (None, true)
        );

        headers.insert("X-Query-Timeout", HeaderValue::from_static("tomorrow"));
        assert!(query_timeout_from_headers(&headers, None).is_err());
    }
}

/// Format elapsed duration with appropriate unit suffix
fn format_elapsed(duration: Duration) -> (f64, &'static str) {
    let nanos = duration.as_nanos() as f64;
    if nanos < 1_000.0 {
        (nanos, "ns")
    } else if nanos < 1_000_000.0 {
        (nanos / 1_000.0, "µs")
    } else if nanos < 1_000_000_000.0 {
        (nanos / 1_000_000.0, "ms")
    } else {
        (nanos / 1_000_000_000.0, "s")
    }
}

#[derive(Debug, serde::Deserialize)]
struct SqlRequest {
    query: String,
    /// Stable caller supplied idempotency key for durable DDL.
    #[serde(default)]
    operation_id: Option<String>,
}

fn query_timeout_from_headers(
    headers: &HeaderMap,
    default: Option<Duration>,
) -> Result<(Option<Duration>, bool), AppError> {
    let raw = headers
        .get("X-Query-Timeout")
        .map(|value| {
            value
                .to_str()
                .map_err(|_| "X-Query-Timeout must be ASCII".to_string())
        })
        .transpose()
        .map_err(AppError::BadRequest)?;
    parse_query_timeout(raw, default).map_err(AppError::BadRequest)
}

pub(crate) fn parse_query_timeout(
    raw: Option<&str>,
    default: Option<Duration>,
) -> Result<(Option<Duration>, bool), String> {
    let Some(raw) = raw else {
        return Ok((default, false));
    };
    let nanos = crate::query::parse::expr::parse_duration_nanos(raw)
        .map_err(|error| format!("invalid query timeout: {error}"))?;
    Ok(((nanos != 0).then(|| Duration::from_nanos(nanos)), true))
}

pub(crate) fn effective_absolute_deadline(
    started_at: std::time::Instant,
    query_timeout: Option<Duration>,
    db_timeout: Duration,
) -> std::time::Instant {
    started_at + query_timeout.map_or(db_timeout, |timeout| timeout.min(db_timeout))
}

pub(crate) fn stored_session_query_timeout(
    state: &AppState,
    session_id: Option<&str>,
    resolved: Option<&crate::auth::ResolvedSubject>,
    database: &str,
) -> Option<Duration> {
    let session_id = session_id?;
    let owner = match resolved {
        Some(subject) => crate::session::SessionOwner::authenticated(
            subject.principal_identity().to_string(),
            database.to_string(),
        ),
        None => crate::session::SessionOwner::anonymous(database.to_string()),
    };
    state
        .sessions
        .get_query_timeout(session_id, &owner)
        .ok()
        .flatten()
}

fn legacy_query_json(
    result: &crate::query::BatchResult,
    started_at: std::time::Instant,
) -> serde_json::Value {
    let json_result = BatchResultJson::from(result);
    let (elapsed, elapsed_unit) = format_elapsed(started_at.elapsed());
    json!({
        "results": json_result.results,
        "completed": json_result.completed,
        "error": json_result.error,
        "elapsed_us": started_at.elapsed().as_micros() as u64,
        "elapsed": format!("{:.2}{}", elapsed, elapsed_unit)
    })
}

fn v1_query_json(
    result: &crate::query::BatchResult,
    started_at: std::time::Instant,
) -> serde_json::Value {
    let json_result = TypedBatchResultJson::from(result);
    let mut response = json!({
        "results": json_result.results,
        "completed": json_result.completed,
        "elapsed_us": started_at.elapsed().as_micros() as u64
    });
    if let Some(error) = json_result.error {
        response["error"] = json!(error);
    }
    response
}

fn transport_timeout_error(result: &crate::query::BatchResult) -> Option<AppError> {
    let error = result.error.as_ref()?;
    let execute = match error.code {
        "SDB-SV003" => crate::query::ExecuteError::ServerDeadlineExceeded,
        "SDB-QE014" => crate::query::ExecuteError::QueryTimeout(error.message.clone()),
        _ => return None,
    };
    Some(AppError::QueryFailed(crate::query::QueryError::Execute(
        execute,
    )))
}

pub(crate) async fn maybe_enqueue_create_index_job(
    state: &Arc<AppState>,
    parsed: &crate::query::Query,
    database: Arc<Database>,
    database_name: &str,
    resolved_subject: Option<&crate::auth::ResolvedSubject>,
    force_background: bool,
    operation_id: Option<String>,
) -> Result<Option<(ddl_jobs::DdlJob, bool)>, AppError> {
    let Some(ast) = parsed.single_create_index() else {
        return Ok(None);
    };

    authorize_request(
        state,
        resolved_subject.map(crate::auth::ResolvedSubject::subject),
        Action::Create,
        database_name,
        None,
    )
    .await?;

    if let Some(operation_id) = operation_id.as_deref()
        && let Some(existing) = state.ddl_jobs.get(operation_id)
    {
        let requested_index = IndexDef {
            name: crate::schema::generate_index_name(&ast.collection, ast.index_type, &ast.fields),
            fields: ast.fields.clone(),
            unique: ast.unique,
            index_type: ast.index_type,
            hnsw_params: ast.hnsw_params.clone(),
            analyzer: if ast.index_type == crate::schema::IndexType::FullText {
                Some(
                    ast.analyzer
                        .clone()
                        .unwrap_or_else(|| "standard".to_string()),
                )
            } else {
                ast.analyzer.clone()
            },
        };
        if existing.matches_create_index(database_name, &ast.collection, &requested_index) {
            return Ok(Some((existing, false)));
        }
        return Err(StorageError::Conflict(format!(
            "operation_id '{operation_id}' is already bound to a different operation"
        ))
        .into());
    }

    let collection = ast.collection.clone();
    let validation_database = database.clone();
    let (index, total_items) = state
        .admission
        .run_db(move || {
            let index = crate::query::execute::ddl::prepare_create_index(
                &validation_database,
                &ast.collection,
                ast.fields,
                ast.unique,
                ast.index_type,
                ast.hnsw_params,
                ast.analyzer,
            )?;
            let total_items = validation_database
                .count_documents(&ast.collection)
                .map_err(crate::query::ExecuteError::Storage)?;
            Ok::<_, crate::query::ExecuteError>((index, total_items))
        })
        .await?
        .map_err(|error| AppError::QueryFailed(crate::query::QueryError::Execute(error)))?;

    if !force_background
        && operation_id.is_none()
        && total_items < ddl_jobs::ASYNC_CREATE_INDEX_THRESHOLD
    {
        return Ok(None);
    }

    let job = state.ddl_jobs.enqueue_create_index_with_operation_id(
        operation_id,
        database_name.to_string(),
        collection,
        index,
        total_items,
    )?;
    #[cfg(feature = "fault-injection")]
    crate::fault_injection::check("ddl_after_enqueue")?;
    Ok(Some(job))
}

fn create_index_accepted_response(job: ddl_jobs::DdlJob, created: bool) -> Response {
    (
        StatusCode::ACCEPTED,
        Json(json!({
            "operation_id": job.operation_id,
            "state": job.state,
            "status": "running",
            "status_url": format!("/v1/operations/{}", job.operation_id),
            "created": created,
        })),
    )
        .into_response()
}

pub fn create_router(
    namespace: Arc<Namespace>,
    query_timeout_default: Option<Duration>,
    auth: Option<Arc<crate::auth::AuthService>>,
) -> (Router, Arc<AppState>) {
    create_router_with_admission(
        namespace,
        query_timeout_default,
        auth,
        AdmissionConfig::default(),
    )
}

pub fn create_router_with_admission(
    namespace: Arc<Namespace>,
    query_timeout_default: Option<Duration>,
    auth: Option<Arc<crate::auth::AuthService>>,
    admission_config: AdmissionConfig,
) -> (Router, Arc<AppState>) {
    create_router_with_admission_and_cors(
        namespace,
        query_timeout_default,
        auth,
        admission_config,
        CorsConfig::Disabled,
    )
}

pub fn create_router_with_admission_and_cors(
    namespace: Arc<Namespace>,
    query_timeout_default: Option<Duration>,
    auth: Option<Arc<crate::auth::AuthService>>,
    admission: AdmissionConfig,
    cors: CorsConfig,
) -> (Router, Arc<AppState>) {
    create_router_with_config(
        namespace,
        query_timeout_default,
        auth,
        HttpServerConfig {
            admission,
            cors,
            ..Default::default()
        },
    )
}

pub fn create_router_with_config(
    namespace: Arc<Namespace>,
    query_timeout_default: Option<Duration>,
    auth: Option<Arc<crate::auth::AuthService>>,
    config: HttpServerConfig,
) -> (Router, Arc<AppState>) {
    let HttpServerConfig {
        admission,
        cors,
        metrics,
        trusted_proxy_hops,
        login_body_limit,
        http_body_limit,
    } = config;
    let sessions = Arc::new(SessionManager::new(SessionConfig::default()));
    let metrics = Arc::new(MetricsCollector::new(metrics));

    let admission = Arc::new(AdmissionController::new(admission));
    let ddl_jobs = DdlJobManager::open(namespace.clone(), admission.clone())
        .expect("failed to open durable DDL job store");
    let state = Arc::new(AppState {
        namespace,
        sessions,
        metrics,
        admission,
        ddl_jobs,
        query_timeout_default,
        trusted_proxy_hops,
        auth,
    });

    let router = Router::new()
        .route("/sql", post(sql_handler))
        .route("/v1/query", post(v1_query_handler))
        .route("/v1/operations", get(operation_list_handler))
        .route(
            "/v1/operations/{operation_id}",
            get(operation_status_handler).delete(operation_cancel_handler),
        )
        .route(
            "/v1/operations/{operation_id}/cancel",
            post(operation_cancel_handler),
        )
        .route("/v1/capabilities", get(capabilities::capabilities_handler))
        .route("/v1/collections", get(v1_collections_handler))
        .route(
            "/collections/{collection}/documents",
            get(list_documents_handler).post(create_document_handler),
        )
        .route(
            "/collections/{collection}/documents/{key}",
            get(get_document_handler)
                .put(update_document_handler)
                .delete(delete_document_handler),
        )
        // Edge routes
        .route(
            "/nodes/{from}/edges/{label}/{to}",
            post(create_edge_handler)
                .get(get_edge_handler)
                .delete(delete_edge_handler),
        )
        .route("/nodes/{id}/out/{label}", get(list_edges_out_handler))
        .route("/nodes/{id}/in/{label}", get(list_edges_in_handler))
        // Database management routes
        .route(
            "/databases",
            get(list_databases_handler).post(create_database_handler),
        )
        .route(
            "/databases/{name}",
            axum::routing::delete(drop_database_handler),
        )
        .route("/databases/{name}/stats", get(database_stats_handler))
        // Auth endpoints
        .route(
            "/auth/login",
            post(login_handler).layer(DefaultBodyLimit::max(login_body_limit)),
        )
        .route("/v1/auth/me", get(auth_me_handler))
        .route(
            "/v1/admin/users",
            get(list_managed_users_handler).post(create_managed_user_handler),
        )
        .route(
            "/v1/admin/users/{username}",
            get(get_managed_user_handler)
                .patch(update_managed_user_handler)
                .delete(drop_managed_user_handler),
        )
        .route(
            "/v1/admin/users/{username}/password",
            axum::routing::put(update_managed_user_password_handler),
        )
        .route(
            "/v1/admin/api-keys",
            get(list_managed_api_keys_handler).post(create_managed_api_key_handler),
        )
        .route(
            "/v1/admin/api-keys/{name}",
            axum::routing::delete(drop_managed_api_key_handler),
        )
        // Metrics endpoints
        .route("/metrics", get(metrics::metrics_handler))
        .route("/stats", get(metrics::stats_handler));

    // UI routes (when feature enabled)
    #[cfg(feature = "ui")]
    let router = router
        .route("/", get(ui::root_redirect))
        .route("/_station", get(ui::index_handler))
        .route("/_station/", get(ui::index_handler))
        .route("/_station/{*path}", get(ui::static_handler));

    let router = router
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::middleware::auth_middleware,
        ))
        .layer(DefaultBodyLimit::max(http_body_limit));
    let router = match cors.layer() {
        Some(cors) => router.layer(cors),
        None => router,
    }
    .with_state(state.clone());

    (router, state)
}

async fn operation_status_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<axum::Extension<Subject>>,
    Path(operation_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    if let Some(job) = state.ddl_jobs.get(&operation_id) {
        authorize_request(
            &state,
            subject.as_ref().map(|value| &value.0),
            Action::Create,
            &job.database,
            None,
        )
        .await?;
        return Ok(Json(job.public_json()));
    }
    let operation = state
        .admission
        .operation(&operation_id)
        .ok_or(AppError::NotFound)?;
    authorize_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Create,
        &operation.database,
        None,
    )
    .await?;
    let response = match operation.status {
        OperationStatus::Running => json!({
            "operation_id": operation_id,
            "status": "running"
        }),
        OperationStatus::Completed { result } => json!({
            "operation_id": operation_id,
            "status": "completed",
            "result": result
        }),
        OperationStatus::Failed { code, message } => json!({
            "operation_id": operation_id,
            "status": "failed",
            "error": { "code": code, "message": message }
        }),
    };
    Ok(Json(response))
}

async fn operation_list_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<axum::Extension<Subject>>,
    headers: HeaderMap,
    Query(query): Query<OperationsQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let database = extract_database_name(&headers)?;
    authorize_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Create,
        database,
        None,
    )
    .await?;
    let mut operations: Vec<_> = state
        .ddl_jobs
        .list(Some(database), query.active, query.limit)
        .iter()
        .map(|job| job.public_json())
        .collect();
    operations.extend(
        state
            .admission
            .list_operations(database, query.active, query.limit)
            .into_iter()
            .map(|operation| admission_operation_json(&operation)),
    );
    operations.truncate(query.limit.unwrap_or(100).min(1_000));
    Ok(Json(json!({
        "count": operations.len(),
        "operations": operations,
    })))
}

fn admission_operation_json(
    operation: &crate::server::admission::TrackedOperationSnapshot,
) -> serde_json::Value {
    let (state, error) = match &operation.status {
        OperationStatus::Running => ("BUILDING", None),
        OperationStatus::Completed { .. } => ("READY", None),
        OperationStatus::Failed { code, message } => {
            ("FAILED", Some(json!({ "code": code, "message": message })))
        }
    };
    json!({
        "operation_id": operation.operation_id,
        "operation_type": operation.operation_type,
        "database": operation.database,
        "state": state,
        "status": state.to_ascii_lowercase(),
        "created_at": operation.created_at.to_rfc3339(),
        "finished_at": operation.finished_at.map(|value| value.to_rfc3339()),
        "error": error,
    })
}

async fn operation_cancel_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<axum::Extension<Subject>>,
    Path(operation_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let Some(job) = state.ddl_jobs.get(&operation_id) else {
        let operation = state
            .admission
            .operation(&operation_id)
            .ok_or(AppError::NotFound)?;
        authorize_request(
            &state,
            subject.as_ref().map(|value| &value.0),
            Action::Create,
            &operation.database,
            None,
        )
        .await?;
        return Err(AppError::BadRequest(
            "operation cannot be cancelled safely after blocking execution started".to_string(),
        ));
    };
    authorize_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Create,
        &job.database,
        None,
    )
    .await?;
    match state.ddl_jobs.cancel(&operation_id) {
        Ok(job) => Ok(Json(job.public_json())),
        Err(CancelError::NotFound) => Err(AppError::NotFound),
        Err(CancelError::UnsafePhase(phase)) => Err(AppError::BadRequest(format!(
            "operation cannot be cancelled safely during {phase:?}"
        ))),
        Err(CancelError::Terminal(state)) => Err(AppError::BadRequest(format!(
            "operation is already terminal ({state:?})"
        ))),
        Err(CancelError::Storage(error)) => Err(AppError::Storage(error)),
    }
}

fn extract_database_name(headers: &HeaderMap) -> Result<&str, AppError> {
    headers
        .get("X-Database")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("X-Database header required".to_string()))
}

/// Resolve a database from X-Database without blocking the async runtime.
async fn extract_database(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Arc<Database>, AppError> {
    let namespace = state.namespace.clone();
    let database = extract_database_name(headers)?.to_string();
    Ok(state
        .admission
        .run_db(move || namespace.get_database(&database))
        .await??)
}

pub(crate) async fn authorize_request(
    state: &AppState,
    subject: Option<&Subject>,
    action: Action,
    database: &str,
    collection: Option<&str>,
) -> Result<(), AppError> {
    let auth = state.auth.clone();
    let subject = subject.cloned();
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
        .await?
        .map_err(|error| AppError::Forbidden(error.to_string()))
}

async fn authorize_database_request(
    state: &AppState,
    subject: Option<&Subject>,
    action: Action,
    headers: &HeaderMap,
    collection: Option<&str>,
) -> Result<(), AppError> {
    authorize_request(
        state,
        subject,
        action,
        extract_database_name(headers)?,
        collection,
    )
    .await
}

/// POST /sql
async fn sql_handler(
    State(state): State<Arc<AppState>>,
    resolved: Option<axum::Extension<crate::auth::ResolvedSubject>>,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(req): Json<SqlRequest>,
) -> Result<Response, AppError> {
    let peer = connect_info.map(|Extension(ConnectInfo(peer))| peer);
    state.admission.try_acquire_query_for(resolve_client_ip(
        peer,
        &headers,
        state.trusted_proxy_hops,
    ))?;

    let session_id = headers
        .get("X-Session-Id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let start = std::time::Instant::now();
    let (query_timeout, timeout_is_explicit) =
        query_timeout_from_headers(&headers, state.query_timeout_default)?;
    let server_deadline = start + state.admission.db_deadline();

    let database_name = extract_database_name(&headers)?.to_string();
    let database = extract_database(&state, &headers).await?;

    let resolved_subject = resolved.map(|axum::Extension(subject)| subject);
    let tracked_query_timeout = if timeout_is_explicit {
        query_timeout
    } else {
        stored_session_query_timeout(
            &state,
            session_id.as_deref(),
            resolved_subject.as_ref(),
            &database_name,
        )
        .or(query_timeout)
    };
    let response_deadline =
        effective_absolute_deadline(start, tracked_query_timeout, state.admission.db_deadline());
    let operation_id = req.operation_id;
    let query_str = req.query;
    let (query_hash, normalized) = crate::metrics::normalize_query(&query_str);
    let parsed = match QueryBuilder::parse(&query_str) {
        Ok(parsed) => parsed,
        Err(error) => {
            state.metrics.record_query_end(
                query_hash,
                start.elapsed().as_micros() as u64,
                0,
                0,
                true,
            );
            state
                .metrics
                .store_query_fingerprint(query_hash, normalized.as_deref());
            return Err(AppError::QueryFailed(error.into()));
        }
    };
    if let Some((job, created)) = maybe_enqueue_create_index_job(
        &state,
        &parsed,
        database.clone(),
        &database_name,
        resolved_subject.as_ref(),
        false,
        operation_id,
    )
    .await?
    {
        return Ok(create_index_accepted_response(job, created));
    }
    let contains_ddl = parsed.contains_ddl();

    let sessions = state.sessions.clone();
    let auth_arc = state.auth.clone();
    let sid = session_id.clone();
    let operation_scope = crate::server::admission::OperationScope::ddl(database_name.clone());
    let operation = move || {
        let auth_ref = auth_arc.as_deref();
        let mut ctx = ExecuteContext::with_session(&sessions, sid.as_deref())
            .with_database(database_name)
            .with_auth(auth_ref)
            .with_resolved_subject(resolved_subject)
            .with_server_db_deadline(start, server_deadline);
        ctx = if timeout_is_explicit {
            ctx.with_explicit_query_timeout(query_timeout)
        } else {
            ctx.with_query_timeout(query_timeout)
        };
        parsed
            .bind(database)
            .map_err(query::QueryError::from)?
            .execute(&ctx)
            .map_err(query::QueryError::from)
    };
    let result = if contains_ddl {
        state
            .admission
            .run_db_with_timeout(
                response_deadline.saturating_duration_since(std::time::Instant::now()),
                operation_scope,
                operation,
                move |result| match result {
                    Ok(batch) => crate::server::admission::OperationOutcome::Completed {
                        result: Some(legacy_query_json(batch, start)),
                    },
                    Err(error) => crate::server::admission::OperationOutcome::Failed {
                        code: crate::error::ErrorCode::code(error).to_string(),
                        message: crate::error::safe_query_error_message(error),
                    },
                },
            )
            .await?
    } else {
        state.admission.run_db_cooperative(operation).await?
    };

    // Record metrics
    let elapsed_us = start.elapsed().as_micros() as u64;
    let (rows_returned, is_error) = match &result {
        Ok(batch) => {
            let rows: u64 = batch.results.iter().map(|r| r.row_count() as u64).sum();
            (rows, false)
        }
        Err(_) => (0, true),
    };

    state
        .metrics
        .record_query_end(query_hash, elapsed_us, rows_returned, 0, is_error);
    state
        .metrics
        .store_query_fingerprint(query_hash, normalized.as_deref());

    let result = result.map_err(AppError::QueryFailed)?;
    if let Some(error) = transport_timeout_error(&result) {
        return Err(error);
    }

    Ok(Json(legacy_query_json(&result, start)).into_response())
}

/// POST /v1/query
async fn v1_query_handler(
    State(state): State<Arc<AppState>>,
    resolved: Option<axum::Extension<crate::auth::ResolvedSubject>>,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(req): Json<SqlRequest>,
) -> Result<Response, AppError> {
    let peer = connect_info.map(|Extension(ConnectInfo(peer))| peer);
    state.admission.try_acquire_query_for(resolve_client_ip(
        peer,
        &headers,
        state.trusted_proxy_hops,
    ))?;

    let session_id = headers
        .get("X-Session-Id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let start = std::time::Instant::now();
    let (query_timeout, timeout_is_explicit) =
        query_timeout_from_headers(&headers, state.query_timeout_default)?;
    let server_deadline = start + state.admission.db_deadline();
    let database_name = extract_database_name(&headers)?.to_string();
    let database = extract_database(&state, &headers).await?;
    let resolved_subject = resolved.map(|axum::Extension(subject)| subject);
    let tracked_query_timeout = if timeout_is_explicit {
        query_timeout
    } else {
        stored_session_query_timeout(
            &state,
            session_id.as_deref(),
            resolved_subject.as_ref(),
            &database_name,
        )
        .or(query_timeout)
    };
    let response_deadline =
        effective_absolute_deadline(start, tracked_query_timeout, state.admission.db_deadline());
    let operation_id = req.operation_id;
    let query_str = req.query;
    let (query_hash, normalized) = crate::metrics::normalize_query(&query_str);
    let parsed = match QueryBuilder::parse(&query_str) {
        Ok(parsed) => parsed,
        Err(error) => {
            state.metrics.record_query_end(
                query_hash,
                start.elapsed().as_micros() as u64,
                0,
                0,
                true,
            );
            state
                .metrics
                .store_query_fingerprint(query_hash, normalized.as_deref());
            return Err(AppError::QueryFailed(error.into()));
        }
    };
    if let Some((job, created)) = maybe_enqueue_create_index_job(
        &state,
        &parsed,
        database.clone(),
        &database_name,
        resolved_subject.as_ref(),
        false,
        operation_id,
    )
    .await?
    {
        return Ok(create_index_accepted_response(job, created));
    }
    let contains_ddl = parsed.contains_ddl();

    let sessions = state.sessions.clone();
    let auth_arc = state.auth.clone();
    let sid = session_id.clone();
    let operation_scope = crate::server::admission::OperationScope::ddl(database_name.clone());
    let operation = move || {
        let auth_ref = auth_arc.as_deref();
        let mut ctx = ExecuteContext::with_session(&sessions, sid.as_deref())
            .with_database(database_name)
            .with_auth(auth_ref)
            .with_resolved_subject(resolved_subject)
            .with_server_db_deadline(start, server_deadline);
        ctx = if timeout_is_explicit {
            ctx.with_explicit_query_timeout(query_timeout)
        } else {
            ctx.with_query_timeout(query_timeout)
        };
        parsed
            .bind(database)
            .map_err(query::QueryError::from)?
            .execute(&ctx)
            .map_err(query::QueryError::from)
    };
    let result = if contains_ddl {
        state
            .admission
            .run_db_with_timeout(
                response_deadline.saturating_duration_since(std::time::Instant::now()),
                operation_scope,
                operation,
                move |result| match result {
                    Ok(batch) => crate::server::admission::OperationOutcome::Completed {
                        result: Some(v1_query_json(batch, start)),
                    },
                    Err(error) => crate::server::admission::OperationOutcome::Failed {
                        code: crate::error::ErrorCode::code(error).to_string(),
                        message: crate::error::safe_query_error_message(error),
                    },
                },
            )
            .await?
    } else {
        state.admission.run_db_cooperative(operation).await?
    };

    let elapsed_us = start.elapsed().as_micros() as u64;
    let (rows_returned, is_error) = match &result {
        Ok(batch) => {
            let rows: u64 = batch.results.iter().map(|r| r.row_count() as u64).sum();
            (rows, false)
        }
        Err(_) => (0, true),
    };

    state
        .metrics
        .record_query_end(query_hash, elapsed_us, rows_returned, 0, is_error);
    state
        .metrics
        .store_query_fingerprint(query_hash, normalized.as_deref());

    let result = result.map_err(AppError::QueryFailed)?;
    if let Some(error) = transport_timeout_error(&result) {
        return Err(error);
    }
    Ok(Json(v1_query_json(&result, start)).into_response())
}

/// GET /v1/collections?include=stats,schema
async fn v1_collections_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<axum::Extension<Subject>>,
    headers: HeaderMap,
    Query(query): Query<V1CollectionsQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    authorize_database_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Select,
        &headers,
        None,
    )
    .await?;
    let include = query.include.unwrap_or_default();
    let include_stats = include.split(',').any(|part| part.trim() == "stats");
    let include_schema = include.split(',').any(|part| part.trim() == "schema");

    let database = extract_database(&state, &headers).await?;

    let collections = state
        .admission
        .run_db(move || {
            let mut names = database.list_collections();
            names.sort();

            let mut items = Vec::with_capacity(names.len());
            for name in names {
                let schema = database.get_schema(&name);
                let mode = schema
                    .as_ref()
                    .map(|schema| match schema.mode {
                        crate::schema::SchemaMode::Strict => "strict",
                        crate::schema::SchemaMode::Flexible => "flexible",
                    })
                    .unwrap_or("flexible");

                let mut item = json!({
                    "name": name.clone(),
                    "mode": mode,
                });

                if include_stats {
                    item["stats"] = json!({
                        "document_count": database.count_documents(&name)?,
                    });
                }

                if include_schema {
                    item["schema"] = match schema {
                        Some(schema) => collection_schema_to_v1_json(&schema),
                        None => json!({
                            "name": name,
                            "mode": mode,
                            "fields": [],
                            "indexes": [],
                        }),
                    };
                }

                items.push(item);
            }

            Ok::<_, StorageError>(items)
        })
        .await??;
    let count = collections.len();

    Ok(Json(json!({
        "collections": collections,
        "count": count,
    })))
}

/// GET /collections/:collection/documents
async fn list_documents_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<axum::Extension<Subject>>,
    headers: HeaderMap,
    Path(collection): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    authorize_database_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Select,
        &headers,
        Some(&collection),
    )
    .await?;
    // Validate collection name (reject empty strings from double-slash URLs)
    if collection.is_empty() {
        return Err(AppError::BadRequest(
            "Collection name cannot be empty".to_string(),
        ));
    }

    let database = extract_database(&state, &headers).await?;
    let metrics = state.metrics.clone();
    let coll_name = collection.clone();

    let (docs, next_cursor) = state
        .admission
        .run_db({
            let after = query.after.clone();
            move || {
                database.list_documents(&collection, after.as_deref(), query.limit.clamp(1, 1000))
            }
        })
        .await??;

    metrics.record_collection_read(&coll_name);

    let documents: Vec<serde_json::Value> = docs.iter().map(document_to_json).collect();
    let has_more = next_cursor.is_some();

    Ok(Json(json!({
        "documents": documents,
        "next_cursor": next_cursor,
        "has_more": has_more
    })))
}

/// POST /collections/:collection/documents
async fn create_document_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<axum::Extension<Subject>>,
    headers: HeaderMap,
    Path(collection): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    authorize_database_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Insert,
        &headers,
        Some(&collection),
    )
    .await?;
    // Validate collection name
    if collection.is_empty() {
        return Err(AppError::BadRequest(
            "Collection name cannot be empty".to_string(),
        ));
    }

    // Extract key from id if provided, otherwise generate nanoid
    let key = match body.get("id").and_then(|v| v.as_str()) {
        Some(id) => parse_key_from_id(id, &collection)?,
        None => nanoid::nanoid!(),
    };

    let id = format!("{}:{}", collection, key);

    let doc = Document {
        id: id.clone(),
        fields: extract_user_fields(body),
    };

    // Atomic create (handles TOCTOU)
    let database = extract_database(&state, &headers).await?;
    let created = state
        .admission
        .run_db({
            let doc = doc.clone();
            move || database.create_document(&doc)
        })
        .await??;

    if created {
        state.metrics.record_collection_write(&collection);
        Ok((StatusCode::CREATED, Json(document_to_json(&doc))))
    } else {
        Err(AppError::Conflict(id))
    }
}

/// GET /collections/:collection/documents/:key
async fn get_document_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<axum::Extension<Subject>>,
    headers: HeaderMap,
    Path((collection, key)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    authorize_database_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Select,
        &headers,
        Some(&collection),
    )
    .await?;
    let database = extract_database(&state, &headers).await?;
    let coll_name = collection.clone();
    let doc = state
        .admission
        .run_db(move || database.get_document(&collection, &key))
        .await??;

    state.metrics.record_collection_read(&coll_name);

    match doc {
        Some(doc) => Ok(Json(document_to_json(&doc))),
        None => Err(AppError::NotFound),
    }
}

/// PUT /collections/:collection/documents/:key
async fn update_document_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<axum::Extension<Subject>>,
    headers: HeaderMap,
    Path((collection, key)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    authorize_database_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Update,
        &headers,
        Some(&collection),
    )
    .await?;
    let id = format!("{}:{}", collection, key);

    let doc = Document {
        id: id.clone(),
        fields: extract_user_fields(body),
    };

    // Atomic update
    let database = extract_database(&state, &headers).await?;
    let updated = state
        .admission
        .run_db({
            let doc = doc.clone();
            move || database.update_document(&doc)
        })
        .await??;

    if updated {
        state.metrics.record_collection_write(&collection);
        Ok(Json(document_to_json(&doc)))
    } else {
        Err(AppError::NotFound)
    }
}

/// DELETE /collections/:collection/documents/:key
async fn delete_document_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<axum::Extension<Subject>>,
    headers: HeaderMap,
    Path((collection, key)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    authorize_database_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Delete,
        &headers,
        Some(&collection),
    )
    .await?;
    let database = extract_database(&state, &headers).await?;
    let coll_name = collection.clone();
    let deleted = state
        .admission
        .run_db(move || database.delete_document(&collection, &key))
        .await??;

    if deleted {
        state.metrics.record_collection_delete(&coll_name);
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound)
    }
}

/// Parse key from a full id like "collection:key"
fn parse_key_from_id(id: &str, expected_collection: &str) -> Result<String, AppError> {
    match id.split_once(':') {
        Some((col, key)) => {
            if col != expected_collection {
                return Err(AppError::BadRequest(format!(
                    "ID collection '{}' does not match URL collection '{}'",
                    col, expected_collection
                )));
            }
            if key.is_empty() {
                return Err(AppError::BadRequest("Key cannot be empty".to_string()));
            }
            Ok(key.to_string())
        }
        None => {
            // No colon, treat the whole id as the key
            if id.is_empty() {
                return Err(AppError::BadRequest("Key cannot be empty".to_string()));
            }
            Ok(id.to_string())
        }
    }
}

/// Convert Document to JSON for response
fn document_to_json(doc: &Document) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("id".to_string(), json!(doc.id));

    for (k, v) in &doc.fields {
        obj.insert(k.clone(), serde_json::Value::from(v.clone()));
    }

    serde_json::Value::Object(obj)
}

/// Convert Edge to JSON for response
fn edge_to_json(edge: &Edge) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("id".to_string(), json!(edge.id));
    obj.insert("from".to_string(), json!(edge.from));
    obj.insert("to".to_string(), json!(edge.to));

    for (k, v) in &edge.fields {
        obj.insert(k.clone(), serde_json::Value::from(v.clone()));
    }

    serde_json::Value::Object(obj)
}

/// POST /nodes/:from/edges/:label/:to
async fn create_edge_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<axum::Extension<Subject>>,
    headers: HeaderMap,
    Path((from, label, to)): Path<(String, String, String)>,
    Json(body): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    authorize_database_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Traverse,
        &headers,
        None,
    )
    .await?;
    let database = extract_database(&state, &headers).await?;
    let fields = extract_user_fields(body);

    let edge = Edge::new(from, label, to).with_fields(fields);

    state
        .admission
        .run_db({
            let edge = edge.clone();
            move || database.create_edge(&edge)
        })
        .await??;

    Ok((StatusCode::CREATED, Json(edge_to_json(&edge))))
}

/// GET /nodes/:from/edges/:label/:to
async fn get_edge_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<axum::Extension<Subject>>,
    headers: HeaderMap,
    Path((from, label, to)): Path<(String, String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    authorize_database_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Traverse,
        &headers,
        None,
    )
    .await?;
    let database = extract_database(&state, &headers).await?;
    let edge = state
        .admission
        .run_db(move || database.get_edge(&from, &label, &to))
        .await??;

    match edge {
        Some(e) => Ok(Json(edge_to_json(&e))),
        None => Err(AppError::NotFound),
    }
}

/// DELETE /nodes/:from/edges/:label/:to
async fn delete_edge_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<axum::Extension<Subject>>,
    headers: HeaderMap,
    Path((from, label, to)): Path<(String, String, String)>,
) -> Result<StatusCode, AppError> {
    authorize_database_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Delete,
        &headers,
        None,
    )
    .await?;
    let database = extract_database(&state, &headers).await?;
    let deleted = state
        .admission
        .run_db(move || database.delete_edge(&from, &label, &to))
        .await??;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound)
    }
}

/// GET /nodes/:id/out/:label
async fn list_edges_out_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<axum::Extension<Subject>>,
    headers: HeaderMap,
    Path((id, label)): Path<(String, String)>,
    Query(query): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    authorize_database_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Traverse,
        &headers,
        None,
    )
    .await?;
    let database = extract_database(&state, &headers).await?;
    let (edges, next_cursor) = state
        .admission
        .run_db({
            let after = query.after.clone();
            move || {
                database.list_edges_out(&id, &label, after.as_deref(), query.limit.clamp(1, 1000))
            }
        })
        .await??;

    let edges_json: Vec<serde_json::Value> = edges.iter().map(edge_to_json).collect();
    let has_more = next_cursor.is_some();

    Ok(Json(json!({
        "edges": edges_json,
        "next_cursor": next_cursor,
        "has_more": has_more
    })))
}

/// GET /nodes/:id/in/:label
async fn list_edges_in_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<axum::Extension<Subject>>,
    headers: HeaderMap,
    Path((id, label)): Path<(String, String)>,
    Query(query): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    authorize_database_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Traverse,
        &headers,
        None,
    )
    .await?;
    let database = extract_database(&state, &headers).await?;
    let (edges, next_cursor) = state
        .admission
        .run_db({
            let after = query.after.clone();
            move || {
                database.list_edges_in(&id, &label, after.as_deref(), query.limit.clamp(1, 1000))
            }
        })
        .await??;

    let edges_json: Vec<serde_json::Value> = edges.iter().map(edge_to_json).collect();
    let has_more = next_cursor.is_some();

    Ok(Json(json!({
        "edges": edges_json,
        "next_cursor": next_cursor,
        "has_more": has_more
    })))
}

// --- Database management handlers ---

/// GET /databases
async fn list_databases_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<axum::Extension<Subject>>,
) -> Result<Json<serde_json::Value>, AppError> {
    authorize_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Manage,
        "",
        None,
    )
    .await?;
    Ok(Json(json!(state.namespace.list_databases())))
}

/// POST /databases
async fn create_database_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<axum::Extension<Subject>>,
    Json(body): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let name = body["name"]
        .as_str()
        .ok_or_else(|| AppError::BadRequest("Missing 'name' field".to_string()))?
        .to_string();
    authorize_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Manage,
        &name,
        None,
    )
    .await?;

    state
        .admission
        .run_db({
            let namespace = state.namespace.clone();
            let name = name.clone();
            move || namespace.create_database(&name)
        })
        .await??;

    Ok((
        StatusCode::CREATED,
        Json(json!({"name": name, "status": "created"})),
    ))
}

/// DELETE /databases/:name
async fn drop_database_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<axum::Extension<Subject>>,
    Path(name): Path<String>,
) -> Result<StatusCode, AppError> {
    authorize_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Manage,
        &name,
        None,
    )
    .await?;
    state
        .admission
        .run_db({
            let namespace = state.namespace.clone();
            move || namespace.drop_database(&name)
        })
        .await??;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /databases/:name/stats
async fn database_stats_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<axum::Extension<Subject>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    authorize_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Manage,
        &name,
        None,
    )
    .await?;
    let stats = state
        .admission
        .run_db({
            let namespace = state.namespace.clone();
            let name = name.clone();
            move || {
                namespace
                    .get_database(&name)
                    .map(|database| database.stats())
            }
        })
        .await??;
    Ok(Json(json!({
        "name": name,
        "collections": stats.collections,
        "edge_labels": stats.edge_labels,
        "btree_indexes": stats.btree_indexes,
        "vector_indexes": stats.vector_indexes,
        "fts_indexes": stats.fts_indexes,
        "disk_size": stats.disk_size,
        "disk_size_human": stats.disk_size_human(),
    })))
}

// --- Auth handlers ---

#[derive(Debug, serde::Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

fn resolve_client_ip(
    peer: Option<SocketAddr>,
    headers: &HeaderMap,
    trusted_proxy_hops: usize,
) -> Option<IpAddr> {
    let peer_ip = peer.map(|address| address.ip());
    if trusted_proxy_hops == 0 {
        return peer_ip;
    }
    let Some(forwarded) = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
    else {
        return peer_ip;
    };
    let Some(chain): Option<Vec<IpAddr>> = forwarded
        .split(',')
        .map(|value| value.trim().parse::<IpAddr>().ok())
        .collect()
    else {
        return peer_ip;
    };
    chain
        .len()
        .checked_sub(trusted_proxy_hops)
        .and_then(|index| chain.get(index).copied())
        .or(peer_ip)
}

/// POST /auth/login
async fn login_handler(
    State(state): State<Arc<AppState>>,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let auth = state
        .auth
        .clone()
        .ok_or_else(|| AppError::BadRequest("Auth not initialized".to_string()))?;
    let peer = connect_info.map(|Extension(ConnectInfo(peer))| peer);
    let client_ip = resolve_client_ip(peer, &headers, state.trusted_proxy_hops);
    let permit = state.admission.try_acquire_login_for(client_ip)?;
    let jwt_ttl = auth.jwt_config().ttl_secs();
    let username = req.username;
    let password = req.password;
    let token = state
        .admission
        .run_login(permit, move || auth.login(&username, &password))
        .await?
        .map_err(|_| AppError::Unauthorized("invalid credentials".to_string()))?;

    let ttl = i64::try_from(jwt_ttl)
        .ok()
        .and_then(chrono::Duration::try_seconds)
        .ok_or_else(|| AppError::BadRequest("JWT lifetime is out of range".to_string()))?;
    let expires_at = chrono::Utc::now()
        .checked_add_signed(ttl)
        .ok_or_else(|| AppError::BadRequest("JWT expiry is out of range".to_string()))?;

    Ok(Json(json!({
        "token": token,
        "expires_at": expires_at.to_rfc3339()
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdminListQuery {
    #[serde(default = "default_limit")]
    limit: usize,
    after: Option<String>,
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateManagedUserRequest {
    username: String,
    password: String,
    #[serde(default)]
    attributes: std::collections::HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateManagedUserRequest {
    #[serde(default)]
    set: std::collections::HashMap<String, String>,
    #[serde(default)]
    remove: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateManagedUserPasswordRequest {
    password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateManagedApiKeyRequest {
    name: String,
    username: String,
}

async fn identity_admin_auth(
    state: &Arc<AppState>,
    subject: Option<&Subject>,
) -> Result<Arc<crate::auth::AuthService>, AppError> {
    let auth = state
        .auth
        .clone()
        .ok_or_else(|| AppError::BadRequest("Auth not initialized".to_string()))?;
    authorize_request(state, subject, Action::Manage, "", None).await?;
    Ok(auth)
}

fn validate_identity_name(value: &str, label: &str) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::BadRequest(format!("{label} cannot be empty")));
    }
    if value.chars().count() > 128 {
        return Err(AppError::BadRequest(format!(
            "{label} cannot exceed 128 characters"
        )));
    }
    Ok(value.to_string())
}

/// GET /v1/auth/me
async fn auth_me_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<Extension<Subject>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let subject = subject
        .map(|Extension(subject)| subject)
        .ok_or_else(|| AppError::Unauthorized("authentication required".to_string()))?;
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    let is_api_key = crate::auth::api_key::is_api_key(token);
    let credential_name = if is_api_key {
        let auth = state
            .auth
            .clone()
            .ok_or_else(|| AppError::BadRequest("Auth not initialized".to_string()))?;
        let token = token.to_string();
        state
            .admission
            .run_db(move || auth.api_key_name_for_token(&token))
            .await??
    } else {
        None
    };

    Ok((
        [(CACHE_CONTROL, "no-store"), (PRAGMA, "no-cache")],
        Json(json!({
            "username": subject.user_id,
            "attributes": subject.attributes,
            "authMethod": if is_api_key { "apiKey" } else { "credentials" },
            "credentialName": credential_name,
        })),
    ))
}

/// GET /v1/admin/users
async fn list_managed_users_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<Extension<Subject>>,
    Query(query): Query<AdminListQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let auth =
        identity_admin_auth(&state, subject.as_ref().map(|Extension(subject)| subject)).await?;
    let after = query.after;
    let (users, next_cursor) = state
        .admission
        .run_db(move || auth.list_users_typed(after.as_deref(), query.limit))
        .await??;
    Ok(Json(json!({ "items": users, "nextCursor": next_cursor })))
}

/// POST /v1/admin/users
async fn create_managed_user_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<Extension<Subject>>,
    Json(request): Json<CreateManagedUserRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let auth =
        identity_admin_auth(&state, subject.as_ref().map(|Extension(subject)| subject)).await?;
    let username = validate_identity_name(&request.username, "Username")?;
    if request.password.is_empty() {
        return Err(AppError::BadRequest("Password cannot be empty".to_string()));
    }
    if request.attributes.keys().any(|key| key.trim().is_empty()) {
        return Err(AppError::BadRequest(
            "Attribute names cannot be empty".to_string(),
        ));
    }
    let password = request.password;
    let attributes = request.attributes;
    let user = state
        .admission
        .run_db(move || {
            auth.create_user_with_attributes_typed(&username, &password, attributes)?;
            auth.get_user_typed(&username)
        })
        .await??;
    Ok((StatusCode::CREATED, Json(json!({ "user": user }))))
}

/// GET /v1/admin/users/:username
async fn get_managed_user_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<Extension<Subject>>,
    Path(username): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let auth =
        identity_admin_auth(&state, subject.as_ref().map(|Extension(subject)| subject)).await?;
    let user = state
        .admission
        .run_db(move || auth.get_user_typed(&username))
        .await??;
    Ok(Json(json!({ "user": user })))
}

/// PATCH /v1/admin/users/:username
async fn update_managed_user_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<Extension<Subject>>,
    Path(username): Path<String>,
    Json(request): Json<UpdateManagedUserRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let auth =
        identity_admin_auth(&state, subject.as_ref().map(|Extension(subject)| subject)).await?;
    if request.set.keys().any(|key| key.trim().is_empty())
        || request.remove.iter().any(|key| key.trim().is_empty())
    {
        return Err(AppError::BadRequest(
            "Attribute names cannot be empty".to_string(),
        ));
    }
    let user = state
        .admission
        .run_db(move || {
            auth.alter_user_attributes_typed(&username, request.set, &request.remove)?;
            auth.get_user_typed(&username)
        })
        .await??;
    Ok(Json(json!({ "user": user })))
}

/// PUT /v1/admin/users/:username/password
async fn update_managed_user_password_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<Extension<Subject>>,
    Path(username): Path<String>,
    Json(request): Json<UpdateManagedUserPasswordRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let auth =
        identity_admin_auth(&state, subject.as_ref().map(|Extension(subject)| subject)).await?;
    if request.password.is_empty() {
        return Err(AppError::BadRequest("Password cannot be empty".to_string()));
    }
    let user = state
        .admission
        .run_db(move || {
            auth.alter_user_password_typed(&username, &request.password)?;
            auth.get_user_typed(&username)
        })
        .await??;
    Ok(Json(json!({ "user": user })))
}

/// DELETE /v1/admin/users/:username
async fn drop_managed_user_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<Extension<Subject>>,
    Path(username): Path<String>,
) -> Result<StatusCode, AppError> {
    let auth =
        identity_admin_auth(&state, subject.as_ref().map(|Extension(subject)| subject)).await?;
    state
        .admission
        .run_db(move || auth.drop_user_typed(&username))
        .await??;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /v1/admin/api-keys
async fn list_managed_api_keys_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<Extension<Subject>>,
    Query(query): Query<AdminListQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let auth =
        identity_admin_auth(&state, subject.as_ref().map(|Extension(subject)| subject)).await?;
    let after = query.after;
    let username = query.username;
    let (keys, next_cursor) = state
        .admission
        .run_db(move || {
            auth.list_api_keys_typed(after.as_deref(), query.limit, username.as_deref())
        })
        .await??;
    Ok(Json(json!({ "items": keys, "nextCursor": next_cursor })))
}

/// POST /v1/admin/api-keys
async fn create_managed_api_key_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<Extension<Subject>>,
    Json(request): Json<CreateManagedApiKeyRequest>,
) -> Result<impl IntoResponse, AppError> {
    let auth =
        identity_admin_auth(&state, subject.as_ref().map(|Extension(subject)| subject)).await?;
    let name = validate_identity_name(&request.name, "API key name")?;
    let username = validate_identity_name(&request.username, "Username")?;
    let (key, secret) = state
        .admission
        .run_db(move || {
            let secret = auth.create_api_key_typed(&name, &username)?;
            let key = auth.get_api_key_typed(&name)?;
            Ok::<_, crate::auth::AuthError>((key, secret))
        })
        .await??;
    Ok((
        StatusCode::CREATED,
        [(CACHE_CONTROL, "no-store"), (PRAGMA, "no-cache")],
        Json(json!({ "key": key, "secret": secret })),
    ))
}

/// DELETE /v1/admin/api-keys/:name
async fn drop_managed_api_key_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<Extension<Subject>>,
    Path(name): Path<String>,
) -> Result<StatusCode, AppError> {
    let auth =
        identity_admin_auth(&state, subject.as_ref().map(|Extension(subject)| subject)).await?;
    state
        .admission
        .run_db(move || auth.drop_api_key_typed(&name))
        .await??;
    Ok(StatusCode::NO_CONTENT)
}
