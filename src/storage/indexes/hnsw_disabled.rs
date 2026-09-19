//! HNSW backend stub used when the `hnsw` feature is disabled.

use std::path::PathBuf;
use std::sync::Arc;

use fjall::OptimisticTxKeyspace;

use crate::document::{Document, Value};
use crate::error::StorageError;
use crate::schema::HnswParams;

use super::traits::{IndexBackend, IndexType, RangeOp, ScanResult};

fn unavailable() -> StorageError {
    StorageError::InvalidConfig(
        "HNSW vector support is not enabled in this build; rebuild with --features hnsw".into(),
    )
}

/// Helper to extract vector from document field.
pub(crate) fn extract_vector(doc: &Document, field: &str) -> Option<Vec<f32>> {
    match doc.fields.get(field) {
        Some(Value::Array(arr)) => Some(
            arr.iter()
                .filter_map(|v| match v {
                    Value::Float(f) => Some(*f as f32),
                    Value::Int(i) => Some(*i as f32),
                    _ => None,
                })
                .collect::<Vec<f32>>(),
        ),
        _ => None,
    }
}

/// Placeholder shadow index type for builds without HNSW support.
pub struct HnswInstance;

/// Disabled HNSW backend.
pub struct HnswIndex;

impl HnswIndex {
    pub fn new(_base_path: PathBuf) -> Self {
        Self
    }

    pub fn get_or_create_instance(
        &self,
        _collection: &str,
        _field: &str,
        _params: &HnswParams,
    ) -> Result<Arc<HnswInstance>, StorageError> {
        Err(unavailable())
    }

    pub(crate) fn init_build(
        &self,
        _collection: &str,
        _build_name: &str,
        _params: &HnswParams,
    ) -> Result<(), StorageError> {
        Err(unavailable())
    }

    pub(crate) fn index_build_document(
        &self,
        _collection: &str,
        _build_name: &str,
        _source_field: &str,
        _doc: &Document,
    ) -> Result<(), StorageError> {
        Err(unavailable())
    }

    pub(crate) fn promote_build(
        &self,
        _collection: &str,
        _build_name: &str,
        _field: &str,
        _params: &HnswParams,
    ) -> Result<(), StorageError> {
        Err(unavailable())
    }

    pub(crate) fn remove_build(
        &self,
        _collection: &str,
        _build_name: &str,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    pub(crate) fn cleanup_orphaned_builds(&self) -> Result<(), StorageError> {
        Ok(())
    }

    pub fn search(
        &self,
        _collection: &str,
        _index_name: &str,
        _vector: &[f32],
        _k: usize,
        _effort: Option<usize>,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        Err(unavailable())
    }

    pub fn validate_vector(
        &self,
        _collection: &str,
        _field: &str,
        _vector: &[f32],
    ) -> Result<(), StorageError> {
        Err(unavailable())
    }

    pub fn start_reindex(
        &self,
        _collection: &str,
        _field: &str,
        _params: &HnswParams,
    ) -> Result<(), StorageError> {
        Err(unavailable())
    }

    pub fn get_shadow(&self, _collection: &str, _field: &str) -> Option<Arc<HnswInstance>> {
        None
    }

    pub fn add_to_shadow(
        &self,
        _collection: &str,
        _field: &str,
        _doc_id: &str,
        _vector: &[f32],
    ) -> Result<bool, StorageError> {
        Err(unavailable())
    }

    pub fn finish_reindex<F, G>(
        &self,
        _collection: &str,
        _field: &str,
        _acquire_lock: F,
    ) -> Result<(), StorageError>
    where
        F: FnOnce() -> G,
    {
        Err(unavailable())
    }

    pub fn cancel_reindex(&self, _collection: &str, _field: &str) -> Result<(), StorageError> {
        Ok(())
    }

    pub fn remove_index(&self, _collection: &str, _field: &str) -> Result<(), StorageError> {
        Err(unavailable())
    }

    pub fn commit(&self) -> Result<(), StorageError> {
        Ok(())
    }
}

impl Default for HnswIndex {
    fn default() -> Self {
        Self::new(PathBuf::from("data"))
    }
}

impl IndexBackend for HnswIndex {
    fn name(&self) -> &'static str {
        "hnsw"
    }

    fn index_type(&self) -> IndexType {
        IndexType::Hnsw
    }

    fn index_document(
        &self,
        _keyspace: &OptimisticTxKeyspace,
        _fields: &[String],
        _doc: &Document,
    ) -> Result<(), StorageError> {
        Err(unavailable())
    }

    fn update_document(
        &self,
        _keyspace: &OptimisticTxKeyspace,
        _fields: &[String],
        _old_doc: Option<&Document>,
        _new_doc: Option<&Document>,
    ) -> Result<(), StorageError> {
        Err(unavailable())
    }

    fn scan_eq(
        &self,
        _keyspace: &OptimisticTxKeyspace,
        _field: &str,
        _value: &Value,
        _limit: usize,
    ) -> Result<ScanResult, StorageError> {
        Err(unavailable())
    }

    fn scan_range(
        &self,
        _keyspace: &OptimisticTxKeyspace,
        _field: &str,
        _op: &RangeOp,
        _value: &Value,
        _limit: usize,
    ) -> Result<ScanResult, StorageError> {
        Err(unavailable())
    }

    fn scan_range_between(
        &self,
        _keyspace: &OptimisticTxKeyspace,
        _field: &str,
        _lower: &Value,
        _lower_inclusive: bool,
        _upper: &Value,
        _upper_inclusive: bool,
        _limit: usize,
    ) -> Result<ScanResult, StorageError> {
        Err(unavailable())
    }

    fn scan_compound_eq(
        &self,
        _keyspace: &OptimisticTxKeyspace,
        _field_values: &[(&str, &Value)],
        _limit: usize,
    ) -> Result<ScanResult, StorageError> {
        Err(unavailable())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_hnsw_backend_reports_feature_error() {
        let hnsw = HnswIndex;
        let err = match hnsw.get_or_create_instance("items", "embedding", &HnswParams::default()) {
            Ok(_) => panic!("HNSW should be unavailable without the hnsw feature"),
            Err(err) => err,
        };

        assert!(
            matches!(err, StorageError::InvalidConfig(message) if message.contains("--features hnsw"))
        );
    }
}
