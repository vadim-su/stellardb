//! SchemaManager - handles schema persistence and registry

use fjall::OptimisticTxKeyspace;

use crate::error::StorageError;
use crate::schema::{
    ANALYZER_PREFIX, AnalyzerDef, COLLECTION_PREFIX, CollectionSchema, IndexDef, SchemaMode,
    SchemaRegistry, analyzer_key, analyzer_name_from_key, collection_key, collection_name_from_key,
    deserialize_analyzer, deserialize_schema, serialize_analyzer, serialize_schema,
};

/// Manages schema persistence and in-memory registry
///
/// When loading schemas from storage, if deserialization of a schema fails,
/// a warning is logged and the collection is registered as schemaless rather
/// than silently dropping the error.
pub struct SchemaManager {
    meta: OptimisticTxKeyspace,
    registry: SchemaRegistry,
}

impl SchemaManager {
    /// Create a new SchemaManager
    pub fn new(meta: OptimisticTxKeyspace) -> Self {
        Self {
            meta,
            registry: SchemaRegistry::new(),
        }
    }

    /// Get the schema registry
    pub fn registry(&self) -> &SchemaRegistry {
        &self.registry
    }

    /// Load all schemas from the meta keyspace
    pub fn load_all(&self) -> Result<Vec<(String, Option<CollectionSchema>)>, StorageError> {
        let mut results = Vec::new();

        for item in self.meta.inner().prefix(COLLECTION_PREFIX) {
            let (key, value): (fjall::Slice, fjall::Slice) = item.into_inner()?;

            if let Some(name) = collection_name_from_key(&key) {
                if !value.is_empty() {
                    match deserialize_schema(&value) {
                        Ok(schema) => {
                            self.registry.register(schema.clone());
                            results.push((name, Some(schema)));
                        }
                        Err(err) => {
                            tracing::warn!(
                                collection = %name,
                                error = %err,
                                "Schema deserialization failed, falling back to schemaless"
                            );
                            self.registry.register_schemaless(&name);
                            results.push((name, None));
                        }
                    }
                } else {
                    // Empty value = schemaless collection
                    self.registry.register_schemaless(&name);
                    results.push((name, None));
                }
            }
        }

        Ok(results)
    }

    /// Get schema for a collection
    pub fn get(&self, collection: &str) -> Option<CollectionSchema> {
        self.registry.get(collection)
    }

    /// Check if collection is known (has schema or is schemaless)
    pub fn contains(&self, collection: &str) -> bool {
        self.registry.contains(collection)
    }

    /// Save a schema for a collection
    pub fn save(&self, schema: &CollectionSchema) -> Result<(), StorageError> {
        let key = collection_key(&schema.name);
        let value = serialize_schema(schema)?;
        self.meta.insert(&key, &value)?;
        self.registry.register(schema.clone());
        Ok(())
    }

    /// Save collection metadata without schema (schemaless)
    pub fn save_collection_meta(&self, collection: &str) -> Result<(), StorageError> {
        let meta_key = format!("coll\x00{}", collection);
        self.meta.insert(meta_key.as_bytes(), b"")?;
        if !self.registry.contains(collection) {
            self.registry.register_schemaless(collection);
        }
        Ok(())
    }

    /// Delete a schema
    pub fn delete(&self, collection: &str) -> Result<bool, StorageError> {
        if !self.registry.contains(collection) {
            return Ok(false);
        }

        // Remove schema metadata
        let key = collection_key(collection);
        self.meta.remove(&key)?;

        // Remove collection metadata
        let coll_key = format!("coll\x00{}", collection);
        self.meta.remove(coll_key.as_bytes())?;

        self.registry.remove(collection);
        Ok(true)
    }

    /// Atomically add an index to a collection's schema.
    /// This ensures the schema exists first, then atomically adds the index.
    /// The schema returned by registry.add_index is cloned while holding the write lock,
    /// so when we persist it, we persist the exact state that includes our addition
    /// plus any other concurrent additions that happened before we acquired the lock.
    /// Returns true if index was added.
    pub fn add_index(&self, collection: &str, index: IndexDef) -> Result<bool, StorageError> {
        // Ensure schema exists first
        self.ensure_schema(collection)?;

        // Atomically add index and get the updated schema
        // The registry returns a clone taken while holding the write lock
        if let Some(schema) = self.registry.add_index(collection, index) {
            // Persist the schema we got back (which includes all concurrent modifications)
            let key = collection_key(&schema.name);
            let value = serialize_schema(&schema)?;
            self.meta.insert(&key, &value)?;
            Ok(true)
        } else {
            // Index already exists
            Ok(false)
        }
    }

