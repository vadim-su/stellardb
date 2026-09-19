//! Keyspace management for collections

use std::collections::HashMap;
use std::sync::Arc;

use fjall::{KeyspaceCreateOptions, OptimisticTxDatabase, OptimisticTxKeyspace};
use parking_lot::RwLock;

use crate::error::StorageError;

pub(crate) const INDEX_BUILD_MARKER: &str = "__build_";

/// Holds document and unique lock keyspaces for a collection
#[derive(Clone)]
pub struct CollectionKeyspaces {
    /// Document keyspace: "docs_{collection}"
    pub docs: OptimisticTxKeyspace,
    /// Unique constraint lock keyspace: "ulock_{collection}"
    pub unique_locks: OptimisticTxKeyspace,
    /// Existence registry keyspace: "exists_{collection}"
    pub exists: OptimisticTxKeyspace,
}

/// Manages per-collection keyspaces
#[derive(Clone)]
pub struct KeyspaceManager {
    db: Arc<OptimisticTxDatabase>,
    collections: Arc<RwLock<HashMap<String, CollectionKeyspaces>>>,
    /// Index keyspaces: (collection, index_name) -> keyspace
    index_keyspaces: Arc<RwLock<HashMap<(String, String), OptimisticTxKeyspace>>>,
}

impl KeyspaceManager {
    fn copy_keyspace_batched(
        &self,
        source: &OptimisticTxKeyspace,
        target: &OptimisticTxKeyspace,
    ) -> Result<(), StorageError> {
        const COPY_BATCH_ITEMS: usize = 4_096;
        const COPY_BATCH_BYTES: usize = 64 * 1024 * 1024;

        let mut batch = self.db.inner().batch();
        let mut batch_items = 0usize;
        let mut batch_bytes = 0usize;
        for item in source.inner().iter() {
            let (key, value): (fjall::Slice, fjall::Slice) = item.into_inner()?;
            let item_bytes = key.len().saturating_add(value.len());
            if batch_items > 0
                && (batch_items >= COPY_BATCH_ITEMS
                    || batch_bytes.saturating_add(item_bytes) > COPY_BATCH_BYTES)
            {
                batch.commit()?;
                batch = self.db.inner().batch();
                batch_items = 0;
                batch_bytes = 0;
            }
            batch.insert(target.inner(), key, value);
            batch_items += 1;
            batch_bytes = batch_bytes.saturating_add(item_bytes);
        }
        batch.commit()?;
        Ok(())
    }

