//! Database - thin facade coordinating storage components

use std::path::Path;
use std::sync::Arc;

use fjall::{KeyspaceCreateOptions, OptimisticTxDatabase, PersistMode, Readable};
use parking_lot::RwLock;
use rkyv::rancor;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Value};
use crate::edge::Edge;
use crate::error::StorageError;
use crate::schema::{
    AnalyzerDef, CollectionSchema, HnswParams, IndexDef, IndexType, SchemaRegistry, validate_name,
};

use super::IndexBuildConfig;
use super::WriteOp;
use super::constraints::UniqueConstraintManager;
use super::documents::DocumentStore;
use super::edges::EdgeStore;
use super::indexes::IndexManager;
use super::keyspaces::KeyspaceManager;
use super::schema::SchemaManager;

#[path = "database/batch.rs"]
mod batch;

/// Storage statistics
#[derive(Debug, Clone, Default)]
pub struct StorageStats {
    /// Number of collections
    pub collections: usize,
    /// Number of edge labels (edge types)
    pub edge_labels: usize,
    /// Number of B-tree indexes
    pub btree_indexes: usize,
    /// Number of vector (HNSW) indexes
    pub vector_indexes: usize,
    /// Number of full-text search indexes
    pub fts_indexes: usize,
    /// Total disk size in bytes
    pub disk_size: u64,
}

const INDEX_JOBS_KEYSPACE: &str = "_index_jobs";
const STORAGE_FORMAT_VERSION_KEY: &[u8] = b"storage_format_version";
const CURRENT_STORAGE_FORMAT_VERSION: u64 = 1;
const FJALL_KEYSPACES_DIR: &str = "keyspaces";
const FJALL_LOCK_FILE: &str = "lock";
const FJALL_VERSION_FILE: &str = "version";
const LEGACY_FTS_DIR: &str = "fts";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExternalIndexJob {
    collection: String,
    key: String,
    old_doc: Option<Document>,
    new_doc: Option<Document>,
}

impl StorageStats {
    /// Total number of indexes
    pub fn total_indexes(&self) -> usize {
        self.btree_indexes + self.vector_indexes + self.fts_indexes
    }

    /// Format disk size as human-readable string
    pub fn disk_size_human(&self) -> String {
        let size = self.disk_size;
        if size < 1024 {
            format!("{} B", size)
        } else if size < 1024 * 1024 {
            format!("{:.1} KB", size as f64 / 1024.0)
        } else if size < 1024 * 1024 * 1024 {
            format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
        } else {
            format!("{:.1} GB", size as f64 / (1024.0 * 1024.0 * 1024.0))
        }
    }

    /// Format indexes summary for display (compact: "18 idx (15b/1v/2f)")
    pub fn indexes_summary(&self) -> String {
        let total = self.total_indexes();
        if total == 0 {
            return String::new();
        }

        let mut parts = Vec::new();
        if self.btree_indexes > 0 {
            parts.push(format!("{}b", self.btree_indexes));
        }
        if self.vector_indexes > 0 {
            parts.push(format!("{}v", self.vector_indexes));
        }
        if self.fts_indexes > 0 {
            parts.push(format!("{}f", self.fts_indexes));
        }

        format!("{} idx ({})", total, parts.join("/"))
    }
}

/// Main storage coordinator - delegates to specialized components
pub struct Database {
    db: Arc<OptimisticTxDatabase>,

    // Components
    keyspaces: KeyspaceManager,
    index_jobs: fjall::OptimisticTxKeyspace,
    documents: DocumentStore,
    edges: EdgeStore,
    indexes: IndexManager,
    constraints: UniqueConstraintManager,
    schemas: SchemaManager,
    index_build_config: RwLock<IndexBuildConfig>,
    #[cfg(test)]
    fail_next_external_index_replay_commit: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    fail_next_external_index_apply_commit: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    external_index_apply_commit_count: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    external_index_job_cleanup_commit_count: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    fail_next_external_index_job_cleanup_commit: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    fail_next_set_document_commit_error: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    fail_next_update_commit_error: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    fail_next_create_document_commit_error: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    fail_next_delete_document_commit_error: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    fail_next_create_edge_commit_error: std::sync::atomic::AtomicBool,
}

impl Database {
    /// Open or create a database at the given path
    ///
    /// Directory structure:
    /// - `path/storage/` - Fjall database (keyspaces, journals, etc.)
    /// - `path/indexes/fts/` - Full-text search indexes (Tantivy)
    /// - `path/indexes/vector/` - Vector indexes (HNSW)
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        Self::upgrade_legacy_root_storage_layout(path)?;

        // Fjall stores data in storage/ subdirectory
        let storage_path = path.join(super::DIR_STORAGE);
        let db = Arc::new(OptimisticTxDatabase::builder(&storage_path).open()?);

        // Create shared keyspaces
        let meta = db.keyspace("_meta", KeyspaceCreateOptions::default)?;
        Self::ensure_storage_format_version(&meta)?;
        let index_jobs = db.keyspace(INDEX_JOBS_KEYSPACE, KeyspaceCreateOptions::default)?;

        // Create components
        let keyspaces = KeyspaceManager::new(db.clone());
        let documents = DocumentStore::new(keyspaces.clone());
        let edges = EdgeStore::new(db.clone())?;
        let indexes = IndexManager::new(keyspaces.clone(), path.to_path_buf());
        let constraints = UniqueConstraintManager::new();
        let schemas = SchemaManager::new(meta);

        let manager = Self {
            db,
            keyspaces,
            index_jobs,
            documents,
            edges,
            indexes,
            constraints,
            schemas,
            index_build_config: RwLock::new(IndexBuildConfig::default()),
            #[cfg(test)]
            fail_next_external_index_replay_commit: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            fail_next_external_index_apply_commit: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            external_index_apply_commit_count: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            external_index_job_cleanup_commit_count: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            fail_next_external_index_job_cleanup_commit: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            fail_next_set_document_commit_error: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            fail_next_update_commit_error: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            fail_next_create_document_commit_error: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            fail_next_delete_document_commit_error: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            fail_next_create_edge_commit_error: std::sync::atomic::AtomicBool::new(false),
        };

        // Load existing collections from meta
        manager.load_existing_collections()?;

        // Load custom analyzers
        let analyzers = manager.load_all_analyzers();
        manager.indexes.fts_backend().load_analyzers(analyzers);

        manager.replay_external_index_jobs()?;

