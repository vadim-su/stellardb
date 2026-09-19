//! Thread-safe in-memory schema registry for StellarDB.

use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};

use super::{CollectionSchema, IndexDef};
use crate::util::suggestions::find_similar;

/// Thread-safe in-memory cache for collection schemas.
///
/// Tracks two things:
/// 1. `schemas` - collections with explicit schema definitions
/// 2. `known_collections` - all known collections (with or without schemas)
///
/// This allows the bind stage to distinguish between:
/// - Collection doesn't exist → error
/// - Collection exists without schema → flexible (no validation)
/// - Collection exists with schema → validate
pub struct SchemaRegistry {
    schemas: RwLock<HashMap<String, CollectionSchema>>,
    known_collections: RwLock<HashSet<String>>,
}

impl SchemaRegistry {
    /// Create a new empty schema registry.
    pub fn new() -> Self {
        SchemaRegistry {
            schemas: RwLock::new(HashMap::new()),
            known_collections: RwLock::new(HashSet::new()),
        }
    }

    /// Get a schema by collection name (cloned).
    /// Returns None if collection has no explicit schema (but may still exist).
    pub fn get(&self, collection: &str) -> Option<CollectionSchema> {
        self.schemas.read().get(collection).cloned()
    }

    /// Register a schema, inserting by its name.
    /// Also marks the collection as known.
    pub fn register(&self, schema: CollectionSchema) {
        let name = schema.name.clone();
        self.schemas.write().insert(name.clone(), schema);
        self.known_collections.write().insert(name);
    }

    /// Register a collection without a schema (schemaless/flexible).
    /// The collection will be known but have no validation.
    pub fn register_schemaless(&self, collection: &str) {
        self.known_collections
            .write()
            .insert(collection.to_string());
    }

    /// Remove a schema by collection name.
    /// Note: Does NOT remove from known_collections (collection still exists).
    pub fn remove(&self, collection: &str) -> Option<CollectionSchema> {
        self.schemas.write().remove(collection)
    }

    /// Remove a collection entirely (schema + known).
    /// Use when dropping a collection.
    pub fn remove_collection(&self, collection: &str) {
        self.schemas.write().remove(collection);
        self.known_collections.write().remove(collection);
    }

    /// Check if a schema exists for the given collection.
    pub fn contains(&self, collection: &str) -> bool {
        self.schemas.read().contains_key(collection)
    }

    /// Check if a collection is known (exists), with or without schema.
    pub fn collection_exists(&self, collection: &str) -> bool {
        self.known_collections.read().contains(collection)
    }

    /// List all registered collection names (those with schemas).
    pub fn list(&self) -> Vec<String> {
        self.schemas.read().keys().cloned().collect()
    }

    /// List all known collections (with or without schemas).
    pub fn list_all_collections(&self) -> Vec<String> {
        self.known_collections.read().iter().cloned().collect()
    }

    /// Return the number of registered schemas.
    pub fn len(&self) -> usize {
        self.schemas.read().len()
    }

    /// Check if the registry is empty (no schemas).
    pub fn is_empty(&self) -> bool {
        self.schemas.read().is_empty()
    }

    /// Return total number of known collections.
    pub fn collection_count(&self) -> usize {
        self.known_collections.read().len()
    }

    /// Atomically add an index to a collection's schema.
    /// Returns Some(updated_schema) if index was added, None if it already exists.
    /// The returned schema is a clone taken while holding the write lock, ensuring
    /// it reflects the state immediately after the modification.
    pub fn add_index(&self, collection: &str, index: IndexDef) -> Option<CollectionSchema> {
        let mut schemas = self.schemas.write();
        if let Some(schema) = schemas.get_mut(collection)
            && !schema.indexes.iter().any(|i| i.fields == index.fields)
        {
            schema.indexes.push(index);
            // Clone while holding the lock to get consistent state
            return Some(schema.clone());
        }
        None
    }

    /// Atomically remove an index from a collection's schema.
    /// Returns Some(updated_schema) if index was removed, None otherwise.
    pub fn remove_index(&self, collection: &str, fields: &[String]) -> Option<CollectionSchema> {
        let mut schemas = self.schemas.write();
        if let Some(schema) = schemas.get_mut(collection) {
            let len_before = schema.indexes.len();
            schema.indexes.retain(|i| i.fields != fields);
            if schema.indexes.len() < len_before {
                return Some(schema.clone());
            }
        }
        None
    }

    /// Find similar collection names (for error suggestions).
    pub fn find_similar_collections(&self, name: &str) -> Vec<String> {
        let collections = self.known_collections.read();
        let candidates: Vec<&str> = collections.iter().map(|s| s.as_str()).collect();
        find_similar(name, candidates.into_iter(), 3)
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }
}

impl Default for SchemaRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{FieldDef, FieldType, SchemaMode};

    fn create_test_schema(name: &str) -> CollectionSchema {
        CollectionSchema {
            name: name.to_string(),
            mode: SchemaMode::Flexible,
            fields: vec![FieldDef {
                name: "title".to_string(),
                field_type: FieldType::String,
                required: true,
                default: None,
            }],
            indexes: vec![],
        }
    }

    #[test]
    fn test_register_and_get() {
        let registry = SchemaRegistry::new();
        let schema = create_test_schema("posts");

        assert!(registry.get("posts").is_none());
        assert!(!registry.contains("posts"));

        registry.register(schema.clone());

        assert!(registry.contains("posts"));
        let retrieved = registry.get("posts").unwrap();
        assert_eq!(retrieved.name, "posts");
        assert_eq!(retrieved.fields.len(), 1);
        assert_eq!(retrieved.fields[0].name, "title");
    }

    #[test]
    fn test_remove() {
        let registry = SchemaRegistry::new();
        let schema = create_test_schema("users");

        registry.register(schema);
        assert!(registry.contains("users"));
        assert_eq!(registry.len(), 1);

        let removed = registry.remove("users");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().name, "users");

        assert!(!registry.contains("users"));
        assert!(registry.is_empty());

        // Removing non-existent schema returns None
        assert!(registry.remove("users").is_none());
    }

    #[test]
    fn test_list() {
        let registry = SchemaRegistry::new();

        assert!(registry.list().is_empty());

        registry.register(create_test_schema("posts"));
        registry.register(create_test_schema("users"));
        registry.register(create_test_schema("comments"));

        let mut names = registry.list();
        names.sort();

        assert_eq!(names.len(), 3);
        assert_eq!(names, vec!["comments", "posts", "users"]);
    }
}
