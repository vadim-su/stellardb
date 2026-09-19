//! Execution context trait for abstracting storage access
//!
//! The monolithic storage interface is split into sub-traits for narrower bounds:
//! - `DocumentOps` — CRUD operations on documents
//! - `IndexOps` — index lookups (BTree, compound, FTS, HNSW)
//! - `EdgeOps` — graph edge operations
//!
//! `ExecutionContext` is a super-trait that combines all three plus variable scope.

use crate::document::{Document, Value};
use crate::edge::Edge;
use crate::error::StorageError;

use super::VarScope;

// =============================================================================
// Sub-trait: DocumentOps
// =============================================================================

/// Document CRUD operations
pub trait DocumentOps {
    /// Get a single document by key
    fn get_document(&self, collection: &str, key: &str) -> Result<Option<Document>, StorageError>;

    /// List documents from collection
    fn list_documents(&self, collection: &str, limit: usize)
    -> Result<Vec<Document>, StorageError>;

    /// List documents with cursor-based pagination.
    /// Returns (documents, next_cursor). When next_cursor is None, there are no more results.
    fn list_documents_paged(
        &self,
        collection: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Document>, Option<String>), StorageError>;

    /// Set/update a document (upsert)
    fn set_document(&self, doc: &Document) -> Result<(), StorageError>;

    /// Create a document atomically (fails if already exists)
    /// Returns Ok(true) if created, Ok(false) if already exists
    fn create_document(&self, doc: &Document) -> Result<bool, StorageError>;

    /// Create multiple documents as one storage batch when the context supports it.
    ///
    /// Transactional and test contexts may use the default implementation, which
    /// preserves their existing read-your-writes behavior.
    fn create_documents(&self, docs: &[Document]) -> Result<Vec<bool>, StorageError> {
        docs.iter().map(|doc| self.create_document(doc)).collect()
    }

    /// Delete a document, returns true if existed
    fn delete_document(&self, collection: &str, key: &str) -> Result<bool, StorageError>;

    /// Get documents by list of IDs
    fn get_documents_by_ids(
        &self,
        collection: &str,
        doc_ids: &[String],
    ) -> Result<Vec<Document>, StorageError>;

    /// Check if a collection exists
    fn collection_exists(&self, name: &str) -> bool;
}

// =============================================================================
// Sub-trait: IndexOps
// =============================================================================

/// Index lookup operations (BTree, compound, FTS, HNSW)
pub trait IndexOps {
    /// Index scan for equality
    fn scan_index_eq(
        &self,
        collection: &str,
        field: &str,
        value: &Value,
        limit: usize,
    ) -> Result<(Vec<String>, bool), StorageError>;

    /// Scan index for multiple equality values (IN clause)
    fn scan_index_in(
        &self,
        collection: &str,
        field: &str,
        values: &[Value],
        limit: usize,
    ) -> Result<(Vec<String>, bool), StorageError>;

    /// Index scan for range
    fn scan_index_range(
        &self,
        collection: &str,
        field: &str,
        value: &Value,
        op: &str,
        limit: usize,
    ) -> Result<(Vec<String>, bool), StorageError>;

    /// Index scan for range between (field >= lower AND field <= upper)
    #[allow(clippy::too_many_arguments)]
    fn scan_index_range_between(
        &self,
        collection: &str,
        field: &str,
        lower: &Value,
        lower_inclusive: bool,
        upper: &Value,
        upper_inclusive: bool,
        limit: usize,
    ) -> Result<(Vec<String>, bool), StorageError>;

    /// Compound index scan
    fn scan_compound_index_eq(
        &self,
        collection: &str,
        field_values: &[(&str, &Value)],
        limit: usize,
    ) -> Result<(Vec<String>, bool), StorageError>;

    /// Compound index scan returning documents directly (avoids N+1)
    fn scan_compound_index_eq_docs(
        &self,
        collection: &str,
        field_values: &[(&str, &Value)],
        limit: usize,
    ) -> Result<(Vec<Document>, bool), StorageError>;

    /// Check if index exists for field
    fn find_index_for_field(&self, collection: &str, field: &str) -> Option<Vec<String>>;

    /// Find compound index
    fn find_compound_index(&self, collection: &str, fields: &[&str]) -> Option<Vec<String>>;

    /// Find FTS index that covers a specific field
    fn find_fts_index_for_field(&self, collection: &str, field: &str) -> Option<String>;

    /// Find any FTS index for a collection
    fn find_fts_index(&self, collection: &str) -> Option<String>;

