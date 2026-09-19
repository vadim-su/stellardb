//! ExecutionContext implementation for Transaction
//!
//! This allows Transaction to be used as an execution context for the query engine,
//! supporting read-your-writes semantics within a transaction.

use std::cmp::Ordering;
use std::collections::HashMap;

use crate::document::{Document, Value};
use crate::edge::Edge;
use crate::error::StorageError;
use crate::query::execute::{DocumentOps, EdgeOps, ExecutionContext, IndexOps};
use crate::transaction::{PendingWrite, Transaction};

/// Resolve a dot-separated field path against a document's fields map.
fn resolve_nested_field(fields: &HashMap<String, Value>, path: &str) -> Option<Value> {
    let mut segments = path.split('.');
    let root = segments.next()?;
    let mut current = fields.get(root)?;
    for segment in segments {
        match current {
            Value::Object(map) => current = map.get(segment)?,
            _ => return None,
        }
    }
    Some(current.clone())
}

/// Get field value from document, supporting nested paths
fn get_field_value(doc: &Document, field: &str) -> Option<Value> {
    if field == "id" {
        return Some(Value::String(doc.id.clone()));
    }
    if field.contains('.') {
        resolve_nested_field(&doc.fields, field)
    } else {
        doc.fields.get(field).cloned()
    }
}

/// Check if document field matches expected value
fn field_matches(doc: &Document, field: &str, expected: &Value) -> bool {
    get_field_value(doc, field).as_ref() == Some(expected)
}

/// Compare two values for ordering
fn values_compare(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => a.partial_cmp(b),
        (Value::Float(a), Value::Float(b)) => a.partial_cmp(b),
        (Value::Int(a), Value::Float(b)) => (*a as f64).partial_cmp(b),
        (Value::Float(a), Value::Int(b)) => a.partial_cmp(&(*b as f64)),
        (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
        (Value::Bool(a), Value::Bool(b)) => a.partial_cmp(b),
        _ => None,
    }
}

/// Check if document field matches range predicate
fn field_matches_range(doc: &Document, field: &str, value: &Value, op: &str) -> bool {
    let actual = match get_field_value(doc, field) {
        Some(v) => v,
        None => return false,
    };

    let cmp = match values_compare(&actual, value) {
        Some(c) => c,
        None => return false,
    };

    match op {
        "gt" => cmp == Ordering::Greater,
        "gte" => cmp == Ordering::Greater || cmp == Ordering::Equal,
        "lt" => cmp == Ordering::Less,
        "lte" => cmp == Ordering::Less || cmp == Ordering::Equal,
        _ => false,
    }
}

/// Check if document field is between two values
fn field_matches_between(
    doc: &Document,
    field: &str,
    lower: &Value,
    lower_inclusive: bool,
    upper: &Value,
    upper_inclusive: bool,
) -> bool {
    let actual = match get_field_value(doc, field) {
        Some(v) => v,
        None => return false,
    };

    let lower_cmp = match values_compare(&actual, lower) {
        Some(c) => c,
        None => return false,
    };
    let upper_cmp = match values_compare(&actual, upper) {
        Some(c) => c,
        None => return false,
    };

    let lower_ok = if lower_inclusive {
        lower_cmp != Ordering::Less
    } else {
        lower_cmp == Ordering::Greater
    };
    let upper_ok = if upper_inclusive {
        upper_cmp != Ordering::Greater
    } else {
        upper_cmp == Ordering::Less
    };

    lower_ok && upper_ok
}

impl DocumentOps for Transaction {
    fn get_document(&self, collection: &str, key: &str) -> Result<Option<Document>, StorageError> {
        self.get(collection, key).map_err(StorageError::from)
    }

    fn list_documents(
        &self,
        collection: &str,
        limit: usize,
    ) -> Result<Vec<Document>, StorageError> {
        Transaction::list_documents(self, collection, limit).map_err(StorageError::from)
    }

    fn list_documents_paged(
        &self,
        collection: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Document>, Option<String>), StorageError> {
        // Transaction merges pending writes with storage, so we load all then paginate in memory.
        // This is acceptable because transactions are short-lived and bounded.
        let all = Transaction::list_documents(self, collection, usize::MAX - 1)
            .map_err(StorageError::from)?;

        let start = if let Some(cursor) = after {
            // Find the position after the cursor key
            all.iter()
                .position(|d| d.key() == cursor)
                .map(|i| i + 1)
                .unwrap_or(all.len())
        } else {
            0
        };

        let page: Vec<Document> = all.into_iter().skip(start).take(limit).collect();
        let next_cursor = if page.len() == limit {
            page.last().map(|d| d.key().to_string())
        } else {
            None
        };

        Ok((page, next_cursor))
    }

