//! FTS backend stub used when the `fts` feature is disabled.

use std::path::PathBuf;
use std::sync::Arc;

use fjall::OptimisticTxKeyspace;

use crate::document::{Document, Value};
use crate::error::StorageError;
use crate::schema::AnalyzerDef;

use super::analyzers::AnalyzerRegistry;
use super::traits::{IndexBackend, IndexType, RangeOp, ScanResult};

fn unavailable() -> StorageError {
    StorageError::InvalidConfig(
        "FULLTEXT support is not enabled in this build; rebuild with --features fts".into(),
    )
}

/// Statistics from a REINDEX operation.
#[derive(Debug, Clone, Default)]
pub struct ReindexStats {
    pub docs_indexed: usize,
    pub docs_skipped: usize,
    pub concurrent_modifications: usize,
}

/// Placeholder shadow index type for no-FTS builds.
pub struct FtsIndexInstance;

/// Disabled FTS backend.
pub struct FtsIndex {
    analyzer_registry: AnalyzerRegistry,
}

/// No-op counterpart of the FTS commit guard for builds without FULLTEXT.
pub(crate) struct FtsCommitGuard<'a>(std::marker::PhantomData<&'a ()>);

impl FtsIndex {
    pub fn new(_base_path: PathBuf) -> Self {
        Self {
            analyzer_registry: AnalyzerRegistry::new(),
        }
    }

    pub fn analyzer_registry(&self) -> &AnalyzerRegistry {
        &self.analyzer_registry
    }

    pub fn init_index(
        &self,
        _collection: &str,
        _index_name: &str,
        _fields: &[String],
        _analyzer: Option<&str>,
    ) -> Result<(), StorageError> {
        Err(unavailable())
    }

    pub(crate) fn init_build(
        &self,
        _collection: &str,
        _build_name: &str,
        _fields: &[String],
        _analyzer: Option<&str>,
    ) -> Result<(), StorageError> {
        Err(unavailable())
    }

    pub(crate) fn index_build_document(
        &self,
        _collection: &str,
        _build_name: &str,
        _fields: &[String],
        _doc: &Document,
    ) -> Result<(), StorageError> {
        Err(unavailable())
    }

    pub(crate) fn promote_build(
        &self,
        _collection: &str,
        _build_name: &str,
        _index_name: &str,
        _analyzer: Option<&str>,
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

    pub fn open_existing(
        &self,
        _collection: &str,
        _index_name: &str,
        _analyzer_name: Option<&str>,
    ) -> Result<bool, StorageError> {
        Err(unavailable())
    }

    pub fn search(
        &self,
        _collection: &str,
        _index_name: &str,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        Err(unavailable())
    }

    pub fn search_with_boosts(
        &self,
        _collection: &str,
        _index_name: &str,
        _query: &str,
        _field_boosts: &[(String, f64)],
        _limit: usize,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        Err(unavailable())
    }

    pub fn remove_index(&self, _collection: &str, _index_name: &str) -> Result<(), StorageError> {
        Err(unavailable())
    }

    pub fn commit_all(&self) -> Result<(), StorageError> {
        Ok(())
    }

    pub(crate) fn acquire_commit_guard(&self) -> FtsCommitGuard<'_> {
        FtsCommitGuard(std::marker::PhantomData)
    }

    pub(crate) fn commit_all_guarded(
        &self,
        _guard: &FtsCommitGuard<'_>,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    pub fn flush(&self) -> Result<(), StorageError> {
        Ok(())
    }

    pub fn start_reindex(
        &self,
        _collection: &str,
        _index_name: &str,
        _fields: &[String],
        _analyzer_name: Option<&str>,
    ) -> Result<(), StorageError> {
        Err(unavailable())
    }

    pub fn get_shadow(
        &self,
        _collection: &str,
        _index_name: &str,
    ) -> Option<Arc<FtsIndexInstance>> {
        None
    }

    pub fn index_to_shadow(
        &self,
        _collection: &str,
        _index_name: &str,
        _doc: &Document,
        _fields: &[String],
    ) -> Result<bool, StorageError> {
        Err(unavailable())
    }

    pub fn finish_reindex<F, G>(
        &self,
        _collection: &str,
        _index_name: &str,
        _acquire_lock: F,
    ) -> Result<ReindexStats, StorageError>
    where
        F: FnOnce() -> G,
    {
        Err(unavailable())
    }

    pub fn cancel_reindex(&self, _collection: &str, _index_name: &str) -> Result<(), StorageError> {
        Ok(())
    }

    pub fn load_analyzers(&self, analyzers: Vec<AnalyzerDef>) {
        self.analyzer_registry.load(analyzers);
    }
}

impl Default for FtsIndex {
    fn default() -> Self {
        Self::new(PathBuf::from("data"))
    }
}

impl IndexBackend for FtsIndex {
    fn name(&self) -> &'static str {
        "fts"
    }

    fn index_type(&self) -> IndexType {
        IndexType::FullText
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
    fn disabled_fts_backend_reports_feature_error() {
        let fts = FtsIndex::default();
        let err = fts
            .init_index("docs", "body_fts", &["body".to_string()], None)
            .expect_err("FTS should be unavailable without the fts feature");

        assert!(
            matches!(err, StorageError::InvalidConfig(message) if message.contains("--features fts"))
        );
    }
}