    /// Create a new KeyspaceManager
    pub fn new(db: Arc<OptimisticTxDatabase>) -> Self {
        Self {
            db,
            collections: Arc::new(RwLock::new(HashMap::new())),
            index_keyspaces: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get or create keyspaces for a collection
    pub fn ensure_exists(&self, name: &str) -> Result<(), StorageError> {
        // Fast path: already exists
        if self.collections.read().contains_key(name) {
            return Ok(());
        }

        // Slow path: create new keyspaces
        let mut collections = self.collections.write();

        // Double-check after acquiring write lock
        if !collections.contains_key(name) {
            let keyspaces = self.create_keyspaces(name)?;
            collections.insert(name.to_string(), keyspaces);
        }

        Ok(())
    }

    /// Create keyspaces for a collection
    fn create_keyspaces(&self, name: &str) -> Result<CollectionKeyspaces, StorageError> {
        let docs = self
            .db
            .keyspace(&format!("docs_{}", name), KeyspaceCreateOptions::default)?;
        let unique_locks = self
            .db
            .keyspace(&format!("ulock_{}", name), KeyspaceCreateOptions::default)?;
        let exists = self
            .db
            .keyspace(&format!("exists_{}", name), KeyspaceCreateOptions::default)?;

        Ok(CollectionKeyspaces {
            docs,
            unique_locks,
            exists,
        })
    }

    /// Open existing keyspaces (for loading at startup)
    pub fn open_existing(&self, name: &str) -> Result<(), StorageError> {
        let keyspaces = self.create_keyspaces(name)?;
        self.collections.write().insert(name.to_string(), keyspaces);
        Ok(())
    }

    /// Check if collection exists
    pub fn exists(&self, name: &str) -> bool {
        self.collections.read().contains_key(name)
    }

    /// List all collection names
    pub fn list(&self) -> Vec<String> {
        self.collections.read().keys().cloned().collect()
    }

    /// Get document keyspace for a collection
    pub fn docs_keyspace(&self, collection: &str) -> Option<OptimisticTxKeyspace> {
        self.collections
            .read()
            .get(collection)
            .map(|ks| ks.docs.clone())
    }

    /// Get unique locks keyspace for a collection
    pub fn unique_locks_keyspace(&self, collection: &str) -> Option<OptimisticTxKeyspace> {
        self.collections
            .read()
            .get(collection)
            .map(|ks| ks.unique_locks.clone())
    }

    /// Get existence registry keyspace for a collection
    pub fn exists_keyspace(&self, collection: &str) -> Option<OptimisticTxKeyspace> {
        self.collections
            .read()
            .get(collection)
            .map(|ks| ks.exists.clone())
    }

    /// Get all keyspaces for a collection
    pub fn get(&self, collection: &str) -> Option<CollectionKeyspaces> {
        self.collections.read().get(collection).cloned()
    }

    /// Remove a collection's keyspaces (for drop collection)
    pub fn remove(&self, collection: &str) -> Option<CollectionKeyspaces> {
        self.collections.write().remove(collection)
    }

    /// Clear all data in a collection's keyspaces
    pub fn clear(&self, collection: &str) -> Result<bool, StorageError> {
        if let Some(ks) = self.collections.read().get(collection) {
            let _ = ks.docs.inner().clear();
            let _ = ks.unique_locks.inner().clear();
            let _ = ks.exists.inner().clear();

            // Clear all index keyspaces
            self.clear_index_keyspaces(collection)?;

            Ok(true)
        } else {
            Ok(false)
        }
    }

    // =========================================================================
    // Index keyspace management methods
    // =========================================================================

    /// Create or get keyspace for a specific index
    pub fn get_or_create_index_keyspace(
        &self,
        collection: &str,
        index_name: &str,
    ) -> Result<OptimisticTxKeyspace, StorageError> {
        let key = (collection.to_string(), index_name.to_string());

        // Fast path: already exists
        if let Some(ks) = self.index_keyspaces.read().get(&key) {
            return Ok(ks.clone());
        }

        // Slow path: create new keyspace
        let mut index_keyspaces = self.index_keyspaces.write();

        // Double-check after acquiring write lock
        if let Some(ks) = index_keyspaces.get(&key) {
            return Ok(ks.clone());
        }

        let keyspace_name = format!("idx_{}_{}", collection, index_name);
        let ks = self
            .db
            .keyspace(&keyspace_name, KeyspaceCreateOptions::default)?;
        index_keyspaces.insert(key, ks.clone());

        Ok(ks)
    }

    /// Create an isolated keyspace used by an unpublished index build.
    pub fn get_or_create_index_build_keyspace(
        &self,
        collection: &str,
        build_id: &str,
    ) -> Result<OptimisticTxKeyspace, StorageError> {
        self.get_or_create_index_keyspace(collection, &format!("{INDEX_BUILD_MARKER}{build_id}"))
    }

    pub fn get_or_create_index_build_unique_keyspace(
        &self,
        collection: &str,
        build_id: &str,
    ) -> Result<OptimisticTxKeyspace, StorageError> {
        self.get_or_create_index_keyspace(
            collection,
            &format!("{INDEX_BUILD_MARKER}{build_id}_unique"),
        )
    }

    /// Copy a completed build into the final, still planner-invisible keyspace.
    /// The caller publishes the schema only after the database has durably
    /// flushed this copy.
    pub fn promote_index_build_keyspace(
        &self,
        collection: &str,
        build_id: &str,
        index_name: &str,
    ) -> Result<(), StorageError> {
        let build_name = format!("{INDEX_BUILD_MARKER}{build_id}");
        let build = self
            .get_index_keyspace(collection, &build_name)
            .ok_or_else(|| StorageError::Index(format!("index build '{build_id}' not found")))?;
        let target = self.get_or_create_index_keyspace(collection, index_name)?;

        target.inner().clear()?;
        self.copy_keyspace_batched(&build, &target)
    }

    pub fn promote_index_build_unique_keyspace(
        &self,
        collection: &str,
        build_id: &str,
    ) -> Result<(), StorageError> {
        let source_name = format!("{INDEX_BUILD_MARKER}{build_id}_unique");
        let Some(source) = self.get_index_keyspace(collection, &source_name) else {
            return Ok(());
        };
        let target = self
            .collections
            .read()
            .get(collection)
            .map(|keyspaces| keyspaces.unique_locks.clone())
            .ok_or_else(|| StorageError::CollectionNotLoaded(collection.to_string()))?;
        self.copy_keyspace_batched(&source, &target)
    }

    /// Remove all data belonging to a temporary build artifact.
    pub fn drop_index_build_keyspace(
        &self,
        collection: &str,
        build_id: &str,
    ) -> Result<(), StorageError> {
        self.drop_index_keyspace(collection, &format!("{INDEX_BUILD_MARKER}{build_id}"))?;
        self.drop_index_keyspace(
            collection,
            &format!("{INDEX_BUILD_MARKER}{build_id}_unique"),
        )
    }

    /// Discover temporary keyspaces, including artifacts that were never
    /// inserted into the in-memory map because the process died mid-build.
    pub fn list_index_build_keyspace_names(&self) -> Vec<String> {
        self.db
            .list_keyspace_names()
            .into_iter()
            .map(|name| name.to_string())
            .filter(|name| name.contains(INDEX_BUILD_MARKER))
            .collect()
    }

    /// Clear a confirmed orphan directly by its durable Fjall keyspace name.
    pub fn clear_index_build_keyspace_by_name(&self, name: &str) -> Result<(), StorageError> {
        if !name.contains(INDEX_BUILD_MARKER) || !self.db.keyspace_exists(name) {
            return Ok(());
        }
        let keyspace = self.db.keyspace(name, KeyspaceCreateOptions::default)?;
        self.db.inner().delete_keyspace(keyspace.as_ref().clone())?;
        self.index_keyspaces
            .write()
            .retain(|_, handle| handle.as_ref().name().to_string() != name);
        Ok(())
    }

    /// Get keyspace for a specific index (returns None if not exists)
    pub fn get_index_keyspace(
        &self,
        collection: &str,
        index_name: &str,
    ) -> Option<OptimisticTxKeyspace> {
        let key = (collection.to_string(), index_name.to_string());
        self.index_keyspaces.read().get(&key).cloned()
    }

    /// Drop keyspace for a specific index
    pub fn drop_index_keyspace(
        &self,
        collection: &str,
        index_name: &str,
    ) -> Result<(), StorageError> {
        let key = (collection.to_string(), index_name.to_string());

        // Delete durable metadata as well as data. Fjall keeps existing handles
        // safe until their final reference is dropped.
        if let Some(ks) = self.index_keyspaces.write().remove(&key) {
            self.db.inner().delete_keyspace(ks.as_ref().clone())?;
        }

        Ok(())
    }

    /// List all index keyspaces for a collection
    pub fn list_index_keyspaces(&self, collection: &str) -> Vec<String> {
        self.index_keyspaces
            .read()
            .keys()
            .filter(|(c, _)| c == collection)
            .map(|(_, name)| name.clone())
            .collect()
    }

    /// Clear all index keyspaces for a collection (for DROP COLLECTION)
    pub fn clear_index_keyspaces(&self, collection: &str) -> Result<(), StorageError> {
        let index_names = self.list_index_keyspaces(collection);
        for index_name in index_names {
            self.drop_index_keyspace(collection, &index_name)?;
        }
        Ok(())
    }
}
