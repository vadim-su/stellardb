//! IndexManager - coordinates index backends and operations

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::{RwLock, RwLockReadGuard};

use crate::document::{Document, Value};
use crate::error::StorageError;
use crate::schema::IndexDef;
use crate::schema::IndexType as SchemaIndexType;
use crate::storage::KeyspaceManager;

use super::btree::BTreeIndex;
#[cfg(feature = "fts")]
use super::fts::FtsCommitGuard;
#[cfg(not(feature = "fts"))]
use super::fts_disabled::FtsCommitGuard;
use super::registry::IndexRegistry;
use super::traits::{IndexBackend, IndexType, RangeOp, ScanResult};
use super::{FtsIndex, HnswIndex, extract_vector};

/// Convert schema IndexType to traits IndexType
fn convert_index_type(schema_type: SchemaIndexType) -> IndexType {
    match schema_type {
        SchemaIndexType::BTree => IndexType::BTree,
        SchemaIndexType::Hnsw => IndexType::Hnsw,
        SchemaIndexType::FullText => IndexType::FullText,
    }
}

/// Coordinates index backends and provides unified index operations
pub struct IndexManager {
    keyspaces: KeyspaceManager,
    registry: IndexRegistry,
    backends: HashMap<IndexType, Arc<dyn IndexBackend>>,
    /// BTree backend stored separately for transactional updates
    btree_backend: Arc<BTreeIndex>,
    /// FTS backend stored separately for direct search access
    fts_backend: Arc<FtsIndex>,
    /// HNSW backend stored separately for direct search access
    hnsw_backend: Arc<HnswIndex>,
    /// Lock to protect index operations from concurrent DDL (DROP INDEX, REINDEX).
    /// Query execution takes read lock at start, DDL takes write lock.
    /// This ensures DDL waits for all active queries to complete.
    ddl_lock: RwLock<()>,
}

/// Guard that holds a read lock on index DDL operations.
///
/// Acquired at the start of query execution and held until the query completes.
/// This prevents DROP INDEX and REINDEX from running while queries are in-flight,
/// ensuring that a query that plans to use an index will still have access to it
/// during execution.
///
/// # Example
/// ```ignore
/// let _guard = index_manager.acquire_query_guard();
/// // Plan and execute query...
/// // Guard dropped automatically when query finishes
/// ```
pub struct QueryExecutionGuard<'a> {
    _guard: RwLockReadGuard<'a, ()>,
}

/// Prevents automatic FTS commits from splitting a durable external-index
/// batch into multiple visible commits.
pub(crate) struct ExternalIndexCommitGuard<'a> {
    fts: FtsCommitGuard<'a>,
}

impl IndexManager {
    fn build_name(build_id: &str) -> String {
        format!(
            "{}{build_id}",
            crate::storage::keyspaces::INDEX_BUILD_MARKER
        )
    }

    /// Create a new IndexManager with default backends
    ///
    /// The `data_path` is used for storing FTS index files (Tantivy indexes).
    pub fn new(keyspaces: KeyspaceManager, data_path: PathBuf) -> Self {
        let mut backends: HashMap<IndexType, Arc<dyn IndexBackend>> = HashMap::new();

        // Create BTree backend and store separately for transactional updates
        let btree_backend = Arc::new(BTreeIndex::new());
        backends.insert(IndexType::BTree, btree_backend.clone());

        // Create FTS backend with the data path for Tantivy index storage
        let fts_backend = Arc::new(FtsIndex::new(data_path.clone()));

        // Register FTS backend
        backends.insert(IndexType::FullText, fts_backend.clone());

        // Create HNSW backend with the data path for vector index storage
        let hnsw_backend = Arc::new(HnswIndex::new(data_path));

        // Register HNSW backend
        backends.insert(IndexType::Hnsw, hnsw_backend.clone());

        Self {
            keyspaces,
            registry: IndexRegistry::new(),
            backends,
            btree_backend,
            fts_backend,
            hnsw_backend,
            ddl_lock: RwLock::new(()),
        }
    }

