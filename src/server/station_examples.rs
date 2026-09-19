//! Operator-curated example queries shown in Station's query workspace.
//!
//! Loaded once at startup from a JSON file keyed by database name:
//!
//! ```json
//! { "shop": [ { "title": "Top products", "description": "…", "sql": "SELECT …" } ] }
//! ```
//!
//! Examples are served per database and gated by `SELECT` authorization on
//! that database, so anonymous and read-only subjects see exactly what they
//! may run.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::auth::{Action, Subject};
use crate::error::AppError;
use crate::server::{AppState, authorize_request, extract_database_name};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StationExample {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub sql: String,
}

pub type StationExamples = HashMap<String, Vec<StationExample>>;

pub fn load(path: &Path) -> Result<StationExamples, String> {
    let raw =
        std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let examples: StationExamples = serde_json::from_slice(&raw)
        .map_err(|error| format!("invalid station examples in {}: {error}", path.display()))?;
    for (database, items) in &examples {
        if let Some(bad) = items
            .iter()
            .find(|item| item.title.trim().is_empty() || item.sql.trim().is_empty())
        {
            return Err(format!(
                "station example for '{database}' has an empty title or sql: {bad:?}"
            ));
        }
    }
    Ok(examples)
}

/// GET /v1/station/examples — examples for the `X-Database` database.
pub async fn examples_handler(
    State(state): State<Arc<AppState>>,
    subject: Option<axum::Extension<Subject>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, AppError> {
    let database = extract_database_name(&headers)?;
    authorize_request(
        &state,
        subject.as_ref().map(|value| &value.0),
        Action::Select,
        database,
        None,
    )
    .await?;
    let items = state
        .station_examples
        .get(database)
        .map(Vec::as_slice)
        .unwrap_or_default();
    Ok(Json(json!({ "items": items })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_examples_without_sql() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("examples.json");
        std::fs::write(&path, r#"{"shop":[{"title":"x","sql":"  "}]}"#).unwrap();
        assert!(load(&path).unwrap_err().contains("empty title or sql"));
    }
}
