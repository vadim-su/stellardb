//! Full-Text Search index backend using Tantivy
//!
//! This module provides full-text search capabilities with BM25 ranking
//! via the Tantivy search engine library.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use fjall::OptimisticTxKeyspace;
use parking_lot::{Mutex, MutexGuard, RwLock};
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, OwnedValue, STORED, STRING, Schema, TextFieldIndexing, TextOptions};
use tantivy::tokenizer::TokenizerManager;
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term};

use crate::document::{Document, Value};
use crate::error::StorageError;

use super::super::{DIR_FTS, DIR_INDEXES};
use super::traits::{IndexBackend, IndexType, RangeOp, ScanResult};

/// Number of documents to batch before auto-commit
const AUTO_COMMIT_THRESHOLD: usize = 1000;

/// Interval between automatic commits in milliseconds (visibility delay)
const COMMIT_INTERVAL_MS: u64 = 100;

/// Commands for the background commit thread
enum CommitCommand {
    /// Trigger immediate commit and reply when done
    Flush(std::sync::mpsc::Sender<Result<(), StorageError>>),
    /// Signal that threshold was reached (non-blocking)
    ThresholdReached,
    /// Graceful shutdown
    Shutdown,
}

/// Spawn the background commit thread
fn spawn_commit_thread(
    instances: Arc<RwLock<HashMap<(String, String), FtsIndexState>>>,
    commit_gate: Arc<Mutex<()>>,
    rx: std::sync::mpsc::Receiver<CommitCommand>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("fts-commit".into())
        .spawn(move || {
            use std::sync::mpsc::RecvTimeoutError;
            let timeout = std::time::Duration::from_millis(COMMIT_INTERVAL_MS);

            loop {
                match rx.recv_timeout(timeout) {
                    Ok(CommitCommand::Flush(reply)) => {
                        let _commit_guard = commit_gate.lock();
                        let result = commit_all_instances(&instances);
                        let _ = reply.send(result);
                    }
                    Ok(CommitCommand::ThresholdReached) => {
                        let _commit_guard = commit_gate.lock();
                        #[cfg(feature = "fault-injection")]
                        if has_pending_instances(&instances)
                            && crate::fault_injection::check("fts_background_commit").is_err()
                        {
                            continue;
                        }
                        let _ = commit_all_instances(&instances);
                    }
                    Ok(CommitCommand::Shutdown) => {
                        let _commit_guard = commit_gate.lock();
                        let _ = commit_all_instances(&instances);
                        break;
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        let _commit_guard = commit_gate.lock();
                        #[cfg(feature = "fault-injection")]
                        if has_pending_instances(&instances)
                            && crate::fault_injection::check("fts_background_commit").is_err()
                        {
                            continue;
                        }
                        let _ = commit_all_instances(&instances);
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        break;
                    }
                }
            }
        })
        .expect("failed to spawn fts-commit thread")
}
#[cfg(feature = "fault-injection")]
fn has_pending_instances(instances: &RwLock<HashMap<(String, String), FtsIndexState>>) -> bool {
    instances.read().values().any(|state| match state {
        FtsIndexState::Active(index) => {
            index
                .pending_docs
                .load(std::sync::atomic::Ordering::Relaxed)
                > 0
        }
        FtsIndexState::Rebuilding { active, shadow, .. } => {
            active
                .pending_docs
                .load(std::sync::atomic::Ordering::Relaxed)
                > 0
                || shadow
                    .pending_docs
                    .load(std::sync::atomic::Ordering::Relaxed)
                    > 0
        }
    })
}

/// Commit all active FTS index instances and reload readers for immediate visibility
fn commit_all_instances(
    instances: &RwLock<HashMap<(String, String), FtsIndexState>>,
) -> Result<(), StorageError> {
    let instances = instances.read();
    for state in instances.values() {
        match state {
            FtsIndexState::Active(idx) => {
                let pending = idx.pending_docs.load(std::sync::atomic::Ordering::Relaxed);
                if pending > 0 {
                    // force_commit() commits the writer and reloads the reader
                    idx.force_commit()?;
                }
            }
            FtsIndexState::Rebuilding { active, shadow, .. } => {
                if active
                    .pending_docs
                    .load(std::sync::atomic::Ordering::Relaxed)
                    > 0
                {
                    active.force_commit()?;
                }
                if shadow
                    .pending_docs
                    .load(std::sync::atomic::Ordering::Relaxed)
                    > 0
                {
                    shadow.force_commit()?;
                }
            }
        }
    }
    Ok(())
}

/// Default analyzer name when none is specified
const DEFAULT_ANALYZER: &str = "standard";

/// Create a tokenizer manager with the specified analyzer
fn create_tokenizer_manager(
    analyzer_name: &str,
    registry: &super::analyzers::AnalyzerRegistry,
) -> Result<TokenizerManager, StorageError> {
    let analyzer = registry
        .get(analyzer_name)
        .ok_or_else(|| StorageError::Index(format!("Unknown analyzer: {}", analyzer_name)))?;

    let manager = TokenizerManager::default();
    manager.register(analyzer_name, analyzer);
    Ok(manager)
}

/// Represents a single Tantivy index instance for a collection/index pair
struct FtsIndexInstance {
    /// The Tantivy index
    index: Index,
    /// Reader for searching (refreshed on commit)
    reader: IndexReader,
    /// Writer for indexing (requires synchronization)
    writer: RwLock<IndexWriter>,
    /// Field storing the document ID
    doc_id_field: Field,
    /// Fields for text content (field_name -> tantivy Field)
    content_fields: HashMap<String, Field>,
    /// Counter for pending documents (for auto-commit)
    pending_docs: std::sync::atomic::AtomicUsize,
    /// Analyzer name used for this index
    analyzer_name: String,
    /// Channel to notify background thread (for threshold notifications)
    commit_tx: std::sync::mpsc::Sender<CommitCommand>,
}