    /// Acquire a query execution guard.
    ///
    /// This should be called at the start of query execution (before planning)
    /// and held until the query completes. The guard prevents DDL operations
    /// (DROP INDEX, REINDEX) from modifying indexes while the query is running.
    ///
    /// # Returns
    /// A guard that holds a read lock. DDL operations acquire a write lock,
    /// so they will block until all query guards are dropped.
    pub fn acquire_query_guard(&self) -> QueryExecutionGuard<'_> {
        QueryExecutionGuard {
            _guard: self.ddl_lock.read(),
        }
    }

    /// Acquire a DDL lock for exclusive index operations.
    ///
    /// Used by DROP INDEX and REINDEX to ensure no queries are using indexes
    /// while they are being modified.
    pub(crate) fn acquire_ddl_lock(&self) -> parking_lot::RwLockWriteGuard<'_, ()> {
        self.ddl_lock.write()
    }

    /// Get the FTS backend for direct search access
    ///
    /// This provides access to FTS-specific methods like `search()` and `search_with_boosts()`
    /// that are not part of the generic IndexBackend trait.
    pub fn fts_backend(&self) -> &Arc<FtsIndex> {
        &self.fts_backend
    }

    /// Get the HNSW backend for direct search access
    ///
    /// This provides access to HNSW-specific methods like `search()`
    /// that are not part of the generic IndexBackend trait.
    pub fn hnsw_backend(&self) -> &Arc<HnswIndex> {
        &self.hnsw_backend
    }

    pub(crate) fn btree_backend(&self) -> &Arc<BTreeIndex> {
        &self.btree_backend
    }

    /// Get the index registry
    pub fn registry(&self) -> &IndexRegistry {
        &self.registry
    }

    /// Get all indexes for a collection
    pub fn get_indexes(&self, collection: &str) -> Vec<IndexDef> {
        self.registry.get_indexes(collection)
    }

    /// Check if an index exists
    pub fn has_index(&self, collection: &str, fields: &[String]) -> bool {
        self.registry.has_index(collection, fields)
    }

    /// Find index for a field
    pub fn find_index_for_field(&self, collection: &str, field: &str) -> Option<(String, bool)> {
        self.registry
            .find_index_for_field(collection, field)
            .map(|idx| (idx.fields[0].clone(), idx.unique))
    }

    /// Find compound index for fields
    pub fn find_compound_index(&self, collection: &str, fields: &[&str]) -> Option<Vec<String>> {
        self.registry
            .find_compound_index(collection, fields)
            .map(|idx| idx.fields)
    }

    /// Find FTS index that covers a specific field
    ///
    /// Returns the FTS index name if found.
    pub fn find_fts_index_for_field(&self, collection: &str, field: &str) -> Option<String> {
        self.registry
            .find_fts_index_for_field(collection, field)
            .map(|idx| idx.name.clone())
    }

    /// Find any FTS index for a collection
    ///
    /// Returns the FTS index name if found.
    pub fn find_fts_index(&self, collection: &str) -> Option<String> {
        self.registry
            .find_fts_index(collection)
            .map(|idx| idx.name.clone())
    }

    /// Find HNSW index that covers a specific field
    ///
    /// Returns the HNSW index name if found.
    pub fn find_hnsw_index_for_field(&self, collection: &str, field: &str) -> Option<String> {
        self.registry
            .find_hnsw_index_for_field(collection, field)
            .map(|idx| {
                // Generate the index name from the field
                idx.fields[0].clone()
            })
    }

    /// Load indexes from metadata at startup
    pub fn load_indexes(&self, collection: &str, indexes: Vec<IndexDef>) {
        // Create keyspaces for BTree indexes
        for index in &indexes {
            if index.index_type == SchemaIndexType::BTree {
                let _ = self
                    .keyspaces
                    .get_or_create_index_keyspace(collection, &index.name);
            }
        }
        self.registry.load(collection, indexes);
    }

    /// Allocate backend-specific storage for an unpublished build.
    pub(crate) fn initialize_build(
        &self,
        collection: &str,
        index: &IndexDef,
        build_id: &str,
    ) -> Result<(), StorageError> {
        self.keyspaces
            .get_or_create_index_build_keyspace(collection, build_id)?;
        let build_name = Self::build_name(build_id);
        match index.index_type {
            SchemaIndexType::BTree => Ok(()),
            SchemaIndexType::FullText => self.fts_backend.init_build(
                collection,
                &build_name,
                &index.fields,
                index.analyzer.as_deref(),
            ),
            SchemaIndexType::Hnsw => {
                let params = index.hnsw_params.as_ref().ok_or_else(|| {
                    StorageError::InvalidConfig("HNSW build is missing parameters".into())
                })?;
                self.hnsw_backend
                    .init_build(collection, &build_name, params)
            }
        }
    }

    /// Backfill only the isolated build artifact. The registry is deliberately
    /// untouched, so the planner cannot observe this data.
    pub(crate) fn build_artifact_with_progress(
        &self,
        collection: &str,
        index: &IndexDef,
        build_id: &str,
        docs: impl Iterator<Item = Result<Document, StorageError>>,
        mut progress: impl FnMut(crate::storage::IndexBuildEvent) -> bool,
    ) -> Result<bool, StorageError> {
        let build_name = Self::build_name(build_id);
        let keyspace = self
            .keyspaces
            .get_or_create_index_build_keyspace(collection, build_id)?;
        let mut processed_items = 0usize;
        let mut last_report = std::time::Instant::now();

        for doc_result in docs {
            let doc = doc_result?;
            match index.index_type {
                SchemaIndexType::BTree => {
                    self.btree_backend
                        .index_document(&keyspace, &index.fields, &doc)?;
                }
                SchemaIndexType::FullText => self.fts_backend.index_build_document(
                    collection,
                    &build_name,
                    &index.fields,
                    &doc,
                )?,
                SchemaIndexType::Hnsw => {
                    let field = index.fields.first().ok_or_else(|| {
                        StorageError::InvalidConfig("HNSW index requires one field".into())
                    })?;
                    self.hnsw_backend
                        .index_build_document(collection, &build_name, field, &doc)?;
                }
            }
            processed_items += 1;
            if processed_items.is_multiple_of(256)
                || last_report.elapsed() >= std::time::Duration::from_millis(250)
            {
                if !progress(crate::storage::IndexBuildEvent::Building { processed_items }) {
                    return Ok(false);
                }
                last_report = std::time::Instant::now();
            }
        }

        if !progress(crate::storage::IndexBuildEvent::Building { processed_items })
            || !progress(crate::storage::IndexBuildEvent::Committing)
        {
            return Ok(false);
        }
        match index.index_type {
            SchemaIndexType::BTree => {}
            SchemaIndexType::FullText => self.fts_backend.commit_all()?,
            SchemaIndexType::Hnsw => self.hnsw_backend.commit()?,
        }
        Ok(true)
    }

    /// Promote a durably flushed artifact to the physical name referenced by
    /// the schema. Schema/registry publication is a separate, final step.
    pub(crate) fn promote_build(
        &self,
        collection: &str,
        index: &IndexDef,
        build_id: &str,
    ) -> Result<(), StorageError> {
        let build_name = Self::build_name(build_id);
        match index.index_type {
            SchemaIndexType::BTree => {
                self.keyspaces
                    .promote_index_build_keyspace(collection, build_id, &index.name)
            }
            SchemaIndexType::FullText => self.fts_backend.promote_build(
                collection,
                &build_name,
                &index.name,
                index.analyzer.as_deref(),
            ),
            SchemaIndexType::Hnsw => {
                let field = index.fields.first().ok_or_else(|| {
                    StorageError::InvalidConfig("HNSW index requires one field".into())
                })?;
                let params = index.hnsw_params.as_ref().ok_or_else(|| {
                    StorageError::InvalidConfig("HNSW build is missing parameters".into())
                })?;
                self.hnsw_backend
                    .promote_build(collection, &build_name, field, params)
            }
        }
    }

    pub(crate) fn cleanup_build(
        &self,
        collection: &str,
        index: &IndexDef,
        build_id: &str,
    ) -> Result<(), StorageError> {
        let build_name = Self::build_name(build_id);
        match index.index_type {
            SchemaIndexType::BTree => {}
            SchemaIndexType::FullText => {
                self.fts_backend.remove_build(collection, &build_name)?;
            }
            SchemaIndexType::Hnsw => {
                self.hnsw_backend.remove_build(collection, &build_name)?;
            }
        }
        self.keyspaces
            .drop_index_build_keyspace(collection, build_id)
    }

    /// Startup reconciliation removes all temporary resources. Unfinished
    /// durable jobs are subsequently restarted from their definitions.
    pub(crate) fn cleanup_orphaned_builds(&self) -> Result<(), StorageError> {
        self.fts_backend.cleanup_orphaned_builds()?;
        self.hnsw_backend.cleanup_orphaned_builds()?;
        for name in self.keyspaces.list_index_build_keyspace_names() {
            self.keyspaces.clear_index_build_keyspace_by_name(&name)?;
        }
        Ok(())
    }

    /// Remove all indexes for a collection
    pub fn remove_collection(&self, collection: &str) {
        self.registry.remove_collection(collection);
    }

    /// Update indexes when a document changes (all index types)
    ///
    /// Note: For transactional consistency, prefer using `update_btree_in_tx()` for BTree
    /// indexes within a transaction, and `update_fts_hnsw()` for FTS/HNSW indexes after commit.
    pub fn update(
        &self,
        collection: &str,
        old_doc: Option<&Document>,
        new_doc: Option<&Document>,
    ) -> Result<(), StorageError> {
        let indexes = self.registry.get_indexes(collection);
        if indexes.is_empty() {
            return Ok(());
        }

        for index in &indexes {
            // Get or create keyspace for this specific index
            let keyspace = self
                .keyspaces
                .get_or_create_index_keyspace(collection, &index.name)?;

            // Use the appropriate backend based on index type
            let backend_type = convert_index_type(index.index_type);
            let backend = self
                .backends
                .get(&backend_type)
                .ok_or_else(|| StorageError::BackendNotFound(format!("{:?}", backend_type)))?;

            backend.update_document(&keyspace, &index.fields, old_doc, new_doc)?;
        }

        Ok(())
    }

    /// Update BTree indexes within a transaction.
    ///
    /// This method updates BTree indexes as part of the same transaction that modifies
    /// the document, ensuring atomicity. Use this instead of `update()` for BTree indexes
    /// when you need transactional consistency.
    pub fn update_btree_in_tx(
        &self,
        tx: &mut fjall::OptimisticWriteTx,
        collection: &str,
        old_doc: Option<&Document>,
        new_doc: Option<&Document>,
    ) -> Result<(), StorageError> {
        let indexes = self.registry.get_indexes(collection);

        for index in &indexes {
            if index.index_type == SchemaIndexType::BTree {
                // Get or create keyspace for this index
                let keyspace = self
                    .keyspaces
                    .get_or_create_index_keyspace(collection, &index.name)?;

                self.btree_backend
                    .update_in_tx(tx, &keyspace, &index.fields, old_doc, new_doc)?;
            }
        }
        Ok(())
    }

    /// Pre-validate HNSW vectors before transaction commit.
    ///
    /// This method checks that all vectors in the document have correct dimensions
    /// for their corresponding HNSW indexes. Call this BEFORE committing the transaction
    /// to ensure atomicity: if validation fails, the document won't be created.
    ///
    /// Returns Ok(()) if all vectors are valid or there are no HNSW indexes.
    pub fn validate_hnsw_vectors(
        &self,
        collection: &str,
        doc: &Document,
    ) -> Result<(), StorageError> {
        let indexes = self.registry.get_indexes(collection);

        for index in &indexes {
            if index.index_type != SchemaIndexType::Hnsw {
                continue;
            }

            let field = match index.fields.first() {
                Some(f) => f,
                None => continue,
            };

            // Extract vector from document
            if let Some(vector) = extract_vector(doc, field)
                && !vector.is_empty()
            {
                // Validate dimension against the HNSW index
                self.hnsw_backend
                    .validate_vector(collection, field, &vector)?;
            }
        }

        Ok(())
    }

    /// Update FTS and HNSW indexes after a document change.
    ///
    /// This method updates only FTS (full-text search) and HNSW (vector) indexes.
    /// These indexes don't support transactional updates, so they should be updated
    /// after the main transaction commits. Use `update_btree_in_tx()` for BTree indexes.
    pub fn update_fts_hnsw(
        &self,
        collection: &str,
        old_doc: Option<&Document>,
        new_doc: Option<&Document>,
    ) -> Result<(), StorageError> {
        let indexes = self.registry.get_indexes(collection);
        if indexes.is_empty() {
            return Ok(());
        }

        for index in &indexes {
            // Skip BTree indexes - they should be updated via update_btree_in_tx()
            if index.index_type == SchemaIndexType::BTree {
                continue;
            }

            // Get or create keyspace for this specific index
            let keyspace = self
                .keyspaces
                .get_or_create_index_keyspace(collection, &index.name)?;

            // Use the appropriate backend based on index type
            let backend_type = convert_index_type(index.index_type);
            let backend = self
                .backends
                .get(&backend_type)
                .ok_or_else(|| StorageError::BackendNotFound(format!("{:?}", backend_type)))?;

            backend.update_document(&keyspace, &index.fields, old_doc, new_doc)?;
        }

        Ok(())
    }

    /// Build an index from existing documents
    pub fn build_index(
        &self,
        collection: &str,
        index: &IndexDef,
        docs: impl Iterator<Item = Result<Document, StorageError>>,
    ) -> Result<(), StorageError> {
        let completed = self.build_index_with_progress(collection, index, docs, |_| true)?;
        debug_assert!(completed, "unconditional index build cannot be cancelled");
        Ok(())
    }

    /// Build an index while reporting batched progress. Progress is emitted at
    /// most once per 256 documents (plus a final update), avoiding a durable
    /// job-store write for every indexed document.
    pub fn build_index_with_progress(
        &self,
        collection: &str,
        index: &IndexDef,
        docs: impl Iterator<Item = Result<Document, StorageError>>,
        mut progress: impl FnMut(crate::storage::IndexBuildEvent) -> bool,
    ) -> Result<bool, StorageError> {
        // Get or create keyspace for this specific index
        let keyspace = self
            .keyspaces
            .get_or_create_index_keyspace(collection, &index.name)?;

        // Use the appropriate backend based on index type
        let backend_type = convert_index_type(index.index_type);
        let backend = self
            .backends
            .get(&backend_type)
            .ok_or_else(|| StorageError::BackendNotFound(format!("{:?}", backend_type)))?;

        let mut processed_items = 0usize;
        let mut last_report = std::time::Instant::now();
        for doc_result in docs {
            let doc = doc_result?;
            backend.index_document(&keyspace, &index.fields, &doc)?;
            processed_items += 1;
            if processed_items.is_multiple_of(256)
                || last_report.elapsed() >= std::time::Duration::from_millis(250)
            {
                if !progress(crate::storage::IndexBuildEvent::Building { processed_items }) {
                    return Ok(false);
                }
                last_report = std::time::Instant::now();
            }
        }

        if !progress(crate::storage::IndexBuildEvent::Building { processed_items })
            || !progress(crate::storage::IndexBuildEvent::Committing)
        {
            return Ok(false);
        }

        // Commit any pending changes (important for FTS which batches commits)
        backend.commit()?;

        Ok(true)
    }

    /// Drop a BTree index by deleting its keyspace.
    ///
    /// IMPORTANT: Caller must hold DDL lock (via acquire_ddl_lock) before calling.
    /// This ensures no concurrent queries are using the index while it's being dropped.
    ///
    /// Note: For HNSW and FTS indexes, use their respective backend methods directly
    /// (hnsw_backend().remove_index(), fts_backend().remove_index()).
    pub fn drop_btree_index(&self, collection: &str, index_name: &str) -> Result<(), StorageError> {
        // Open a durable keyspace that may not be in the map (for example an
        // unpublished artifact found by startup reconciliation).
        self.keyspaces
            .get_or_create_index_keyspace(collection, index_name)?;
        self.keyspaces.drop_index_keyspace(collection, index_name)
    }

    /// Scan index for equality
    ///
    /// Caller must hold a QueryExecutionGuard to prevent DDL operations
    /// from modifying the index during the scan.
    pub fn scan_eq(
        &self,
        collection: &str,
        index_name: &str,
        field: &str,
        value: &Value,
        limit: usize,
    ) -> Result<ScanResult, StorageError> {
        let keyspace = self
            .keyspaces
            .get_index_keyspace(collection, index_name)
            .ok_or_else(|| StorageError::IndexLookupFailed(format!("index '{}'", index_name)))?;

        let backend = self
            .backends
            .get(&IndexType::BTree)
            .ok_or_else(|| StorageError::BackendNotFound("BTree".into()))?;

        backend.scan_eq(&keyspace, field, value, limit)
    }

    /// Scan index for range
    ///
    /// Caller must hold a QueryExecutionGuard to prevent DDL operations
    /// from modifying the index during the scan.
    pub fn scan_range(
        &self,
        collection: &str,
        index_name: &str,
        field: &str,
        op: &str,
        value: &Value,
        limit: usize,
    ) -> Result<ScanResult, StorageError> {
        let keyspace = self
            .keyspaces
            .get_index_keyspace(collection, index_name)
            .ok_or_else(|| StorageError::IndexLookupFailed(format!("index '{}'", index_name)))?;

        let backend = self
            .backends
            .get(&IndexType::BTree)
            .ok_or_else(|| StorageError::BackendNotFound("BTree".into()))?;

        let range_op = RangeOp::from_str(op).ok_or_else(|| {
            StorageError::IndexLookupFailed(format!("unknown range operator: {}", op))
        })?;

        backend.scan_range(&keyspace, field, &range_op, value, limit)
    }

    /// Scan index for range between two values
    ///
    /// Caller must hold a QueryExecutionGuard to prevent DDL operations
    /// from modifying the index during the scan.
    #[allow(clippy::too_many_arguments)]
    pub fn scan_range_between(
        &self,
        collection: &str,
        index_name: &str,
        field: &str,
        lower: &Value,
        lower_inclusive: bool,
        upper: &Value,
        upper_inclusive: bool,
        limit: usize,
    ) -> Result<ScanResult, StorageError> {
        let keyspace = self
            .keyspaces
            .get_index_keyspace(collection, index_name)
            .ok_or_else(|| StorageError::IndexLookupFailed(format!("index '{}'", index_name)))?;

        let backend = self
            .backends
            .get(&IndexType::BTree)
            .ok_or_else(|| StorageError::BackendNotFound("BTree".into()))?;

        backend.scan_range_between(
            &keyspace,
            field,
            lower,
            lower_inclusive,
            upper,
            upper_inclusive,
            limit,
        )
    }

    /// Scan compound index for equality
    ///
    /// Caller must hold a QueryExecutionGuard to prevent DDL operations
    /// from modifying the index during the scan.
    pub fn scan_compound_eq(
        &self,
        collection: &str,
        index_name: &str,
        field_values: &[(&str, &Value)],
        limit: usize,
    ) -> Result<ScanResult, StorageError> {
        let keyspace = self
            .keyspaces
            .get_index_keyspace(collection, index_name)
            .ok_or_else(|| StorageError::IndexLookupFailed(format!("index '{}'", index_name)))?;

        let backend = self
            .backends
            .get(&IndexType::BTree)
            .ok_or_else(|| StorageError::BackendNotFound("BTree".into()))?;

        backend.scan_compound_eq(&keyspace, field_values, limit)
    }

    /// Commit all indexes to disk
    ///
    /// This should be called before shutdown to ensure all pending
    /// index changes (including HNSW id_map metadata) are persisted.
    pub fn commit_all(&self) -> Result<(), StorageError> {
        // Commit HNSW indexes (saves id_map metadata)
        self.hnsw_backend.commit()?;

        // Commit FTS indexes
        self.fts_backend.commit_all()?;

        Ok(())
    }

    /// Hold off automatic FTS commits while a group of external-index jobs is
    /// applied. HNSW has no background committer and serializes save/update via
    /// its existing per-instance operation lock.
    pub(crate) fn acquire_external_commit_guard(&self) -> ExternalIndexCommitGuard<'_> {
        ExternalIndexCommitGuard {
            fts: self.fts_backend.acquire_commit_guard(),
        }
    }

    /// Commit all external indexes while the batch commit guard is held.
    pub(crate) fn commit_all_guarded(
        &self,
        guard: &ExternalIndexCommitGuard<'_>,
    ) -> Result<(), StorageError> {
        self.hnsw_backend.commit()?;
        self.fts_backend.commit_all_guarded(&guard.fts)
    }
}