    fn set_document(&self, doc: &Document) -> Result<(), StorageError> {
        // Transaction uses insert/update pattern - need to handle appropriately
        let collection = doc.collection();
        let key = doc.key();

        // Check if exists to decide insert vs update
        if self
            .get(collection, key)
            .map_err(StorageError::from)?
            .is_some()
        {
            self.update(doc.clone()).map_err(StorageError::from)
        } else {
            self.insert(doc.clone()).map_err(StorageError::from)
        }
    }

    fn create_document(&self, doc: &Document) -> Result<bool, StorageError> {
        let collection = doc.collection();
        let key = doc.key();

        // Check if exists - if so, return false (already exists)
        if self
            .get(collection, key)
            .map_err(StorageError::from)?
            .is_some()
        {
            return Ok(false);
        }

        // Insert new document
        self.insert(doc.clone()).map_err(StorageError::from)?;
        Ok(true)
    }

    fn delete_document(&self, collection: &str, key: &str) -> Result<bool, StorageError> {
        // Check if exists first
        let exists = self
            .get(collection, key)
            .map_err(StorageError::from)?
            .is_some();

        if exists {
            self.delete(collection, key).map_err(StorageError::from)?;
        }
        Ok(exists)
    }

    fn get_documents_by_ids(
        &self,
        collection: &str,
        doc_ids: &[String],
    ) -> Result<Vec<Document>, StorageError> {
        let mut docs = Vec::new();
        for id in doc_ids {
            // Extract key from doc_id (format: "collection:key")
            let key = id.split(':').next_back().unwrap_or(id);
            if let Some(doc) = self.get(collection, key).map_err(StorageError::from)? {
                docs.push(doc);
            }
        }
        Ok(docs)
    }

    fn collection_exists(&self, name: &str) -> bool {
        self.storage.collection_exists(name)
    }
}

impl IndexOps for Transaction {
    // Index operations - merge storage results with pending writes
    // to support read-your-writes semantics within a transaction
    fn scan_index_eq(
        &self,
        collection: &str,
        field: &str,
        value: &Value,
        limit: usize,
    ) -> Result<(Vec<String>, bool), StorageError> {
        // Buffer for filtering - request extra to account for pending deletes
        const BUFFER: usize = 100;

        // 1. Get results from storage
        let (mut storage_ids, mut exceeded) =
            self.storage
                .scan_index_eq(collection, field, value, limit.saturating_add(BUFFER))?;

        // 2. Filter out pending deletes
        storage_ids.retain(|id| !self.is_pending_delete(id));

        // 3. Scan pending inserts, check predicate
        let pending_docs = self
            .pending_inserts_for_collection(collection)
            .map_err(StorageError::from)?;

        for doc in pending_docs {
            if field_matches(&doc, field, value) && !storage_ids.contains(&doc.id) {
                storage_ids.push(doc.id.clone());
            }
        }

        // 4. Apply limit
        if storage_ids.len() > limit {
            exceeded = true;
            storage_ids.truncate(limit);
        }

        Ok((storage_ids, exceeded))
    }

    fn scan_index_in(
        &self,
        collection: &str,
        field: &str,
        values: &[Value],
        limit: usize,
    ) -> Result<(Vec<String>, bool), StorageError> {
        let mut all_ids = Vec::new();
        let mut exceeded = false;

        for value in values {
            let (ids, exc) = self.scan_index_eq(collection, field, value, limit)?;
            all_ids.extend(ids);
            if exc {
                exceeded = true;
            }
            if all_ids.len() >= limit {
                exceeded = true;
                break;
            }
        }

        // Deduplicate
        all_ids.sort();
        all_ids.dedup();

        Ok((all_ids, exceeded))
    }

    fn scan_index_range(
        &self,
        collection: &str,
        field: &str,
        value: &Value,
        op: &str,
        limit: usize,
    ) -> Result<(Vec<String>, bool), StorageError> {
        const BUFFER: usize = 100;

        // 1. Get results from storage
        let (mut storage_ids, mut exceeded) = self.storage.scan_index_range(
            collection,
            field,
            value,
            op,
            limit.saturating_add(BUFFER),
        )?;

        // 2. Filter out pending deletes
        storage_ids.retain(|id| !self.is_pending_delete(id));

        // 3. Scan pending inserts, check range predicate
        let pending_docs = self
            .pending_inserts_for_collection(collection)
            .map_err(StorageError::from)?;

        for doc in pending_docs {
            if field_matches_range(&doc, field, value, op) && !storage_ids.contains(&doc.id) {
                storage_ids.push(doc.id.clone());
            }
        }

        // 4. Apply limit
        if storage_ids.len() > limit {
            exceeded = true;
            storage_ids.truncate(limit);
        }

        Ok((storage_ids, exceeded))
    }

