//! Registry for tracking secondary indexes per collection

use std::collections::HashMap;

use parking_lot::RwLock;

use crate::schema::{IndexDef, IndexType};

/// Tracks which indexes exist on each collection
#[derive(Debug, Default)]
pub struct IndexRegistry {
    /// collection_name -> list of index definitions
    indexes: RwLock<HashMap<String, Vec<IndexDef>>>,
}

impl IndexRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an index for a collection
    pub fn register(&self, collection: &str, index: IndexDef) {
        self.indexes
            .write()
            .entry(collection.to_string())
            .or_default()
            .push(index);
    }

    /// Get all indexes for a collection
    pub fn get_indexes(&self, collection: &str) -> Vec<IndexDef> {
        self.indexes
            .read()
            .get(collection)
            .cloned()
            .unwrap_or_default()
    }

    /// Get unique indexes for a collection
    pub fn get_unique_indexes(&self, collection: &str) -> Vec<IndexDef> {
        self.get_indexes(collection)
            .into_iter()
            .filter(|idx| idx.unique)
            .collect()
    }

    /// Check if an index exists
    pub fn has_index(&self, collection: &str, fields: &[String]) -> bool {
        self.indexes
            .read()
            .get(collection)
            .map(|idxs| idxs.iter().any(|i| i.fields == fields))
            .unwrap_or(false)
    }

    /// Find an index for a field (first matching)
    pub fn find_index_for_field(&self, collection: &str, field: &str) -> Option<IndexDef> {
        self.indexes.read().get(collection).and_then(|idxs| {
            idxs.iter()
                .find(|idx| !idx.fields.is_empty() && idx.fields[0] == field)
                .cloned()
        })
    }

    /// Find a compound index that starts with the given fields (in order)
    pub fn find_compound_index(&self, collection: &str, fields: &[&str]) -> Option<IndexDef> {
        if fields.is_empty() {
            return None;
        }
        self.indexes.read().get(collection).and_then(|idxs| {
            idxs.iter()
                .find(|idx| {
                    if idx.fields.len() >= fields.len() {
                        fields.iter().zip(idx.fields.iter()).all(|(a, b)| *a == b)
                    } else {
                        false
                    }
                })
                .cloned()
        })
    }

    /// Remove an index, returns true if existed
    pub fn remove(&self, collection: &str, fields: &[String]) -> bool {
        let mut indexes = self.indexes.write();
        if let Some(idxs) = indexes.get_mut(collection) {
            let len_before = idxs.len();
            idxs.retain(|i| i.fields != fields);
            return idxs.len() < len_before;
        }
        false
    }

    /// Remove all indexes for a collection
    pub fn remove_collection(&self, collection: &str) {
        self.indexes.write().remove(collection);
    }

    /// Load indexes from metadata (called at startup)
    pub fn load(&self, collection: &str, indexes: Vec<IndexDef>) {
        self.indexes.write().insert(collection.to_string(), indexes);
    }

    /// Find an FTS index that contains the specified field
    ///
    /// Returns the IndexDef if found, or None if no FTS index covers the field.
    pub fn find_fts_index_for_field(&self, collection: &str, field: &str) -> Option<IndexDef> {
        self.indexes.read().get(collection).and_then(|idxs| {
            idxs.iter()
                .find(|idx| {
                    idx.index_type == IndexType::FullText && idx.fields.contains(&field.to_string())
                })
                .cloned()
        })
    }

    /// Find any FTS index for a collection
    ///
    /// Returns the first FTS index found, or None if no FTS index exists.
    pub fn find_fts_index(&self, collection: &str) -> Option<IndexDef> {
        self.indexes.read().get(collection).and_then(|idxs| {
            idxs.iter()
                .find(|idx| idx.index_type == IndexType::FullText)
                .cloned()
        })
    }

    /// Find an HNSW index that contains the specified field
    ///
    /// Returns the IndexDef if found, or None if no HNSW index covers the field.
    pub fn find_hnsw_index_for_field(&self, collection: &str, field: &str) -> Option<IndexDef> {
        self.indexes.read().get(collection).and_then(|idxs| {
            idxs.iter()
                .find(|idx| {
                    idx.index_type == IndexType::Hnsw && idx.fields.contains(&field.to_string())
                })
                .cloned()
        })
    }

    /// Count indexes by type across all collections
    ///
    /// Returns (btree_count, vector_count, fts_count)
    pub fn count_by_type(&self) -> (usize, usize, usize) {
        let indexes = self.indexes.read();
        let mut btree = 0;
        let mut vector = 0;
        let mut fts = 0;

        for idxs in indexes.values() {
            for idx in idxs {
                match idx.index_type {
                    IndexType::BTree => btree += 1,
                    IndexType::Hnsw => vector += 1,
                    IndexType::FullText => fts += 1,
                }
            }
        }

        (btree, vector, fts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_index(collection: &str, fields: &[&str], unique: bool) -> IndexDef {
        let fields_vec: Vec<String> = fields.iter().map(|s| s.to_string()).collect();
        if unique {
            IndexDef::unique(collection, fields_vec)
        } else {
            IndexDef::new(collection, fields_vec)
        }
    }

    #[test]
    fn test_register_and_get_indexes() {
        let registry = IndexRegistry::new();

        // Initially empty
        assert!(registry.get_indexes("users").is_empty());

        // Register an index
        registry.register("users", make_index("users", &["email"], true));
        let indexes = registry.get_indexes("users");
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].fields, vec!["email".to_string()]);
        assert!(indexes[0].unique);

        // Register another index for same collection
        registry.register("users", make_index("users", &["name", "created_at"], false));
        let indexes = registry.get_indexes("users");
        assert_eq!(indexes.len(), 2);
    }

    #[test]
    fn test_has_index() {
        let registry = IndexRegistry::new();

        registry.register("users", make_index("users", &["email"], true));
        registry.register("users", make_index("users", &["name", "age"], false));

        assert!(registry.has_index("users", &["email".to_string()]));
        assert!(registry.has_index("users", &["name".to_string(), "age".to_string()]));
        assert!(!registry.has_index("users", &["nonexistent".to_string()]));
        assert!(!registry.has_index("other", &["email".to_string()]));
    }

    #[test]
    fn test_remove_index() {
        let registry = IndexRegistry::new();

        registry.register("users", make_index("users", &["email"], true));
        registry.register("users", make_index("users", &["name"], false));

        // Remove existing index
        assert!(registry.remove("users", &["email".to_string()]));
        assert!(!registry.has_index("users", &["email".to_string()]));
        assert!(registry.has_index("users", &["name".to_string()]));

        // Try to remove non-existent index
        assert!(!registry.remove("users", &["nonexistent".to_string()]));

        // Try to remove from non-existent collection
        assert!(!registry.remove("other", &["email".to_string()]));
    }

    #[test]
    fn test_remove_collection() {
        let registry = IndexRegistry::new();

        registry.register("users", make_index("users", &["email"], true));
        registry.register("users", make_index("users", &["name"], false));
        registry.register("posts", make_index("posts", &["title"], false));

        registry.remove_collection("users");

        assert!(registry.get_indexes("users").is_empty());
        assert!(!registry.get_indexes("posts").is_empty());
    }

    #[test]
    fn test_load() {
        let registry = IndexRegistry::new();

        // Pre-existing index
        registry.register("users", make_index("users", &["old"], false));

        // Load replaces all indexes for collection
        let indexes = vec![
            make_index("users", &["email"], true),
            make_index("users", &["name", "age"], false),
        ];
        registry.load("users", indexes);

        let loaded = registry.get_indexes("users");
        assert_eq!(loaded.len(), 2);
        assert!(!registry.has_index("users", &["old".to_string()]));
        assert!(registry.has_index("users", &["email".to_string()]));
    }

    #[test]
    fn test_find_index_for_field() {
        let registry = IndexRegistry::new();

        registry.register("users", make_index("users", &["email"], true));
        registry.register("users", make_index("users", &["name", "age"], false));

        let idx = registry.find_index_for_field("users", "email");
        assert!(idx.is_some());
        assert_eq!(idx.unwrap().fields, vec!["email".to_string()]);

        // Compound index found by first field
        let idx = registry.find_index_for_field("users", "name");
        assert!(idx.is_some());
        assert_eq!(
            idx.unwrap().fields,
            vec!["name".to_string(), "age".to_string()]
        );

        // Not found
        assert!(registry.find_index_for_field("users", "missing").is_none());
    }

    #[test]
    fn test_find_compound_index() {
        let registry = IndexRegistry::new();

        registry.register(
            "users",
            make_index("users", &["name", "age", "city"], false),
        );

        // Prefix match
        let idx = registry.find_compound_index("users", &["name"]);
        assert!(idx.is_some());

        let idx = registry.find_compound_index("users", &["name", "age"]);
        assert!(idx.is_some());

        let idx = registry.find_compound_index("users", &["name", "age", "city"]);
        assert!(idx.is_some());

        // Wrong order
        assert!(
            registry
                .find_compound_index("users", &["age", "name"])
                .is_none()
        );

        // Missing field
        assert!(
            registry
                .find_compound_index("users", &["missing"])
                .is_none()
        );
    }
}
