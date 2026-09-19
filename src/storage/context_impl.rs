// src/storage/context_impl.rs
//! ExecutionContext implementation for Database

use crate::document::{Document, Value};
use crate::edge::Edge;
use crate::error::StorageError;
use crate::query::execute::{DocumentOps, EdgeOps, ExecutionContext, IndexOps};
use crate::storage::{Database, WriteOp};

impl DocumentOps for Database {
    fn get_document(&self, collection: &str, key: &str) -> Result<Option<Document>, StorageError> {
        Database::get_document(self, collection, key)
    }

    fn list_documents(
        &self,
        collection: &str,
        limit: usize,
    ) -> Result<Vec<Document>, StorageError> {
        let (docs, _) = Database::list_documents(self, collection, None, limit)?;
        Ok(docs)
    }

    fn list_documents_paged(
        &self,
        collection: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Document>, Option<String>), StorageError> {
        Database::list_documents(self, collection, after, limit)
    }

    fn set_document(&self, doc: &Document) -> Result<(), StorageError> {
        Database::set_document(self, doc)
    }

    fn create_document(&self, doc: &Document) -> Result<bool, StorageError> {
        Database::create_document(self, doc)
    }

    fn create_documents(&self, docs: &[Document]) -> Result<Vec<bool>, StorageError> {
        if docs.is_empty() {
            return Ok(Vec::new());
        }

        let ops = docs.iter().cloned().map(WriteOp::CreateDoc).collect();
        Database::atomic_write_batch(self, ops)?;
        Ok(vec![true; docs.len()])
    }

    fn delete_document(&self, collection: &str, key: &str) -> Result<bool, StorageError> {
        Database::delete_document(self, collection, key)
    }

    fn get_documents_by_ids(
        &self,
        collection: &str,
        doc_ids: &[String],
    ) -> Result<Vec<Document>, StorageError> {
        Database::get_documents_by_ids(self, collection, doc_ids)
    }

    fn collection_exists(&self, name: &str) -> bool {
        Database::collection_exists(self, name)
    }
}

impl IndexOps for Database {
    fn scan_index_eq(
        &self,
        collection: &str,
        field: &str,
        value: &Value,
        limit: usize,
    ) -> Result<(Vec<String>, bool), StorageError> {
        Database::scan_index_eq(self, collection, field, value, limit)
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
        Database::scan_index_range(self, collection, field, value, op, limit)
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
        Database::scan_index_range_between(
            self,
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
        Database::scan_compound_index_eq(self, collection, field_values, limit)
    }

    fn scan_compound_index_eq_docs(
        &self,
        collection: &str,
        field_values: &[(&str, &Value)],
        limit: usize,
    ) -> Result<(Vec<Document>, bool), StorageError> {
        Database::scan_compound_index_eq_docs(self, collection, field_values, limit)
    }

    fn find_index_for_field(&self, collection: &str, field: &str) -> Option<Vec<String>> {
        // Database returns Option<(String, bool)>, but trait expects Option<Vec<String>>
        // Convert the single field to a vec containing just that field
        Database::find_index_for_field(self, collection, field).map(|(f, _)| vec![f])
    }

    fn find_compound_index(&self, collection: &str, fields: &[&str]) -> Option<Vec<String>> {
        Database::find_compound_index(self, collection, fields)
    }

    fn find_fts_index_for_field(&self, collection: &str, field: &str) -> Option<String> {
        Database::find_fts_index_for_field(self, collection, field)
    }

    fn find_fts_index(&self, collection: &str) -> Option<String> {
        Database::find_fts_index(self, collection)
    }

    fn fts_search(
        &self,
        collection: &str,
        index_name: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        Database::fts_search(self, collection, index_name, query, limit)
    }

    fn fts_search_with_boosts(
        &self,
        collection: &str,
        index_name: &str,
        query: &str,
        field_boosts: &[(String, f64)],
        limit: usize,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        Database::fts_search_with_boosts(self, collection, index_name, query, field_boosts, limit)
    }

    fn find_hnsw_index_for_field(&self, collection: &str, field: &str) -> Option<String> {
        Database::find_hnsw_index_for_field(self, collection, field)
    }

    fn vector_search(
        &self,
        collection: &str,
        index_name: &str,
        vector: &[f32],
        k: usize,
        effort: Option<usize>,
    ) -> Result<Vec<(String, f32)>, StorageError> {
        Database::vector_search(self, collection, index_name, vector, k, effort)
    }
}

impl EdgeOps for Database {
    fn create_edge(&self, edge: &Edge) -> Result<(), StorageError> {
        // Use validated version to ensure from/to documents exist
        // and to enable OCC conflict detection with concurrent deletes
        Database::create_edge_validated(self, edge)
    }

    fn get_edge(&self, from: &str, label: &str, to: &str) -> Result<Option<Edge>, StorageError> {
        Database::get_edge(self, from, label, to)
    }

    fn delete_edge(&self, from: &str, label: &str, to: &str) -> Result<bool, StorageError> {
        Database::delete_edge(self, from, label, to)
    }

    fn list_edges_out(
        &self,
        from: &str,
        label: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Edge>, Option<String>), StorageError> {
        Database::list_edges_out(self, from, label, after, limit)
    }

    fn list_edges_in(
        &self,
        to: &str,
        label: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Edge>, Option<String>), StorageError> {
        Database::list_edges_in(self, to, label, after, limit)
    }

    fn batch_list_edges_out(
        &self,
        from_nodes: &[&str],
        label: &str,
        limit_per_node: usize,
    ) -> Result<std::collections::HashMap<String, Vec<Edge>>, StorageError> {
        Database::batch_list_edges_out(self, from_nodes, label, limit_per_node)
    }

    fn batch_list_edges_in(
        &self,
        to_nodes: &[&str],
        label: &str,
        limit_per_node: usize,
    ) -> Result<std::collections::HashMap<String, Vec<Edge>>, StorageError> {
        Database::batch_list_edges_in(self, to_nodes, label, limit_per_node)
    }
}

impl ExecutionContext for Database {}