impl FtsIndexInstance {
    /// Create a new FTS index instance
    fn new(
        index_path: &PathBuf,
        fields: &[String],
        analyzer_name: &str,
        registry: &super::analyzers::AnalyzerRegistry,
        commit_tx: std::sync::mpsc::Sender<CommitCommand>,
    ) -> Result<Self, StorageError> {
        // Build Tantivy schema
        let mut schema_builder = Schema::builder();

        // Document ID field - stored and indexed as STRING (not tokenized) for exact match deletion
        let doc_id_field = schema_builder.add_text_field("_doc_id", STRING | STORED);

        // Text field options with the specified analyzer
        let text_options = TextOptions::default()
            .set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer(analyzer_name)
                    .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions),
            )
            .set_stored();

        // Add text fields for indexing with the analyzer
        let mut content_fields = HashMap::new();
        for field_name in fields {
            let field = schema_builder.add_text_field(field_name, text_options.clone());
            content_fields.insert(field_name.clone(), field);
        }

        let schema = schema_builder.build();

        // Create index directory
        std::fs::create_dir_all(index_path).map_err(|e| {
            StorageError::Io(format!("Failed to create FTS index directory: {}", e))
        })?;

        // Try to open existing index or create new one
        let mut index = Index::open_in_dir(index_path)
            .or_else(|_| Index::create_in_dir(index_path, schema.clone()))
            .map_err(|e| StorageError::Index(format!("Failed to open/create FTS index: {}", e)))?;

        // Register the analyzer with the index
        index.set_tokenizers(create_tokenizer_manager(analyzer_name, registry)?);

        // Create reader with automatic reload on commits
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| StorageError::Index(format!("Failed to create FTS reader: {}", e)))?;

        // Create writer with 50MB heap
        let writer = index
            .writer(50_000_000)
            .map_err(|e| StorageError::Index(format!("Failed to create FTS writer: {}", e)))?;

        Ok(Self {
            index,
            reader,
            writer: RwLock::new(writer),
            doc_id_field,
            content_fields,
            pending_docs: std::sync::atomic::AtomicUsize::new(0),
            analyzer_name: analyzer_name.to_string(),
            commit_tx,
        })
    }

    /// Open an existing FTS index instance from disk
    fn open(
        index_path: &PathBuf,
        analyzer_name: &str,
        registry: &super::analyzers::AnalyzerRegistry,
        commit_tx: std::sync::mpsc::Sender<CommitCommand>,
    ) -> Result<Self, StorageError> {
        let mut index = Index::open_in_dir(index_path)
            .map_err(|e| StorageError::Index(format!("Failed to open FTS index: {}", e)))?;

        // Register the analyzer with the index
        index.set_tokenizers(create_tokenizer_manager(analyzer_name, registry)?);

        let schema = index.schema();

        // Get doc_id field
        let doc_id_field = schema
            .get_field("_doc_id")
            .map_err(|_| StorageError::Index("FTS index missing _doc_id field".into()))?;

        // Collect all content fields (all fields except _doc_id)
        let mut content_fields = HashMap::new();
        for (field, entry) in schema.fields() {
            let name = entry.name();
            if name != "_doc_id" {
                content_fields.insert(name.to_string(), field);
            }
        }

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| StorageError::Index(format!("Failed to create FTS reader: {}", e)))?;

        let writer = index
            .writer(50_000_000)
            .map_err(|e| StorageError::Index(format!("Failed to create FTS writer: {}", e)))?;

        Ok(Self {
            index,
            reader,
            writer: RwLock::new(writer),
            doc_id_field,
            content_fields,
            pending_docs: std::sync::atomic::AtomicUsize::new(0),
            analyzer_name: analyzer_name.to_string(),
            commit_tx,
        })
    }

    /// Index a document with auto-commit after threshold
    fn index_doc(&self, doc: &Document, fields: &[String]) -> Result<(), StorageError> {
        let mut tantivy_doc = TantivyDocument::new();

        // Add document ID
        tantivy_doc.add_text(self.doc_id_field, &doc.id);

        // Add text content from specified fields
        for field_name in fields {
            if let Some(&tantivy_field) = self.content_fields.get(field_name)
                && let Some(value) = doc.fields.get(field_name)
            {
                let text = value_to_text(value);
                if !text.is_empty() {
                    tantivy_doc.add_text(tantivy_field, &text);
                }
            }
        }

        let writer = self.writer.write();
        writer.add_document(tantivy_doc).map_err(|e| {
            StorageError::Index(format!("Failed to add document to FTS index: {}", e))
        })?;
        drop(writer);

        // Notify background thread if threshold reached (non-blocking)
        let pending = self
            .pending_docs
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        if pending >= AUTO_COMMIT_THRESHOLD {
            // Non-blocking notification - background thread will commit
            let _ = self.commit_tx.send(CommitCommand::ThresholdReached);
        }

        Ok(())
    }

    /// Commit any pending changes to the index
    fn commit(&self) -> Result<(), StorageError> {
        let pending = self.pending_docs.load(std::sync::atomic::Ordering::Relaxed);
        if pending > 0 {
            self.force_commit()?;
        }
        Ok(())
    }

    /// Force a commit regardless of pending count (for reindex finish)
    fn force_commit(&self) -> Result<(), StorageError> {
        let mut writer = self.writer.write();
        #[cfg(feature = "fault-injection")]
        crate::fault_injection::check("tantivy_commit")
            .map_err(|error| StorageError::Index(error.to_string()))?;
        writer
            .commit()
            .map_err(|e| StorageError::Index(format!("Failed to commit FTS index: {}", e)))?;
        self.pending_docs
            .store(0, std::sync::atomic::Ordering::SeqCst);
        // Reload reader for immediate visibility
        self.reader
            .reload()
            .map_err(|e| StorageError::Index(format!("Failed to reload FTS reader: {}", e)))?;
        Ok(())
    }

    /// Stop all indexing and merge workers before moving or deleting the index directory.
    fn wait_merging_threads(self) -> Result<(), StorageError> {
        self.writer
            .into_inner()
            .wait_merging_threads()
            .map_err(|e| StorageError::Index(format!("Failed to stop FTS merge workers: {}", e)))
    }

    /// Delete a document from the index by ID (with auto-commit)
    fn delete_doc(&self, doc_id: &str) -> Result<(), StorageError> {
        let term = Term::from_field_text(self.doc_id_field, doc_id);

        let writer = self.writer.write();
        writer.delete_term(term);
        drop(writer);

        // Notify background thread if threshold reached (non-blocking)
        let pending = self
            .pending_docs
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        if pending >= AUTO_COMMIT_THRESHOLD {
            // Non-blocking notification - background thread will commit
            let _ = self.commit_tx.send(CommitCommand::ThresholdReached);
        }

        Ok(())
    }

    /// Search the index
    fn search(&self, query_str: &str, limit: usize) -> Result<Vec<(String, f32)>, StorageError> {
        // Use current reader snapshot - eventual consistency
        // Call flush() on FtsIndex if you need strong consistency
        let searcher = self.reader.searcher();

        // Build query parser for all content fields
        let field_vec: Vec<Field> = self.content_fields.values().copied().collect();

        if field_vec.is_empty() {
            return Ok(Vec::new());
        }

        let query_parser = QueryParser::for_index(&self.index, field_vec);

        let query = query_parser
            .parse_query(query_str)
            .map_err(|e| StorageError::Index(format!("Failed to parse FTS query: {}", e)))?;

        let top_docs = searcher
            .search(&query, &TopDocs::with_limit(limit))
            .map_err(|e| StorageError::Index(format!("FTS search failed: {}", e)))?;

        let mut results = Vec::with_capacity(top_docs.len());
        for (score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher
                .doc(doc_address)
                .map_err(|e| StorageError::Index(format!("Failed to retrieve FTS doc: {}", e)))?;

            if let Some(doc_id_value) = doc.get_first(self.doc_id_field)
                && let Some(id_str) = owned_value_as_str(&OwnedValue::from(doc_id_value))
            {
                results.push((id_str.to_string(), score));
            }
        }

        Ok(results)
    }

    /// Search with specific fields and optional boosts
    fn search_with_boosts(
        &self,
        query_str: &str,
        field_boosts: &[(String, f64)],
        limit: usize,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        // Use current reader snapshot - eventual consistency
        // Call flush() on FtsIndex if you need strong consistency
        let searcher = self.reader.searcher();

        // Build query parser for specified fields with boosts
        let mut field_vec: Vec<Field> = Vec::new();
        for (field_name, _boost) in field_boosts {
            if let Some(&field) = self.content_fields.get(field_name) {
                field_vec.push(field);
            }
        }

        if field_vec.is_empty() {
            return Ok(Vec::new());
        }

        // Create query parser
        let mut query_parser = QueryParser::for_index(&self.index, field_vec);

        // Set field boosts
        for (field_name, boost) in field_boosts {
            if let Some(&field) = self.content_fields.get(field_name) {
                query_parser.set_field_boost(field, *boost as f32);
            }
        }

        let query = query_parser
            .parse_query(query_str)
            .map_err(|e| StorageError::Index(format!("Failed to parse FTS query: {}", e)))?;

        let top_docs = searcher
            .search(&query, &TopDocs::with_limit(limit))
            .map_err(|e| StorageError::Index(format!("FTS search failed: {}", e)))?;

        let mut results = Vec::with_capacity(top_docs.len());
        for (score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher
                .doc(doc_address)
                .map_err(|e| StorageError::Index(format!("Failed to retrieve FTS doc: {}", e)))?;

            if let Some(doc_id_value) = doc.get_first(self.doc_id_field)
                && let Some(id_str) = owned_value_as_str(&OwnedValue::from(doc_id_value))
            {
                results.push((id_str.to_string(), score));
            }
        }

        Ok(results)
    }
}

/// Maximum number of concurrent modifications to track during REINDEX.
/// If exceeded, REINDEX will be aborted to prevent unbounded memory growth.
const MAX_CONCURRENT_MODIFICATIONS: usize = 100_000;

/// Statistics from a REINDEX operation
#[derive(Debug, Clone, Default)]
pub struct ReindexStats {
    /// Number of documents indexed from snapshot
    pub docs_indexed: usize,
    /// Number of documents skipped (handled by concurrent ops)
    pub docs_skipped: usize,
    /// Number of concurrent modifications tracked
    pub concurrent_modifications: usize,
}

/// State of an FTS index (for REINDEX support)
enum FtsIndexState {
    /// Normal operation
    Active(Arc<FtsIndexInstance>),
    /// Rebuilding: writes go to both, reads from active
    Rebuilding {
        active: Arc<FtsIndexInstance>,
        shadow: Arc<FtsIndexInstance>,
        /// Document IDs that were modified by concurrent operations during rebuild.
        /// These docs should not be overwritten by the REINDEX loop.
        /// Limited to MAX_CONCURRENT_MODIFICATIONS to prevent unbounded memory growth.
        concurrent_modified: Arc<parking_lot::Mutex<std::collections::HashSet<String>>>,
        /// Flag to signal REINDEX should abort (too many concurrent modifications)
        overflow: Arc<std::sync::atomic::AtomicBool>,
        /// Statistics for this rebuild operation
        stats: Arc<parking_lot::Mutex<ReindexStats>>,
    },
}