    fn scan_index_range_between(
        &self,
        collection: &str,
        field: &str,
        lower: &Value,
        lower_inclusive: bool,
        upper: &Value,
        upper_inclusive: bool,
        limit: usize,
    ) -> Result<(Vec<String>, bool), StorageError> {
        const BUFFER: usize = 100;

        // 1. Get results from storage
        let (mut storage_ids, mut exceeded) = self.storage.scan_index_range_between(
            collection,
            field,
            lower,
            lower_inclusive,
            upper,
            upper_inclusive,
            limit.saturating_add(BUFFER),
        )?;

        // 2. Filter out pending deletes
        storage_ids.retain(|id| !self.is_pending_delete(id));

        // 3. Scan pending inserts, check between predicate
        let pending_docs = self
            .pending_inserts_for_collection(collection)
            .map_err(StorageError::from)?;

        for doc in pending_docs {
            if field_matches_between(&doc, field, lower, lower_inclusive, upper, upper_inclusive)
                && !storage_ids.contains(&doc.id)
            {
                storage_ids.push(doc.id.clone());
            }
        }

        // 4. Apply limit
        if storage_ids.len() > limit {
            exceeded = true;
            storage_ids.truncate(limit);
        }

        Ok((storage_ids, exceeded))
    }

    fn scan_compound_index_eq(
        &self,
        collection: &str,
        field_values: &[(&str, &Value)],
        limit: usize,
    ) -> Result<(Vec<String>, bool), StorageError> {
        const BUFFER: usize = 100;

        // 1. Get results from storage
        let (mut storage_ids, mut exceeded) = self.storage.scan_compound_index_eq(
            collection,
            field_values,
            limit.saturating_add(BUFFER),
        )?;

        // 2. Filter out pending deletes
        storage_ids.retain(|id| !self.is_pending_delete(id));

        // 3. Scan pending inserts, check ALL fields match
        let pending_docs = self
            .pending_inserts_for_collection(collection)
            .map_err(StorageError::from)?;

        for doc in pending_docs {
            let all_match = field_values
                .iter()
                .all(|(field, value)| field_matches(&doc, field, value));

            if all_match && !storage_ids.contains(&doc.id) {
                storage_ids.push(doc.id.clone());
            }
        }

        // 4. Apply limit
        if storage_ids.len() > limit {
            exceeded = true;
            storage_ids.truncate(limit);
        }

        Ok((storage_ids, exceeded))
    }

    fn scan_compound_index_eq_docs(
        &self,
        collection: &str,
        field_values: &[(&str, &Value)],
        limit: usize,
    ) -> Result<(Vec<Document>, bool), StorageError> {
        let (mut docs, _) =
            self.storage
                .scan_compound_index_eq_docs(collection, field_values, limit + 100)?;

        // Merge pending writes for read-your-writes semantics
        let pending_writes = self.pending_writes.read();

        // Filter out deleted/updated docs
        docs.retain(|doc| {
            let full_key = format!("doc:{}:{}", collection, doc.key()).into_bytes();
            !pending_writes.contains_key(&full_key)
        });

        // Add matching pending inserts
        let prefix = format!("doc:{}:", collection);
        for (full_key, write) in pending_writes.iter() {
            let key_str = String::from_utf8_lossy(full_key);
            if key_str.starts_with(&prefix)
                && let PendingWrite::Insert(bytes) = write
                && let Ok(doc) = rkyv::from_bytes::<Document, rkyv::rancor::Error>(bytes)
            {
                // Check if document matches all field_values
                let matches = field_values
                    .iter()
                    .all(|(field, value)| field_matches(&doc, field, value));
                if matches {
                    docs.push(doc);
                }
            }
        }

        let exceeded = docs.len() > limit;
        if exceeded {
            docs.truncate(limit);
        }
        Ok((docs, exceeded))
    }

    fn find_index_for_field(&self, collection: &str, field: &str) -> Option<Vec<String>> {
        self.storage
            .find_index_for_field(collection, field)
            .map(|(f, _)| vec![f])
    }

    fn find_compound_index(&self, collection: &str, fields: &[&str]) -> Option<Vec<String>> {
        self.storage.find_compound_index(collection, fields)
    }

    fn find_fts_index_for_field(&self, collection: &str, field: &str) -> Option<String> {
        self.storage.find_fts_index_for_field(collection, field)
    }

    fn find_fts_index(&self, collection: &str) -> Option<String> {
        self.storage.find_fts_index(collection)
    }