    /// Search FTS index on a single field or using default index
    ///
    /// Returns (doc_id, score) pairs sorted by relevance.
    /// The index_name follows the pattern "{field}_fts" for single field indexes.
    fn fts_search(
        &self,
        collection: &str,
        index_name: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, f32)>, StorageError>;

    /// Search FTS index with field boosts
    ///
    /// Returns (doc_id, score) pairs sorted by relevance.
    /// The field_boosts specify which fields to search and their boost factors.
    fn fts_search_with_boosts(
        &self,
        collection: &str,
        index_name: &str,
        query: &str,
        field_boosts: &[(String, f64)],
        limit: usize,
    ) -> Result<Vec<(String, f32)>, StorageError>;

    /// Find HNSW index for a field
    fn find_hnsw_index_for_field(&self, collection: &str, field: &str) -> Option<String> {
        let _ = (collection, field);
        None
    }

    /// KNN vector search
    /// Returns (doc_id, distance) pairs sorted by distance.
    fn vector_search(
        &self,
        collection: &str,
        index_name: &str,
        vector: &[f32],
        k: usize,
        effort: Option<usize>,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        let _ = (collection, index_name, vector, k, effort);
        Ok(vec![])
    }
}

// =============================================================================
// Sub-trait: EdgeOps
// =============================================================================

/// Graph edge operations
pub trait EdgeOps {
    /// Create or update an edge
    fn create_edge(&self, edge: &Edge) -> Result<(), StorageError>;

    /// Get an edge
    fn get_edge(&self, from: &str, label: &str, to: &str) -> Result<Option<Edge>, StorageError>;

    /// Delete an edge
    fn delete_edge(&self, from: &str, label: &str, to: &str) -> Result<bool, StorageError>;

    /// List outgoing edges
    fn list_edges_out(
        &self,
        from: &str,
        label: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Edge>, Option<String>), StorageError>;

    /// List incoming edges
    fn list_edges_in(
        &self,
        to: &str,
        label: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Edge>, Option<String>), StorageError>;

    /// Batch load outgoing edges for multiple source nodes.
    /// Returns edges grouped by source node ID.
    /// Default implementation calls list_edges_out sequentially.
    fn batch_list_edges_out(
        &self,
        from_nodes: &[&str],
        label: &str,
        limit_per_node: usize,
    ) -> Result<std::collections::HashMap<String, Vec<Edge>>, StorageError> {
        let mut result = std::collections::HashMap::with_capacity(from_nodes.len());
        for &node in from_nodes {
            let (edges, _cursor) = self.list_edges_out(node, label, None, limit_per_node)?;
            result.insert(node.to_string(), edges);
        }
        Ok(result)
    }

    /// Batch load incoming edges for multiple target nodes.
    /// Returns edges grouped by target node ID.
    /// Default implementation calls list_edges_in sequentially.
    fn batch_list_edges_in(
        &self,
        to_nodes: &[&str],
        label: &str,
        limit_per_node: usize,
    ) -> Result<std::collections::HashMap<String, Vec<Edge>>, StorageError> {
        let mut result = std::collections::HashMap::with_capacity(to_nodes.len());
        for &node in to_nodes {
            let (edges, _cursor) = self.list_edges_in(node, label, None, limit_per_node)?;
            result.insert(node.to_string(), edges);
        }
        Ok(result)
    }
}

// =============================================================================
// Super-trait: ExecutionContext
// =============================================================================

/// Combined execution context — implements all sub-traits plus variable scope.
pub trait ExecutionContext: DocumentOps + IndexOps + EdgeOps {
    /// Get variable scope for LET variables (default: None)
    fn vars(&self) -> Option<&VarScope> {
        None
    }
}

// =============================================================================
// BatchContext
// =============================================================================

/// Execution context wrapper that adds variable scope for batch execution
pub struct BatchContext<'a, 'v, Ctx: ExecutionContext + ?Sized> {
    storage: &'a Ctx,
    vars: &'v VarScope,
}

impl<'a, 'v, Ctx: ExecutionContext + ?Sized> BatchContext<'a, 'v, Ctx> {
    /// Create a batch context with variable scope
    pub fn new(storage: &'a Ctx, vars: &'v VarScope) -> Self {
        Self { storage, vars }
    }
}

impl<Ctx: ExecutionContext + ?Sized> DocumentOps for BatchContext<'_, '_, Ctx> {
    fn get_document(&self, collection: &str, key: &str) -> Result<Option<Document>, StorageError> {
        self.storage.get_document(collection, key)
    }

    fn list_documents(
        &self,
        collection: &str,
        limit: usize,
    ) -> Result<Vec<Document>, StorageError> {
        self.storage.list_documents(collection, limit)
    }