/// FTS index backend using Tantivy.
///
/// This provides full-text search capabilities with BM25 ranking.
/// Each (collection, index_name) pair gets its own Tantivy index instance.
pub struct FtsIndex {
    /// Map of (collection, index_name) -> FtsIndexState
    instances: Arc<RwLock<HashMap<(String, String), FtsIndexState>>>,
    /// Base path for FTS index storage
    base_path: PathBuf,
    /// Custom analyzer registry
    analyzer_registry: super::analyzers::AnalyzerRegistry,
    /// Channel to send commands to background commit thread
    commit_tx: std::sync::mpsc::Sender<CommitCommand>,
    /// Prevents the background thread from committing partway through an
    /// external-index batch. The per-index writer locks still serialize the
    /// actual Tantivy operations.
    commit_gate: Arc<Mutex<()>>,
    /// Handle to background commit thread (for join on drop)
    commit_thread: Option<std::thread::JoinHandle<()>>,
}

/// Holds off automatic FTS commits while a durable external-index batch is
/// being applied.
pub(crate) struct FtsCommitGuard<'a> {
    _guard: MutexGuard<'a, ()>,
}

impl FtsIndex {
    /// Create a new FtsIndex with the given base path for storage
    pub fn new(base_path: PathBuf) -> Self {
        let instances = Arc::new(RwLock::new(HashMap::new()));
        let (commit_tx, commit_rx) = std::sync::mpsc::channel();
        let commit_gate = Arc::new(Mutex::new(()));

        let commit_thread =
            spawn_commit_thread(Arc::clone(&instances), Arc::clone(&commit_gate), commit_rx);

        Self {
            instances,
            base_path,
            analyzer_registry: super::analyzers::AnalyzerRegistry::new(),
            commit_tx,
            commit_gate,
            commit_thread: Some(commit_thread),
        }
    }

    /// Get reference to the analyzer registry
    pub fn analyzer_registry(&self) -> &super::analyzers::AnalyzerRegistry {
        &self.analyzer_registry
    }

    /// Get the path for a specific index
    fn index_path(&self, collection: &str, index_name: &str) -> PathBuf {
        self.base_path
            .join(DIR_INDEXES)
            .join(DIR_FTS)
            .join(collection)
            .join(index_name)
    }

    /// Get or create an FTS index instance for a collection/index pair
    fn get_or_create_index(
        &self,
        collection: &str,
        index_name: &str,
        fields: &[String],
        analyzer_name: Option<&str>,
    ) -> Result<Arc<FtsIndexInstance>, StorageError> {
        let key = (collection.to_string(), index_name.to_string());
        let analyzer = analyzer_name.unwrap_or(DEFAULT_ANALYZER);

        // Fast path: check if already exists
        {
            let instances = self.instances.read();
            if let Some(state) = instances.get(&key) {
                return Ok(match state {
                    FtsIndexState::Active(idx) => Arc::clone(idx),
                    FtsIndexState::Rebuilding { active, .. } => Arc::clone(active),
                });
            }
        }

        // Slow path: create new instance
        let mut instances = self.instances.write();

        // Double-check after acquiring write lock
        if let Some(state) = instances.get(&key) {
            return Ok(match state {
                FtsIndexState::Active(idx) => Arc::clone(idx),
                FtsIndexState::Rebuilding { active, .. } => Arc::clone(active),
            });
        }

        let index_path = self.index_path(collection, index_name);
        let instance = Arc::new(FtsIndexInstance::new(
            &index_path,
            fields,
            analyzer,
            &self.analyzer_registry,
            self.commit_tx.clone(),
        )?);

        instances.insert(key, FtsIndexState::Active(Arc::clone(&instance)));
        Ok(instance)
    }

    /// Get an existing FTS index instance
    fn get_index(&self, collection: &str, index_name: &str) -> Option<Arc<FtsIndexInstance>> {
        let key = (collection.to_string(), index_name.to_string());
        let instances = self.instances.read();
        instances.get(&key).map(|state| match state {
            FtsIndexState::Active(idx) => Arc::clone(idx),
            FtsIndexState::Rebuilding { active, .. } => Arc::clone(active),
        })
    }

    /// Initialize an FTS index with specific analyzer (must be called before indexing)
    ///
    /// This creates the index instance with the specified analyzer. If no analyzer
    /// is provided, the default "standard" analyzer is used.
    pub fn init_index(
        &self,
        collection: &str,
        index_name: &str,
        fields: &[String],
        analyzer_name: Option<&str>,
    ) -> Result<(), StorageError> {
        self.get_or_create_index(collection, index_name, fields, analyzer_name)?;
        Ok(())
    }

    /// Initialize an unpublished build under a unique physical name.
    pub(crate) fn init_build(
        &self,
        collection: &str,
        build_name: &str,
        fields: &[String],
        analyzer_name: Option<&str>,
    ) -> Result<(), StorageError> {
        self.get_or_create_index(collection, build_name, fields, analyzer_name)?;
        Ok(())
    }

    pub(crate) fn index_build_document(
        &self,
        collection: &str,
        build_name: &str,
        fields: &[String],
        doc: &Document,
    ) -> Result<(), StorageError> {
        let instance = self.get_index(collection, build_name).ok_or_else(|| {
            StorageError::Index(format!(
                "FTS build {collection}.{build_name} not initialized"
            ))
        })?;
        instance.index_doc(doc, fields)
    }

    /// Atomically move a flushed build directory into its final physical name,
    /// then reopen it so future writes never refer to the temporary path.
    pub(crate) fn promote_build(
        &self,
        collection: &str,
        build_name: &str,
        index_name: &str,
        analyzer_name: Option<&str>,
    ) -> Result<(), StorageError> {
        // Stop automatic commits and keep the registry write-locked through
        // writer shutdown and rename. IndexWriter::drop kills Tantivy's
        // segment updater but does not wait for its merge thread. Reopening the
        // renamed directory before that thread exits creates two writers over
        // one set of managed files; the old updater may then garbage-collect a
        // segment that the new writer is about to commit.
        let build_key = (collection.to_string(), build_name.to_string());
        let _commit_guard = self.commit_gate.lock();
        let mut instances = self.instances.write();
        match instances.get(&build_key) {
            Some(FtsIndexState::Active(instance)) => instance.commit()?,
            Some(FtsIndexState::Rebuilding { .. }) => {
                return Err(StorageError::Index(
                    "temporary FTS build is rebuilding".into(),
                ));
            }
            None => {
                return Err(StorageError::Index(format!(
                    "FTS build {collection}.{build_name} not found"
                )));
            }
        }
        let instance = match instances.remove(&build_key) {
            Some(FtsIndexState::Active(instance)) => instance,
            Some(FtsIndexState::Rebuilding { .. }) | None => unreachable!(),
        };
        if Arc::strong_count(&instance) != 1 {
            instances.insert(build_key, FtsIndexState::Active(instance));
            return Err(StorageError::Conflict(
                "FTS build lifecycle changed during promotion".into(),
            ));
        }
        let instance = Arc::into_inner(instance).ok_or_else(|| {
            StorageError::Conflict("FTS build lifecycle changed during promotion".into())
        })?;
        instance.wait_merging_threads()?;

        let build_path = self.index_path(collection, build_name);
        let target_path = self.index_path(collection, index_name);
        if target_path.exists() {
            std::fs::remove_dir_all(&target_path).map_err(|error| {
                StorageError::Io(format!("Failed to replace FTS index directory: {error}"))
            })?;
        }
        std::fs::rename(&build_path, &target_path).map_err(|error| {
            StorageError::Io(format!("Failed to publish FTS index directory: {error}"))
        })?;
        let instance = Arc::new(FtsIndexInstance::open(
            &target_path,
            analyzer_name.unwrap_or(DEFAULT_ANALYZER),
            &self.analyzer_registry,
            self.commit_tx.clone(),
        )?);
        instances.insert(
            (collection.to_string(), index_name.to_string()),
            FtsIndexState::Active(instance),
        );
        Ok(())
    }

    pub(crate) fn remove_build(
        &self,
        collection: &str,
        build_name: &str,
    ) -> Result<(), StorageError> {
        self.remove_index(collection, build_name)
    }

