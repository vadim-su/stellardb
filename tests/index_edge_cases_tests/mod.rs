//! Comprehensive edge case tests for secondary indexes.
//!
//! Organized by priority:
//! 1. Critical - bugs that would cause data corruption or silent failures
//! 2. Important - correctness issues with compound/nested indexes
//! 3. Useful - robustness tests for edge values and boundaries

mod boundary_values;
mod collection_interaction;
mod compound_indexes;
mod concurrent_ops;
mod decimal;
mod drop_index;
mod heterogeneous_types;
mod nested_fields;
mod numeric_types;
mod order_by;
mod range_queries;
mod unique_constraints;

use std::sync::Arc;
use stellardb::Database;
use tempfile::TempDir;

pub fn setup() -> (TempDir, Arc<Database>) {
    let tmp = TempDir::new().unwrap();
    let storage = Arc::new(Database::open(tmp.path()).unwrap());
    (tmp, storage)
}

pub fn run(storage: &Arc<Database>, sql: &str) -> serde_json::Value {
    stellardb::query_json(storage.clone(), sql)
}

pub fn run_err(storage: &Arc<Database>, sql: &str) -> String {
    stellardb::try_run_sql!(storage.clone(), sql)
        .unwrap_err()
        .to_string()
}