    fn list_documents_paged(
        &self,
        collection: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Document>, Option<String>), StorageError> {
        self.storage.list_documents_paged(collection, after, limit)
    }

    fn set_document(&self, doc: &Document) -> Result<(), StorageError> {
        self.storage.set_document(doc)
    }

    fn create_document(&self, doc: &Document) -> Result<bool, StorageError> {
        self.storage.create_document(doc)
    }

    fn create_documents(&self, docs: &[Document]) -> Result<Vec<bool>, StorageError> {
        self.storage.create_documents(docs)
    }

    fn delete_document(&self, collection: &str, key: &str) -> Result<bool, StorageError> {
        self.storage.delete_document(collection, key)
    }

    fn get_documents_by_ids(
        &self,
        collection: &str,
        doc_ids: &[String],
    ) -> Result<Vec<Document>, StorageError> {
        self.storage.get_documents_by_ids(collection, doc_ids)
    }

    fn collection_exists(&self, name: &str) -> bool {
        self.storage.collection_exists(name)
    }
}

impl<Ctx: ExecutionContext + ?Sized> IndexOps for BatchContext<'_, '_, Ctx> {
    fn scan_index_eq(
        &self,
        collection: &str,
        field: &str,
        value: &Value,
        limit: usize,
    ) -> Result<(Vec<String>, bool), StorageError> {
        self.storage.scan_index_eq(collection, field, value, limit)
    }

    fn scan_index_in(
        &self,
        collection: &str,
        field: &str,
        values: &[Value],
        limit: usize,
    ) -> Result<(Vec<String>, bool), StorageError> {
        self.storage.scan_index_in(collection, field, values, limit)
    }

    fn scan_index_range(
        &self,
        collection: &str,
        field: &str,
        value: &Value,
        op: &str,
        limit: usize,
    ) -> Result<(Vec<String>, bool), StorageError> {
        self.storage
            .scan_index_range(collection, field, value, op, limit)
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
        self.storage.scan_index_range_between(
            collection,
            field,
            lower,
            lower_inclusive,
            upper,
            upper_inclusive,
            limit,
        )
    }

    fn scan_compound_index_eq(
        &self,
        collection: &str,
        field_values: &[(&str, &Value)],
        limit: usize,
    ) -> Result<(Vec<String>, bool), StorageError> {
        self.storage
            .scan_compound_index_eq(collection, field_values, limit)
    }

    fn scan_compound_index_eq_docs(
        &self,
        collection: &str,
        field_values: &[(&str, &Value)],
        limit: usize,
    ) -> Result<(Vec<Document>, bool), StorageError> {
        self.storage
            .scan_compound_index_eq_docs(collection, field_values, limit)
    }