    /// Delete build directories that cannot be matched to a live worker. This
    /// is called during startup before unfinished jobs are restarted cleanly.
    pub(crate) fn cleanup_orphaned_builds(&self) -> Result<(), StorageError> {
        let root = self.base_path.join(DIR_INDEXES).join(DIR_FTS);
        let Ok(collections) = std::fs::read_dir(&root) else {
            return Ok(());
        };
        for collection in collections {
            let collection = collection.map_err(|error| StorageError::Io(error.to_string()))?;
            if !collection.path().is_dir() {
                continue;
            }
            for entry in std::fs::read_dir(collection.path())
                .map_err(|error| StorageError::Io(error.to_string()))?
            {
                let entry = entry.map_err(|error| StorageError::Io(error.to_string()))?;
                if entry.file_name().to_str().is_some_and(|name| {
                    name.starts_with(crate::storage::keyspaces::INDEX_BUILD_MARKER)
                }) {
                    std::fs::remove_dir_all(entry.path())
                        .map_err(|error| StorageError::Io(error.to_string()))?;
                }
            }
        }
        Ok(())
    }

    /// Open an existing FTS index from disk (used during startup)
    ///
    /// Returns Ok(true) if index was opened, Ok(false) if index doesn't exist on disk.
    pub fn open_existing(
        &self,
        collection: &str,
        index_name: &str,
        analyzer_name: Option<&str>,
    ) -> Result<bool, StorageError> {
        let key = (collection.to_string(), index_name.to_string());
        let analyzer = analyzer_name.unwrap_or(DEFAULT_ANALYZER);

        // Check if already loaded
        {
            let instances = self.instances.read();
            if instances.contains_key(&key) {
                return Ok(true);
            }
        }

        let index_path = self.index_path(collection, index_name);

        // Check if index exists on disk
        if !index_path.exists() {
            return Ok(false);
        }

        // Open the existing index
        let instance = Arc::new(FtsIndexInstance::open(
            &index_path,
            analyzer,
            &self.analyzer_registry,
            self.commit_tx.clone(),
        )?);

        let mut instances = self.instances.write();
        instances.insert(key, FtsIndexState::Active(instance));

        Ok(true)
    }

    /// Search an FTS index
    ///
    /// Returns a list of (doc_id, score) pairs sorted by relevance.
    pub fn search(
        &self,
        collection: &str,
        index_name: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        let key = (collection.to_string(), index_name.to_string());

        let instances = self.instances.read();
        let state = instances.get(&key).ok_or_else(|| {
            StorageError::Index(format!("FTS index {}.{} not found", collection, index_name))
        })?;

        let instance = match state {
            FtsIndexState::Active(idx) => idx,
            FtsIndexState::Rebuilding { active, .. } => active,
        };

        instance.search(query, limit)
    }

    /// Search an FTS index with field boosts
    ///
    /// Returns a list of (doc_id, score) pairs sorted by relevance.
    pub fn search_with_boosts(
        &self,
        collection: &str,
        index_name: &str,
        query: &str,
        field_boosts: &[(String, f64)],
        limit: usize,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        let key = (collection.to_string(), index_name.to_string());
        let instances = self.instances.read();
        let state = instances.get(&key).ok_or_else(|| {
            StorageError::Index(format!("FTS index {}.{} not found", collection, index_name))
        })?;
        let instance = match state {
            FtsIndexState::Active(idx) => idx,
            FtsIndexState::Rebuilding { active, .. } => active,
        };
        instance.search_with_boosts(query, field_boosts, limit)
    }

    /// Remove an FTS index instance and its data after all Tantivy workers stop.
    pub fn remove_index(&self, collection: &str, index_name: &str) -> Result<(), StorageError> {
        let key = (collection.to_string(), index_name.to_string());
        let _commit_guard = self.commit_gate.lock();
        let mut instances = self.instances.write();
        let state = instances.remove(&key);

        if let Some(state) = state {
            let lifecycle_busy = match &state {
                FtsIndexState::Active(instance) => Arc::strong_count(instance) != 1,
                FtsIndexState::Rebuilding { active, shadow, .. } => {
                    Arc::strong_count(active) != 1 || Arc::strong_count(shadow) != 1
                }
            };
            if lifecycle_busy {
                instances.insert(key, state);
                return Err(StorageError::Conflict(
                    "FTS index lifecycle changed during removal".into(),
                ));
            }

            match state {
                FtsIndexState::Active(instance) => {
                    Arc::into_inner(instance)
                        .ok_or_else(|| {
                            StorageError::Conflict(
                                "FTS index lifecycle changed during removal".into(),
                            )
                        })?
                        .wait_merging_threads()?;
                }
                FtsIndexState::Rebuilding { active, shadow, .. } => {
                    Arc::into_inner(active)
                        .ok_or_else(|| {
                            StorageError::Conflict(
                                "FTS index lifecycle changed during removal".into(),
                            )
                        })?
                        .wait_merging_threads()?;
                    Arc::into_inner(shadow)
                        .ok_or_else(|| {
                            StorageError::Conflict(
                                "FTS index lifecycle changed during removal".into(),
                            )
                        })?
                        .wait_merging_threads()?;
                }
            }
        }

        let index_path = self.index_path(collection, index_name);
        if index_path.exists() {
            std::fs::remove_dir_all(&index_path).map_err(|e| {
                StorageError::Io(format!("Failed to remove FTS index directory: {}", e))
            })?;
        }

        Ok(())
    }

    /// Commit all pending changes across all index instances
    ///
    /// Call this after bulk indexing operations to ensure all changes are persisted.
    pub fn commit_all(&self) -> Result<(), StorageError> {
        let guard = self.acquire_commit_guard();
        self.commit_all_guarded(&guard)
    }

