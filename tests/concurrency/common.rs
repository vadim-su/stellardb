//! Common utilities for concurrency tests.

use std::sync::Arc;
use stellardb::Database;
use tempfile::TempDir;

/// Shared test setup - creates a temporary database.
///
/// Returns a tuple of (TempDir, Arc<Database>).
/// TempDir must be kept alive for the duration of the test.
pub fn setup() -> (TempDir, Arc<Database>) {
    let tmp = TempDir::new().unwrap();
    let storage = Database::open(tmp.path()).unwrap();
    (tmp, Arc::new(storage))
}

/// Run a query, panic on error.
pub fn run(storage: &Arc<Database>, sql: &str) -> serde_json::Value {
    stellardb::query_json(storage.clone(), sql)
}

/// Run a query, return Result.
pub fn try_run(storage: &Arc<Database>, sql: &str) -> Result<serde_json::Value, String> {
    use stellardb::query::execute::operators::operator::rows_to_json;
    stellardb::try_run_sql!(storage.clone(), sql)
        .map(|result| rows_to_json(&result.rows))
        .map_err(|e| e.to_string())
}