    fn find_index_for_field(&self, collection: &str, field: &str) -> Option<Vec<String>> {
        self.storage.find_index_for_field(collection, field)
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

impl<Ctx: ExecutionContext + ?Sized> EdgeOps for BatchContext<'_, '_, Ctx> {
    fn create_edge(&self, edge: &Edge) -> Result<(), StorageError> {
        self.storage.create_edge(edge)
    }

    fn get_edge(&self, from: &str, label: &str, to: &str) -> Result<Option<Edge>, StorageError> {
        self.storage.get_edge(from, label, to)
    }

    fn delete_edge(&self, from: &str, label: &str, to: &str) -> Result<bool, StorageError> {
        self.storage.delete_edge(from, label, to)
    }

    fn list_edges_out(
        &self,
        from: &str,
        label: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Edge>, Option<String>), StorageError> {
        self.storage.list_edges_out(from, label, after, limit)
    }

    fn list_edges_in(
        &self,
        to: &str,
        label: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Edge>, Option<String>), StorageError> {
        self.storage.list_edges_in(to, label, after, limit)
    }

    fn batch_list_edges_out(
        &self,
        from_nodes: &[&str],
        label: &str,
        limit_per_node: usize,
    ) -> Result<std::collections::HashMap<String, Vec<Edge>>, StorageError> {
        self.storage
            .batch_list_edges_out(from_nodes, label, limit_per_node)
    }

    fn batch_list_edges_in(
        &self,
        to_nodes: &[&str],
        label: &str,
        limit_per_node: usize,
    ) -> Result<std::collections::HashMap<String, Vec<Edge>>, StorageError> {
        self.storage
            .batch_list_edges_in(to_nodes, label, limit_per_node)
    }
}

impl<Ctx: ExecutionContext + ?Sized> ExecutionContext for BatchContext<'_, '_, Ctx> {
    fn vars(&self) -> Option<&VarScope> {
        Some(self.vars)
    }
}

// =============================================================================
// NullContext
// =============================================================================

/// A null execution context that returns None for all document lookups.
/// Used for expression evaluation when context is not available.
pub struct NullContext;

impl DocumentOps for NullContext {
    fn get_document(&self, _: &str, _: &str) -> Result<Option<Document>, StorageError> {
        Ok(None)
    }

    fn list_documents(&self, _: &str, _: usize) -> Result<Vec<Document>, StorageError> {
        Ok(vec![])
    }

    fn list_documents_paged(
        &self,
        _: &str,
        _: Option<&str>,
        _: usize,
    ) -> Result<(Vec<Document>, Option<String>), StorageError> {
        Ok((vec![], None))
    }

    fn set_document(&self, _: &Document) -> Result<(), StorageError> {
        Ok(())
    }

    fn create_document(&self, _: &Document) -> Result<bool, StorageError> {
        Ok(false)
    }

    fn delete_document(&self, _: &str, _: &str) -> Result<bool, StorageError> {
        Ok(false)
    }

    fn get_documents_by_ids(&self, _: &str, _: &[String]) -> Result<Vec<Document>, StorageError> {
        Ok(vec![])
    }

    fn collection_exists(&self, _: &str) -> bool {
        false
    }
}

impl IndexOps for NullContext {
    fn scan_index_eq(
        &self,
        _: &str,
        _: &str,
        _: &Value,
        _: usize,
    ) -> Result<(Vec<String>, bool), StorageError> {
        Ok((vec![], false))
    }

    fn scan_index_in(
        &self,
        _: &str,
        _: &str,
        _: &[Value],
        _: usize,
    ) -> Result<(Vec<String>, bool), StorageError> {
        Ok((vec![], false))
    }

    fn scan_index_range(
        &self,
        _: &str,
        _: &str,
        _: &Value,
        _: &str,
        _: usize,
    ) -> Result<(Vec<String>, bool), StorageError> {
        Ok((vec![], false))
    }

    fn scan_index_range_between(
        &self,
        _: &str,
        _: &str,
        _: &Value,
        _: bool,
        _: &Value,
        _: bool,
        _: usize,
    ) -> Result<(Vec<String>, bool), StorageError> {
        Ok((vec![], false))
    }

    fn scan_compound_index_eq(
        &self,
        _: &str,
        _: &[(&str, &Value)],
        _: usize,
    ) -> Result<(Vec<String>, bool), StorageError> {
        Ok((vec![], false))
    }

    fn scan_compound_index_eq_docs(
        &self,
        _: &str,
        _: &[(&str, &Value)],
        _: usize,
    ) -> Result<(Vec<Document>, bool), StorageError> {
        Ok((vec![], false))
    }

    fn find_index_for_field(&self, _: &str, _: &str) -> Option<Vec<String>> {
        None
    }

    fn find_compound_index(&self, _: &str, _: &[&str]) -> Option<Vec<String>> {
        None
    }

    fn find_fts_index_for_field(&self, _: &str, _: &str) -> Option<String> {
        None
    }

    fn find_fts_index(&self, _: &str) -> Option<String> {
        None
    }

    fn fts_search(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: usize,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        Ok(vec![])
    }

    fn fts_search_with_boosts(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &[(String, f64)],
        _: usize,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        Ok(vec![])
    }
}

impl EdgeOps for NullContext {
    fn create_edge(&self, _: &Edge) -> Result<(), StorageError> {
        Ok(())
    }

    fn get_edge(&self, _: &str, _: &str, _: &str) -> Result<Option<Edge>, StorageError> {
        Ok(None)
    }

    fn delete_edge(&self, _: &str, _: &str, _: &str) -> Result<bool, StorageError> {
        Ok(false)
    }

    fn list_edges_out(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: usize,
    ) -> Result<(Vec<Edge>, Option<String>), StorageError> {
        Ok((vec![], None))
    }

    fn list_edges_in(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: usize,
    ) -> Result<(Vec<Edge>, Option<String>), StorageError> {
        Ok((vec![], None))
    }

    fn batch_list_edges_out(
        &self,
        _from_nodes: &[&str],
        _label: &str,
        _limit_per_node: usize,
    ) -> Result<std::collections::HashMap<String, Vec<Edge>>, StorageError> {
        Ok(std::collections::HashMap::new())
    }

    fn batch_list_edges_in(
        &self,
        _to_nodes: &[&str],
        _label: &str,
        _limit_per_node: usize,
    ) -> Result<std::collections::HashMap<String, Vec<Edge>>, StorageError> {
        Ok(std::collections::HashMap::new())
    }
}

impl ExecutionContext for NullContext {}