    /// Block automatic commits until the returned guard is dropped.
    pub(crate) fn acquire_commit_guard(&self) -> FtsCommitGuard<'_> {
        FtsCommitGuard {
            _guard: self.commit_gate.lock(),
        }
    }

    /// Commit while the caller is already holding the automatic-commit gate.
    pub(crate) fn commit_all_guarded(
        &self,
        _guard: &FtsCommitGuard<'_>,
    ) -> Result<(), StorageError> {
        let instances = self.instances.read();
        for state in instances.values() {
            match state {
                FtsIndexState::Active(idx) => idx.commit()?,
                FtsIndexState::Rebuilding { active, shadow, .. } => {
                    active.commit()?;
                    shadow.commit()?;
                }
            }
        }
        Ok(())
    }

    /// Flush all pending writes immediately and wait for completion.
    ///
    /// Use this when strong consistency is required (e.g., in tests or
    /// when you need to immediately search for just-written documents).
    pub fn flush(&self) -> Result<(), StorageError> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();

        self.commit_tx
            .send(CommitCommand::Flush(reply_tx))
            .map_err(|_| StorageError::Index("FTS commit thread not running".into()))?;

        reply_rx
            .recv()
            .map_err(|_| StorageError::Index("FTS commit thread died".into()))?
    }

    /// Start rebuilding an index (creates shadow)
    pub fn start_reindex(
        &self,
        collection: &str,
        index_name: &str,
        fields: &[String],
        analyzer_name: Option<&str>,
    ) -> Result<(), StorageError> {
        let key = (collection.to_string(), index_name.to_string());
        let mut instances = self.instances.write();

        let active = match instances.get(&key) {
            Some(FtsIndexState::Active(idx)) => Arc::clone(idx),
            Some(FtsIndexState::Rebuilding { .. }) => {
                return Err(StorageError::Index(
                    "REINDEX already in progress for this index".into(),
                ));
            }
            None => {
                return Err(StorageError::Index(format!(
                    "Index {}.{} not found",
                    collection, index_name
                )));
            }
        };

        // Use the same analyzer as the active index, or the provided one
        let analyzer = analyzer_name
            .map(|s| s.to_string())
            .unwrap_or_else(|| active.analyzer_name.clone());

        // Create shadow in temp directory. A previous interrupted rebuild may have
        // left a shadow behind, but no live rebuild owns it while the state is Active.
        let shadow_path = self.index_path(collection, &format!("{}_shadow", index_name));
        if shadow_path.exists() {
            std::fs::remove_dir_all(&shadow_path).map_err(|e| {
                StorageError::Io(format!("Failed to remove stale FTS shadow index: {}", e))
            })?;
        }
        let shadow = Arc::new(FtsIndexInstance::new(
            &shadow_path,
            fields,
            &analyzer,
            &self.analyzer_registry,
            self.commit_tx.clone(),
        )?);

        instances.insert(
            key,
            FtsIndexState::Rebuilding {
                active,
                shadow,
                concurrent_modified: Arc::new(parking_lot::Mutex::new(
                    std::collections::HashSet::new(),
                )),
                overflow: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                stats: Arc::new(parking_lot::Mutex::new(ReindexStats::default())),
            },
        );
        Ok(())
    }

    /// Get shadow instance for rebuilding (used during reindex)
    #[allow(private_interfaces)]
    pub fn get_shadow(&self, collection: &str, index_name: &str) -> Option<Arc<FtsIndexInstance>> {
        let key = (collection.to_string(), index_name.to_string());
        let instances = self.instances.read();
        match instances.get(&key) {
            Some(FtsIndexState::Rebuilding { shadow, .. }) => Some(Arc::clone(shadow)),
            _ => None,
        }
    }

    /// Index a document to the shadow index during reindex
    ///
    /// Returns Ok(true) if document was indexed, Ok(false) if no reindex in progress.
    pub fn index_to_shadow(
        &self,
        collection: &str,
        index_name: &str,
        doc: &Document,
        fields: &[String],
    ) -> Result<bool, StorageError> {
        let key = (collection.to_string(), index_name.to_string());
        let instances = self.instances.read();
        match instances.get(&key) {
            Some(FtsIndexState::Rebuilding {
                shadow,
                concurrent_modified,
                overflow,
                stats,
                ..
            }) => {
                // Check if overflow occurred - abort indexing
                if overflow.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err(StorageError::Index(
                        "REINDEX aborted: too many concurrent modifications".into(),
                    ));
                }

                // Skip if this doc was already modified by a concurrent operation.
                // Concurrent modifications have priority over the snapshot data.
                if concurrent_modified.lock().contains(&doc.id) {
                    stats.lock().docs_skipped += 1;
                    return Ok(true); // Already handled by concurrent operation
                }

                // Delete first to avoid duplicates if doc is also being written
                // concurrently via index_document during rebuild
                shadow.delete_doc(&doc.id)?;
                shadow.index_doc(doc, fields)?;
                stats.lock().docs_indexed += 1;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Complete reindex by swapping shadow to active
    /// Returns statistics about the reindex operation
    ///
    /// The `acquire_lock` closure is called just before the swap operation
    /// to ensure exclusive access during the atomic rename. This allows
    /// queries to continue running while the shadow index is being built.
    pub fn finish_reindex<F, G>(
        &self,
        collection: &str,
        index_name: &str,
        acquire_lock: F,
    ) -> Result<ReindexStats, StorageError>
    where
        F: FnOnce() -> G,
    {
        let key = (collection.to_string(), index_name.to_string());

        // Get paths upfront
        let main_path = self.index_path(collection, index_name);
        let shadow_path = self.index_path(collection, &format!("{}_shadow", index_name));

        // CRITICAL: Acquire DDL lock FIRST to avoid deadlock.
        // Lock order must be: DDL lock -> instances lock
        // This prevents deadlock with queries that hold DDL read lock and need instances read lock.
        let _ddl_lock = acquire_lock();

        // Now safe to acquire instances write lock
        let mut instances = self.instances.write();

        // Phase 1: Validate state and commit shadow while the rebuild remains registered.
        // If commit fails, callers can still cancel the rebuild and keep using active.
        let (analyzer_name, final_stats) = match instances.get(&key) {
            Some(FtsIndexState::Rebuilding {
                shadow,
                concurrent_modified,
                stats,
                overflow,
                ..
            }) => {
                if overflow.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err(StorageError::Index(
                        "REINDEX aborted: too many concurrent modifications".into(),
                    ));
                }

                shadow.force_commit()?;

                let mut final_stats = stats.lock().clone();
                final_stats.concurrent_modifications = concurrent_modified.lock().len();
                (shadow.analyzer_name.clone(), final_stats)
            }
            Some(FtsIndexState::Active(_)) | None => {
                return Err(StorageError::Index("No reindex in progress".into()));
            }
        };

        let state = instances
            .remove(&key)
            .ok_or_else(|| StorageError::Index("No reindex in progress".into()))?;
        let (active, shadow, concurrent_modified, overflow, stats) = match state {
            FtsIndexState::Rebuilding {
                active,
                shadow,
                concurrent_modified,
                overflow,
                stats,
            } => (active, shadow, concurrent_modified, overflow, stats),
            FtsIndexState::Active(active) => {
                instances.insert(key, FtsIndexState::Active(active));
                return Err(StorageError::Index("No reindex in progress".into()));
            }
        };

        // No operation can acquire another instance reference while the write lock is held.
        // An already-held reference means the lifecycle is still busy and the swap is retryable.
        if Arc::strong_count(&active) != 1 || Arc::strong_count(&shadow) != 1 {
            instances.insert(
                key,
                FtsIndexState::Rebuilding {
                    active,
                    shadow,
                    concurrent_modified,
                    overflow,
                    stats,
                },
            );
            return Err(StorageError::Conflict(
                "FTS index lifecycle changed during REINDEX".into(),
            ));
        }

        let active = Arc::into_inner(active).ok_or_else(|| {
            StorageError::Conflict("FTS index lifecycle changed during REINDEX".into())
        })?;
        let shadow = Arc::into_inner(shadow).ok_or_else(|| {
            StorageError::Conflict("FTS index lifecycle changed during REINDEX".into())
        })?;

        // Tantivy's IndexWriter::drop kills the segment updater but does not wait for its
        // merge thread. A merge can otherwise recreate files while main_path is removed,
        // leaving rename to fail with ENOTEMPTY on macOS.
        active.wait_merging_threads()?;
        shadow.wait_merging_threads()?;

        // Phase 2: File operations (under both DDL lock and instances write lock).
        if main_path.exists() {
            std::fs::remove_dir_all(&main_path).map_err(|e| {
                StorageError::Io(format!("Failed to remove active FTS index: {}", e))
            })?;
        }

        std::fs::rename(&shadow_path, &main_path)
            .map_err(|e| StorageError::Io(format!("Failed to rename shadow index: {}", e)))?;

        // Sync the parent directory to ensure rename is visible
        if let Ok(dir) = std::fs::File::open(main_path.parent().unwrap_or(&main_path)) {
            let _ = dir.sync_all();
        }

        // Phase 3: Re-open the index from the new (main) path
        let new_instance = Arc::new(FtsIndexInstance::open(
            &main_path,
            &analyzer_name,
            &self.analyzer_registry,
            self.commit_tx.clone(),
        )?);

        // Force reader reload to ensure all segments are visible
        new_instance
            .reader
            .reload()
            .map_err(|e| StorageError::Index(format!("Failed to reload FTS reader: {}", e)))?;

        // Phase 4: Insert new active state (still under write lock)
        instances.insert(key, FtsIndexState::Active(new_instance));

        Ok(final_stats)
    }

    /// Cancel reindex and cleanup shadow
    pub fn cancel_reindex(&self, collection: &str, index_name: &str) -> Result<(), StorageError> {
        let key = (collection.to_string(), index_name.to_string());
        let mut instances = self.instances.write();

        if let Some(FtsIndexState::Rebuilding { active, .. }) = instances.get(&key) {
            let active = Arc::clone(active);
            instances.insert(key, FtsIndexState::Active(active));

            // Cleanup shadow directory
            let shadow_path = self.index_path(collection, &format!("{}_shadow", index_name));
            let _ = std::fs::remove_dir_all(&shadow_path);
        }

        Ok(())
    }
}

impl Default for FtsIndex {
    fn default() -> Self {
        Self::new(PathBuf::from("data"))
    }
}

impl Drop for FtsIndex {
    fn drop(&mut self) {
        // Signal the background thread to shutdown
        let _ = self.commit_tx.send(CommitCommand::Shutdown);

        // Wait for thread to finish
        if let Some(handle) = self.commit_thread.take() {
            let _ = handle.join();
        }
    }
}