    fn fts_search(
        &self,
        collection: &str,
        index_name: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        self.storage
            .fts_search(collection, index_name, query, limit)
    }

    fn fts_search_with_boosts(
        &self,
        collection: &str,
        index_name: &str,
        query: &str,
        field_boosts: &[(String, f64)],
        limit: usize,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        self.storage
            .fts_search_with_boosts(collection, index_name, query, field_boosts, limit)
    }

    fn find_hnsw_index_for_field(&self, collection: &str, field: &str) -> Option<String> {
        self.storage.find_hnsw_index_for_field(collection, field)
    }

    fn vector_search(
        &self,
        collection: &str,
        index_name: &str,
        vector: &[f32],
        k: usize,
        effort: Option<usize>,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        self.storage
            .vector_search(collection, index_name, vector, k, effort)
    }
}

impl EdgeOps for Transaction {
    fn create_edge(&self, edge: &Edge) -> Result<(), StorageError> {
        self.create_edge(edge.clone()).map_err(StorageError::from)
    }

    fn get_edge(&self, from: &str, label: &str, to: &str) -> Result<Option<Edge>, StorageError> {
        Transaction::get_edge(self, from, label, to).map_err(StorageError::from)
    }

    fn delete_edge(&self, from: &str, label: &str, to: &str) -> Result<bool, StorageError> {
        // Check if exists first
        let exists = Transaction::get_edge(self, from, label, to)
            .map_err(StorageError::from)?
            .is_some();

        if exists {
            Transaction::delete_edge(self, from, label, to).map_err(StorageError::from)?;
        }
        Ok(exists)
    }

    fn list_edges_out(
        &self,
        from: &str,
        label: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Edge>, Option<String>), StorageError> {
        // Get edges from storage
        let (mut edges, _) = self
            .storage
            .list_edges_out(from, label, after, limit + 100)?;

        // Merge pending edge writes for read-your-writes semantics
        let pending = self.pending_edges.read();

        // Filter out edges deleted in this transaction
        edges.retain(|e| {
            let key = (e.from.clone(), e.label.clone(), e.to.clone());
            !matches!(pending.get(&key), Some(PendingWrite::Delete))
        });

        // Add pending inserts that match from/label
        for ((pfrom, plabel, _pto), write) in pending.iter() {
            if pfrom == from
                && plabel == label
                && let PendingWrite::Insert(bytes) = write
                && let Ok(edge) = rkyv::from_bytes::<Edge, rkyv::rancor::Error>(bytes)
                && !edges
                    .iter()
                    .any(|e| e.from == edge.from && e.label == edge.label && e.to == edge.to)
            {
                edges.push(edge);
            }
        }

        // Sort by target for consistent ordering
        edges.sort_by(|a, b| a.to.cmp(&b.to));

        let cursor = if edges.len() > limit {
            edges.truncate(limit);
            edges.last().map(|e| e.to.clone())
        } else {
            None
        };

        Ok((edges, cursor))
    }

    fn list_edges_in(
        &self,
        to: &str,
        label: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Edge>, Option<String>), StorageError> {
        // Get edges from storage
        let (mut edges, _) = self.storage.list_edges_in(to, label, after, limit + 100)?;

        // Merge pending edge writes for read-your-writes semantics
        let pending = self.pending_edges.read();

        // Filter out edges deleted in this transaction
        edges.retain(|e| {
            let key = (e.from.clone(), e.label.clone(), e.to.clone());
            !matches!(pending.get(&key), Some(PendingWrite::Delete))
        });

        // Add pending inserts that match to/label
        for ((_pfrom, plabel, pto), write) in pending.iter() {
            if pto == to
                && plabel == label
                && let PendingWrite::Insert(bytes) = write
                && let Ok(edge) = rkyv::from_bytes::<Edge, rkyv::rancor::Error>(bytes)
                && !edges
                    .iter()
                    .any(|e| e.from == edge.from && e.label == edge.label && e.to == edge.to)
            {
                edges.push(edge);
            }
        }

        // Sort by source for consistent ordering
        edges.sort_by(|a, b| a.from.cmp(&b.from));

        let cursor = if edges.len() > limit {
            edges.truncate(limit);
            edges.last().map(|e| e.from.clone())
        } else {
            None
        };

        Ok((edges, cursor))
    }

