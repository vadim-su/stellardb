//! HNSW vector index backend using usearch
//!
//! Provides approximate nearest neighbor search with configurable distance metrics.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use bimap::BiMap;
use parking_lot::{Mutex, RwLock};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use crate::document::{Document, Value};
use crate::error::StorageError;
use crate::schema::{DistanceMetric, HnswParams};

use super::super::{DIR_INDEXES, DIR_VECTOR};
use super::traits::{IndexBackend, IndexType, RangeOp, ScanResult};

/// Helper to extract vector from document field
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

/// HNSW index instance for a single collection/field pair
pub struct HnswInstance {
    /// The usearch index
    index: Index,
    /// Guards usearch index operations. The underlying native index is mutated
    /// from document writes, searches, commits, and REINDEX shadow builds.
    op_lock: Mutex<()>,
    /// Vector dimension
    dimension: usize,
    /// Bidirectional mapping: internal_id (u64) <-> doc_id (String)
    id_map: RwLock<BiMap<u64, String>>,
    /// Next internal ID to assign
    next_id: AtomicU64,
    /// Path to index file
    path: PathBuf,
    /// Pending changes counter for batched commits
    pending: AtomicUsize,
}

impl HnswInstance {
    /// Get the path to the id_map metadata file
    fn metadata_path(index_path: &Path) -> PathBuf {
        index_path.with_extension("meta")
    }

    /// Metadata format version
    const META_VERSION: u32 = 1;

    /// Save id_map and next_id to metadata file.
    ///
    /// Uses atomic write (temp file + rename) to prevent corruption from crashes
    /// during save. Either the old or new version will exist completely.
    fn save_metadata(&self) -> Result<(), StorageError> {
        let meta_path = Self::metadata_path(&self.path);
        let tmp_path = meta_path.with_extension("meta.tmp");

        let file = std::fs::File::create(&tmp_path).map_err(|e| {
            StorageError::Io(format!("Failed to create HNSW metadata temp file: {}", e))
        })?;
        let mut writer = std::io::BufWriter::new(file);

        // Write version header
        writeln!(writer, "v{}", Self::META_VERSION)
            .map_err(|e| StorageError::Io(format!("Failed to write HNSW metadata: {}", e)))?;

        // Write next_id on second line
        let next_id = self.next_id.load(Ordering::SeqCst);
        writeln!(writer, "{}", next_id)
            .map_err(|e| StorageError::Io(format!("Failed to write HNSW metadata: {}", e)))?;

        // Write id_map entries: internal_id<TAB>doc_id
        let id_map = self.id_map.read();
        for (internal_id, doc_id) in id_map.iter() {
            writeln!(writer, "{}\t{}", internal_id, doc_id)
                .map_err(|e| StorageError::Io(format!("Failed to write HNSW metadata: {}", e)))?;
        }

        writer
            .flush()
            .map_err(|e| StorageError::Io(format!("Failed to flush HNSW metadata: {}", e)))?;

        // Ensure data is on disk before rename
        drop(writer);

        // Atomic rename: replaces old file completely or not at all
        std::fs::rename(&tmp_path, &meta_path)
            .map_err(|e| StorageError::Io(format!("Failed to rename HNSW metadata file: {}", e)))?;

        Ok(())
    }

    /// Load id_map and next_id from metadata file.
    ///
    /// Supports both the versioned format (v1+) and the legacy unversioned format.
    fn load_metadata(index_path: &Path) -> Result<(BiMap<u64, String>, u64), StorageError> {
        let meta_path = Self::metadata_path(index_path);

        if !meta_path.exists() {
            return Ok((BiMap::new(), 0));
        }

        let file = std::fs::File::open(&meta_path)
            .map_err(|e| StorageError::Io(format!("Failed to open HNSW metadata file: {}", e)))?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        // Read first line — could be version header ("v1") or next_id (legacy format)
        let first_line = match lines.next() {
            Some(Ok(line)) => line,
            Some(Err(e)) => {
                return Err(StorageError::Io(format!(
                    "Failed to read HNSW metadata: {}",
                    e
                )));
            }
            None => return Ok((BiMap::new(), 0)),
        };

        // Detect format: versioned starts with "v", legacy starts with a number
        let next_id = if let Some(version_str) = first_line.strip_prefix('v') {
            // Versioned format
            let _version: u32 = version_str.parse().map_err(|e| {
                StorageError::Io(format!("Failed to parse HNSW metadata version: {}", e))
            })?;
            // Read next_id from second line
            match lines.next() {
                Some(Ok(line)) => line.parse::<u64>().map_err(|e| {
                    StorageError::Io(format!("Failed to parse HNSW next_id: {}", e))
                })?,
                Some(Err(e)) => {
                    return Err(StorageError::Io(format!(
                        "Failed to read HNSW metadata: {}",
                        e
                    )));
                }
                None => return Ok((BiMap::new(), 0)),
            }
        } else {
            // Legacy unversioned format: first line is next_id directly
            first_line
                .parse::<u64>()
                .map_err(|e| StorageError::Io(format!("Failed to parse HNSW next_id: {}", e)))?
        };

        // Read id_map entries
        let mut id_map = BiMap::new();
        for line_result in lines {
            let line = line_result.map_err(|e| {
                StorageError::Io(format!("Failed to read HNSW metadata line: {}", e))
            })?;
            if let Some((internal_id_str, doc_id)) = line.split_once('\t') {
                let internal_id = internal_id_str.parse::<u64>().map_err(|e| {
                    StorageError::Io(format!("Failed to parse HNSW internal_id: {}", e))
                })?;
                id_map.insert(internal_id, doc_id.to_string());
            }
        }

        Ok((id_map, next_id))
    }