    /// Atomically remove an index from a collection's schema.
    /// Returns true if index was removed.
    pub fn remove_index(&self, collection: &str, fields: &[String]) -> Result<bool, StorageError> {
        if let Some(schema) = self.registry.remove_index(collection, fields) {
            let key = collection_key(&schema.name);
            let value = serialize_schema(&schema)?;
            self.meta.insert(&key, &value)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Ensure a schema exists for a collection, creating a flexible one if needed.
    /// Returns the schema (existing or newly created).
    pub fn ensure_schema(&self, collection: &str) -> Result<CollectionSchema, StorageError> {
        if let Some(schema) = self.registry.get(collection) {
            return Ok(schema);
        }

        // Create a default flexible schema
        let schema = CollectionSchema {
            name: collection.to_string(),
            mode: SchemaMode::Flexible,
            fields: vec![],
            indexes: vec![],
        };

        self.save(&schema)?;
        Ok(schema)
    }

    // =========================================================================
    // Analyzer persistence
    // =========================================================================

    /// Save a custom analyzer definition
    pub fn save_analyzer(&self, def: &AnalyzerDef) -> Result<(), StorageError> {
        let key = analyzer_key(&def.name);
        let bytes = serialize_analyzer(def)?;
        self.meta.insert(&key, &bytes)?;
        Ok(())
    }

    /// Get a custom analyzer definition by name
    pub fn get_analyzer(&self, name: &str) -> Option<AnalyzerDef> {
        let key = analyzer_key(name);
        self.meta
            .inner()
            .get(&key)
            .ok()
            .flatten()
            .and_then(|bytes| deserialize_analyzer(&bytes).ok())
    }

    /// List all custom analyzer names
    pub fn list_analyzers(&self) -> Vec<String> {
        let mut names = Vec::new();
        for item in self.meta.inner().prefix(ANALYZER_PREFIX) {
            if let Ok((key, _)) = item.into_inner()
                && let Some(name) = analyzer_name_from_key(&key)
            {
                names.push(name);
            }
        }
        names
    }

    /// Delete a custom analyzer
    pub fn delete_analyzer(&self, name: &str) -> Result<bool, StorageError> {
        let key = analyzer_key(name);
        let existed = self.meta.inner().get(&key)?.is_some();
        self.meta.remove(&key)?;
        Ok(existed)
    }

    /// Load all custom analyzers (called at startup)
    pub fn load_all_analyzers(&self) -> Vec<AnalyzerDef> {
        let mut analyzers = Vec::new();
        for item in self.meta.inner().prefix(ANALYZER_PREFIX) {
            if let Ok((_, bytes)) = item.into_inner()
                && let Ok(analyzer) = deserialize_analyzer(&bytes)
            {
                analyzers.push(analyzer);
            }
        }
        analyzers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Helper: create a SchemaManager backed by a temp fjall database.
    fn temp_manager() -> (SchemaManager, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = Arc::new(
            fjall::OptimisticTxDatabase::builder(tmp.path())
                .open()
                .unwrap(),
        );
        let ks = db
            .keyspace("meta", fjall::KeyspaceCreateOptions::default)
            .unwrap();
        (SchemaManager::new(ks), tmp)
    }

    #[test]
    fn test_save_and_load_schema() {
        let (mgr, _tmp) = temp_manager();

        let schema = CollectionSchema {
            name: "users".to_string(),
            mode: SchemaMode::Flexible,
            fields: vec![],
            indexes: vec![],
        };
        mgr.save(&schema).unwrap();

        let results = mgr.load_all().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "users");
        assert!(results[0].1.is_some());
        assert_eq!(results[0].1.as_ref().unwrap().name, "users");
    }

    #[test]
    fn test_corrupted_schema_falls_back_to_schemaless() {
        let (mgr, _tmp) = temp_manager();

        // Write corrupted bytes directly under a collection key
        let key = collection_key("broken");
        mgr.meta.insert(&key, b"not_valid_rkyv_data").unwrap();

        // load_all should NOT error — it logs a warning and falls back to schemaless
        let results = mgr.load_all().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "broken");
        // Should fall back to schemaless (None) instead of erroring
        assert!(results[0].1.is_none());
        // Collection should be registered in known_collections
        assert!(mgr.registry().collection_exists("broken"));
    }

    #[test]
    fn test_empty_value_is_schemaless() {
        let (mgr, _tmp) = temp_manager();

        // Empty value = explicitly schemaless
        let key = collection_key("schemaless_coll");
        mgr.meta.insert(&key, b"").unwrap();

        let results = mgr.load_all().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "schemaless_coll");
        assert!(results[0].1.is_none());
        // Collection should be registered in known_collections
        assert!(mgr.registry().collection_exists("schemaless_coll"));
    }
}