    fn batch_list_edges_out(
        &self,
        from_nodes: &[&str],
        label: &str,
        limit_per_node: usize,
    ) -> Result<std::collections::HashMap<String, Vec<Edge>>, StorageError> {
        let mut results = self
            .storage
            .batch_list_edges_out(from_nodes, label, limit_per_node)?;

        // Merge pending edge writes
        let pending = self.pending_edges.read();

        // Process deletions and insertions
        for node in from_nodes {
            let edges = results.entry(node.to_string()).or_default();

            // Filter out deleted edges
            edges.retain(|e| {
                let key = (e.from.clone(), e.label.clone(), e.to.clone());
                !matches!(pending.get(&key), Some(PendingWrite::Delete))
            });

            // Add pending inserts
            for ((pfrom, plabel, _pto), write) in pending.iter() {
                if pfrom.as_str() == *node
                    && plabel == label
                    && let PendingWrite::Insert(bytes) = write
                    && let Ok(edge) = rkyv::from_bytes::<Edge, rkyv::rancor::Error>(bytes)
                    && !edges
                        .iter()
                        .any(|e| e.from == edge.from && e.label == edge.label && e.to == edge.to)
                {
                    edges.push(edge);
                }
            }

            edges.sort_by(|a, b| a.to.cmp(&b.to));
            edges.truncate(limit_per_node);
        }

        Ok(results)
    }

    fn batch_list_edges_in(
        &self,
        to_nodes: &[&str],
        label: &str,
        limit_per_node: usize,
    ) -> Result<std::collections::HashMap<String, Vec<Edge>>, StorageError> {
        let mut results = self
            .storage
            .batch_list_edges_in(to_nodes, label, limit_per_node)?;

        // Merge pending edge writes
        let pending = self.pending_edges.read();

        for node in to_nodes {
            let edges = results.entry(node.to_string()).or_default();

            // Filter out deleted edges
            edges.retain(|e| {
                let key = (e.from.clone(), e.label.clone(), e.to.clone());
                !matches!(pending.get(&key), Some(PendingWrite::Delete))
            });

            // Add pending inserts
            for ((_pfrom, plabel, pto), write) in pending.iter() {
                if pto.as_str() == *node
                    && plabel == label
                    && let PendingWrite::Insert(bytes) = write
                    && let Ok(edge) = rkyv::from_bytes::<Edge, rkyv::rancor::Error>(bytes)
                    && !edges
                        .iter()
                        .any(|e| e.from == edge.from && e.label == edge.label && e.to == edge.to)
                {
                    edges.push(edge);
                }
            }

            edges.sort_by(|a, b| a.from.cmp(&b.from));
            edges.truncate(limit_per_node);
        }

        Ok(results)
    }
}

