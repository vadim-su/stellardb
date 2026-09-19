use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Request},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::json;
use std::sync::Arc;

use crate::server::AppState;

/// Paths that don't require authentication
const PUBLIC_PATHS: &[&str] = &["/", "/auth/login", "/health"];

pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let path = request.uri().path();

    // Skip auth for public paths and UI assets
    if PUBLIC_PATHS.contains(&path) || path == "/_station" || path.starts_with("/_station/") {
        return next.run(request).await;
    }

    let auth = match &state.auth {
        Some(a) => a.clone(),
        None => return next.run(request).await, // Auth not configured (e.g., tests)
    };

    let token = match extract_bearer_token(request.headers()) {
        Some(t) => t.to_string(),
        None => {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                axum::Json(json!({"error": "Authorization header required"})),
            )
                .into_response();
        }
    };

    let authentication = match state
        .admission
        .run_db(move || auth.authenticate_resolved(&token))
        .await
    {
        Ok(result) => result,
        Err(error) => return crate::error::AppError::from(error).into_response(),
    };

    match authentication {
        Ok(resolved) => {
            let mut request = request;
            request.extensions_mut().insert(resolved.subject().clone());
            request.extensions_mut().insert(resolved);
            next.run(request).await
        }
        Err(_) => (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::Json(json!({"error": "invalid credentials"})),
        )
            .into_response(),
    }
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}