/// Load custom analyzers from storage into the FTS backend
impl FtsIndex {
    /// Load custom analyzers (called during startup)
    pub fn load_analyzers(&self, analyzers: Vec<crate::schema::AnalyzerDef>) {
        self.analyzer_registry.load(analyzers);
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
        fields: &[String],
        doc: &Document,
    ) -> Result<(), StorageError> {
        // Extract collection from doc.id (format: "collection:key")
        let collection = doc.collection();

        // Derive index name from fields (sorted to ensure consistency)
        let mut sorted_fields = fields.to_vec();
        sorted_fields.sort();
        let index_name = format!("{}_fts", sorted_fields.join("_"));

        let key = (collection.to_string(), index_name.clone());
        let instances = self.instances.read();

        match instances.get(&key) {
            Some(FtsIndexState::Active(idx)) => idx.index_doc(doc, fields),
            Some(FtsIndexState::Rebuilding {
                active,
                shadow,
                concurrent_modified,
                overflow,
                ..
            }) => {
                // Check if already overflowed
                if overflow.load(std::sync::atomic::Ordering::Relaxed) {
                    // Still write to active to not lose data, but skip shadow
                    return active.index_doc(doc, fields);
                }

                // Track this doc as concurrently modified so REINDEX loop won't overwrite it
                {
                    let mut cm = concurrent_modified.lock();
                    if cm.len() >= MAX_CONCURRENT_MODIFICATIONS {
                        overflow.store(true, std::sync::atomic::Ordering::Relaxed);
                        drop(cm);
                        // Still write to active to not lose data
                        return active.index_doc(doc, fields);
                    }
                    cm.insert(doc.id.clone());
                }

                // Write to both during rebuild
                active.index_doc(doc, fields)?;
                shadow.index_doc(doc, fields)
            }
            None => {
                drop(instances);
                // Use default analyzer when index is created lazily
                let instance = self.get_or_create_index(collection, &index_name, fields, None)?;
                instance.index_doc(doc, fields)
            }
        }
    }

    fn update_document(
        &self,
        _keyspace: &OptimisticTxKeyspace,
        fields: &[String],
        old_doc: Option<&Document>,
        new_doc: Option<&Document>,
    ) -> Result<(), StorageError> {
        // Derive index name from fields
        let mut sorted_fields = fields.to_vec();
        sorted_fields.sort();
        let index_name = format!("{}_fts", sorted_fields.join("_"));

        // Determine collection from either doc
        let collection = old_doc
            .map(|d| d.collection())
            .or_else(|| new_doc.map(|d| d.collection()));

        let collection = match collection {
            Some(c) => c,
            None => return Ok(()), // No docs provided, nothing to do
        };

        let key = (collection.to_string(), index_name.clone());
        let instances = self.instances.read();

        match instances.get(&key) {
            Some(FtsIndexState::Active(idx)) => {
                // Delete old document if it exists
                if let Some(old) = old_doc {
                    idx.delete_doc(&old.id)?;
                } else if let Some(new) = new_doc {
                    // Make replay idempotent when a durable index job was
                    // applied before crash but not marked complete.
                    idx.delete_doc(&new.id)?;
                }
                // Index new document if it exists
                if let Some(new) = new_doc {
                    idx.index_doc(new, fields)?;
                }
            }
            Some(FtsIndexState::Rebuilding {
                active,
                shadow,
                concurrent_modified,
                overflow,
                ..
            }) => {
                // Check if already overflowed
                let is_overflow = overflow.load(std::sync::atomic::Ordering::Relaxed);

                if is_overflow {
                    // Still process on active to not lose data, skip shadow
                    if let Some(old) = old_doc {
                        active.delete_doc(&old.id)?;
                    } else if let Some(new) = new_doc {
                        active.delete_doc(&new.id)?;
                    }
                    if let Some(new) = new_doc {
                        active.index_doc(new, fields)?;
                    }
                } else {
                    // Track this doc as concurrently modified so REINDEX loop won't overwrite it
                    let mut should_skip_shadow = false;
                    {
                        let mut cm = concurrent_modified.lock();
                        if let Some(old) = old_doc {
                            if cm.len() >= MAX_CONCURRENT_MODIFICATIONS {
                                overflow.store(true, std::sync::atomic::Ordering::Relaxed);
                                should_skip_shadow = true;
                            } else {
                                cm.insert(old.id.clone());
                            }
                        }
                        if let Some(new) = new_doc {
                            if cm.len() >= MAX_CONCURRENT_MODIFICATIONS {
                                overflow.store(true, std::sync::atomic::Ordering::Relaxed);
                                should_skip_shadow = true;
                            } else {
                                cm.insert(new.id.clone());
                            }
                        }
                    }

                    if let Some(old) = old_doc {
                        active.delete_doc(&old.id)?;
                        if !should_skip_shadow {
                            shadow.delete_doc(&old.id)?;
                        }
                    } else if let Some(new) = new_doc {
                        active.delete_doc(&new.id)?;
                        if !should_skip_shadow {
                            shadow.delete_doc(&new.id)?;
                        }
                    }
                    if let Some(new) = new_doc {
                        active.index_doc(new, fields)?;
                        if !should_skip_shadow {
                            shadow.index_doc(new, fields)?;
                        }
                    }
                }
            }
            None => {
                // Index doesn't exist yet, need to create it
                drop(instances);

                // Delete old document if it exists (from existing index if any)
                if let Some(old) = old_doc
                    && let Some(instance) = self.get_index(collection, &index_name)
                {
                    instance.delete_doc(&old.id)?;
                }

                // Index new document if it exists
                if let Some(new) = new_doc {
                    // Use default analyzer when index is created lazily
                    let instance =
                        self.get_or_create_index(collection, &index_name, fields, None)?;
                    instance.index_doc(new, fields)?;
                }
            }
        }

        Ok(())
    }

    fn scan_eq(
        &self,
        _keyspace: &OptimisticTxKeyspace,
        _field: &str,
        _value: &Value,
        _limit: usize,
    ) -> Result<ScanResult, StorageError> {
        // FTS doesn't support equality scans in the traditional sense.
        // Use search() method for text queries instead.
        Ok(ScanResult::empty())
    }

    fn scan_range(
        &self,
        _keyspace: &OptimisticTxKeyspace,
        _field: &str,
        _op: &RangeOp,
        _value: &Value,
        _limit: usize,
    ) -> Result<ScanResult, StorageError> {
        // FTS doesn't support range scans
        Ok(ScanResult::empty())
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
        // FTS doesn't support range scans
        Ok(ScanResult::empty())
    }

    fn scan_compound_eq(
        &self,
        _keyspace: &OptimisticTxKeyspace,
        _field_values: &[(&str, &Value)],
        _limit: usize,
    ) -> Result<ScanResult, StorageError> {
        // FTS doesn't support compound equality scans
        Ok(ScanResult::empty())
    }

    fn commit(&self) -> Result<(), StorageError> {
        self.commit_all()
    }
}

/// Extract string from an OwnedValue
fn owned_value_as_str(value: &OwnedValue) -> Option<&str> {
    match value {
        OwnedValue::Str(s) => Some(s.as_str()),
        _ => None,
    }
}