    /// Create a new HnswInstance with the given path and parameters
    fn new(path: PathBuf, params: &HnswParams) -> Result<Self, StorageError> {
        let metric = match params.metric {
            DistanceMetric::Cosine => MetricKind::Cos,
            DistanceMetric::Euclidean => MetricKind::L2sq,
            DistanceMetric::Dot => MetricKind::IP,
        };

        let options = IndexOptions {
            dimensions: params.dimension,
            metric,
            quantization: ScalarKind::F32,
            connectivity: params.m,
            expansion_add: params.ef_construction,
            expansion_search: params.ef_construction,
            multi: false,
        };

        // Create directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                StorageError::Io(format!("Failed to create HNSW index directory: {}", e))
            })?;
        }

        // Try to load existing or create new
        let (index, id_map, next_id) = if path.exists() {
            let idx = Index::new(&options).map_err(|e| {
                StorageError::Index(format!("Failed to create HNSW index for loading: {}", e))
            })?;
            idx.load(path.to_str().unwrap_or_default())
                .map_err(|e| StorageError::Index(format!("Failed to load HNSW index: {}", e)))?;

            // Load id_map and next_id from metadata
            let (id_map, next_id) = Self::load_metadata(&path)?;

            (idx, id_map, next_id)
        } else {
            let idx = Index::new(&options)
                .map_err(|e| StorageError::Index(format!("Failed to create HNSW index: {}", e)))?;
            (idx, BiMap::new(), 0)
        };

        // Reserve some capacity
        index
            .reserve(10000)
            .map_err(|e| StorageError::Index(format!("Failed to reserve HNSW capacity: {}", e)))?;

        Ok(Self {
            index,
            op_lock: Mutex::new(()),
            dimension: params.dimension,
            id_map: RwLock::new(id_map),
            next_id: AtomicU64::new(next_id),
            path,
            pending: AtomicUsize::new(0),
        })
    }

    /// Add a vector to the index
    pub fn add_vector(&self, doc_id: &str, vector: &[f32]) -> Result<(), StorageError> {
        if vector.len() != self.dimension {
            return Err(StorageError::InvalidOperation(format!(
                "Vector dimension mismatch: expected {}, got {}",
                self.dimension,
                vector.len()
            )));
        }

        let internal_id = self.next_id.fetch_add(1, Ordering::SeqCst);

        let _op = self.op_lock.lock();

        self.index
            .add(internal_id, vector)
            .map_err(|e| StorageError::Index(format!("Failed to add vector: {}", e)))?;

        self.id_map.write().insert(internal_id, doc_id.to_string());
        self.pending.fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    /// Remove a vector from the index
    pub fn remove_vector(&self, doc_id: &str) -> Result<(), StorageError> {
        let _op = self.op_lock.lock();

        // Hold a single write lock for the entire operation to avoid a gap
        // where another thread could add a vector with the same doc_id between
        // reading the id_map and updating it.
        let mut id_map = self.id_map.write();
        if let Some(&internal_id) = id_map.get_by_right(doc_id) {
            self.index
                .remove(internal_id)
                .map_err(|e| StorageError::Index(format!("Failed to remove vector: {}", e)))?;
            id_map.remove_by_right(doc_id);
            self.pending.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Search for nearest neighbors
    pub fn search(
        &self,
        vector: &[f32],
        k: usize,
        _effort: Option<usize>,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        if vector.len() != self.dimension {
            return Err(StorageError::InvalidOperation(format!(
                "Query vector dimension mismatch: expected {}, got {}",
                self.dimension,
                vector.len()
            )));
        }

        let _op = self.op_lock.lock();

        let results = self
            .index
            .search(vector, k)
            .map_err(|e| StorageError::Index(format!("HNSW search failed: {}", e)))?;

        let id_map = self.id_map.read();
        let mut output = Vec::with_capacity(results.keys.len());

        for (key, distance) in results.keys.iter().zip(results.distances.iter()) {
            if let Some(doc_id) = id_map.get_by_left(key) {
                output.push((doc_id.clone(), *distance));
            }
        }

        Ok(output)
    }

    /// Validate that a vector has the correct dimension for this index
    ///
    /// Returns Ok(()) if dimension matches, error otherwise.
    /// Use this to pre-validate vectors before committing transactions.
    pub fn validate_dimension(&self, vector: &[f32]) -> Result<(), StorageError> {
        if vector.len() != self.dimension {
            return Err(StorageError::InvalidOperation(format!(
                "Vector dimension mismatch: expected {}, got {}",
                self.dimension,
                vector.len()
            )));
        }
        Ok(())
    }

    /// Save the index to disk (including id_map metadata)
    pub fn save(&self) -> Result<(), StorageError> {
        let _op = self.op_lock.lock();

        // Save the HNSW index itself
        self.index
            .save(self.path.to_str().unwrap_or_default())
            .map_err(|e| StorageError::Index(format!("Failed to save HNSW index: {}", e)))?;

        // Save id_map and next_id to metadata file
        self.save_metadata()?;

        self.pending.store(0, Ordering::Relaxed);
        Ok(())
    }

    /// Commit pending changes (saves if there are pending changes)
    pub fn commit(&self) -> Result<(), StorageError> {
        let pending = self.pending.load(Ordering::Relaxed);
        if pending > 0 {
            self.save()?;
        }
        Ok(())
    }

    /// Clear all vectors from the index
    ///
    /// Removes all vectors and resets the id_map. Used when dropping an index.
    pub fn clear(&self) -> Result<(), StorageError> {
        let _op = self.op_lock.lock();

        // Clear all vectors from the usearch index
        let mut id_map = self.id_map.write();
        let internal_ids: Vec<u64> = id_map.left_values().copied().collect();
        for internal_id in internal_ids {
            // Ignore errors from removing non-existent vectors
            let _ = self.index.remove(internal_id);
        }

        // Clear the id_map
        id_map.clear();

        // Reset next_id to 0
        self.next_id.store(0, Ordering::SeqCst);

        // Mark as having pending changes so save() will persist
        self.pending.fetch_add(1, Ordering::Relaxed);

        Ok(())
    }
}

/// State of an HNSW index (for REINDEX support)
enum HnswIndexState {
    /// Normal operation
    Active(Arc<HnswInstance>),
    /// Rebuilding: writes go to both, reads from active
    Rebuilding {
        active: Arc<HnswInstance>,
        shadow: Arc<HnswInstance>,
    },
}

/// HNSW vector index backend
pub struct HnswIndex {
    /// Map of (collection, index_name) -> HnswIndexState
    instances: RwLock<HashMap<(String, String), HnswIndexState>>,
    /// Base path for index storage
    base_path: PathBuf,
}

impl HnswIndex {
    /// Create a new HnswIndex with the given base path
    pub fn new(base_path: PathBuf) -> Self {
        Self {
            instances: RwLock::new(HashMap::new()),
            base_path,
        }
    }

    /// Get path for a specific index
    fn index_path(&self, collection: &str, field: &str) -> PathBuf {
        self.base_path
            .join(DIR_INDEXES)
            .join(DIR_VECTOR)
            .join(collection)
            .join(format!("{}.idx", field))
    }

    /// Get or create an HNSW instance for a collection/field pair
    pub fn get_or_create_instance(
        &self,
        collection: &str,
        field: &str,
        params: &HnswParams,
    ) -> Result<Arc<HnswInstance>, StorageError> {
        let key = (collection.to_string(), field.to_string());

        // Fast path - read lock
        {
            let instances = self.instances.read();
            if let Some(state) = instances.get(&key) {
                return Ok(match state {
                    HnswIndexState::Active(idx) => Arc::clone(idx),
                    HnswIndexState::Rebuilding { active, .. } => Arc::clone(active),
                });
            }
        }

        // Slow path - write lock
        let mut instances = self.instances.write();
        if let Some(state) = instances.get(&key) {
            return Ok(match state {
                HnswIndexState::Active(idx) => Arc::clone(idx),
                HnswIndexState::Rebuilding { active, .. } => Arc::clone(active),
            });
        }

        let path = self.index_path(collection, field);
        let instance = Arc::new(HnswInstance::new(path, params)?);
        instances.insert(key, HnswIndexState::Active(Arc::clone(&instance)));
        Ok(instance)
    }

    pub(crate) fn init_build(
        &self,
        collection: &str,
        build_name: &str,
        params: &HnswParams,
    ) -> Result<(), StorageError> {
        self.get_or_create_instance(collection, build_name, params)?;
        Ok(())
    }

    pub(crate) fn index_build_document(
        &self,
        collection: &str,
        build_name: &str,
        source_field: &str,
        doc: &Document,
    ) -> Result<(), StorageError> {
        let Some(vector) = extract_vector(doc, source_field) else {
            return Ok(());
        };
        if vector.is_empty() {
            return Ok(());
        }
        let instance = self.get_instance(collection, build_name).ok_or_else(|| {
            StorageError::Index(format!(
                "HNSW build {collection}.{build_name} not initialized"
            ))
        })?;
        instance.add_vector(&doc.id, &vector)
    }

    /// Persist and atomically rename a temporary vector build, then reopen it
    /// under the logical field name used by the planner.
    pub(crate) fn promote_build(
        &self,
        collection: &str,
        build_name: &str,
        field: &str,
        params: &HnswParams,
    ) -> Result<(), StorageError> {
        let build_key = (collection.to_string(), build_name.to_string());
        let instance = {
            let mut instances = self.instances.write();
            match instances.remove(&build_key) {
                Some(HnswIndexState::Active(instance)) => instance,
                Some(HnswIndexState::Rebuilding { .. }) => {
                    return Err(StorageError::Index(
                        "temporary HNSW build is rebuilding".into(),
                    ));
                }
                None => {
                    return Err(StorageError::Index(format!(
                        "HNSW build {collection}.{build_name} not found"
                    )));
                }
            }
        };
        instance.save()?;
        drop(instance);

        let build_path = self.index_path(collection, build_name);
        let target_path = self.index_path(collection, field);
        let build_meta = HnswInstance::metadata_path(&build_path);
        let target_meta = HnswInstance::metadata_path(&target_path);
        if target_path.exists() {
            std::fs::remove_file(&target_path)
                .map_err(|error| StorageError::Io(error.to_string()))?;
        }
        if target_meta.exists() {
            std::fs::remove_file(&target_meta)
                .map_err(|error| StorageError::Io(error.to_string()))?;
        }
        std::fs::rename(&build_path, &target_path)
            .map_err(|error| StorageError::Io(format!("Failed to publish HNSW index: {error}")))?;
        std::fs::rename(&build_meta, &target_meta).map_err(|error| {
            StorageError::Io(format!("Failed to publish HNSW metadata: {error}"))
        })?;
        self.get_or_create_instance(collection, field, params)?;
        Ok(())
    }

    pub(crate) fn remove_build(
        &self,
        collection: &str,
        build_name: &str,
    ) -> Result<(), StorageError> {
        self.remove_index(collection, build_name)
    }

    pub(crate) fn cleanup_orphaned_builds(&self) -> Result<(), StorageError> {
        let root = self.base_path.join(DIR_INDEXES).join(DIR_VECTOR);
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
                    std::fs::remove_file(entry.path())
                        .map_err(|error| StorageError::Io(error.to_string()))?;
                }
            }
        }
        Ok(())
    }

    /// Get an existing instance for a collection/field pair
    fn get_instance(&self, collection: &str, field: &str) -> Option<Arc<HnswInstance>> {
        let key = (collection.to_string(), field.to_string());
        let instances = self.instances.read();
        instances.get(&key).map(|state| match state {
            HnswIndexState::Active(idx) => Arc::clone(idx),
            HnswIndexState::Rebuilding { active, .. } => Arc::clone(active),
        })
    }

    /// KNN search - returns (doc_id, distance) pairs
    pub fn search(
        &self,
        collection: &str,
        index_name: &str,
        vector: &[f32],
        k: usize,
        effort: Option<usize>,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        let instance = self.get_instance(collection, index_name).ok_or_else(|| {
            StorageError::Index(format!(
                "HNSW index {}.{} not found",
                collection, index_name
            ))
        })?;

        // Commit pending changes before search for consistency
        instance.commit()?;
        instance.search(vector, k, effort)
    }

    /// Validate a vector's dimension for an index
    ///
    /// Returns Ok(()) if the index doesn't exist or the vector has correct dimension.
    /// Returns error if the vector has wrong dimension.
    ///
    /// This is used to pre-validate vectors before committing transactions,
    /// ensuring atomicity: if validation fails, the document won't be created.
    pub fn validate_vector(
        &self,
        collection: &str,
        field: &str,
        vector: &[f32],
    ) -> Result<(), StorageError> {
        if let Some(instance) = self.get_instance(collection, field) {
            instance.validate_dimension(vector)?;
        }
        // If index doesn't exist, that's fine - no validation needed
        Ok(())
    }

    /// Start rebuilding an index (creates shadow)
    pub fn start_reindex(
        &self,
        collection: &str,
        field: &str,
        params: &HnswParams,
    ) -> Result<(), StorageError> {
        let key = (collection.to_string(), field.to_string());
        let mut instances = self.instances.write();

        let active = match instances.get(&key) {
            Some(HnswIndexState::Active(idx)) => Arc::clone(idx),
            Some(HnswIndexState::Rebuilding { .. }) => {
                return Err(StorageError::Index(
                    "REINDEX already in progress for this index".into(),
                ));
            }
            None => {
                return Err(StorageError::Index(format!(
                    "Index {}.{} not found",
                    collection, field
                )));
            }
        };

        // Create shadow in temp location
        let shadow_path = self.index_path(collection, &format!("{}_shadow", field));
        let shadow = Arc::new(HnswInstance::new(shadow_path, params)?);

        instances.insert(key, HnswIndexState::Rebuilding { active, shadow });
        Ok(())
    }

    /// Get shadow instance for rebuilding (used during reindex)
    pub fn get_shadow(&self, collection: &str, field: &str) -> Option<Arc<HnswInstance>> {
        let key = (collection.to_string(), field.to_string());
        let instances = self.instances.read();
        match instances.get(&key) {
            Some(HnswIndexState::Rebuilding { shadow, .. }) => Some(Arc::clone(shadow)),
            _ => None,
        }
    }

    /// Add a vector to the shadow index during reindex
    ///
    /// Returns Ok(true) if vector was added, Ok(false) if no reindex in progress.
    pub fn add_to_shadow(
        &self,
        collection: &str,
        field: &str,
        doc_id: &str,
        vector: &[f32],
    ) -> Result<bool, StorageError> {
        let key = (collection.to_string(), field.to_string());
        let instances = self.instances.read();
        match instances.get(&key) {
            Some(HnswIndexState::Rebuilding { shadow, .. }) => {
                shadow.add_vector(doc_id, vector)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Complete reindex by swapping shadow to active
    ///
    /// The `acquire_lock` closure is called just before the swap operation
    /// to ensure exclusive access during the atomic rename. This allows
    /// queries to continue running while the shadow index is being built.
    pub fn finish_reindex<F, G>(
        &self,
        collection: &str,
        field: &str,
        acquire_lock: F,
    ) -> Result<(), StorageError>
    where
        F: FnOnce() -> G,
    {
        let key = (collection.to_string(), field.to_string());

        // CRITICAL: Acquire DDL lock FIRST to avoid deadlock.
        // Lock order must be: DDL lock -> instances lock
        // This prevents deadlock with queries that hold DDL read lock and need instances read lock.
        let _ddl_lock = acquire_lock();

        // Now safe to acquire instances write lock
        let mut instances = self.instances.write();

        let shadow = match instances.get(&key) {
            Some(HnswIndexState::Rebuilding { shadow, .. }) => Arc::clone(shadow),
            _ => {
                return Err(StorageError::Index("No reindex in progress".into()));
            }
        };

        // Commit shadow before swap
        shadow.commit()?;

        // Swap to new index
        instances.insert(key.clone(), HnswIndexState::Active(shadow));

        // Delete old index file and rename shadow (best-effort)
        let old_path = self.index_path(collection, field);
        let shadow_path = self.index_path(collection, &format!("{}_shadow", field));

        // Remove old, rename shadow to main
        let _ = std::fs::remove_file(&old_path);
        let _ = std::fs::rename(&shadow_path, &old_path);

        Ok(())
    }

    /// Cancel reindex and cleanup shadow
    pub fn cancel_reindex(&self, collection: &str, field: &str) -> Result<(), StorageError> {
        let key = (collection.to_string(), field.to_string());
        let mut instances = self.instances.write();

        if let Some(HnswIndexState::Rebuilding { active, .. }) = instances.get(&key) {
            let active = Arc::clone(active);
            instances.insert(key, HnswIndexState::Active(active));

            // Cleanup shadow file
            let shadow_path = self.index_path(collection, &format!("{}_shadow", field));
            let _ = std::fs::remove_file(&shadow_path);
        }

        Ok(())
    }

    /// Remove an HNSW index instance and its data.
    ///
    /// Used by DROP INDEX to clean up vector indexes.
    pub fn remove_index(&self, collection: &str, field: &str) -> Result<(), StorageError> {
        let key = (collection.to_string(), field.to_string());

        // Remove from instances map
        {
            let mut instances = self.instances.write();
            instances.remove(&key);
        }

        // Remove the index file
        let index_path = self.index_path(collection, field);
        if index_path.exists() {
            std::fs::remove_file(&index_path).map_err(|e| {
                StorageError::Io(format!("Failed to remove HNSW index file: {}", e))
            })?;
        }

        // Remove metadata file and temp file (if mid-save crash left one)
        let meta_path = HnswInstance::metadata_path(&index_path);
        if meta_path.exists() {
            std::fs::remove_file(&meta_path).map_err(|e| {
                StorageError::Io(format!("Failed to remove HNSW metadata file: {}", e))
            })?;
        }
        let tmp_path = meta_path.with_extension("meta.tmp");
        let _ = std::fs::remove_file(&tmp_path);

        tracing::debug!("Removed HNSW index {}.{}", collection, field);
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
        _keyspace: &fjall::OptimisticTxKeyspace,
        fields: &[String],
        doc: &Document,
    ) -> Result<(), StorageError> {
        // For HNSW, we expect a single field (the vector field)
        let field = match fields.first() {
            Some(f) => f,
            None => return Ok(()), // No field to index
        };

        // Extract vector from document
        let vector = match doc.fields.get(field) {
            Some(Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| match v {
                    Value::Float(f) => Some(*f as f32),
                    Value::Int(i) => Some(*i as f32),
                    _ => None,
                })
                .collect::<Vec<f32>>(),
            _ => return Ok(()), // Skip if no vector field
        };

        if vector.is_empty() {
            return Ok(());
        }

        // Get collection from doc.id (format: "collection:key")
        let collection = doc.id.split(':').next().unwrap_or(&doc.id);
        let key = (collection.to_string(), field.to_string());

        let instances = self.instances.read();
        match instances.get(&key) {
            Some(HnswIndexState::Active(idx)) => {
                idx.add_vector(&doc.id, &vector)?;
            }
            Some(HnswIndexState::Rebuilding { active, shadow }) => {
                // Write to both during rebuild
                active.add_vector(&doc.id, &vector)?;
                shadow.add_vector(&doc.id, &vector)?;
            }
            None => {
                // If no instance exists, the index hasn't been created yet - that's OK
            }
        }

        Ok(())
    }

    fn update_document(
        &self,
        _keyspace: &fjall::OptimisticTxKeyspace,
        fields: &[String],
        old_doc: Option<&Document>,
        new_doc: Option<&Document>,
    ) -> Result<(), StorageError> {
        let field = match fields.first() {
            Some(f) => f,
            None => return Ok(()),
        };

        // Determine collection from either doc
        let collection = old_doc
            .map(|d| d.id.split(':').next().unwrap_or(&d.id))
            .or_else(|| new_doc.map(|d| d.id.split(':').next().unwrap_or(&d.id)));

        let collection = match collection {
            Some(c) => c,
            None => return Ok(()), // No docs provided, nothing to do
        };

        let key = (collection.to_string(), field.to_string());
        let instances = self.instances.read();

        match instances.get(&key) {
            Some(HnswIndexState::Active(idx)) => {
                // Remove old vector if exists
                if let Some(old) = old_doc {
                    idx.remove_vector(&old.id)?;
                } else if let Some(new) = new_doc {
                    // Make durable job replay idempotent when the previous
                    // apply succeeded but the job was not marked complete.
                    idx.remove_vector(&new.id)?;
                }
                // Add new vector if exists
                if let Some(new) = new_doc
                    && let Some(vector) = extract_vector(new, field)
                    && !vector.is_empty()
                {
                    idx.add_vector(&new.id, &vector)?;
                }
            }
            Some(HnswIndexState::Rebuilding { active, shadow }) => {
                // Remove from both if old_doc exists
                if let Some(old) = old_doc {
                    active.remove_vector(&old.id)?;
                    shadow.remove_vector(&old.id)?;
                } else if let Some(new) = new_doc {
                    active.remove_vector(&new.id)?;
                    shadow.remove_vector(&new.id)?;
                }
                // Add to both if new_doc exists
                if let Some(new) = new_doc
                    && let Some(vector) = extract_vector(new, field)
                    && !vector.is_empty()
                {
                    active.add_vector(&new.id, &vector)?;
                    shadow.add_vector(&new.id, &vector)?;
                }
            }
            None => {
                // Index doesn't exist yet, nothing to do
            }
        }

        Ok(())
    }

    fn scan_eq(
        &self,
        _keyspace: &fjall::OptimisticTxKeyspace,
        _field: &str,
        _value: &Value,
        _limit: usize,
    ) -> Result<ScanResult, StorageError> {
        // HNSW doesn't support equality scans
        Ok(ScanResult::empty())
    }

    fn scan_range(
        &self,
        _keyspace: &fjall::OptimisticTxKeyspace,
        _field: &str,
        _op: &RangeOp,
        _value: &Value,
        _limit: usize,
    ) -> Result<ScanResult, StorageError> {
        Ok(ScanResult::empty())
    }

    fn scan_range_between(
        &self,
        _keyspace: &fjall::OptimisticTxKeyspace,
        _field: &str,
        _lower: &Value,
        _lower_inclusive: bool,
        _upper: &Value,
        _upper_inclusive: bool,
        _limit: usize,
    ) -> Result<ScanResult, StorageError> {
        Ok(ScanResult::empty())
    }

    fn scan_compound_eq(
        &self,
        _keyspace: &fjall::OptimisticTxKeyspace,
        _field_values: &[(&str, &Value)],
        _limit: usize,
    ) -> Result<ScanResult, StorageError> {
        Ok(ScanResult::empty())
    }

    fn commit(&self) -> Result<(), StorageError> {
        let instances = self.instances.read();
        for state in instances.values() {
            match state {
                HnswIndexState::Active(idx) => idx.commit()?,
                HnswIndexState::Rebuilding { active, shadow } => {
                    active.commit()?;
                    shadow.commit()?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::HnswParams;
    use tempfile::TempDir;

    #[test]
    fn test_hnsw_index_creation() {
        let tmp_dir = TempDir::new().unwrap();
        let hnsw = HnswIndex::new(tmp_dir.path().to_path_buf());
        assert_eq!(hnsw.name(), "hnsw");
        assert_eq!(hnsw.index_type(), IndexType::Hnsw);
    }

    #[test]
    fn test_hnsw_add_and_search() {
        let tmp_dir = TempDir::new().unwrap();
        let hnsw = HnswIndex::new(tmp_dir.path().to_path_buf());

        let params = HnswParams::new(3);
        let instance = hnsw
            .get_or_create_instance("test", "embedding", &params)
            .unwrap();

        instance.add_vector("test:doc1", &[1.0, 0.0, 0.0]).unwrap();
        instance.add_vector("test:doc2", &[0.0, 1.0, 0.0]).unwrap();
        instance.add_vector("test:doc3", &[0.0, 0.0, 1.0]).unwrap();
        instance.commit().unwrap();

        let results = instance.search(&[1.0, 0.1, 0.0], 2, None).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "test:doc1"); // Closest to query
    }

    #[test]
    fn test_hnsw_dimension_mismatch() {
        let tmp_dir = TempDir::new().unwrap();
        let hnsw = HnswIndex::new(tmp_dir.path().to_path_buf());

        let params = HnswParams::new(3);
        let instance = hnsw
            .get_or_create_instance("test", "embedding", &params)
            .unwrap();

        // Wrong dimension
        let result = instance.add_vector("test:doc1", &[1.0, 0.0]);
        assert!(matches!(result, Err(StorageError::InvalidOperation(_))));
    }

    #[test]
    fn test_hnsw_search_dimension_mismatch() {
        let tmp_dir = TempDir::new().unwrap();
        let hnsw = HnswIndex::new(tmp_dir.path().to_path_buf());

        let params = HnswParams::new(3);
        let instance = hnsw
            .get_or_create_instance("test", "embedding", &params)
            .unwrap();

        instance.add_vector("test:doc1", &[1.0, 0.0, 0.0]).unwrap();

        // Search with wrong dimension
        let result = instance.search(&[1.0, 0.0], 2, None);
        assert!(matches!(result, Err(StorageError::InvalidOperation(_))));
    }

    #[test]
    fn test_hnsw_remove_vector() {
        let tmp_dir = TempDir::new().unwrap();
        let hnsw = HnswIndex::new(tmp_dir.path().to_path_buf());

        let params = HnswParams::new(3);
        let instance = hnsw
            .get_or_create_instance("test", "embedding", &params)
            .unwrap();

        instance.add_vector("test:doc1", &[1.0, 0.0, 0.0]).unwrap();
        instance.add_vector("test:doc2", &[0.0, 1.0, 0.0]).unwrap();

        instance.remove_vector("test:doc1").unwrap();

        let results = instance.search(&[1.0, 0.0, 0.0], 10, None).unwrap();
        // doc1 should be removed, only doc2 should be returned
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "test:doc2");
    }

    #[test]
    fn test_hnsw_high_level_search() {
        let tmp_dir = TempDir::new().unwrap();
        let hnsw = HnswIndex::new(tmp_dir.path().to_path_buf());

        let params = HnswParams::new(3);
        let instance = hnsw
            .get_or_create_instance("products", "vector", &params)
            .unwrap();

        instance
            .add_vector("products:item1", &[1.0, 0.0, 0.0])
            .unwrap();
        instance
            .add_vector("products:item2", &[0.0, 1.0, 0.0])
            .unwrap();

        // Use the high-level search method
        let results = hnsw
            .search("products", "vector", &[1.0, 0.0, 0.0], 5, None)
            .unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].0, "products:item1");
    }

    #[test]
    fn test_hnsw_index_not_found() {
        let tmp_dir = TempDir::new().unwrap();
        let hnsw = HnswIndex::new(tmp_dir.path().to_path_buf());

        // Try to search on non-existent index
        let result = hnsw.search("nonexistent", "field", &[1.0, 0.0, 0.0], 5, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_hnsw_euclidean_metric() {
        let tmp_dir = TempDir::new().unwrap();
        let hnsw = HnswIndex::new(tmp_dir.path().to_path_buf());

        let mut params = HnswParams::new(3);
        params.metric = DistanceMetric::Euclidean;
        let instance = hnsw
            .get_or_create_instance("test", "embedding", &params)
            .unwrap();

        instance.add_vector("test:doc1", &[0.0, 0.0, 0.0]).unwrap();
        instance.add_vector("test:doc2", &[1.0, 0.0, 0.0]).unwrap();
        instance.add_vector("test:doc3", &[2.0, 0.0, 0.0]).unwrap();

        let results = instance.search(&[0.5, 0.0, 0.0], 3, None).unwrap();
        assert_eq!(results.len(), 3);
        // For Euclidean/L2sq, distances should be squared
        // Distance to doc1: 0.25, doc2: 0.25, doc3: 2.25
    }

    #[test]
    fn test_hnsw_multiple_instances() {
        let tmp_dir = TempDir::new().unwrap();
        let hnsw = HnswIndex::new(tmp_dir.path().to_path_buf());

        let params1 = HnswParams::new(3);
        let params2 = HnswParams::new(5);

        let instance1 = hnsw
            .get_or_create_instance("collection1", "field1", &params1)
            .unwrap();
        let instance2 = hnsw
            .get_or_create_instance("collection2", "field2", &params2)
            .unwrap();

        instance1
            .add_vector("collection1:doc1", &[1.0, 0.0, 0.0])
            .unwrap();
        instance2
            .add_vector("collection2:doc1", &[1.0, 0.0, 0.0, 0.0, 0.0])
            .unwrap();

        // Each instance should have its own dimension
        assert!(
            instance1
                .add_vector("collection1:doc2", &[0.0, 1.0, 0.0])
                .is_ok()
        );
        assert!(
            instance2
                .add_vector("collection2:doc2", &[0.0, 1.0, 0.0, 0.0, 0.0])
                .is_ok()
        );
    }

    #[test]
    fn test_hnsw_persistence_survives_reload() {
        // This test verifies that HNSW index data survives a "restart"
        // (simulated by dropping and recreating HnswIndex)
        let tmp_dir = TempDir::new().unwrap();
        let params = HnswParams::new(3);

        // Phase 1: Create index, add vectors, save
        {
            let hnsw = HnswIndex::new(tmp_dir.path().to_path_buf());
            let instance = hnsw
                .get_or_create_instance("products", "embedding", &params)
                .unwrap();

            instance
                .add_vector("products:laptop", &[1.0, 0.0, 0.0])
                .unwrap();
            instance
                .add_vector("products:phone", &[0.0, 1.0, 0.0])
                .unwrap();
            instance
                .add_vector("products:tablet", &[0.0, 0.0, 1.0])
                .unwrap();

            // Explicitly save
            instance.save().unwrap();
        }
        // HnswIndex dropped here, simulating server shutdown

        // Phase 2: Reload and verify search still returns doc_ids
        {
            let hnsw = HnswIndex::new(tmp_dir.path().to_path_buf());
            let instance = hnsw
                .get_or_create_instance("products", "embedding", &params)
                .unwrap();

            // Search should find vectors AND return correct doc_ids
            let results = instance.search(&[1.0, 0.1, 0.0], 3, None).unwrap();

            // CRITICAL: This is the bug - after reload, id_map is empty
            // so search returns no results even though vectors are in the index
            assert_eq!(results.len(), 3, "Should find all 3 vectors after reload");
            assert_eq!(
                results[0].0, "products:laptop",
                "Closest vector should be laptop"
            );
        }
    }

    #[test]
    fn test_hnsw_persistence_with_updates() {
        // Test that updates (add/remove) are correctly persisted
        let tmp_dir = TempDir::new().unwrap();
        let params = HnswParams::new(3);

        // Phase 1: Create, add some vectors
        {
            let hnsw = HnswIndex::new(tmp_dir.path().to_path_buf());
            let instance = hnsw
                .get_or_create_instance("items", "vec", &params)
                .unwrap();

            instance.add_vector("items:a", &[1.0, 0.0, 0.0]).unwrap();
            instance.add_vector("items:b", &[0.0, 1.0, 0.0]).unwrap();
            instance.add_vector("items:c", &[0.0, 0.0, 1.0]).unwrap();
            instance.save().unwrap();
        }

        // Phase 2: Reload, remove one, add another
        {
            let hnsw = HnswIndex::new(tmp_dir.path().to_path_buf());
            let instance = hnsw
                .get_or_create_instance("items", "vec", &params)
                .unwrap();

            instance.remove_vector("items:b").unwrap();
            instance.add_vector("items:d", &[0.5, 0.5, 0.0]).unwrap();
            instance.save().unwrap();
        }

        // Phase 3: Verify final state
        {
            let hnsw = HnswIndex::new(tmp_dir.path().to_path_buf());
            let instance = hnsw
                .get_or_create_instance("items", "vec", &params)
                .unwrap();

            let results = instance.search(&[1.0, 0.0, 0.0], 10, None).unwrap();

            // Should have 3 items: a, c, d (b was removed)
            assert_eq!(results.len(), 3, "Should have 3 vectors after update");

            let doc_ids: Vec<&str> = results.iter().map(|(id, _)| id.as_str()).collect();
            assert!(doc_ids.contains(&"items:a"), "Should contain items:a");
            assert!(
                !doc_ids.contains(&"items:b"),
                "Should NOT contain items:b (removed)"
            );
            assert!(doc_ids.contains(&"items:c"), "Should contain items:c");
            assert!(doc_ids.contains(&"items:d"), "Should contain items:d (new)");
        }
    }

    #[test]
    fn test_hnsw_clear() {
        let tmp_dir = TempDir::new().unwrap();
        let hnsw = HnswIndex::new(tmp_dir.path().to_path_buf());

        let params = HnswParams::new(3);
        let instance = hnsw
            .get_or_create_instance("test", "embedding", &params)
            .unwrap();

        // Add some vectors
        instance.add_vector("test:doc1", &[1.0, 0.0, 0.0]).unwrap();
        instance.add_vector("test:doc2", &[0.0, 1.0, 0.0]).unwrap();
        instance.add_vector("test:doc3", &[0.0, 0.0, 1.0]).unwrap();

        // Verify vectors exist
        let results = instance.search(&[1.0, 0.0, 0.0], 10, None).unwrap();
        assert_eq!(results.len(), 3);

        // Clear the instance
        instance.clear().unwrap();

        // Verify all vectors are gone
        let results = instance.search(&[1.0, 0.0, 0.0], 10, None).unwrap();
        assert_eq!(results.len(), 0, "Should have no vectors after clear");

        // Verify we can add new vectors after clear
        instance.add_vector("test:new1", &[1.0, 0.0, 0.0]).unwrap();
        let results = instance.search(&[1.0, 0.0, 0.0], 10, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "test:new1");
    }

    #[test]
    fn test_hnsw_remove_index() {
        let tmp_dir = TempDir::new().unwrap();
        let hnsw = HnswIndex::new(tmp_dir.path().to_path_buf());

        let params = HnswParams::new(3);

        // Create two instances with different fields
        let instance1 = hnsw
            .get_or_create_instance("products", "embedding", &params)
            .unwrap();
        let instance2 = hnsw
            .get_or_create_instance("products", "features", &params)
            .unwrap();

        // Add vectors to both
        instance1
            .add_vector("products:item1", &[1.0, 0.0, 0.0])
            .unwrap();
        instance2
            .add_vector("products:item1", &[0.0, 1.0, 0.0])
            .unwrap();

        // Save so files exist
        instance1.save().unwrap();
        instance2.save().unwrap();

        // Remove the "embedding" index
        hnsw.remove_index("products", "embedding").unwrap();

        // "features" index should still have data
        let results = instance2.search(&[1.0, 0.0, 0.0], 10, None).unwrap();
        assert_eq!(results.len(), 1, "features index should be untouched");

        // Verify embedding index files are removed
        let embedding_path = tmp_dir
            .path()
            .join("indexes")
            .join("vector")
            .join("products")
            .join("embedding.idx");
        assert!(
            !embedding_path.exists(),
            "embedding index file should be removed"
        );
    }
}
