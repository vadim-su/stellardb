//! HTTP handlers for metrics endpoints

use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::State,
    http::header,
    response::{IntoResponse, Response},
};

use crate::auth::{Action, Subject};
use crate::error::AppError;
use crate::metrics::{format_json_stats, format_prometheus};
use crate::server::{AppState, authorize_request};

/// GET /metrics - Prometheus format
pub async fn metrics_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<Extension<Subject>>,
) -> Result<Response, AppError> {
    authorize_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Manage,
        "",
        None,
    )
    .await?;
    let storage_stats = crate::storage::StorageStats::default();
    let body = format_prometheus(&state.metrics, &storage_stats);

    Ok(([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response())
}

/// GET /stats - JSON format
///
/// Returns server-level metrics. For per-database storage stats,
/// use GET /databases/{name}/stats instead.
pub async fn stats_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<Extension<Subject>>,
) -> Result<Json<serde_json::Value>, AppError> {
    authorize_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Manage,
        "",
        None,
    )
    .await?;
    let storage_stats = crate::storage::StorageStats::default();
    let json = format_json_stats(&state.metrics, &storage_stats, 50);
    Ok(Json(json))
}