impl ExecutionContext for Transaction {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::IndexDef;
    use crate::storage::Database;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Arc<Database>) {
        let dir = TempDir::new().unwrap();
        let storage = Arc::new(Database::open(dir.path()).unwrap());
        (dir, storage)
    }

    /// Helper to create and register a BTree index
    fn create_index(storage: &Database, collection: &str, fields: &[&str]) {
        let field_strings: Vec<String> = fields.iter().map(|s| s.to_string()).collect();
        let index_def = IndexDef::new(collection, field_strings);
        storage.build_index(collection, &index_def).unwrap();
        storage.index_registry().register(collection, index_def);
    }

    #[test]
    fn test_execution_context_get_document() {
        let (_dir, storage) = setup();
        let tx = Transaction::new("tx1".to_string(), storage);

        // Insert a document through transaction
        let doc = Document {
            id: "user:alice".to_string(),
            fields: [("name".to_string(), Value::String("Alice".to_string()))]
                .into_iter()
                .collect(),
        };
        tx.insert(doc).unwrap();

        // Get through DocumentOps trait
        let result: Option<Document> = DocumentOps::get_document(&tx, "user", "alice").unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "user:alice");
    }

    #[test]
    fn test_execution_context_set_document() {
        let (_dir, storage) = setup();
        let tx = Transaction::new("tx1".to_string(), storage);

        // Set a document through DocumentOps trait
        let doc = Document {
            id: "user:bob".to_string(),
            fields: [("name".to_string(), Value::String("Bob".to_string()))]
                .into_iter()
                .collect(),
        };
        DocumentOps::set_document(&tx, &doc).unwrap();

        // Verify it's readable
        let result = tx.get("user", "bob").unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "user:bob");
    }

    #[test]
    fn test_execution_context_delete_document() {
        let (_dir, storage) = setup();
        let tx = Transaction::new("tx1".to_string(), storage);

        // Insert a document first
        let doc = Document {
            id: "user:charlie".to_string(),
            fields: [("name".to_string(), Value::String("Charlie".to_string()))]
                .into_iter()
                .collect(),
        };
        tx.insert(doc).unwrap();

        // Delete through DocumentOps trait
        let deleted = DocumentOps::delete_document(&tx, "user", "charlie").unwrap();
        assert!(deleted);

        // Verify it's gone
        let result = tx.get("user", "charlie").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_execution_context_list_documents() {
        let (_dir, storage) = setup();
        let tx = Transaction::new("tx1".to_string(), storage);

        // Insert some documents
        for i in 0..3 {
            let doc = Document {
                id: format!("user:user{}", i),
                fields: [("name".to_string(), Value::String(format!("User {}", i)))]
                    .into_iter()
                    .collect(),
            };
            tx.insert(doc).unwrap();
        }

        // List through DocumentOps trait
        let docs = DocumentOps::list_documents(&tx, "user", 10).unwrap();
        assert_eq!(docs.len(), 3);
    }

    #[test]
    fn test_scan_eq_sees_pending_insert() {
        let (_dir, storage) = setup();

        // Insert a doc first to create the collection (required for index creation)
        let seed = Document {
            id: "user:seed".to_string(),
            fields: [("age".to_string(), Value::Int(99))].into_iter().collect(),
        };
        storage.set_document(&seed).unwrap();

        // Create index on age field
        create_index(&storage, "user", &["age"]);

        let tx = Transaction::new("tx1".to_string(), storage);

        // Insert doc with age=25
        let doc = Document {
            id: "user:alice".to_string(),
            fields: [("age".to_string(), Value::Int(25))].into_iter().collect(),
        };
        tx.insert(doc).unwrap();

        // scan_index_eq should find it
        let (ids, _) = IndexOps::scan_index_eq(&tx, "user", "age", &Value::Int(25), 10).unwrap();
        assert!(ids.contains(&"user:alice".to_string()));
    }

    #[test]
    fn test_scan_eq_hides_pending_delete() {
        let (_dir, storage) = setup();

        // Insert doc into storage (this also creates the collection)
        let doc = Document {
            id: "user:bob".to_string(),
            fields: [("age".to_string(), Value::Int(30))].into_iter().collect(),
        };
        storage.set_document(&doc).unwrap();

        // Create index after doc exists
        create_index(&storage, "user", &["age"]);

        let tx = Transaction::new("tx1".to_string(), storage);

        // Delete in transaction
        tx.delete("user", "bob").unwrap();

        // scan_index_eq should NOT find it
        let (ids, _) = IndexOps::scan_index_eq(&tx, "user", "age", &Value::Int(30), 10).unwrap();
        assert!(!ids.contains(&"user:bob".to_string()));
    }

    #[test]
    fn test_scan_eq_update_uses_new_value() {
        let (_dir, storage) = setup();

        // Insert doc with age=20
        let doc = Document {
            id: "user:charlie".to_string(),
            fields: [("age".to_string(), Value::Int(20))].into_iter().collect(),
        };
        storage.set_document(&doc).unwrap();

        // Create index after doc exists
        create_index(&storage, "user", &["age"]);

        let tx = Transaction::new("tx1".to_string(), storage);

        // Update to age=25
        let updated = Document {
            id: "user:charlie".to_string(),
            fields: [("age".to_string(), Value::Int(25))].into_iter().collect(),
        };
        tx.update(updated).unwrap();

        // scan for age=25 should find it
        let (ids, _) = IndexOps::scan_index_eq(&tx, "user", "age", &Value::Int(25), 10).unwrap();
        assert!(ids.contains(&"user:charlie".to_string()));

        // scan for age=20 should NOT find it (old value)
        let (ids, _) = IndexOps::scan_index_eq(&tx, "user", "age", &Value::Int(20), 10).unwrap();
        assert!(!ids.contains(&"user:charlie".to_string()));
    }

    #[test]
    fn test_scan_eq_deduplication() {
        let (_dir, storage) = setup();

        // Insert doc
        let doc = Document {
            id: "user:dave".to_string(),
            fields: [("age".to_string(), Value::Int(40))].into_iter().collect(),
        };
        storage.set_document(&doc).unwrap();

        // Create index after doc exists
        create_index(&storage, "user", &["age"]);

        let tx = Transaction::new("tx1".to_string(), storage);

        // Update same doc (same value)
        let updated = Document {
            id: "user:dave".to_string(),
            fields: [("age".to_string(), Value::Int(40))].into_iter().collect(),
        };
        tx.update(updated).unwrap();

        // Should appear only once
        let (ids, _) = IndexOps::scan_index_eq(&tx, "user", "age", &Value::Int(40), 10).unwrap();
        let count = ids.iter().filter(|id| *id == "user:dave").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_scan_range_gt_pending_insert() {
        let (_dir, storage) = setup();

        // Insert a seed doc to create the collection
        let seed = Document {
            id: "user:seed".to_string(),
            fields: [("age".to_string(), Value::Int(99))].into_iter().collect(),
        };
        storage.set_document(&seed).unwrap();

        create_index(&storage, "user", &["age"]);

        let tx = Transaction::new("tx1".to_string(), storage);

        // Insert doc with age=30
        let doc = Document {
            id: "user:eve".to_string(),
            fields: [("age".to_string(), Value::Int(30))].into_iter().collect(),
        };
        tx.insert(doc).unwrap();

        // age > 25 should find it
        let (ids, _) =
            IndexOps::scan_index_range(&tx, "user", "age", &Value::Int(25), "gt", 10).unwrap();
        assert!(ids.contains(&"user:eve".to_string()));

        // age > 35 should NOT find it
        let (ids, _) =
            IndexOps::scan_index_range(&tx, "user", "age", &Value::Int(35), "gt", 10).unwrap();
        assert!(!ids.contains(&"user:eve".to_string()));
    }

    #[test]
    fn test_scan_range_lt_pending_insert() {
        let (_dir, storage) = setup();

        // Insert a seed doc to create the collection
        let seed = Document {
            id: "user:seed".to_string(),
            fields: [("age".to_string(), Value::Int(99))].into_iter().collect(),
        };
        storage.set_document(&seed).unwrap();

        create_index(&storage, "user", &["age"]);

        let tx = Transaction::new("tx1".to_string(), storage);

        let doc = Document {
            id: "user:frank".to_string(),
            fields: [("age".to_string(), Value::Int(20))].into_iter().collect(),
        };
        tx.insert(doc).unwrap();

        // age < 25 should find it
        let (ids, _) =
            IndexOps::scan_index_range(&tx, "user", "age", &Value::Int(25), "lt", 10).unwrap();
        assert!(ids.contains(&"user:frank".to_string()));

        // age < 15 should NOT find it
        let (ids, _) =
            IndexOps::scan_index_range(&tx, "user", "age", &Value::Int(15), "lt", 10).unwrap();
        assert!(!ids.contains(&"user:frank".to_string()));
    }

    #[test]
    fn test_scan_range_between_pending_insert() {
        let (_dir, storage) = setup();

        // Insert a seed doc to create the collection
        let seed = Document {
            id: "user:seed".to_string(),
            fields: [("age".to_string(), Value::Int(99))].into_iter().collect(),
        };
        storage.set_document(&seed).unwrap();

        create_index(&storage, "user", &["age"]);

        let tx = Transaction::new("tx1".to_string(), storage);

        let doc = Document {
            id: "user:grace".to_string(),
            fields: [("age".to_string(), Value::Int(25))].into_iter().collect(),
        };
        tx.insert(doc).unwrap();

        // age BETWEEN 20 AND 30 should find it
        let (ids, _) = IndexOps::scan_index_range_between(
            &tx,
            "user",
            "age",
            &Value::Int(20),
            true,
            &Value::Int(30),
            true,
            10,
        )
        .unwrap();
        assert!(ids.contains(&"user:grace".to_string()));

        // age BETWEEN 30 AND 40 should NOT find it
        let (ids, _) = IndexOps::scan_index_range_between(
            &tx,
            "user",
            "age",
            &Value::Int(30),
            true,
            &Value::Int(40),
            true,
            10,
        )
        .unwrap();
        assert!(!ids.contains(&"user:grace".to_string()));
    }

    #[test]
    fn test_scan_range_hides_pending_delete() {
        let (_dir, storage) = setup();

        let doc = Document {
            id: "user:henry".to_string(),
            fields: [("age".to_string(), Value::Int(35))].into_iter().collect(),
        };
        storage.set_document(&doc).unwrap();

        create_index(&storage, "user", &["age"]);

        let tx = Transaction::new("tx1".to_string(), storage);
        tx.delete("user", "henry").unwrap();

        // age > 30 should NOT find deleted doc
        let (ids, _) =
            IndexOps::scan_index_range(&tx, "user", "age", &Value::Int(30), "gt", 10).unwrap();
        assert!(!ids.contains(&"user:henry".to_string()));
    }

    #[test]
    fn test_scan_compound_all_fields_match() {
        let (_dir, storage) = setup();

        // Insert a seed doc to create the collection
        let seed = Document {
            id: "user:seed".to_string(),
            fields: [("city".to_string(), Value::String("seed".to_string()))]
                .into_iter()
                .collect(),
        };
        storage.set_document(&seed).unwrap();

        create_index(&storage, "user", &["city", "age"]);

        let tx = Transaction::new("tx1".to_string(), storage);

        let doc = Document {
            id: "user:ivan".to_string(),
            fields: [
                ("city".to_string(), Value::String("NYC".to_string())),
                ("age".to_string(), Value::Int(28)),
            ]
            .into_iter()
            .collect(),
        };
        tx.insert(doc).unwrap();

        // Both fields match
        let (ids, _) = IndexOps::scan_compound_index_eq(
            &tx,
            "user",
            &[
                ("city", &Value::String("NYC".to_string())),
                ("age", &Value::Int(28)),
            ],
            10,
        )
        .unwrap();
        assert!(ids.contains(&"user:ivan".to_string()));
    }

    #[test]
    fn test_scan_compound_partial_match_excluded() {
        let (_dir, storage) = setup();

        // Insert a seed doc to create the collection
        let seed = Document {
            id: "user:seed".to_string(),
            fields: [("city".to_string(), Value::String("seed".to_string()))]
                .into_iter()
                .collect(),
        };
        storage.set_document(&seed).unwrap();

        create_index(&storage, "user", &["city", "age"]);

        let tx = Transaction::new("tx1".to_string(), storage);

        let doc = Document {
            id: "user:julia".to_string(),
            fields: [
                ("city".to_string(), Value::String("LA".to_string())),
                ("age".to_string(), Value::Int(32)),
            ]
            .into_iter()
            .collect(),
        };
        tx.insert(doc).unwrap();

        // City matches but age doesn't
        let (ids, _) = IndexOps::scan_compound_index_eq(
            &tx,
            "user",
            &[
                ("city", &Value::String("LA".to_string())),
                ("age", &Value::Int(99)),
            ],
            10,
        )
        .unwrap();
        assert!(!ids.contains(&"user:julia".to_string()));
    }

    #[test]
    fn test_scan_nested_field() {
        let (_dir, storage) = setup();

        // Insert a seed doc to create the collection
        let seed = Document {
            id: "user:seed".to_string(),
            fields: [("name".to_string(), Value::String("seed".to_string()))]
                .into_iter()
                .collect(),
        };
        storage.set_document(&seed).unwrap();

        create_index(&storage, "user", &["address.city"]);

        let tx = Transaction::new("tx1".to_string(), storage);

        let doc = Document {
            id: "user:kate".to_string(),
            fields: [(
                "address".to_string(),
                Value::Object(
                    [("city".to_string(), Value::String("Boston".to_string()))]
                        .into_iter()
                        .collect(),
                ),
            )]
            .into_iter()
            .collect(),
        };
        tx.insert(doc).unwrap();

        let (ids, _) = IndexOps::scan_index_eq(
            &tx,
            "user",
            "address.city",
            &Value::String("Boston".to_string()),
            10,
        )
        .unwrap();
        assert!(ids.contains(&"user:kate".to_string()));
    }

    #[test]
    fn test_delete_then_reinsert_visible() {
        let (_dir, storage) = setup();

        let doc = Document {
            id: "user:leo".to_string(),
            fields: [("age".to_string(), Value::Int(45))].into_iter().collect(),
        };
        storage.set_document(&doc).unwrap();

        create_index(&storage, "user", &["age"]);

        let tx = Transaction::new("tx1".to_string(), storage);

        // Delete then re-insert with different age
        tx.delete("user", "leo").unwrap();
        let new_doc = Document {
            id: "user:leo".to_string(),
            fields: [("age".to_string(), Value::Int(46))].into_iter().collect(),
        };
        tx.insert(new_doc).unwrap();

        // Should find with new age
        let (ids, _) = IndexOps::scan_index_eq(&tx, "user", "age", &Value::Int(46), 10).unwrap();
        assert!(ids.contains(&"user:leo".to_string()));

        // Should NOT find with old age
        let (ids, _) = IndexOps::scan_index_eq(&tx, "user", "age", &Value::Int(45), 10).unwrap();
        assert!(!ids.contains(&"user:leo".to_string()));
    }

    #[test]
    fn test_limit_applied_after_merge() {
        let (_dir, storage) = setup();

        // Insert a seed doc to create the collection
        let seed = Document {
            id: "user:seed".to_string(),
            fields: [("status".to_string(), Value::String("inactive".to_string()))]
                .into_iter()
                .collect(),
        };
        storage.set_document(&seed).unwrap();

        create_index(&storage, "user", &["status"]);

        let tx = Transaction::new("tx1".to_string(), storage);

        // Insert 5 docs
        for i in 0..5 {
            let doc = Document {
                id: format!("user:user{}", i),
                fields: [("status".to_string(), Value::String("active".to_string()))]
                    .into_iter()
                    .collect(),
            };
            tx.insert(doc).unwrap();
        }

        // Request limit 3
        let (ids, exceeded) = IndexOps::scan_index_eq(
            &tx,
            "user",
            "status",
            &Value::String("active".to_string()),
            3,
        )
        .unwrap();

        assert_eq!(ids.len(), 3);
        assert!(exceeded);
    }
}