/// Convert a Value to text for indexing
fn value_to_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Decimal(d) => d.as_decimal().to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(arr) => arr.iter().map(value_to_text).collect::<Vec<_>>().join(" "),
        Value::Object(obj) => obj
            .values()
            .map(value_to_text)
            .collect::<Vec<_>>()
            .join(" "),
        Value::Null => String::new(),
        Value::Reference(id) => id.clone(), // Index the reference ID as text
        Value::Datetime(ms) => {
            // Convert to ISO8601 for text indexing
            chrono::DateTime::from_timestamp_millis(*ms)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| ms.to_string())
        }
        Value::Duration(ns) => format!("{}ns", ns),
        Value::Bytes(_) => String::new(), // Binary data not meaningful for text search
        Value::Range { start, end } => format!("{} {}", value_to_text(start), value_to_text(end)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_doc(id: &str, fields: Vec<(&str, Value)>) -> Document {
        Document {
            id: id.to_string(),
            fields: fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
    }

    #[test]
    fn test_fts_index_create_and_search() {
        let tmp_dir = TempDir::new().unwrap();
        let fts = FtsIndex::new(tmp_dir.path().to_path_buf());

        // Create index
        let fields = vec!["title".to_string(), "body".to_string()];
        let instance = fts
            .get_or_create_index("articles", "articles_fts", &fields, None)
            .unwrap();

        // Index some documents
        let doc1 = create_test_doc(
            "articles:1",
            vec![
                ("title", Value::String("Learning Rust".to_string())),
                (
                    "body",
                    Value::String("Rust is a systems programming language".to_string()),
                ),
            ],
        );
        instance.index_doc(&doc1, &fields).unwrap();

        let doc2 = create_test_doc(
            "articles:2",
            vec![
                ("title", Value::String("Database Design".to_string())),
                (
                    "body",
                    Value::String("Building efficient databases".to_string()),
                ),
            ],
        );
        instance.index_doc(&doc2, &fields).unwrap();

        // Flush to ensure documents are visible
        fts.flush().unwrap();

        // Search for "rust"
        let results = fts.search("articles", "articles_fts", "rust", 10).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "articles:1");
        assert!(results[0].1 > 0.0);
    }

    #[test]
    fn test_fts_index_update_document() {
        let tmp_dir = TempDir::new().unwrap();
        let fts = FtsIndex::new(tmp_dir.path().to_path_buf());

        let fields = vec!["content".to_string()];
        let instance = fts
            .get_or_create_index("posts", "posts_fts", &fields, None)
            .unwrap();

        // Index initial document
        let doc1 = create_test_doc(
            "posts:1",
            vec![("content", Value::String("hello world".to_string()))],
        );
        instance.index_doc(&doc1, &fields).unwrap();

        // Flush to ensure document is visible
        fts.flush().unwrap();

        // Verify initial search
        let results = fts.search("posts", "posts_fts", "hello", 10).unwrap();
        assert_eq!(results.len(), 1);

        // Delete and add updated document
        instance.delete_doc("posts:1").unwrap();
        let doc2 = create_test_doc(
            "posts:1",
            vec![("content", Value::String("goodbye world".to_string()))],
        );
        instance.index_doc(&doc2, &fields).unwrap();

        // Flush to ensure updates are visible
        fts.flush().unwrap();

        // Old content should not match
        let results = fts.search("posts", "posts_fts", "hello", 10).unwrap();
        assert_eq!(results.len(), 0);

        // New content should match
        let results = fts.search("posts", "posts_fts", "goodbye", 10).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_fts_index_with_boosts() {
        let tmp_dir = TempDir::new().unwrap();
        let fts = FtsIndex::new(tmp_dir.path().to_path_buf());

        let fields = vec!["title".to_string(), "body".to_string()];
        let instance = fts
            .get_or_create_index("articles", "articles_fts", &fields, None)
            .unwrap();

        // Document with "rust" in title
        let doc1 = create_test_doc(
            "articles:1",
            vec![
                ("title", Value::String("Rust Guide".to_string())),
                ("body", Value::String("Learn programming".to_string())),
            ],
        );
        instance.index_doc(&doc1, &fields).unwrap();

        // Document with "rust" in body
        let doc2 = create_test_doc(
            "articles:2",
            vec![
                ("title", Value::String("Programming".to_string())),
                ("body", Value::String("Rust basics".to_string())),
            ],
        );
        instance.index_doc(&doc2, &fields).unwrap();

        // Flush to ensure documents are visible
        fts.flush().unwrap();

        // Search with title boost
        let boosts = vec![("title".to_string(), 2.0), ("body".to_string(), 1.0)];
        let results = fts
            .search_with_boosts("articles", "articles_fts", "rust", &boosts, 10)
            .unwrap();

        // Both should match
        assert_eq!(results.len(), 2);

        // Document with "rust" in title should rank higher due to boost
        assert_eq!(results[0].0, "articles:1");
        assert!(results[0].1 > results[1].1);
    }

    #[test]
    fn test_fts_index_empty_search() {
        let tmp_dir = TempDir::new().unwrap();
        let fts = FtsIndex::new(tmp_dir.path().to_path_buf());

        let fields = vec!["content".to_string()];
        fts.get_or_create_index("empty", "empty_fts", &fields, None)
            .unwrap();

        let results = fts.search("empty", "empty_fts", "nonexistent", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_fts_index_multiple_matches() {
        let tmp_dir = TempDir::new().unwrap();
        let fts = FtsIndex::new(tmp_dir.path().to_path_buf());

        let fields = vec!["body".to_string()];
        let instance = fts
            .get_or_create_index("posts", "posts_fts", &fields, None)
            .unwrap();

        // Index documents with varying term frequency
        let doc1 = create_test_doc(
            "posts:1",
            vec![(
                "body",
                Value::String("rust rust rust programming".to_string()),
            )],
        );
        instance.index_doc(&doc1, &fields).unwrap();

        let doc2 = create_test_doc(
            "posts:2",
            vec![("body", Value::String("rust programming".to_string()))],
        );
        instance.index_doc(&doc2, &fields).unwrap();

        let doc3 = create_test_doc(
            "posts:3",
            vec![("body", Value::String("python programming".to_string()))],
        );
        instance.index_doc(&doc3, &fields).unwrap();

        // Flush to ensure documents are visible
        fts.flush().unwrap();

        // Search for rust
        let results = fts.search("posts", "posts_fts", "rust", 10).unwrap();

        // Should find 2 documents
        assert_eq!(results.len(), 2);

        // Doc with more occurrences should rank higher (BM25)
        assert_eq!(results[0].0, "posts:1");
        assert!(results[0].1 > results[1].1);
    }

    #[test]
    fn test_value_to_text() {
        assert_eq!(value_to_text(&Value::String("hello".to_string())), "hello");
        assert_eq!(value_to_text(&Value::Int(42)), "42");
        assert_eq!(value_to_text(&Value::Float(2.75)), "2.75");
        assert_eq!(value_to_text(&Value::Bool(true)), "true");
        assert_eq!(value_to_text(&Value::Null), "");

        let arr = Value::Array(vec![
            Value::String("a".to_string()),
            Value::String("b".to_string()),
        ]);
        assert_eq!(value_to_text(&arr), "a b");
    }

    #[test]
    fn test_fts_bulk_indexing_performance() {
        // This test verifies that bulk indexing with auto-commit is fast
        // Previously, each document triggered a commit, making this extremely slow
        let tmp_dir = TempDir::new().unwrap();
        let fts = FtsIndex::new(tmp_dir.path().to_path_buf());

        let fields = vec!["content".to_string()];
        let instance = fts
            .get_or_create_index("bulk", "bulk_fts", &fields, None)
            .unwrap();

        // Index 2000 documents (more than AUTO_COMMIT_THRESHOLD)
        let start = std::time::Instant::now();
        for i in 0..2000 {
            let doc = create_test_doc(
                &format!("bulk:{}", i),
                vec![(
                    "content",
                    Value::String(format!("document number {} with searchable content", i)),
                )],
            );
            instance.index_doc(&doc, &fields).unwrap();
        }
        // Flush to ensure all docs are visible
        fts.flush().unwrap();
        let elapsed = start.elapsed();

        // Should complete in reasonable time (< 5 seconds for 2000 docs)
        // Previously with commit-per-doc this would take many minutes
        assert!(
            elapsed.as_secs() < 10,
            "Bulk indexing took too long: {:?}",
            elapsed
        );

        // Verify documents are searchable
        let results = fts.search("bulk", "bulk_fts", "searchable", 100).unwrap();
        assert!(results.len() >= 100, "Expected at least 100 results");
    }

    #[test]
    fn test_fts_commit_all() {
        let tmp_dir = TempDir::new().unwrap();
        let fts = FtsIndex::new(tmp_dir.path().to_path_buf());

        // Create two different indexes
        let fields1 = vec!["title".to_string()];
        let fields2 = vec!["body".to_string()];

        let instance1 = fts
            .get_or_create_index("test", "title_fts", &fields1, None)
            .unwrap();
        let instance2 = fts
            .get_or_create_index("test", "body_fts", &fields2, None)
            .unwrap();

        // Add docs without triggering auto-commit
        let doc1 = create_test_doc(
            "test:1",
            vec![("title", Value::String("hello".to_string()))],
        );
        instance1.index_doc(&doc1, &fields1).unwrap();

        let doc2 = create_test_doc("test:2", vec![("body", Value::String("world".to_string()))]);
        instance2.index_doc(&doc2, &fields2).unwrap();

        // Flush to ensure all docs are visible
        fts.flush().unwrap();

        // Both should be searchable
        let r1 = fts.search("test", "title_fts", "hello", 10).unwrap();
        assert_eq!(r1.len(), 1);

        let r2 = fts.search("test", "body_fts", "world", 10).unwrap();
        assert_eq!(r2.len(), 1);
    }

    #[test]
    fn test_fts_stemming() {
        // Test that stemming works: "laptop" should match "laptops" and vice versa
        let tmp_dir = TempDir::new().unwrap();
        let fts = FtsIndex::new(tmp_dir.path().to_path_buf());

        let fields = vec!["name".to_string()];
        let instance = fts
            .get_or_create_index("products", "name_fts", &fields, Some("english"))
            .unwrap();

        // Index documents with plural forms
        let doc1 = create_test_doc(
            "products:1",
            vec![("name", Value::String("Gaming Laptops".to_string()))],
        );
        instance.index_doc(&doc1, &fields).unwrap();

        let doc2 = create_test_doc(
            "products:2",
            vec![("name", Value::String("Desktop Computers".to_string()))],
        );
        instance.index_doc(&doc2, &fields).unwrap();

        let doc3 = create_test_doc(
            "products:3",
            vec![("name", Value::String("Running Shoes".to_string()))],
        );
        instance.index_doc(&doc3, &fields).unwrap();

        // Flush to ensure all docs are visible
        fts.flush().unwrap();

        // Singular should match plural: "laptop" -> "laptops"
        let results = fts.search("products", "name_fts", "laptop", 10).unwrap();
        assert_eq!(results.len(), 1, "singular 'laptop' should match 'Laptops'");
        assert_eq!(results[0].0, "products:1");

        // Plural should also work: "laptops" -> "laptops"
        let results = fts.search("products", "name_fts", "laptops", 10).unwrap();
        assert_eq!(results.len(), 1, "plural 'laptops' should match 'Laptops'");

        // "computer" should match "computers"
        let results = fts.search("products", "name_fts", "computer", 10).unwrap();
        assert_eq!(
            results.len(),
            1,
            "singular 'computer' should match 'Computers'"
        );
        assert_eq!(results[0].0, "products:2");

        // "shoe" should match "shoes"
        let results = fts.search("products", "name_fts", "shoe", 10).unwrap();
        assert_eq!(results.len(), 1, "singular 'shoe' should match 'Shoes'");
        assert_eq!(results[0].0, "products:3");

        // "running" should match "Running" (also tests verb forms)
        let results = fts.search("products", "name_fts", "run", 10).unwrap();
        assert_eq!(results.len(), 1, "'run' should match 'Running'");
        assert_eq!(results[0].0, "products:3");
    }

    #[test]
    fn test_fts_eventual_consistency() {
        // Verify that search without flush may not see recent writes
        let tmp_dir = TempDir::new().unwrap();
        let fts = FtsIndex::new(tmp_dir.path().to_path_buf());

        let fields = vec!["content".to_string()];
        let instance = fts
            .get_or_create_index("test", "test_fts", &fields, None)
            .unwrap();

        let doc = create_test_doc(
            "test:1",
            vec![("content", Value::String("unique_term_xyz".to_string()))],
        );
        instance.index_doc(&doc, &fields).unwrap();

        // Immediate search MAY not find it (eventual consistency)
        // This is expected behavior - not a test failure
        let _ = fts.search("test", "test_fts", "unique_term_xyz", 10);

        // But after flush, it MUST be found
        fts.flush().unwrap();
        let results = fts
            .search("test", "test_fts", "unique_term_xyz", 10)
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_fts_flush_blocks_until_committed() {
        let tmp_dir = TempDir::new().unwrap();
        let fts = FtsIndex::new(tmp_dir.path().to_path_buf());

        let fields = vec!["content".to_string()];
        let instance = fts
            .get_or_create_index("test", "test_fts", &fields, None)
            .unwrap();

        for i in 0..100 {
            let doc = create_test_doc(
                &format!("test:{}", i),
                vec![("content", Value::String(format!("document number {}", i)))],
            );
            instance.index_doc(&doc, &fields).unwrap();
        }

        fts.flush().unwrap();

        let results = fts.search("test", "test_fts", "document", 200).unwrap();
        assert_eq!(results.len(), 100);
    }

    #[test]
    fn test_fts_visibility_after_interval() {
        let tmp_dir = TempDir::new().unwrap();
        let fts = FtsIndex::new(tmp_dir.path().to_path_buf());

        let fields = vec!["content".to_string()];
        let instance = fts
            .get_or_create_index("test", "test_fts", &fields, None)
            .unwrap();

        let doc = create_test_doc(
            "test:1",
            vec![("content", Value::String("visibility_test_term".to_string()))],
        );
        instance.index_doc(&doc, &fields).unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut results;
        loop {
            results = fts
                .search("test", "test_fts", "visibility_test_term", 10)
                .unwrap();
            if !results.is_empty() || std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // Now should be visible without explicit flush
        assert_eq!(
            results.len(),
            1,
            "Document should be visible after commit interval"
        );
    }

    #[test]
    fn test_reindex_waits_for_active_merge_workers_before_swap() {
        let tmp_dir = TempDir::new().unwrap();
        let fts = FtsIndex::new(tmp_dir.path().to_path_buf());
        let fields = vec!["content".to_string()];
        let instance = fts
            .get_or_create_index("docs", "content_fts", &fields, None)
            .unwrap();

        let mut merge_policy = tantivy::merge_policy::LogMergePolicy::default();
        merge_policy.set_min_num_segments(3);
        instance
            .writer
            .read()
            .set_merge_policy(Box::new(merge_policy));

        let docs: Vec<_> = (0..100)
            .map(|i| {
                create_test_doc(
                    &format!("docs:{}", i),
                    vec![(
                        "content",
                        Value::String(format!("merge lifecycle document {}", i)),
                    )],
                )
            })
            .collect();

        // Create enough committed segments to schedule background merges in the active index.
        for batch in docs.chunks(10) {
            for doc in batch {
                instance.index_doc(doc, &fields).unwrap();
            }
            instance.force_commit().unwrap();
        }
        drop(instance);

        fts.start_reindex("docs", "content_fts", &fields, None)
            .unwrap();
        for doc in &docs {
            assert!(
                fts.index_to_shadow("docs", "content_fts", doc, &fields)
                    .unwrap()
            );
        }

        fts.finish_reindex("docs", "content_fts", || ()).unwrap();

        let results = fts
            .search("docs", "content_fts", "lifecycle", docs.len())
            .unwrap();
        assert_eq!(results.len(), docs.len());
    }

    #[test]
    fn test_build_promotion_waits_for_active_merge_workers_before_rename() {
        let tmp_dir = TempDir::new().unwrap();
        let fts = FtsIndex::new(tmp_dir.path().to_path_buf());
        let fields = vec!["content".to_string()];
        let build_name = "__build_merge_lifecycle";
        let instance = fts
            .get_or_create_index("docs", build_name, &fields, None)
            .unwrap();

        let mut merge_policy = tantivy::merge_policy::LogMergePolicy::default();
        merge_policy.set_min_num_segments(3);
        instance
            .writer
            .read()
            .set_merge_policy(Box::new(merge_policy));

        for batch in 0..20 {
            for offset in 0..100 {
                let id = batch * 100 + offset;
                let doc = create_test_doc(
                    &format!("docs:{id}"),
                    vec![(
                        "content",
                        Value::String(format!("promoted merge lifecycle document {id}")),
                    )],
                );
                instance.index_doc(&doc, &fields).unwrap();
            }
            instance.force_commit().unwrap();
        }
        drop(instance);

        fts.promote_build("docs", build_name, "content_fts", None)
            .unwrap();
        for batch in 20..24 {
            for offset in 0..100 {
                let id = batch * 100 + offset;
                let doc = create_test_doc(
                    &format!("docs:{id}"),
                    vec![(
                        "content",
                        Value::String(format!("promoted merge lifecycle document {id}")),
                    )],
                );
                let active = fts.get_index("docs", "content_fts").unwrap();
                active.index_doc(&doc, &fields).unwrap();
            }
            fts.commit_all().unwrap();
        }

        let results = fts
            .search("docs", "content_fts", "promoted", 2_500)
            .unwrap();
        assert_eq!(results.len(), 2_400);
    }

    #[test]
    fn test_fts_graceful_shutdown_commits_pending() {
        let tmp_dir = TempDir::new().unwrap();
        let base_path = tmp_dir.path().to_path_buf();

        {
            let fts = FtsIndex::new(base_path.clone());

            let fields = vec!["content".to_string()];
            let instance = fts
                .get_or_create_index("test", "test_fts", &fields, None)
                .unwrap();

            for i in 0..50 {
                let doc = create_test_doc(
                    &format!("test:{}", i),
                    vec![("content", Value::String(format!("shutdown_test {}", i)))],
                );
                instance.index_doc(&doc, &fields).unwrap();
            }
            // FtsIndex dropped here - should commit pending
        }

        // Reopen and verify
        let fts2 = FtsIndex::new(base_path);
        fts2.open_existing("test", "test_fts", None).unwrap();

        // Need to flush to ensure reader sees the data
        fts2.flush().unwrap();

        let results = fts2
            .search("test", "test_fts", "shutdown_test", 100)
            .unwrap();
        assert_eq!(
            results.len(),
            50,
            "All documents should be persisted on shutdown"
        );
    }
}