        Ok(manager)
    }

    fn upgrade_legacy_root_storage_layout(path: &Path) -> Result<(), StorageError> {
        let storage_path = path.join(super::DIR_STORAGE);
        if !Self::looks_like_legacy_fjall_root(path) {
            return Ok(());
        }

        if Self::looks_like_legacy_fjall_root(&storage_path) {
            return Ok(());
        }

        if storage_path.exists() && !Self::is_empty_dir(&storage_path)? {
            return Err(StorageError::InvalidOperation(format!(
                "cannot upgrade legacy storage layout: {} exists and is not empty",
                storage_path.display()
            )));
        }

        std::fs::create_dir_all(&storage_path).map_err(|e| {
            StorageError::Io(format!(
                "failed to create storage directory during legacy layout upgrade: {}",
                e
            ))
        })?;

        Self::move_legacy_entry(path, &storage_path, FJALL_KEYSPACES_DIR)?;
        Self::move_legacy_entry(path, &storage_path, FJALL_VERSION_FILE)?;
        Self::move_legacy_entry(path, &storage_path, FJALL_LOCK_FILE)?;

        for entry in std::fs::read_dir(path).map_err(|e| {
            StorageError::Io(format!(
                "failed to read database directory during legacy layout upgrade: {}",
                e
            ))
        })? {
            let entry = entry.map_err(|e| {
                StorageError::Io(format!(
                    "failed to read database directory entry during legacy layout upgrade: {}",
                    e
                ))
            })?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };

            if name.ends_with(".jnl") {
                Self::move_legacy_entry(path, &storage_path, name)?;
            }
        }

        Self::upgrade_legacy_fts_layout(path)?;

        Ok(())
    }

    fn looks_like_legacy_fjall_root(path: &Path) -> bool {
        path.join(FJALL_VERSION_FILE).is_file() && path.join(FJALL_KEYSPACES_DIR).is_dir()
    }

    fn is_empty_dir(path: &Path) -> Result<bool, StorageError> {
        let mut entries = std::fs::read_dir(path).map_err(|e| {
            StorageError::Io(format!(
                "failed to read directory {}: {}",
                path.display(),
                e
            ))
        })?;
        Ok(entries.next().is_none())
    }

    fn move_legacy_entry(from_dir: &Path, to_dir: &Path, name: &str) -> Result<(), StorageError> {
        let from = from_dir.join(name);
        if !from.exists() {
            return Ok(());
        }

        let to = to_dir.join(name);
        if to.exists() {
            return Err(StorageError::InvalidOperation(format!(
                "cannot upgrade legacy storage layout: destination already exists: {}",
                to.display()
            )));
        }

        std::fs::rename(&from, &to).map_err(|e| {
            StorageError::Io(format!(
                "failed to move legacy storage entry {} to {}: {}",
                from.display(),
                to.display(),
                e
            ))
        })
    }

    fn upgrade_legacy_fts_layout(path: &Path) -> Result<(), StorageError> {
        let from = path.join(LEGACY_FTS_DIR);
        if !from.exists() {
            return Ok(());
        }

        let to = path.join(super::DIR_INDEXES).join(super::DIR_FTS);
        if to.exists() {
            return Err(StorageError::InvalidOperation(format!(
                "cannot upgrade legacy FTS layout: destination already exists: {}",
                to.display()
            )));
        }

        let parent = to.parent().ok_or_else(|| {
            StorageError::InvalidOperation(format!(
                "invalid legacy FTS destination path: {}",
                to.display()
            ))
        })?;
        std::fs::create_dir_all(parent).map_err(|e| {
            StorageError::Io(format!(
                "failed to create index directory during legacy FTS layout upgrade: {}",
                e
            ))
        })?;
        std::fs::rename(&from, &to).map_err(|e| {
            StorageError::Io(format!(
                "failed to move legacy FTS directory {} to {}: {}",
                from.display(),
                to.display(),
                e
            ))
        })
    }

    fn ensure_storage_format_version(
        meta: &fjall::OptimisticTxKeyspace,
    ) -> Result<(), StorageError> {
        let Some(stored) = meta.get(STORAGE_FORMAT_VERSION_KEY)? else {
            meta.insert(
                STORAGE_FORMAT_VERSION_KEY,
                CURRENT_STORAGE_FORMAT_VERSION.to_string().as_bytes(),
            )?;
            return Ok(());
        };

        let stored = std::str::from_utf8(stored.as_ref()).map_err(|e| {
            StorageError::InvalidConfig(format!("invalid storage format version marker: {}", e))
        })?;
        let version = stored.parse::<u64>().map_err(|e| {
            StorageError::InvalidConfig(format!("invalid storage format version marker: {}", e))
        })?;

        if version > CURRENT_STORAGE_FORMAT_VERSION {
            return Err(StorageError::InvalidConfig(format!(
                "unsupported storage format version {}; this binary supports up to {}",
                version, CURRENT_STORAGE_FORMAT_VERSION
            )));
        }

        Ok(())
    }

    fn has_external_indexes(&self, collection: &str) -> bool {
        self.indexes
            .get_indexes(collection)
            .iter()
            .any(|index| matches!(index.index_type, IndexType::FullText | IndexType::Hnsw))
    }

    fn external_index_job_key(collection: &str, key: &str) -> String {
        format!("{}:{}", collection, key)
    }

    fn write_external_index_job(
        &self,
        tx: &mut fjall::OptimisticWriteTx,
        collection: &str,
        key: &str,
        old_doc: Option<&Document>,
        new_doc: Option<&Document>,
    ) -> Result<(), StorageError> {
        if !self.has_external_indexes(collection) {
            return Ok(());
        }

        let job = ExternalIndexJob {
            collection: collection.to_string(),
            key: key.to_string(),
            old_doc: old_doc.cloned(),
            new_doc: new_doc.cloned(),
        };
        let bytes =
            serde_json::to_vec(&job).map_err(|e| StorageError::Serialization(e.to_string()))?;
        tx.insert(
            &self.index_jobs,
            Self::external_index_job_key(collection, key).as_bytes(),
            &bytes,
        );
        Ok(())
    }

    fn apply_external_index_job(&self, job: &ExternalIndexJob) -> Result<(), StorageError> {
        self.indexes
            .update_fts_hnsw(&job.collection, job.old_doc.as_ref(), job.new_doc.as_ref())
    }

    fn complete_external_index_jobs(&self, jobs: &[ExternalIndexJob]) -> Result<(), StorageError> {
        let keys = jobs
            .iter()
            .map(|job| Self::external_index_job_key(&job.collection, &job.key).into_bytes())
            .collect();
        self.complete_external_index_job_keys(keys)
    }

    fn complete_external_index_job_keys(&self, keys: Vec<Vec<u8>>) -> Result<(), StorageError> {
        if keys.is_empty() {
            return Ok(());
        }

        let mut tx = self.db.write_tx()?;
        for key in keys {
            tx.remove(&self.index_jobs, key);
        }

        #[cfg(test)]
        if self
            .fail_next_external_index_job_cleanup_commit
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(StorageError::Index(
                "forced external index job cleanup commit failure".to_string(),
            ));
        }

        match tx.commit()? {
            Ok(()) => {
                #[cfg(test)]
                self.external_index_job_cleanup_commit_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
            Err(fjall::Conflict) => Err(StorageError::Conflict(
                "external index job cleanup conflict".to_string(),
            )),
        }
    }

    fn apply_and_complete_external_index_job(
        &self,
        collection: &str,
        key: &str,
        old_doc: Option<&Document>,
        new_doc: Option<&Document>,
    ) -> Result<(), StorageError> {
        if !self.has_external_indexes(collection) {
            return Ok(());
        }

        let job = ExternalIndexJob {
            collection: collection.to_string(),
            key: key.to_string(),
            old_doc: old_doc.cloned(),
            new_doc: new_doc.cloned(),
        };
        self.apply_and_complete_external_index_jobs(std::slice::from_ref(&job))
    }

    fn apply_and_complete_external_index_jobs(
        &self,
        jobs: &[ExternalIndexJob],
    ) -> Result<(), StorageError> {
        if jobs.is_empty() {
            return Ok(());
        }

        // Keep the 100 ms/threshold FTS committer from making part of this
        // durable batch visible before the rest has been applied.
        let commit_guard = self.indexes.acquire_external_commit_guard();
        #[cfg(feature = "fault-injection")]
        crate::fault_injection::check("external_before_commit")?;

        for job in jobs {
            self.apply_external_index_job(job)?;
        }

        #[cfg(test)]
        if self
            .fail_next_external_index_apply_commit
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(StorageError::Index(
                "forced external index apply commit failure".to_string(),
            ));
        }

        self.indexes.commit_all_guarded(&commit_guard)?;

        #[cfg(test)]
        self.external_index_apply_commit_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        #[cfg(feature = "fault-injection")]
        crate::fault_injection::check("external_after_commit")?;

        // Removing a job before the shared external commit succeeds would lose
        // the only crash-recovery record for that primary-storage write. Remove
        // the whole batch in one Fjall transaction after the commit instead.
        self.complete_external_index_jobs(jobs)?;

        Ok(())
    }

    fn try_apply_and_complete_external_index_job(
        &self,
        collection: &str,
        key: &str,
        old_doc: Option<&Document>,
        new_doc: Option<&Document>,
    ) {
        if let Err(err) =
            self.apply_and_complete_external_index_job(collection, key, old_doc, new_doc)
        {
            tracing::warn!(
                collection,
                key,
                error = %err,
                "external index job remains pending after primary write commit"
            );
        }
    }

    fn try_apply_and_complete_external_index_jobs(&self, jobs: &[ExternalIndexJob]) {
        if let Err(err) = self.apply_and_complete_external_index_jobs(jobs) {
            tracing::warn!(
                job_count = jobs.len(),
                error = %err,
                "external index jobs remain pending after primary batch commit"
            );
        }
    }

    fn replay_external_index_jobs(&self) -> Result<(), StorageError> {
        let mut completed = Vec::new();

        for item in self.index_jobs.inner().iter() {
            let (key_bytes, value_bytes) = item.into_inner()?;
            let job: ExternalIndexJob = serde_json::from_slice(&value_bytes)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            self.apply_external_index_job(&job)?;
            completed.push(key_bytes.to_vec());
        }

        let had_completed = !completed.is_empty();
        if had_completed {
            self.commit_replayed_external_index_jobs()?;
        }

        self.complete_external_index_job_keys(completed)?;

        Ok(())
    }

    fn commit_replayed_external_index_jobs(&self) -> Result<(), StorageError> {
        #[cfg(test)]
        if self
            .fail_next_external_index_replay_commit
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(StorageError::Index(
                "forced external index commit failure".to_string(),
            ));
        }

        self.indexes.commit_all()?;

        #[cfg(test)]
        self.external_index_apply_commit_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        Ok(())
    }

    #[cfg(all(test, feature = "fts"))]
    fn fail_next_external_index_replay_commit_for_test(&self) {
        self.fail_next_external_index_replay_commit
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(all(test, feature = "fts"))]
    fn fail_next_external_index_apply_commit_for_test(&self) {
        self.fail_next_external_index_apply_commit
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    fn external_index_apply_commit_count_for_test(&self) -> usize {
        self.external_index_apply_commit_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    #[cfg(test)]
    fn external_index_job_cleanup_commit_count_for_test(&self) -> usize {
        self.external_index_job_cleanup_commit_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    #[cfg(test)]
    fn fail_next_external_index_job_cleanup_commit_for_test(&self) {
        self.fail_next_external_index_job_cleanup_commit
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    fn fail_next_set_document_commit_for_test(&self) {
        self.fail_next_set_document_commit_error
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    fn fail_next_update_commit_for_test(&self) {
        self.fail_next_update_commit_error
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    fn fail_next_create_document_commit_for_test(&self) {
        self.fail_next_create_document_commit_error
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    fn fail_next_delete_document_commit_for_test(&self) {
        self.fail_next_delete_document_commit_error
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    fn fail_next_create_edge_commit_for_test(&self) {
        self.fail_next_create_edge_commit_error
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(all(test, any(feature = "fts", feature = "hnsw")))]
    fn enqueue_external_index_job_for_test(
        &self,
        collection: &str,
        key: &str,
        old_doc: Option<&Document>,
        new_doc: Option<&Document>,
    ) -> Result<(), StorageError> {
        let job = ExternalIndexJob {
            collection: collection.to_string(),
            key: key.to_string(),
            old_doc: old_doc.cloned(),
            new_doc: new_doc.cloned(),
        };
        let bytes =
            serde_json::to_vec(&job).map_err(|e| StorageError::Serialization(e.to_string()))?;
        self.index_jobs.insert(
            Self::external_index_job_key(collection, key).as_bytes(),
            bytes,
        )?;
        Ok(())
    }

    /// Load existing collections from metadata
    fn load_existing_collections(&self) -> Result<(), StorageError> {
        // Load schemas and get collection names
        let collections = self.schemas.load_all()?;

        for (name, schema) in collections {
            // Open keyspaces for this collection
            self.keyspaces.open_existing(&name)?;

            // Load indexes into index manager
            if let Some(schema) = schema {
                // Initialize HNSW instances for vector indexes
                for index in &schema.indexes {
                    if index.index_type == IndexType::Hnsw
                        && let Some(params) = &index.hnsw_params
                    {
                        self.indexes.hnsw_backend().get_or_create_instance(
                            &name,
                            &index.fields[0],
                            params,
                        )?;
                    }

                    // Initialize FTS instances for full-text indexes
                    if index.index_type == IndexType::FullText {
                        // Generate index name from sorted fields
                        let mut sorted_fields = index.fields.clone();
                        sorted_fields.sort();
                        let index_name = format!("{}_fts", sorted_fields.join("_"));

                        self.indexes.fts_backend().open_existing(
                            &name,
                            &index_name,
                            index.analyzer.as_deref(),
                        )?;
                    }
                }
                self.indexes.load_indexes(&name, schema.indexes);
            }
        }

        Ok(())
    }

    // =========================================================================
    // Collection lifecycle
    // =========================================================================

    /// Get or create keyspaces for a collection
    pub fn get_or_create_collection(&self, name: &str) -> Result<(), StorageError> {
        if self.keyspaces.exists(name) {
            return Ok(());
        }

        // Validate collection name
        validate_name("collection", name)
            .map_err(|e| StorageError::InvalidOperation(e.to_string()))?;

        self.keyspaces.ensure_exists(name)?;
        self.schemas.save_collection_meta(name)?;

        Ok(())
    }

    /// Check if collection exists
    pub fn collection_exists(&self, name: &str) -> bool {
        self.keyspaces.exists(name)
    }

    /// List all collections
    pub fn list_collections(&self) -> Vec<String> {
        self.keyspaces.list()
    }

    // =========================================================================
    // Document operations
    // =========================================================================

    /// Get a document by collection and key
    pub fn get_document(
        &self,
        collection: &str,
        key: &str,
    ) -> Result<Option<Document>, StorageError> {
        self.documents.get(collection, key)
    }

    /// Store a document (creates collection if needed)
    pub fn set_document(&self, doc: &Document) -> Result<(), StorageError> {
        let collection = doc.collection();
        let key = doc.key();

        // Ensure collection exists
        self.get_or_create_collection(collection)?;

        // Get keyspaces
        let keyspaces = self.keyspaces.get(collection).ok_or_else(|| {
            StorageError::CollectionNotLoaded("collection missing after creation".into())
        })?;

        // Get old document for index cleanup
        let old_doc = self.documents.get(collection, key)?;
        let is_new = old_doc.is_none();

        // Get indexes for constraint checking
        let indexes = self.indexes.get_indexes(collection);

        // Validate and update unique constraints atomically (uses fetch_update)
        self.constraints.validate_and_update(
            &keyspaces.unique_locks,
            &indexes,
            old_doc.as_ref(),
            Some(doc),
        )?;

        // Start transaction
        let mut tx = match self.db.write_tx() {
            Ok(tx) => tx,
            Err(err) => {
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    old_doc.as_ref(),
                    Some(doc),
                );
                return Err(err.into());
            }
        };

        // Serialize and store document
        let bytes = match rkyv::to_bytes::<rancor::Error>(doc) {
            Ok(bytes) => bytes,
            Err(err) => {
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    old_doc.as_ref(),
                    Some(doc),
                );
                return Err(StorageError::Serialization(err.to_string()));
            }
        };
        tx.insert(&keyspaces.docs, key.as_bytes(), bytes.as_slice());

        // Add existence marker only for new documents
        // UPDATE does NOT touch exists keyspace (key insight for OCC)
        if is_new {
            tx.insert(&keyspaces.exists, key.as_bytes(), []);
        }

        // Update BTree indexes within transaction (before commit)
        if let Err(err) =
            self.indexes
                .update_btree_in_tx(&mut tx, collection, old_doc.as_ref(), Some(doc))
        {
            self.constraints.rollback_validate_and_update(
                &keyspaces.unique_locks,
                &indexes,
                old_doc.as_ref(),
                Some(doc),
            );
            return Err(err);
        }

        // Pre-validate HNSW vectors before commit (ensures atomicity)
        if let Err(err) = self.indexes.validate_hnsw_vectors(collection, doc) {
            self.constraints.rollback_validate_and_update(
                &keyspaces.unique_locks,
                &indexes,
                old_doc.as_ref(),
                Some(doc),
            );
            return Err(err);
        }

        if let Err(err) =
            self.write_external_index_job(&mut tx, collection, key, old_doc.as_ref(), Some(doc))
        {
            self.constraints.rollback_validate_and_update(
                &keyspaces.unique_locks,
                &indexes,
                old_doc.as_ref(),
                Some(doc),
            );
            return Err(err);
        }

        #[cfg(test)]
        if self
            .fail_next_set_document_commit_error
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            self.constraints.rollback_validate_and_update(
                &keyspaces.unique_locks,
                &indexes,
                old_doc.as_ref(),
                Some(doc),
            );
            return Err(StorageError::Index(
                "forced set document commit failure".to_string(),
            ));
        }

        // Commit transaction
        match tx.commit() {
            Err(err) => {
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    old_doc.as_ref(),
                    Some(doc),
                );
                Err(err.into())
            }
            Ok(Ok(())) => {
                // Update FTS/HNSW indexes after successful commit
                self.try_apply_and_complete_external_index_job(
                    collection,
                    key,
                    old_doc.as_ref(),
                    Some(doc),
                );
                #[cfg(feature = "fault-injection")]
                if let Err(error) = crate::fault_injection::check("primary_after_commit") {
                    tracing::warn!(%error, "ignored post-commit injected fault");
                }
                Ok(())
            }
            Ok(Err(fjall::Conflict)) => {
                // Rollback constraint changes since the transaction didn't commit
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    old_doc.as_ref(),
                    Some(doc),
                );
                Err(StorageError::Conflict("set_document write conflict".into()))
            }
        }
    }

    /// Atomically create a document (fails if already exists)
    pub fn create_document(&self, doc: &Document) -> Result<bool, StorageError> {
        let collection = doc.collection();
        let key = doc.key();

        // Ensure collection exists
        self.get_or_create_collection(collection)?;

        // Get keyspaces
        let keyspaces = self.keyspaces.get(collection).ok_or_else(|| {
            StorageError::CollectionNotLoaded("collection missing after creation".into())
        })?;

        // Get indexes for constraint checking
        let indexes = self.indexes.get_indexes(collection);

        // Validate and update unique constraints atomically (uses fetch_update)
        self.constraints
            .validate_and_update(&keyspaces.unique_locks, &indexes, None, Some(doc))?;

        // Use transaction for atomic check-and-insert (while holding constraint guards)
        let mut tx = match self.db.write_tx() {
            Ok(tx) => tx,
            Err(err) => {
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    None,
                    Some(doc),
                );
                return Err(err.into());
            }
        };

        // Check if document already exists (within transaction)
        match tx.get(&keyspaces.docs, key.as_bytes()) {
            Ok(Some(_)) => {
                // Rollback constraint claims since we're not creating
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    None,
                    Some(doc),
                );
                return Ok(false); // Already exists
            }
            Ok(None) => {}
            Err(err) => {
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    None,
                    Some(doc),
                );
                return Err(err.into());
            }
        }

        // Serialize and insert
        let bytes = match rkyv::to_bytes::<rancor::Error>(doc) {
            Ok(bytes) => bytes,
            Err(err) => {
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    None,
                    Some(doc),
                );
                return Err(StorageError::Serialization(err.to_string()));
            }
        };
        tx.insert(&keyspaces.docs, key.as_bytes(), bytes.as_slice());

        // Add existence marker (empty value)
        tx.insert(&keyspaces.exists, key.as_bytes(), []);

        // Update BTree indexes within transaction (before commit)
        if let Err(err) = self
            .indexes
            .update_btree_in_tx(&mut tx, collection, None, Some(doc))
        {
            self.constraints.rollback_validate_and_update(
                &keyspaces.unique_locks,
                &indexes,
                None,
                Some(doc),
            );
            return Err(err);
        }

        // Pre-validate HNSW vectors before commit (ensures atomicity)
        if let Err(err) = self.indexes.validate_hnsw_vectors(collection, doc) {
            self.constraints.rollback_validate_and_update(
                &keyspaces.unique_locks,
                &indexes,
                None,
                Some(doc),
            );
            return Err(err);
        }

        if let Err(err) = self.write_external_index_job(&mut tx, collection, key, None, Some(doc)) {
            self.constraints.rollback_validate_and_update(
                &keyspaces.unique_locks,
                &indexes,
                None,
                Some(doc),
            );
            return Err(err);
        }

        #[cfg(test)]
        if self
            .fail_next_create_document_commit_error
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            self.constraints.rollback_validate_and_update(
                &keyspaces.unique_locks,
                &indexes,
                None,
                Some(doc),
            );
            return Err(StorageError::Index(
                "forced create document commit failure".to_string(),
            ));
        }

        // Commit transaction
        match tx.commit() {
            Err(err) => {
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    None,
                    Some(doc),
                );
                Err(err.into())
            }
            Ok(Ok(())) => {
                // Update FTS/HNSW indexes after successful commit
                self.try_apply_and_complete_external_index_job(collection, key, None, Some(doc));
                Ok(true)
            }
            Ok(Err(fjall::Conflict)) => {
                // Rollback constraint claims since the transaction didn't commit
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    None,
                    Some(doc),
                );
                Ok(false) // Document was created by another thread
            }
        }
    }

    /// Atomically update a document (fails if not exists)
    pub fn update_document(&self, doc: &Document) -> Result<bool, StorageError> {
        let collection = doc.collection();
        let key = doc.key();

        let keyspaces = match self.keyspaces.get(collection) {
            Some(ks) => ks,
            None => return Ok(false),
        };

        // Fetch old document for index cleanup
        let old_doc = self.documents.get(collection, key)?;
        let Some(old_doc) = old_doc else {
            return Ok(false);
        };

        let indexes = self.indexes.get_indexes(collection);
        self.constraints.validate_and_update(
            &keyspaces.unique_locks,
            &indexes,
            Some(&old_doc),
            Some(doc),
        )?;

        // Start transaction
        let mut tx = match self.db.write_tx() {
            Ok(tx) => tx,
            Err(err) => {
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    Some(&old_doc),
                    Some(doc),
                );
                return Err(err.into());
            }
        };

        // Re-check if document exists within transaction (for atomicity)
        match tx.get(&keyspaces.docs, key.as_bytes()) {
            Ok(Some(_)) => {}
            Ok(None) => {
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    Some(&old_doc),
                    Some(doc),
                );
                return Ok(false);
            }
            Err(err) => {
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    Some(&old_doc),
                    Some(doc),
                );
                return Err(err.into());
            }
        }

        // Serialize and update
        let bytes = match rkyv::to_bytes::<rancor::Error>(doc) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    Some(&old_doc),
                    Some(doc),
                );
                return Err(StorageError::Serialization(e.to_string()));
            }
        };

        tx.insert(&keyspaces.docs, key.as_bytes(), bytes.as_slice());

        // Update BTree indexes within transaction (before commit)
        if let Err(err) =
            self.indexes
                .update_btree_in_tx(&mut tx, collection, Some(&old_doc), Some(doc))
        {
            self.constraints.rollback_validate_and_update(
                &keyspaces.unique_locks,
                &indexes,
                Some(&old_doc),
                Some(doc),
            );
            return Err(err);
        }

        // Pre-validate HNSW vectors before commit (ensures atomicity)
        if let Err(err) = self.indexes.validate_hnsw_vectors(collection, doc) {
            self.constraints.rollback_validate_and_update(
                &keyspaces.unique_locks,
                &indexes,
                Some(&old_doc),
                Some(doc),
            );
            return Err(err);
        }

        if let Err(err) =
            self.write_external_index_job(&mut tx, collection, key, Some(&old_doc), Some(doc))
        {
            self.constraints.rollback_validate_and_update(
                &keyspaces.unique_locks,
                &indexes,
                Some(&old_doc),
                Some(doc),
            );
            return Err(err);
        }

        #[cfg(test)]
        if self
            .fail_next_update_commit_error
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            self.constraints.rollback_validate_and_update(
                &keyspaces.unique_locks,
                &indexes,
                Some(&old_doc),
                Some(doc),
            );
            return Err(StorageError::Index(
                "forced update commit failure".to_string(),
            ));
        }

        // Commit
        match tx.commit() {
            Err(err) => {
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    Some(&old_doc),
                    Some(doc),
                );
                Err(err.into())
            }
            Ok(Ok(())) => {
                // Update FTS/HNSW indexes after successful commit
                self.try_apply_and_complete_external_index_job(
                    collection,
                    key,
                    Some(&old_doc),
                    Some(doc),
                );
                Ok(true)
            }
            Ok(Err(fjall::Conflict)) => {
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    Some(&old_doc),
                    Some(doc),
                );
                Ok(false)
            }
        }
    }

    /// Delete a document and clean up connected edges.
    pub fn delete_document(&self, collection: &str, key: &str) -> Result<bool, StorageError> {
        let keyspaces = match self.keyspaces.get(collection) {
            Some(ks) => ks,
            None => return Ok(false),
        };

        // Get document for cleanup
        let old_doc = self.documents.get(collection, key)?;

        if old_doc.is_none() {
            return Ok(false);
        }

        let old_doc = old_doc.expect("old_doc checked for None above");

        // Build full document ID (collection:key)
        let doc_id = format!("{}:{}", collection, key);

        // Check RESTRICT policy before starting transaction
        self.edges.check_delete_allowed(&doc_id)?;

        // Remove unique constraint entries
        let indexes = self.indexes.get_indexes(collection);
        self.constraints
            .remove_for_document(&keyspaces.unique_locks, &indexes, &old_doc)?;

        // Start transaction for atomic document + edge deletion
        let mut tx = match self.db.write_tx() {
            Ok(tx) => tx,
            Err(err) => {
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    Some(&old_doc),
                    None,
                );
                return Err(err.into());
            }
        };

        // Clean up edges within transaction (CASCADE policy)
        // This ensures atomicity with document deletion
        let _edges_deleted = match self.edges.delete_edges_for_node_in_tx(&mut tx, &doc_id) {
            Ok(edges_deleted) => edges_deleted,
            Err(err) => {
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    Some(&old_doc),
                    None,
                );
                return Err(err);
            }
        };

        // Delete document within transaction
        tx.remove(&keyspaces.docs, key.as_bytes());

        // Remove existence marker
        tx.remove(&keyspaces.exists, key.as_bytes());

        // Update BTree indexes within transaction
        if let Err(err) = self
            .indexes
            .update_btree_in_tx(&mut tx, collection, Some(&old_doc), None)
        {
            self.constraints.rollback_validate_and_update(
                &keyspaces.unique_locks,
                &indexes,
                Some(&old_doc),
                None,
            );
            return Err(err);
        }

        if let Err(err) =
            self.write_external_index_job(&mut tx, collection, key, Some(&old_doc), None)
        {
            self.constraints.rollback_validate_and_update(
                &keyspaces.unique_locks,
                &indexes,
                Some(&old_doc),
                None,
            );
            return Err(err);
        }

        #[cfg(test)]
        if self
            .fail_next_delete_document_commit_error
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            self.constraints.rollback_validate_and_update(
                &keyspaces.unique_locks,
                &indexes,
                Some(&old_doc),
                None,
            );
            return Err(StorageError::Index(
                "forced delete document commit failure".to_string(),
            ));
        }

        // Commit atomically
        match tx.commit() {
            Err(err) => {
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    Some(&old_doc),
                    None,
                );
                Err(err.into())
            }
            Ok(Ok(())) => {
                // Update FTS/HNSW indexes after successful commit
                self.try_apply_and_complete_external_index_job(
                    collection,
                    key,
                    Some(&old_doc),
                    None,
                );
                Ok(true)
            }
            Ok(Err(fjall::Conflict)) => {
                // Re-claim constraint entries since the delete didn't commit.
                // Use validate_and_update(None, Some(old_doc)) to re-claim.
                self.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    Some(&old_doc),
                    None,
                );
                Ok(false)
            }
        }
    }

    /// List documents with cursor-based pagination
    pub fn list_documents(
        &self,
        collection: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Document>, Option<String>), StorageError> {
        self.documents.list(collection, after, limit)
    }

    /// Count documents in a collection without loading document bodies.
    pub fn count_documents(&self, collection: &str) -> Result<usize, StorageError> {
        self.documents.count(collection)
    }

    /// Atomically write a batch of operations.
    ///
    /// Routes all operations through the same code paths as individual writes,
    /// including BTree index updates, unique constraint checks, existence markers,
    /// and FTS/HNSW index maintenance.
    pub fn atomic_write_batch(&self, ops: Vec<WriteOp>) -> Result<(), StorageError> {
        batch::atomic_write_batch(self, ops)
    }

    // =========================================================================
    // Edge operations
    // =========================================================================

    /// Create or update an edge
    pub fn create_edge(&self, edge: &Edge) -> Result<(), StorageError> {
        self.edges.create(edge)
    }

    /// Create an edge with validation that from/to documents exist.
    /// Uses existence registry to avoid conflicts with concurrent updates.
    /// DELETE and RELATE will conflict correctly (both touch exists keyspace).
    pub fn create_edge_validated(&self, edge: &Edge) -> Result<(), StorageError> {
        // Start transaction for atomic read + create
        let mut tx = self.db.write_tx()?;

        self.validate_edge_endpoints_in_tx(&mut tx, edge)?;

        // Create edge INSIDE transaction
        self.edges.create_in_tx(&mut tx, edge)?;

        #[cfg(test)]
        if self
            .fail_next_create_edge_commit_error
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(StorageError::Index(
                "forced create edge commit failure".to_string(),
            ));
        }

        // Commit transaction atomically
        match tx.commit()? {
            Ok(()) => Ok(()),
            Err(fjall::Conflict) => {
                // OCC conflict means concurrent DELETE removed the existence marker
                Err(StorageError::NotFound(
                    "Document was deleted during edge creation".to_string(),
                ))
            }
        }
    }

    fn validate_edge_endpoints_in_tx(
        &self,
        tx: &mut fjall::OptimisticWriteTx,
        edge: &Edge,
    ) -> Result<(), StorageError> {
        let (from_collection, from_key) = edge.from.split_once(':').ok_or_else(|| {
            StorageError::Conflict(format!("Invalid document ID format: '{}'", edge.from))
        })?;
        let (to_collection, to_key) = edge.to.split_once(':').ok_or_else(|| {
            StorageError::Conflict(format!("Invalid document ID format: '{}'", edge.to))
        })?;

        let from_ks = self.keyspaces.get(from_collection).ok_or_else(|| {
            StorageError::NotFound(format!("Collection '{}' does not exist", from_collection))
        })?;
        let to_ks = self.keyspaces.get(to_collection).ok_or_else(|| {
            StorageError::NotFound(format!("Collection '{}' does not exist", to_collection))
        })?;

        let from_exists: Option<fjall::Slice> = tx.get(&from_ks.exists, from_key.as_bytes())?;
        if from_exists.is_none() {
            return Err(StorageError::NotFound(format!(
                "Source document '{}' does not exist",
                edge.from
            )));
        }

        let to_exists: Option<fjall::Slice> = tx.get(&to_ks.exists, to_key.as_bytes())?;
        if to_exists.is_none() {
            return Err(StorageError::NotFound(format!(
                "Target document '{}' does not exist",
                edge.to
            )));
        }

        Ok(())
    }

    /// Get an edge
    pub fn get_edge(
        &self,
        from: &str,
        label: &str,
        to: &str,
    ) -> Result<Option<Edge>, StorageError> {
        self.edges.get(from, label, to)
    }

    /// Delete an edge
    pub fn delete_edge(&self, from: &str, label: &str, to: &str) -> Result<bool, StorageError> {
        let count = self.edges.delete(from, label, to)?;
        Ok(count > 0)
    }

    /// List outgoing edges
    pub fn list_edges_out(
        &self,
        from: &str,
        label: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Edge>, Option<String>), StorageError> {
        self.edges.list_out(from, label, after, limit)
    }

    /// List incoming edges
    pub fn list_edges_in(
        &self,
        to: &str,
        label: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Edge>, Option<String>), StorageError> {
        self.edges.list_in(to, label, after, limit)
    }

    /// Batch list outgoing edges for multiple nodes
    pub fn batch_list_edges_out(
        &self,
        from_nodes: &[&str],
        label: &str,
        limit_per_node: usize,
    ) -> Result<std::collections::HashMap<String, Vec<Edge>>, StorageError> {
        self.edges.list_out_batch(from_nodes, label, limit_per_node)
    }

    /// Batch list incoming edges for multiple nodes
    pub fn batch_list_edges_in(
        &self,
        to_nodes: &[&str],
        label: &str,
        limit_per_node: usize,
    ) -> Result<std::collections::HashMap<String, Vec<Edge>>, StorageError> {
        self.edges.list_in_batch(to_nodes, label, limit_per_node)
    }

    // =========================================================================
    // Schema operations
    // =========================================================================

    /// Get schema for a collection
    pub fn get_schema(&self, collection: &str) -> Option<CollectionSchema> {
        self.schemas.get(collection)
    }

    /// Save a schema
    pub fn save_schema(&self, schema: &CollectionSchema) -> Result<(), StorageError> {
        self.get_or_create_collection(&schema.name)?;
        self.schemas.save(schema)?;

        // Initialize HNSW instances for vector indexes
        for index in &schema.indexes {
            if index.index_type == IndexType::Hnsw
                && let Some(params) = &index.hnsw_params
            {
                self.indexes.hnsw_backend().get_or_create_instance(
                    &schema.name,
                    &index.fields[0],
                    params,
                )?;
            }
        }

        // Load indexes into index manager
        self.indexes
            .load_indexes(&schema.name, schema.indexes.clone());

        Ok(())
    }

    /// Delete a collection and its schema
    pub fn delete_schema(&self, collection: &str) -> Result<bool, StorageError> {
        if !self.schemas.contains(collection) && !self.keyspaces.exists(collection) {
            return Ok(false);
        }

        // Remove schema
        self.schemas.delete(collection)?;

        // Remove indexes
        self.indexes.remove_collection(collection);

        // Clear and remove keyspaces
        self.keyspaces.clear(collection)?;
        self.keyspaces.remove(collection);

        Ok(true)
    }

    /// Get reference to schema registry
    pub fn schema_registry(&self) -> &SchemaRegistry {
        self.schemas.registry()
    }

    // =========================================================================
    // Index operations
    // =========================================================================

    /// Get all indexes for a collection
    pub fn get_indexes(&self, collection: &str) -> Vec<IndexDef> {
        self.indexes.get_indexes(collection)
    }

    /// Check if an index exists
    pub fn has_index(&self, collection: &str, fields: &[String]) -> bool {
        self.indexes.has_index(collection, fields)
    }

    /// Get reference to index registry
    pub fn index_registry(&self) -> &super::indexes::IndexRegistry {
        self.indexes.registry()
    }

    /// Build an index from existing documents
    pub fn build_index(&self, collection: &str, index: &IndexDef) -> Result<(), StorageError> {
        let completed = self.build_index_with_progress(collection, index, |_| true)?;
        debug_assert!(completed, "unconditional index build cannot be cancelled");
        Ok(())
    }

    /// Change resource limits for subsequent index builds.
    pub fn set_index_build_config(&self, config: IndexBuildConfig) -> Result<(), StorageError> {
        *self.index_build_config.write() = config.validate()?;
        Ok(())
    }

    pub fn index_build_config(&self) -> IndexBuildConfig {
        *self.index_build_config.read()
    }

    fn build_btree_from_serialized(
        &self,
        collection: &str,
        index: &IndexDef,
        index_keyspace: &fjall::OptimisticTxKeyspace,
        unique_keyspace: Option<&fjall::OptimisticTxKeyspace>,
        mut progress: impl FnMut(crate::storage::IndexBuildEvent) -> bool,
    ) -> Result<bool, StorageError> {
        struct PreparedEntry {
            index_key: Option<Vec<u8>>,
            unique_lock: Option<([u8; 16], Vec<u8>)>,
        }

        let config = self.index_build_config();
        let mut docs = self.documents.iter_serialized(collection)?;
        let mut pending = None;
        let mut processed_items = 0usize;
        let fields_key = index.fields.join(",");

        loop {
            if !progress(crate::storage::IndexBuildEvent::Building { processed_items }) {
                return Ok(false);
            }

            // Serialized input is capped as well as document count. Keeping the
            // cap below the entry budget leaves room for decoded documents and
            // prepared keys; a single oversized document is handled alone.
            let raw_budget = (config.memory_budget_bytes / 2).max(1);
            let mut raw_chunk = Vec::with_capacity(config.chunk_size.min(1024));
            let mut raw_bytes = 0usize;
            while raw_chunk.len() < config.chunk_size {
                let next = match pending.take() {
                    Some(bytes) => Some(Ok(bytes)),
                    None => docs.next(),
                };
                let Some(bytes) = next else { break };
                let bytes = bytes?;
                if !raw_chunk.is_empty() && raw_bytes.saturating_add(bytes.len()) > raw_budget {
                    pending = Some(bytes);
                    break;
                }
                raw_bytes = raw_bytes.saturating_add(bytes.len());
                raw_chunk.push(bytes);
            }
            if raw_chunk.is_empty() {
                break;
            }

            let worker_count = config.worker_count.min(raw_chunk.len());
            let per_worker = raw_chunk.len().div_ceil(worker_count);
            let mut prepared = Vec::with_capacity(raw_chunk.len());
            std::thread::scope(|scope| -> Result<(), StorageError> {
                let mut handles = Vec::with_capacity(worker_count);
                let fields_key = &fields_key;
                for raw_docs in raw_chunk.chunks(per_worker) {
                    handles.push(scope.spawn(
                        move || -> Result<Vec<PreparedEntry>, StorageError> {
                            let mut entries = Vec::with_capacity(raw_docs.len());
                            for raw in raw_docs {
                                let doc = rkyv::from_bytes::<Document, rancor::Error>(raw)
                                    .map_err(|error| {
                                        StorageError::Serialization(error.to_string())
                                    })?;
                                let index_key = self
                                    .indexes
                                    .btree_backend()
                                    .build_index_key(&doc, &index.fields);
                                let unique_lock = unique_keyspace.and_then(|_| {
                                    self.constraints
                                        .get_field_value_for_index(&doc, &index.fields)
                                        .map(|value| {
                                            (
                                                self.constraints
                                                    .compute_unique_lock_key(fields_key, &value),
                                                doc.key().as_bytes().to_vec(),
                                            )
                                        })
                                });
                                entries.push(PreparedEntry {
                                    index_key,
                                    unique_lock,
                                });
                            }
                            Ok(entries)
                        },
                    ));
                }
                for handle in handles {
                    prepared.extend(handle.join().map_err(|_| {
                        StorageError::InvalidOperation("index build worker panicked".into())
                    })??);
                }
                Ok(())
            })?;

            prepared.sort_unstable_by(|left, right| left.index_key.cmp(&right.index_key));
            if let Some(unique_keyspace) = unique_keyspace {
                let mut locks: Vec<_> = prepared
                    .iter()
                    .filter_map(|entry| entry.unique_lock.as_ref())
                    .collect();
                locks.sort_unstable_by_key(|(key, _)| *key);
                for pair in locks.windows(2) {
                    if pair[0].0 == pair[1].0 && pair[0].1 != pair[1].1 {
                        return Err(StorageError::Conflict(format!(
                            "Duplicate value for unique index on {:?}",
                            index.fields
                        )));
                    }
                }
                for (lock_key, doc_key) in locks {
                    if let Some(existing) = unique_keyspace.get(lock_key)?
                        && existing.as_ref() != doc_key.as_slice()
                    {
                        return Err(StorageError::Conflict(format!(
                            "Duplicate value for unique index on {:?}",
                            index.fields
                        )));
                    }
                }
            }

            // Cancellation/deadline callback immediately precedes the write,
            // so no new batch starts after a cancellation is observed.
            if !progress(crate::storage::IndexBuildEvent::Building { processed_items }) {
                return Ok(false);
            }

            let mut offset = 0usize;
            while offset < prepared.len() {
                if !progress(crate::storage::IndexBuildEvent::Building { processed_items }) {
                    return Ok(false);
                }
                let start = offset;
                let mut batch_bytes = 0usize;
                while offset < prepared.len() {
                    let entry = &prepared[offset];
                    let entry_bytes = entry.index_key.as_ref().map_or(0, Vec::len)
                        + entry
                            .unique_lock
                            .as_ref()
                            .map_or(0, |(_, doc_key)| 16 + doc_key.len());
                    if offset > start
                        && batch_bytes.saturating_add(entry_bytes) > config.memory_budget_bytes
                    {
                        break;
                    }
                    batch_bytes = batch_bytes.saturating_add(entry_bytes);
                    offset += 1;
                }

                let mut batch = self.db.inner().batch();
                for entry in &prepared[start..offset] {
                    if let Some(key) = &entry.index_key {
                        batch.insert(index_keyspace.inner(), key.clone(), Vec::<u8>::new());
                    }
                    if let (Some(unique_keyspace), Some((lock_key, doc_key))) =
                        (unique_keyspace, &entry.unique_lock)
                    {
                        batch.insert(unique_keyspace.inner(), *lock_key, doc_key.clone());
                    }
                }
                batch.commit()?;
                #[cfg(feature = "fault-injection")]
                crate::fault_injection::check("btree_backfill_chunk")?;
            }

            processed_items += raw_chunk.len();
            if !progress(crate::storage::IndexBuildEvent::Building { processed_items }) {
                return Ok(false);
            }
        }

        Ok(progress(crate::storage::IndexBuildEvent::Committing))
    }

    /// Build an index with coarse progress/cancellation checkpoints. Returns
    /// `Ok(false)` when cancellation was requested before schema publication.
    pub fn build_index_with_progress(
        &self,
        collection: &str,
        index: &IndexDef,
        mut progress: impl FnMut(crate::storage::IndexBuildEvent) -> bool,
    ) -> Result<bool, StorageError> {
        let keyspaces = self
            .keyspaces
            .get(collection)
            .ok_or_else(|| StorageError::CollectionNotLoaded("collection not found".into()))?;

        if index.index_type == IndexType::BTree {
            let index_keyspace = self
                .keyspaces
                .get_or_create_index_keyspace(collection, &index.name)?;
            return self.build_btree_from_serialized(
                collection,
                index,
                &index_keyspace,
                index.unique.then_some(&keyspaces.unique_locks),
                &mut progress,
            );
        }

        // Build non-B-tree index entries
        let docs_iter = self.documents.iter(collection)?;
        if !self
            .indexes
            .build_index_with_progress(collection, index, docs_iter, &mut progress)?
        {
            return Ok(false);
        }

        Ok(true)
    }

    /// Allocate an isolated physical artifact for a CREATE INDEX lifecycle.
    pub fn initialize_index_build_artifact(
        &self,
        collection: &str,
        index: &IndexDef,
        build_id: &str,
    ) -> Result<(), StorageError> {
        self.indexes.initialize_build(collection, index, build_id)
    }

    /// Backfill and validate an unpublished artifact. Unique constraint locks
    /// are also built in a temporary keyspace and cannot affect live writes.
    pub fn build_index_artifact_with_progress(
        &self,
        collection: &str,
        index: &IndexDef,
        build_id: &str,
        mut progress: impl FnMut(crate::storage::IndexBuildEvent) -> bool,
    ) -> Result<bool, StorageError> {
        if index.index_type == IndexType::BTree {
            let index_build = self
                .keyspaces
                .get_or_create_index_build_keyspace(collection, build_id)?;
            let unique_build = if index.unique {
                Some(
                    self.keyspaces
                        .get_or_create_index_build_unique_keyspace(collection, build_id)?,
                )
            } else {
                None
            };
            return self.build_btree_from_serialized(
                collection,
                index,
                &index_build,
                unique_build.as_ref(),
                &mut progress,
            );
        }

        let docs_iter = self.documents.iter(collection)?;
        if !self.indexes.build_artifact_with_progress(
            collection,
            index,
            build_id,
            docs_iter,
            &mut progress,
        )? {
            return Ok(false);
        }

        Ok(true)
    }

    /// Move an already flushed build into final physical storage and flush the
    /// result. The planner still cannot see it until `publish_ready_index`.
    pub fn promote_index_build_artifact(
        &self,
        collection: &str,
        index: &IndexDef,
        build_id: &str,
    ) -> Result<(), StorageError> {
        self.indexes.promote_build(collection, index, build_id)?;
        if index.unique {
            self.keyspaces
                .promote_index_build_unique_keyspace(collection, build_id)?;
        }
        self.sync()
    }

    /// Durable schema is the publication commit marker. Registry visibility is
    /// installed only after that write succeeds and while the DDL lock is held.
    pub fn publish_ready_index(
        &self,
        collection: &str,
        index: IndexDef,
    ) -> Result<(), StorageError> {
        self.schemas.add_index(collection, index.clone())?;
        self.sync()?;
        if !self.indexes.registry().has_index(collection, &index.fields) {
            self.indexes.registry().register(collection, index);
        }
        Ok(())
    }

    pub fn cleanup_index_build_artifact(
        &self,
        collection: &str,
        index: &IndexDef,
        build_id: &str,
    ) -> Result<(), StorageError> {
        self.indexes.cleanup_build(collection, index, build_id)?;
        self.sync()
    }

    /// Remove confirmed orphaned resources before durable jobs are resumed.
    pub fn reconcile_index_build_artifacts(&self) -> Result<(), StorageError> {
        self.indexes.cleanup_orphaned_builds()?;
        self.sync()
    }

    /// Delete a final-name physical artifact only when durable schema proves it
    /// was never published. Document storage is never touched.
    pub fn cleanup_unpublished_index(
        &self,
        collection: &str,
        index: &IndexDef,
    ) -> Result<(), StorageError> {
        let exact_schema_entry = self
            .schemas
            .get(collection)
            .is_some_and(|schema| schema.indexes.iter().any(|existing| existing == index));
        if exact_schema_entry {
            self.schemas.remove_index(collection, &index.fields)?;
        }
        self.indexes.registry().remove(collection, &index.fields);
        self.drop_index(collection, index)?;
        self.sync()
    }

    /// Add an index to a collection's schema atomically.
    /// Uses atomic operations to prevent race conditions when multiple indexes
    /// are created concurrently.
    pub fn add_index_to_schema(
        &self,
        collection: &str,
        index: IndexDef,
    ) -> Result<(), StorageError> {
        // Use atomic add_index which ensures schema exists and adds atomically
        self.schemas.add_index(collection, index)?;
        Ok(())
    }

    /// Drop an index by type.
    ///
    /// Dispatches to the appropriate backend based on index type.
    /// Caller must provide the index definition since the registry may already have
    /// been updated before calling this method.
    pub fn drop_index(&self, collection: &str, index: &IndexDef) -> Result<(), StorageError> {
        let keyspaces = self
            .keyspaces
            .get(collection)
            .ok_or_else(|| StorageError::CollectionNotLoaded("collection not found".into()))?;

        // Check if this was a unique index (only BTree indexes can be unique)
        if index.index_type == IndexType::BTree && index.unique {
            // Remove unique constraint entries
            let docs_iter = self.documents.iter(collection)?;
            self.constraints
                .drop_for_index(&keyspaces.unique_locks, index, docs_iter)?;
        }

        // Drop index entries based on type
        match index.index_type {
            IndexType::BTree => {
                self.indexes.drop_btree_index(collection, &index.name)?;
            }
            IndexType::Hnsw => {
                // HNSW uses field name as index identifier
                let field = index.fields.first().ok_or_else(|| {
                    StorageError::InvalidConfig("HNSW index requires at least one field".into())
                })?;
                self.indexes
                    .hnsw_backend()
                    .remove_index(collection, field)?;
            }
            IndexType::FullText => {
                // FTS uses sorted fields joined with _fts suffix
                let mut sorted_fields = index.fields.to_vec();
                sorted_fields.sort();
                let fts_index_name = format!("{}_fts", sorted_fields.join("_"));
                self.indexes
                    .fts_backend()
                    .remove_index(collection, &fts_index_name)?;
            }
        }

        Ok(())
    }

    /// Remove an index from schema atomically.
    pub fn remove_index_from_schema(
        &self,
        collection: &str,
        fields: &[String],
    ) -> Result<(), StorageError> {
        // Use atomic remove_index
        self.schemas.remove_index(collection, fields)?;
        Ok(())
    }

    // =========================================================================
    // Index scan operations
    // =========================================================================

    /// Scan index for equality
    pub fn scan_index_eq(
        &self,
        collection: &str,
        field: &str,
        value: &Value,
        limit: usize,
    ) -> Result<(Vec<String>, bool), StorageError> {
        // Find the index for this field
        let indexes = self.indexes.get_indexes(collection);
        let index = indexes
            .iter()
            .find(|idx| {
                idx.index_type == IndexType::BTree && idx.fields.first() == Some(&field.to_string())
            })
            .ok_or_else(|| StorageError::IndexLookupFailed(format!("field '{}'", field)))?;

        let result = self
            .indexes
            .scan_eq(collection, &index.name, field, value, limit)?;
        Ok((result.doc_ids, result.exceeded_limit))
    }

    /// Scan index for range
    pub fn scan_index_range(
        &self,
        collection: &str,
        field: &str,
        value: &Value,
        op: &str,
        limit: usize,
    ) -> Result<(Vec<String>, bool), StorageError> {
        // Find the index for this field
        let indexes = self.indexes.get_indexes(collection);
        let index = indexes
            .iter()
            .find(|idx| {
                idx.index_type == IndexType::BTree && idx.fields.first() == Some(&field.to_string())
            })
            .ok_or_else(|| StorageError::IndexLookupFailed(format!("field '{}'", field)))?;

        let result = self
            .indexes
            .scan_range(collection, &index.name, field, op, value, limit)?;
        Ok((result.doc_ids, result.exceeded_limit))
    }

    /// Scan index for range between
    #[allow(clippy::too_many_arguments)]
    pub fn scan_index_range_between(
        &self,
        collection: &str,
        field: &str,
        lower: &Value,
        lower_inclusive: bool,
        upper: &Value,
        upper_inclusive: bool,
        limit: usize,
    ) -> Result<(Vec<String>, bool), StorageError> {
        // Find the index for this field
        let indexes = self.indexes.get_indexes(collection);
        let index = indexes
            .iter()
            .find(|idx| {
                idx.index_type == IndexType::BTree && idx.fields.first() == Some(&field.to_string())
            })
            .ok_or_else(|| StorageError::IndexLookupFailed(format!("field '{}'", field)))?;

        let result = self.indexes.scan_range_between(
            collection,
            &index.name,
            field,
            lower,
            lower_inclusive,
            upper,
            upper_inclusive,
            limit,
        )?;
        Ok((result.doc_ids, result.exceeded_limit))
    }

    /// Scan compound index for equality
    pub fn scan_compound_index_eq(
        &self,
        collection: &str,
        field_values: &[(&str, &Value)],
        limit: usize,
    ) -> Result<(Vec<String>, bool), StorageError> {
        // Find a compound index whose fields START with the requested fields (prefix match)
        let fields: Vec<String> = field_values.iter().map(|(f, _)| f.to_string()).collect();
        let indexes = self.indexes.get_indexes(collection);
        let index = indexes
            .iter()
            .find(|idx| {
                idx.index_type == IndexType::BTree
                    && idx.fields.len() >= fields.len()
                    && idx.fields[..fields.len()] == fields
            })
            .ok_or_else(|| {
                StorageError::IndexLookupFailed(format!("compound fields {:?}", fields))
            })?;

        let result = self
            .indexes
            .scan_compound_eq(collection, &index.name, field_values, limit)?;
        Ok((result.doc_ids, result.exceeded_limit))
    }

    /// Scan compound index and return documents directly
    pub fn scan_compound_index_eq_docs(
        &self,
        collection: &str,
        field_values: &[(&str, &Value)],
        limit: usize,
    ) -> Result<(Vec<Document>, bool), StorageError> {
        // Find a compound index whose fields START with the requested fields (prefix match)
        let fields: Vec<String> = field_values.iter().map(|(f, _)| f.to_string()).collect();
        let indexes = self.indexes.get_indexes(collection);
        let index = indexes
            .iter()
            .find(|idx| {
                idx.index_type == IndexType::BTree
                    && idx.fields.len() >= fields.len()
                    && idx.fields[..fields.len()] == fields
            })
            .ok_or_else(|| {
                StorageError::IndexLookupFailed(format!("compound fields {:?}", fields))
            })?;

        let result = self
            .indexes
            .scan_compound_eq(collection, &index.name, field_values, limit)?;
        let docs = self.documents.get_by_ids(collection, &result.doc_ids)?;
        Ok((docs, result.exceeded_limit))
    }

    /// Get multiple documents by their IDs
    pub fn get_documents_by_ids(
        &self,
        collection: &str,
        doc_ids: &[String],
    ) -> Result<Vec<Document>, StorageError> {
        self.documents.get_by_ids(collection, doc_ids)
    }

    /// Find a usable index for a field
    pub fn find_index_for_field(&self, collection: &str, field: &str) -> Option<(String, bool)> {
        self.indexes.find_index_for_field(collection, field)
    }

    /// Find a compound index
    pub fn find_compound_index(&self, collection: &str, fields: &[&str]) -> Option<Vec<String>> {
        self.indexes.find_compound_index(collection, fields)
    }

    /// Find FTS index that covers a specific field
    pub fn find_fts_index_for_field(&self, collection: &str, field: &str) -> Option<String> {
        self.indexes.find_fts_index_for_field(collection, field)
    }

    /// Find any FTS index for a collection
    pub fn find_fts_index(&self, collection: &str) -> Option<String> {
        self.indexes.find_fts_index(collection)
    }

    // =========================================================================
    // FTS operations
    // =========================================================================

    /// Search FTS index
    ///
    /// Returns (doc_id, score) pairs sorted by relevance.
    pub fn fts_search(
        &self,
        collection: &str,
        index_name: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        self.indexes
            .fts_backend()
            .search(collection, index_name, query, limit)
    }

    /// Search FTS index with field boosts
    ///
    /// Returns (doc_id, score) pairs sorted by relevance.
    pub fn fts_search_with_boosts(
        &self,
        collection: &str,
        index_name: &str,
        query: &str,
        field_boosts: &[(String, f64)],
        limit: usize,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        self.indexes.fts_backend().search_with_boosts(
            collection,
            index_name,
            query,
            field_boosts,
            limit,
        )
    }

    // =========================================================================
    // Vector search operations
    // =========================================================================

    /// Find HNSW index that covers a specific field
    pub fn find_hnsw_index_for_field(&self, collection: &str, field: &str) -> Option<String> {
        self.indexes.find_hnsw_index_for_field(collection, field)
    }

    /// Initialize an HNSW index instance for a collection/field
    ///
    /// This must be called before building or searching the index.
    pub fn init_hnsw_index(
        &self,
        collection: &str,
        field: &str,
        params: &HnswParams,
    ) -> Result<(), StorageError> {
        self.indexes
            .hnsw_backend()
            .get_or_create_instance(collection, field, params)?;
        Ok(())
    }

    /// Initialize an FTS index instance for a collection
    ///
    /// This must be called before building or searching the index to set the analyzer.
    /// If not called, the default "standard" analyzer will be used.
    pub fn init_fts_index(
        &self,
        collection: &str,
        index_name: &str,
        fields: &[String],
        analyzer: Option<&str>,
    ) -> Result<(), StorageError> {
        self.indexes
            .fts_backend()
            .init_index(collection, index_name, fields, analyzer)
    }

    /// KNN vector search
    ///
    /// Returns (doc_id, distance) pairs sorted by distance.
    pub fn vector_search(
        &self,
        collection: &str,
        index_name: &str,
        vector: &[f32],
        k: usize,
        effort: Option<usize>,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        self.indexes
            .hnsw_backend()
            .search(collection, index_name, vector, k, effort)
    }

    // =========================================================================
    // Component accessors (for REINDEX and similar operations)
    // =========================================================================

    /// Get reference to the document store
    pub fn documents(&self) -> &DocumentStore {
        &self.documents
    }

    /// Get reference to the index manager
    pub fn indexes(&self) -> &IndexManager {
        &self.indexes
    }

    /// Acquire a query execution guard.
    ///
    /// This should be called at the start of query execution and held until
    /// the query completes. The guard prevents DDL operations (DROP INDEX,
    /// REINDEX) from modifying indexes while the query is running.
    ///
    /// # Example
    /// ```ignore
    /// let _guard = storage.acquire_query_guard();
    /// // Plan and execute query...
    /// // Guard dropped automatically when query finishes
    /// ```
    pub fn acquire_query_guard(&self) -> crate::storage::indexes::QueryExecutionGuard<'_> {
        self.indexes.acquire_query_guard()
    }

    // =========================================================================
    // Analyzer operations
    // =========================================================================

    /// Save a custom analyzer definition
    pub fn save_analyzer(&self, def: &AnalyzerDef) -> Result<(), StorageError> {
        self.schemas.save_analyzer(def)
    }

    /// Get a custom analyzer definition by name
    pub fn get_analyzer(&self, name: &str) -> Option<AnalyzerDef> {
        self.schemas.get_analyzer(name)
    }

    /// List all custom analyzer names
    pub fn list_analyzers(&self) -> Vec<String> {
        self.schemas.list_analyzers()
    }

    /// Delete a custom analyzer
    pub fn delete_analyzer(&self, name: &str) -> Result<bool, StorageError> {
        self.schemas.delete_analyzer(name)
    }

    /// Load all custom analyzers (called at startup)
    pub fn load_all_analyzers(&self) -> Vec<AnalyzerDef> {
        self.schemas.load_all_analyzers()
    }

    // =========================================================================
    // Misc operations
    // =========================================================================

    /// Sync all data to disk (including indexes)
    pub fn sync(&self) -> Result<(), StorageError> {
        // Commit all indexes (HNSW, FTS) to ensure metadata is saved
        self.indexes.commit_all()?;

        // Sync Fjall database
        self.db.persist(PersistMode::SyncAll)?;
        Ok(())
    }

    /// Get storage statistics
    pub fn stats(&self) -> StorageStats {
        let (btree_indexes, vector_indexes, fts_indexes) = self.indexes.registry().count_by_type();

        StorageStats {
            collections: self.keyspaces.list().len(),
            edge_labels: self.edges.list_edge_types().len(),
            btree_indexes,
            vector_indexes,
            fts_indexes,
            disk_size: self.db.disk_space().unwrap_or(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::{Edge, EdgeSchema};
    use crate::storage::DIR_STORAGE;
    use crate::storage::WriteOp;
    use crate::storage::encoding::encode_edge_out_key_single;
    use fjall::KeyspaceCreateOptions;
    use rkyv::rancor;
    #[cfg(feature = "fts")]
    use serde_json::json;
    use std::collections::HashMap;
    use tempfile::TempDir;

    #[test]
    fn open_writes_storage_format_version_marker() {
        let tmp = TempDir::new().unwrap();

        {
            let db = Database::open(tmp.path()).unwrap();
            db.sync().unwrap();
        }

        let storage =
            fjall::OptimisticTxDatabase::builder(tmp.path().join(crate::storage::DIR_STORAGE))
                .open()
                .unwrap();
        let meta = storage
            .keyspace("_meta", KeyspaceCreateOptions::default)
            .unwrap();
        let stored = meta
            .get(STORAGE_FORMAT_VERSION_KEY)
            .unwrap()
            .expect("storage format marker should be written on open");

        assert_eq!(
            stored.as_ref(),
            CURRENT_STORAGE_FORMAT_VERSION.to_string().as_bytes()
        );
    }

    #[test]
    fn open_upgrades_legacy_root_storage_layout() {
        let tmp = TempDir::new().unwrap();

        write_legacy_root_layout(tmp.path());

        assert!(
            !tmp.path().join(DIR_STORAGE).exists(),
            "legacy layout should keep Fjall files at the database root"
        );

        let db = Database::open(tmp.path()).unwrap();

        assert!(tmp.path().join(DIR_STORAGE).is_dir());
        assert!(db.collection_exists("users"));
        let doc = db.get_document("users", "alice").unwrap().unwrap();
        assert_eq!(doc.fields.get("name"), Some(&Value::String("Alice".into())));
    }

    #[test]
    fn open_upgrades_legacy_root_storage_layout_after_empty_storage_dir_was_created() {
        let tmp = TempDir::new().unwrap();
        write_legacy_root_layout(tmp.path());
        std::fs::create_dir(tmp.path().join(DIR_STORAGE)).unwrap();

        let db = Database::open(tmp.path()).unwrap();

        assert!(db.collection_exists("users"));
        let doc = db.get_document("users", "alice").unwrap().unwrap();
        assert_eq!(doc.fields.get("name"), Some(&Value::String("Alice".into())));
    }

    fn write_legacy_root_layout(path: &Path) {
        let old = fjall::OptimisticTxDatabase::builder(path).open().unwrap();
        let meta = old
            .keyspace("_meta", KeyspaceCreateOptions::default)
            .unwrap();
        meta.insert(b"coll\x00users", b"").unwrap();

        let docs = old
            .keyspace("docs_users", KeyspaceCreateOptions::default)
            .unwrap();
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), Value::String("Alice".to_string()));
        let doc = Document {
            id: "users:alice".to_string(),
            fields,
        };
        let bytes = rkyv::to_bytes::<rancor::Error>(&doc).unwrap();
        docs.insert(b"alice", bytes.as_slice()).unwrap();

        old.persist(PersistMode::SyncAll).unwrap();
    }

    #[cfg(feature = "fts")]
    fn init_test_fts_index(db: &Database, collection: &str, field: &str) -> IndexDef {
        db.get_or_create_collection(collection).unwrap();
        let index = IndexDef::fulltext(collection, vec![field.to_string()]);
        db.init_fts_index(collection, &index.name, &index.fields, None)
            .unwrap();
        db.build_index(collection, &index).unwrap();
        db.index_registry().register(collection, index.clone());
        db.add_index_to_schema(collection, index.clone()).unwrap();
        index
    }

    #[cfg(feature = "fts")]
    fn test_text_doc(collection: &str, key: &str, text: impl Into<String>) -> Document {
        Document {
            id: format!("{collection}:{key}"),
            fields: [("body".to_string(), Value::String(text.into()))]
                .into_iter()
                .collect(),
        }
    }

    fn pending_external_index_job_count(db: &Database) -> usize {
        db.index_jobs.inner().iter().count()
    }

    #[cfg(feature = "fts")]
    #[test]
    fn atomic_batch_insert_commits_fulltext_once() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();
        let index = init_test_fts_index(&db, "articles", "body");
        let commits_before = db.external_index_apply_commit_count_for_test();
        let cleanups_before = db.external_index_job_cleanup_commit_count_for_test();

        let ops = (0..100)
            .map(|i| {
                WriteOp::InsertDoc(test_text_doc(
                    "articles",
                    &i.to_string(),
                    format!("shared batch token {i}"),
                ))
            })
            .collect();
        db.atomic_write_batch(ops).unwrap();

        assert_eq!(
            db.external_index_apply_commit_count_for_test() - commits_before,
            1,
            "one primary batch must cause one external-index commit"
        );
        assert_eq!(
            db.external_index_job_cleanup_commit_count_for_test() - cleanups_before,
            1,
            "all durable jobs must be removed by one cleanup transaction"
        );
        assert_eq!(pending_external_index_job_count(&db), 0);
        assert_eq!(
            db.fts_search("articles", &index.name, "shared", 200)
                .unwrap()
                .len(),
            100
        );
    }

    #[cfg(feature = "fts")]
    #[test]
    fn sql_multi_row_insert_uses_one_external_and_cleanup_commit() {
        let tmp = TempDir::new().unwrap();
        let db = std::sync::Arc::new(Database::open(tmp.path()).unwrap());
        let index = init_test_fts_index(&db, "articles", "body");
        let commits_before = db.external_index_apply_commit_count_for_test();
        let cleanups_before = db.external_index_job_cleanup_commit_count_for_test();

        let values = (0..100)
            .map(|i| format!(r#"{{id: "doc_{i}", body: "batch token {i}"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        let query = format!("INSERT INTO articles {values}");
        crate::run_sql!(db.clone(), &query);

        assert_eq!(
            db.external_index_apply_commit_count_for_test() - commits_before,
            1
        );
        assert_eq!(
            db.external_index_job_cleanup_commit_count_for_test() - cleanups_before,
            1
        );
        assert_eq!(pending_external_index_job_count(&db), 0);
        assert_eq!(
            db.fts_search("articles", &index.name, "batch", 200)
                .unwrap()
                .len(),
            100
        );
    }

    #[cfg(feature = "fts")]
    #[test]
    fn atomic_batch_update_commits_fulltext_once_and_replaces_terms() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();
        let index = init_test_fts_index(&db, "articles", "body");
        db.atomic_write_batch(
            (0..20)
                .map(|i| {
                    WriteOp::InsertDoc(test_text_doc(
                        "articles",
                        &i.to_string(),
                        "obsolete batch term",
                    ))
                })
                .collect(),
        )
        .unwrap();
        let commits_before = db.external_index_apply_commit_count_for_test();
        let cleanups_before = db.external_index_job_cleanup_commit_count_for_test();

        db.atomic_write_batch(
            (0..20)
                .map(|i| {
                    WriteOp::InsertDoc(test_text_doc(
                        "articles",
                        &i.to_string(),
                        "replacement batch term",
                    ))
                })
                .collect(),
        )
        .unwrap();

        assert_eq!(
            db.external_index_apply_commit_count_for_test() - commits_before,
            1
        );
        assert_eq!(
            db.external_index_job_cleanup_commit_count_for_test() - cleanups_before,
            1
        );
        assert!(
            db.fts_search("articles", &index.name, "obsolete", 100)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            db.fts_search("articles", &index.name, "replacement", 100)
                .unwrap()
                .len(),
            20
        );
    }

    #[cfg(feature = "fts")]
    #[test]
    fn atomic_batch_delete_commits_fulltext_once_and_removes_terms() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();
        let index = init_test_fts_index(&db, "articles", "body");
        db.atomic_write_batch(
            (0..20)
                .map(|i| {
                    WriteOp::InsertDoc(test_text_doc(
                        "articles",
                        &i.to_string(),
                        "deletable batch term",
                    ))
                })
                .collect(),
        )
        .unwrap();
        let commits_before = db.external_index_apply_commit_count_for_test();
        let cleanups_before = db.external_index_job_cleanup_commit_count_for_test();

        db.atomic_write_batch(
            (0..20)
                .map(|i| WriteOp::DeleteDoc {
                    collection: "articles".to_string(),
                    key: i.to_string(),
                })
                .collect(),
        )
        .unwrap();

        assert_eq!(
            db.external_index_apply_commit_count_for_test() - commits_before,
            1
        );
        assert_eq!(
            db.external_index_job_cleanup_commit_count_for_test() - cleanups_before,
            1
        );
        assert!(
            db.fts_search("articles", &index.name, "deletable", 100)
                .unwrap()
                .is_empty()
        );
    }

    #[cfg(feature = "fts")]
    #[test]
    fn atomic_batch_external_commit_failure_keeps_all_jobs_for_replay() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();
        let index = init_test_fts_index(&db, "articles", "body");
        let commits_before = db.external_index_apply_commit_count_for_test();
        let cleanups_before = db.external_index_job_cleanup_commit_count_for_test();
        db.fail_next_external_index_apply_commit_for_test();

        db.atomic_write_batch(
            (0..10)
                .map(|i| {
                    WriteOp::InsertDoc(test_text_doc(
                        "articles",
                        &i.to_string(),
                        "recoverable batch term",
                    ))
                })
                .collect(),
        )
        .expect("a post-commit index failure must not report the primary write as failed");

        for i in 0..10 {
            assert!(
                db.get_document("articles", &i.to_string())
                    .unwrap()
                    .is_some()
            );
        }
        assert_eq!(
            pending_external_index_job_count(&db),
            10,
            "no durable job may be removed when the shared external commit fails"
        );
        assert_eq!(
            db.external_index_apply_commit_count_for_test(),
            commits_before,
            "a failed external commit must not be counted as successful"
        );
        assert_eq!(
            db.external_index_job_cleanup_commit_count_for_test(),
            cleanups_before,
            "cleanup must not start when the external commit fails"
        );

        db.replay_external_index_jobs().unwrap();

        assert_eq!(
            db.external_index_apply_commit_count_for_test() - commits_before,
            1,
            "replay must commit all pending external jobs once"
        );
        assert_eq!(
            db.external_index_job_cleanup_commit_count_for_test() - cleanups_before,
            1,
            "replay must clean all pending jobs in one transaction"
        );
        assert_eq!(pending_external_index_job_count(&db), 0);
        assert_eq!(
            db.fts_search("articles", &index.name, "recoverable", 100)
                .unwrap()
                .len(),
            10
        );
    }

    #[cfg(feature = "fts")]
    #[test]
    fn atomic_batch_cleanup_failure_keeps_all_jobs_for_idempotent_replay() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();
        let index = init_test_fts_index(&db, "articles", "body");
        let commits_before = db.external_index_apply_commit_count_for_test();
        let cleanups_before = db.external_index_job_cleanup_commit_count_for_test();
        db.fail_next_external_index_job_cleanup_commit_for_test();

        db.atomic_write_batch(
            (0..10)
                .map(|i| {
                    WriteOp::InsertDoc(test_text_doc(
                        "articles",
                        &i.to_string(),
                        "cleanup recovery token",
                    ))
                })
                .collect(),
        )
        .expect("cleanup happens after the primary commit and must not report a false failure");

        assert_eq!(
            db.external_index_apply_commit_count_for_test() - commits_before,
            1,
            "external indexes must already be committed before cleanup"
        );
        assert_eq!(
            db.external_index_job_cleanup_commit_count_for_test(),
            cleanups_before,
            "the failed cleanup transaction must not be counted as committed"
        );
        assert_eq!(pending_external_index_job_count(&db), 10);
        assert_eq!(
            db.fts_search("articles", &index.name, "cleanup", 100)
                .unwrap()
                .len(),
            10,
            "the successful external commit must remain immediately visible"
        );

        db.replay_external_index_jobs().unwrap();

        assert_eq!(
            db.external_index_apply_commit_count_for_test() - commits_before,
            2,
            "replay must use one additional external-index commit"
        );
        assert_eq!(
            db.external_index_job_cleanup_commit_count_for_test() - cleanups_before,
            1,
            "replay must remove all jobs in one cleanup transaction"
        );
        assert_eq!(pending_external_index_job_count(&db), 0);
        assert_eq!(
            db.fts_search("articles", &index.name, "cleanup", 100)
                .unwrap()
                .len(),
            10,
            "idempotent replay must not duplicate FTS documents"
        );
    }

    #[cfg(feature = "fts")]
    #[test]
    fn single_document_external_index_cleanup_uses_one_transaction() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();
        let index = init_test_fts_index(&db, "articles", "body");
        let commits_before = db.external_index_apply_commit_count_for_test();
        let cleanups_before = db.external_index_job_cleanup_commit_count_for_test();

        db.set_document(&test_text_doc(
            "articles",
            "single",
            "single operation token",
        ))
        .unwrap();

        assert_eq!(
            db.external_index_apply_commit_count_for_test() - commits_before,
            1
        );
        assert_eq!(
            db.external_index_job_cleanup_commit_count_for_test() - cleanups_before,
            1
        );
        assert_eq!(pending_external_index_job_count(&db), 0);
        assert_eq!(
            db.fts_search("articles", &index.name, "single", 10)
                .unwrap()
                .len(),
            1
        );
    }

    #[cfg(feature = "fts")]
    #[test]
    fn replay_multiple_jobs_uses_one_cleanup_transaction() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();
        let index = init_test_fts_index(&db, "articles", "body");

        for i in 0..10 {
            let doc = test_text_doc("articles", &i.to_string(), "multi replay token");
            db.enqueue_external_index_job_for_test("articles", doc.key(), None, Some(&doc))
                .unwrap();
        }
        let commits_before = db.external_index_apply_commit_count_for_test();
        let cleanups_before = db.external_index_job_cleanup_commit_count_for_test();

        db.replay_external_index_jobs().unwrap();

        assert_eq!(
            db.external_index_apply_commit_count_for_test() - commits_before,
            1
        );
        assert_eq!(
            db.external_index_job_cleanup_commit_count_for_test() - cleanups_before,
            1
        );
        assert_eq!(pending_external_index_job_count(&db), 0);
        assert_eq!(
            db.fts_search("articles", &index.name, "replay", 100)
                .unwrap()
                .len(),
            10
        );
    }

    #[test]
    fn atomic_batch_without_external_indexes_skips_external_commit() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();
        db.get_or_create_collection("plain").unwrap();
        let commits_before = db.external_index_apply_commit_count_for_test();
        let cleanups_before = db.external_index_job_cleanup_commit_count_for_test();

        db.atomic_write_batch(
            (0..20)
                .map(|i| {
                    WriteOp::InsertDoc(Document {
                        id: format!("plain:{i}"),
                        fields: HashMap::new(),
                    })
                })
                .collect(),
        )
        .unwrap();

        assert_eq!(
            db.external_index_apply_commit_count_for_test(),
            commits_before
        );
        assert_eq!(
            db.external_index_job_cleanup_commit_count_for_test(),
            cleanups_before
        );
        assert_eq!(db.count_documents("plain").unwrap(), 20);
    }

    #[cfg(feature = "fts")]
    #[test]
    fn atomic_mixed_batch_across_collections_commits_fulltext_once() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();
        let articles_index = init_test_fts_index(&db, "articles", "body");
        let notes_index = init_test_fts_index(&db, "notes", "body");

        db.atomic_write_batch(vec![
            WriteOp::InsertDoc(test_text_doc("articles", "update", "stale article term")),
            WriteOp::InsertDoc(test_text_doc(
                "articles",
                "delete",
                "removable article term",
            )),
        ])
        .unwrap();
        let commits_before = db.external_index_apply_commit_count_for_test();
        let cleanups_before = db.external_index_job_cleanup_commit_count_for_test();

        db.atomic_write_batch(vec![
            WriteOp::InsertDoc(test_text_doc("articles", "update", "fresh article term")),
            WriteOp::DeleteDoc {
                collection: "articles".to_string(),
                key: "delete".to_string(),
            },
            WriteOp::InsertDoc(test_text_doc("notes", "insert", "brandnew note term")),
        ])
        .unwrap();

        assert_eq!(
            db.external_index_apply_commit_count_for_test() - commits_before,
            1
        );
        assert_eq!(
            db.external_index_job_cleanup_commit_count_for_test() - cleanups_before,
            1
        );
        assert!(
            db.fts_search("articles", &articles_index.name, "stale", 10)
                .unwrap()
                .is_empty()
        );
        assert!(
            db.fts_search("articles", &articles_index.name, "removable", 10)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            db.fts_search("articles", &articles_index.name, "fresh", 10)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            db.fts_search("notes", &notes_index.name, "brandnew", 10)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(pending_external_index_job_count(&db), 0);
    }

    #[cfg(feature = "hnsw")]
    #[test]
    fn atomic_batch_commits_hnsw_once_and_persists_id_map() {
        let tmp = TempDir::new().unwrap();

        {
            let db = Database::open(tmp.path()).unwrap();
            db.get_or_create_collection("items").unwrap();
            let params = HnswParams::new(3);
            let index = IndexDef::hnsw("items", vec!["embedding".to_string()], params.clone());
            db.init_hnsw_index("items", "embedding", &params).unwrap();
            db.build_index("items", &index).unwrap();
            db.index_registry().register("items", index.clone());
            db.add_index_to_schema("items", index).unwrap();
            let commits_before = db.external_index_apply_commit_count_for_test();
            let cleanups_before = db.external_index_job_cleanup_commit_count_for_test();

            db.atomic_write_batch(
                (0..20)
                    .map(|i| {
                        WriteOp::InsertDoc(Document {
                            id: format!("items:{i}"),
                            fields: [(
                                "embedding".to_string(),
                                Value::Array(vec![
                                    Value::Float(1.0),
                                    Value::Float(i as f64 / 100.0),
                                    Value::Float(0.0),
                                ]),
                            )]
                            .into_iter()
                            .collect(),
                        })
                    })
                    .collect(),
            )
            .unwrap();

            assert_eq!(
                db.external_index_apply_commit_count_for_test() - commits_before,
                1
            );
            assert_eq!(
                db.external_index_job_cleanup_commit_count_for_test() - cleanups_before,
                1
            );
            assert_eq!(pending_external_index_job_count(&db), 0);
        }

        let reopened = Database::open(tmp.path()).unwrap();
        let results = reopened
            .vector_search("items", "embedding", &[1.0, 0.0, 0.0], 20, None)
            .unwrap();
        assert_eq!(results.len(), 20, "the committed HNSW id-map must reopen");
    }

    #[cfg(feature = "fts")]
    #[test]
    #[ignore = "manual performance regression benchmark"]
    fn atomic_batch_fulltext_10000_documents_benchmark() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();
        let index = init_test_fts_index(&db, "bench", "body");
        let commits_before = db.external_index_apply_commit_count_for_test();
        let cleanups_before = db.external_index_job_cleanup_commit_count_for_test();
        let started = std::time::Instant::now();

        db.atomic_write_batch(
            (0..10_000)
                .map(|i| {
                    WriteOp::InsertDoc(test_text_doc(
                        "bench",
                        &i.to_string(),
                        format!("benchmark searchable document {i}"),
                    ))
                })
                .collect(),
        )
        .unwrap();

        eprintln!(
            "indexed 10,000 FULLTEXT documents in {:?}",
            started.elapsed()
        );
        assert_eq!(
            db.external_index_apply_commit_count_for_test() - commits_before,
            1
        );
        assert_eq!(
            db.external_index_job_cleanup_commit_count_for_test() - cleanups_before,
            1
        );
        assert_eq!(
            db.fts_search("bench", &index.name, "searchable", 10_000)
                .unwrap()
                .len(),
            10_000
        );
    }

    #[cfg(feature = "fts")]
    #[test]
    fn replay_keeps_external_index_job_when_external_commit_fails() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();
        db.get_or_create_collection("articles").unwrap();

        let index = IndexDef::fulltext("articles", vec!["body".to_string()]);
        db.init_fts_index("articles", &index.name, &index.fields, None)
            .unwrap();
        db.build_index("articles", &index).unwrap();
        db.index_registry().register("articles", index.clone());
        db.add_index_to_schema("articles", index).unwrap();

        let mut fields = HashMap::new();
        fields.insert(
            "body".to_string(),
            Value::String("durable replay token".to_string()),
        );
        let doc = Document {
            id: "articles:a1".to_string(),
            fields,
        };

        let keyspaces = db.keyspaces.get("articles").unwrap();
        let mut tx = db.db.write_tx().unwrap();
        let bytes = rkyv::to_bytes::<rancor::Error>(&doc).unwrap();
        tx.insert(&keyspaces.docs, doc.key().as_bytes(), bytes.as_slice());
        tx.insert(&keyspaces.exists, doc.key().as_bytes(), []);
        db.write_external_index_job(&mut tx, "articles", doc.key(), None, Some(&doc))
            .unwrap();
        tx.commit().unwrap().unwrap();

        let job_key = Database::external_index_job_key("articles", doc.key());
        assert!(db.index_jobs.get(job_key.as_bytes()).unwrap().is_some());

        db.fail_next_external_index_replay_commit_for_test();
        let err = db.replay_external_index_jobs().unwrap_err();

        assert!(
            err.to_string()
                .contains("forced external index commit failure"),
            "unexpected error: {err}"
        );
        assert!(
            db.index_jobs.get(job_key.as_bytes()).unwrap().is_some(),
            "pending external index job must remain durable until external indexes commit"
        );
    }

    #[cfg(feature = "fts")]
    #[test]
    fn set_document_does_not_return_error_after_primary_commit_when_external_commit_fails() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();
        db.get_or_create_collection("articles").unwrap();

        let index = IndexDef::fulltext("articles", vec!["body".to_string()]);
        db.init_fts_index("articles", &index.name, &index.fields, None)
            .unwrap();
        db.build_index("articles", &index).unwrap();
        db.index_registry().register("articles", index.clone());
        db.add_index_to_schema("articles", index).unwrap();

        let doc = Document {
            id: "articles:false_negative".to_string(),
            fields: [(
                "body".to_string(),
                Value::String("primary commit survived".to_string()),
            )]
            .into_iter()
            .collect(),
        };

        db.fail_next_external_index_apply_commit_for_test();
        db.set_document(&doc)
            .expect("post-commit external index failure must not make the write look failed");

        assert!(
            db.get_document("articles", "false_negative")
                .unwrap()
                .is_some(),
            "primary document should be committed"
        );
        assert!(
            db.index_jobs
                .get(Database::external_index_job_key("articles", doc.key()).as_bytes())
                .unwrap()
                .is_some(),
            "durable external index job should remain for replay"
        );
    }

    #[test]
    fn open_rejects_newer_storage_format_version() {
        let tmp = TempDir::new().unwrap();

        {
            let db = Database::open(tmp.path()).unwrap();
            db.sync().unwrap();
        }

        {
            let storage =
                fjall::OptimisticTxDatabase::builder(tmp.path().join(crate::storage::DIR_STORAGE))
                    .open()
                    .unwrap();
            let meta = storage
                .keyspace("_meta", KeyspaceCreateOptions::default)
                .unwrap();
            meta.insert(STORAGE_FORMAT_VERSION_KEY, b"9999").unwrap();
        }

        let err = match Database::open(tmp.path()) {
            Ok(_) => panic!("newer storage format version should be rejected"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("storage format version"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn atomic_write_batch_rolls_back_doc_insert_when_edge_delete_fails() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();

        db.get_or_create_collection("items").unwrap();
        db.create_edge(&Edge::new(
            "users:alice".to_string(),
            "follows".to_string(),
            "users:bob".to_string(),
        ))
        .unwrap();

        let edge_keyspace = db
            .db
            .keyspace("_edge:follows", KeyspaceCreateOptions::default)
            .unwrap();
        edge_keyspace
            .insert(
                encode_edge_out_key_single("users:alice", "users:bob"),
                b"not valid edge data",
            )
            .unwrap();

        let doc = Document {
            id: "items:committed_before_edge_failure".to_string(),
            fields: std::collections::HashMap::new(),
        };

        let result = db.atomic_write_batch(vec![
            WriteOp::InsertDoc(doc),
            WriteOp::DeleteEdge {
                from: "users:alice".to_string(),
                label: "follows".to_string(),
                to: "users:bob".to_string(),
            },
        ]);

        assert!(
            result.is_err(),
            "Corrupted edge data should make edge delete fail"
        );
        assert!(
            db.get_document("items", "committed_before_edge_failure")
                .unwrap()
                .is_none(),
            "Document insert must not survive a failed atomic batch"
        );
    }

    #[test]
    fn delete_document_rolls_back_unique_release_when_edge_cleanup_fails() {
        let tmp = TempDir::new().unwrap();
        let db = std::sync::Arc::new(Database::open(tmp.path()).unwrap());

        crate::run_sql!(db.clone(), "DEFINE COLLECTION users");
        crate::run_sql!(db.clone(), "CREATE INDEX ON users(email) UNIQUE");
        crate::run_sql!(
            db.clone(),
            r#"INSERT INTO users {id: "alice", email: "alice@test.com"}"#
        );
        crate::run_sql!(db.clone(), r#"INSERT INTO users {id: "bob"}"#);

        db.create_edge(&Edge::new(
            "users:alice".to_string(),
            "follows".to_string(),
            "users:bob".to_string(),
        ))
        .unwrap();

        let edge_keyspace = db
            .db
            .keyspace("_edge:follows", KeyspaceCreateOptions::default)
            .unwrap();
        edge_keyspace
            .insert(
                encode_edge_out_key_single("users:alice", "users:bob"),
                b"not valid edge data",
            )
            .unwrap();

        let err = db.delete_document("users", "alice").unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("serialization"),
            "corrupted edge data should make delete fail after UNIQUE release, got: {err}"
        );
        assert!(
            db.get_document("users", "alice").unwrap().is_some(),
            "primary document should remain after failed delete"
        );

        let duplicate = crate::try_run_sql!(
            db.clone(),
            r#"INSERT INTO users {id: "carol", email: "alice@test.com"}"#
        );
        assert!(
            duplicate.is_err(),
            "failed delete must restore alice's UNIQUE claim"
        );
    }

    #[test]
    fn delete_document_rolls_back_unique_release_when_commit_fails() {
        let tmp = TempDir::new().unwrap();
        let db = std::sync::Arc::new(Database::open(tmp.path()).unwrap());

        crate::run_sql!(db.clone(), "DEFINE COLLECTION users");
        crate::run_sql!(db.clone(), "CREATE INDEX ON users(email) UNIQUE");
        crate::run_sql!(
            db.clone(),
            r#"INSERT INTO users {id: "alice", email: "alice@test.com"}"#
        );

        db.fail_next_delete_document_commit_for_test();
        let err = db.delete_document("users", "alice").unwrap_err();
        assert!(
            err.to_string()
                .contains("forced delete document commit failure"),
            "unexpected delete error: {err}"
        );
        assert!(
            db.get_document("users", "alice").unwrap().is_some(),
            "primary document should remain after failed delete commit"
        );

        let duplicate = crate::try_run_sql!(
            db.clone(),
            r#"INSERT INTO users {id: "bob", email: "alice@test.com"}"#
        );
        assert!(
            duplicate.is_err(),
            "failed delete commit must restore alice's UNIQUE claim"
        );
    }

    #[test]
    fn update_document_rolls_back_unique_change_when_commit_fails() {
        let tmp = TempDir::new().unwrap();
        let db = std::sync::Arc::new(Database::open(tmp.path()).unwrap());

        crate::run_sql!(db.clone(), "DEFINE COLLECTION users");
        crate::run_sql!(db.clone(), "CREATE INDEX ON users(email) UNIQUE");
        crate::run_sql!(
            db.clone(),
            r#"INSERT INTO users {id: "alice", email: "old@test.com"}"#
        );

        let updated = Document {
            id: "users:alice".to_string(),
            fields: [(
                "email".to_string(),
                Value::String("new@test.com".to_string()),
            )]
            .into_iter()
            .collect(),
        };

        db.fail_next_update_commit_for_test();
        let err = db.update_document(&updated).unwrap_err();
        assert!(
            err.to_string().contains("forced update commit failure"),
            "unexpected update error: {err}"
        );

        assert_eq!(
            db.get_document("users", "alice")
                .unwrap()
                .unwrap()
                .fields
                .get("email"),
            Some(&Value::String("old@test.com".to_string())),
            "primary document should remain unchanged after failed commit"
        );

        let old_duplicate = crate::try_run_sql!(
            db.clone(),
            r#"INSERT INTO users {id: "old_dupe", email: "old@test.com"}"#
        );
        assert!(
            old_duplicate.is_err(),
            "failed update must restore the old UNIQUE claim"
        );

        crate::run_sql!(
            db.clone(),
            r#"INSERT INTO users {id: "new_holder", email: "new@test.com"}"#
        );
    }

    #[test]
    fn set_document_rolls_back_unique_change_when_commit_fails() {
        let tmp = TempDir::new().unwrap();
        let db = std::sync::Arc::new(Database::open(tmp.path()).unwrap());

        crate::run_sql!(db.clone(), "DEFINE COLLECTION users");
        crate::run_sql!(db.clone(), "CREATE INDEX ON users(email) UNIQUE");
        crate::run_sql!(
            db.clone(),
            r#"INSERT INTO users {id: "alice", email: "old@test.com"}"#
        );

        let replacement = Document {
            id: "users:alice".to_string(),
            fields: [(
                "email".to_string(),
                Value::String("new@test.com".to_string()),
            )]
            .into_iter()
            .collect(),
        };

        db.fail_next_set_document_commit_for_test();
        let err = db.set_document(&replacement).unwrap_err();
        assert!(
            err.to_string()
                .contains("forced set document commit failure"),
            "unexpected set error: {err}"
        );

        assert_eq!(
            db.get_document("users", "alice")
                .unwrap()
                .unwrap()
                .fields
                .get("email"),
            Some(&Value::String("old@test.com".to_string())),
            "primary document should remain unchanged after failed set commit"
        );

        let old_duplicate = crate::try_run_sql!(
            db.clone(),
            r#"INSERT INTO users {id: "old_dupe", email: "old@test.com"}"#
        );
        assert!(
            old_duplicate.is_err(),
            "failed set must restore the old UNIQUE claim"
        );

        crate::run_sql!(
            db.clone(),
            r#"INSERT INTO users {id: "new_holder", email: "new@test.com"}"#
        );
    }

    #[test]
    fn create_document_rolls_back_unique_claim_when_commit_fails() {
        let tmp = TempDir::new().unwrap();
        let db = std::sync::Arc::new(Database::open(tmp.path()).unwrap());

        crate::run_sql!(db.clone(), "DEFINE COLLECTION users");
        crate::run_sql!(db.clone(), "CREATE INDEX ON users(email) UNIQUE");

        let doc = Document {
            id: "users:alice".to_string(),
            fields: [(
                "email".to_string(),
                Value::String("alice@test.com".to_string()),
            )]
            .into_iter()
            .collect(),
        };

        db.fail_next_create_document_commit_for_test();
        let err = db.create_document(&doc).unwrap_err();
        assert!(
            err.to_string()
                .contains("forced create document commit failure"),
            "unexpected create error: {err}"
        );
        assert!(
            db.get_document("users", "alice").unwrap().is_none(),
            "primary document should not exist after failed commit"
        );

        crate::run_sql!(
            db.clone(),
            r#"INSERT INTO users {id: "bob", email: "alice@test.com"}"#
        );
    }

    #[test]
    fn create_edge_validated_does_not_create_edge_when_commit_fails() {
        let tmp = TempDir::new().unwrap();
        let db = std::sync::Arc::new(Database::open(tmp.path()).unwrap());

        crate::run_sql!(db.clone(), "DEFINE COLLECTION users");
        crate::run_sql!(db.clone(), r#"INSERT INTO users {id: "alice"}"#);
        crate::run_sql!(db.clone(), r#"INSERT INTO users {id: "bob"}"#);
        db.edges
            .define_edge_type(EdgeSchema::new("follows"))
            .unwrap();

        db.fail_next_create_edge_commit_for_test();
        let err = db
            .create_edge_validated(&Edge::new(
                "users:alice".to_string(),
                "follows".to_string(),
                "users:bob".to_string(),
            ))
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("forced create edge commit failure"),
            "unexpected create edge error: {err}"
        );

        assert!(
            db.get_edge("users:alice", "follows", "users:bob")
                .unwrap()
                .is_none(),
            "edge should not be visible after failed commit"
        );
    }

    #[cfg(feature = "fts")]
    #[test]
    fn open_replays_pending_external_index_jobs() {
        let tmp = TempDir::new().unwrap();

        {
            let db = Database::open(tmp.path()).unwrap();
            crate::run_sql!(std::sync::Arc::new(db), "DEFINE COLLECTION articles");
        }

        {
            let db = Database::open(tmp.path()).unwrap();
            crate::run_sql!(
                std::sync::Arc::new(db),
                "CREATE INDEX ON articles(title) FULLTEXT"
            );
        }

        let doc = Document {
            id: "articles:replay".to_string(),
            fields: [(
                "title".to_string(),
                Value::String("Recoverable Rust".to_string()),
            )]
            .into_iter()
            .collect(),
        };

        {
            let db = Database::open(tmp.path()).unwrap();
            let keyspaces = db.keyspaces.get("articles").unwrap();
            let jobs = db
                .db
                .keyspace("_index_jobs", KeyspaceCreateOptions::default)
                .unwrap();
            let mut tx = db.db.write_tx().unwrap();
            let bytes = rkyv::to_bytes::<rancor::Error>(&doc).unwrap();
            tx.insert(&keyspaces.docs, doc.key().as_bytes(), bytes.as_slice());
            tx.insert(&keyspaces.exists, doc.key().as_bytes(), []);
            let job = json!({
                "collection": "articles",
                "key": "replay",
                "old_doc": null,
                "new_doc": doc
            });
            tx.insert(&jobs, b"manual-pending-job", job.to_string().as_bytes());
            tx.commit().unwrap().unwrap();
        }

        let reopened = std::sync::Arc::new(Database::open(tmp.path()).unwrap());
        let result = crate::query_json(
            reopened,
            r#"SELECT title FROM articles WHERE title @@ "recoverable""#,
        );
        assert_eq!(result.as_array().unwrap().len(), 1);
    }

    #[cfg(feature = "fts")]
    #[test]
    fn external_index_job_replay_is_idempotent_after_apply_before_complete_crash() {
        let tmp = TempDir::new().unwrap();

        {
            let db = std::sync::Arc::new(Database::open(tmp.path()).unwrap());
            crate::run_sql!(db.clone(), "DEFINE COLLECTION articles");
            crate::run_sql!(db.clone(), "CREATE INDEX ON articles(title) FULLTEXT");
            crate::run_sql!(
                db,
                r#"INSERT INTO articles {id: "dupe", title: "Idempotent Rust"}"#
            );
        }

        let doc = Document {
            id: "articles:dupe".to_string(),
            fields: [(
                "title".to_string(),
                Value::String("Idempotent Rust".to_string()),
            )]
            .into_iter()
            .collect(),
        };

        {
            let db = Database::open(tmp.path()).unwrap();
            let jobs = db
                .db
                .keyspace(INDEX_JOBS_KEYSPACE, KeyspaceCreateOptions::default)
                .unwrap();
            let job = json!({
                "collection": "articles",
                "key": "dupe",
                "old_doc": null,
                "new_doc": doc
            });
            jobs.insert(b"manual-duplicate-replay-job", job.to_string().as_bytes())
                .unwrap();
        }

        let reopened = std::sync::Arc::new(Database::open(tmp.path()).unwrap());
        let result = crate::query_json(
            reopened,
            r#"SELECT title FROM articles WHERE title @@ "idempotent""#,
        );
        assert_eq!(
            result.as_array().unwrap().len(),
            1,
            "Replaying an already-applied index job must not duplicate FTS hits"
        );
    }

    #[cfg(feature = "hnsw")]
    #[test]
    fn open_replays_pending_hnsw_external_index_jobs() {
        let tmp = TempDir::new().unwrap();

        {
            let db = Database::open(tmp.path()).unwrap();
            db.get_or_create_collection("items").unwrap();

            let params = HnswParams::new(3);
            let index = IndexDef::hnsw("items", vec!["embedding".to_string()], params.clone());
            db.init_hnsw_index("items", "embedding", &params).unwrap();
            db.build_index("items", &index).unwrap();
            db.index_registry().register("items", index.clone());
            db.add_index_to_schema("items", index).unwrap();
        }

        let doc = Document {
            id: "items:vector_replay".to_string(),
            fields: [(
                "embedding".to_string(),
                Value::Array(vec![
                    Value::Float(1.0),
                    Value::Float(0.0),
                    Value::Float(0.0),
                ]),
            )]
            .into_iter()
            .collect(),
        };

        {
            let db = Database::open(tmp.path()).unwrap();
            let keyspaces = db.keyspaces.get("items").unwrap();
            let mut tx = db.db.write_tx().unwrap();
            let bytes = rkyv::to_bytes::<rancor::Error>(&doc).unwrap();
            tx.insert(&keyspaces.docs, doc.key().as_bytes(), bytes.as_slice());
            tx.insert(&keyspaces.exists, doc.key().as_bytes(), []);
            tx.commit().unwrap().unwrap();

            db.enqueue_external_index_job_for_test("items", doc.key(), None, Some(&doc))
                .unwrap();
        }

        let reopened = Database::open(tmp.path()).unwrap();
        let results = reopened
            .vector_search("items", "embedding", &[1.0, 0.0, 0.0], 1, None)
            .unwrap();

        assert_eq!(
            results.first().map(|(id, _)| id.as_str()),
            Some("items:vector_replay"),
            "Pending HNSW external index job should be replayed on startup"
        );
    }

    #[cfg(feature = "hnsw")]
    #[test]
    fn hnsw_external_index_job_replay_is_idempotent_after_apply_before_complete_crash() {
        let tmp = TempDir::new().unwrap();

        {
            let db = std::sync::Arc::new(Database::open(tmp.path()).unwrap());
            crate::run_sql!(db.clone(), "DEFINE COLLECTION items");
            crate::run_sql!(
                db.clone(),
                "CREATE INDEX ON items(embedding) HNSW DIMENSION 3 DIST COSINE"
            );
            crate::run_sql!(
                db,
                r#"INSERT INTO items {id: "dupe", embedding: [1.0, 0.0, 0.0]}"#
            );
        }

        let doc = Document {
            id: "items:dupe".to_string(),
            fields: [(
                "embedding".to_string(),
                Value::Array(vec![
                    Value::Float(1.0),
                    Value::Float(0.0),
                    Value::Float(0.0),
                ]),
            )]
            .into_iter()
            .collect(),
        };

        {
            let db = Database::open(tmp.path()).unwrap();
            db.enqueue_external_index_job_for_test("items", doc.key(), None, Some(&doc))
                .unwrap();
        }

        let reopened = Database::open(tmp.path()).unwrap();
        let results = reopened
            .vector_search("items", "embedding", &[1.0, 0.0, 0.0], 10, None)
            .unwrap();
        let duplicate_hits = results
            .iter()
            .filter(|(id, _)| id.as_str() == "items:dupe")
            .count();

        assert_eq!(
            duplicate_hits, 1,
            "Replaying an already-applied HNSW job must not duplicate vector hits: {results:?}"
        );
    }
}
