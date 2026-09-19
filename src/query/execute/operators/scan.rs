//! Scan operators for data access
//!
//! This module contains operators for reading data from storage:
//! - TableScanOp: Full table scan
//! - KeyLookupOp: Direct key lookup O(1)
//! - IndexScanOp: Index scan with threshold-based fallback

use crate::document::{Document, Value};
use crate::query::error::ExecuteError;
use crate::query::execute::context::ExecutionContext;
use crate::query::execute::operators::operator::{Operator, Row};
use crate::query::plan::{IndexLookup, IndexRef};

/// Chunk size for streaming table scans
const SCAN_CHUNK_SIZE: usize = 1000;

/// Full table scan operator
///
/// Uses cursor-based pagination to avoid loading all documents into memory at once.
/// Documents are fetched in chunks of SCAN_CHUNK_SIZE from storage.
pub struct TableScanOp<'a, Ctx: ExecutionContext> {
    ctx: &'a Ctx,
    collection: String,
    limit: Option<usize>,

    // State
    chunk: Vec<Document>,
    chunk_pos: usize,
    cursor: Option<String>,
    exhausted: bool,
    total_yielded: usize,
    opened: bool,
}

impl<'a, Ctx: ExecutionContext> TableScanOp<'a, Ctx> {
    pub fn new(ctx: &'a Ctx, collection: String, limit: Option<usize>) -> Self {
        Self {
            ctx,
            collection,
            limit,
            chunk: Vec::new(),
            chunk_pos: 0,
            cursor: None,
            exhausted: false,
            total_yielded: 0,
            opened: false,
        }
    }

    /// Fetch the next chunk of documents from storage.
    fn fetch_chunk(&mut self) -> Result<(), ExecuteError> {
        let remaining = self
            .limit
            .map(|l| l.saturating_sub(self.total_yielded))
            .unwrap_or(SCAN_CHUNK_SIZE);
        let fetch_size = remaining.min(SCAN_CHUNK_SIZE);

        if fetch_size == 0 {
            self.exhausted = true;
            return Ok(());
        }

        let (docs, next_cursor) = self
            .ctx
            .list_documents_paged(&self.collection, self.cursor.as_deref(), fetch_size)
            .map_err(ExecuteError::Storage)?;

        self.exhausted = next_cursor.is_none();
        self.cursor = next_cursor;
        self.chunk = docs;
        self.chunk_pos = 0;
        Ok(())
    }
}

impl<Ctx: ExecutionContext> Operator for TableScanOp<'_, Ctx> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        self.chunk.clear();
        self.chunk_pos = 0;
        self.cursor = None;
        self.exhausted = false;
        self.total_yielded = 0;
        self.opened = true;

        // Fetch the first chunk
        self.fetch_chunk()?;
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        if !self.opened {
            return Err(ExecuteError::OperatorNotInitialized("TableScanOp"));
        }

        // Check limit
        if let Some(limit) = self.limit
            && self.total_yielded >= limit
        {
            return Ok(None);
        }

        // If current chunk is exhausted, fetch the next one
        if self.chunk_pos >= self.chunk.len() {
            if self.exhausted {
                return Ok(None);
            }
            self.fetch_chunk()?;
            if self.chunk.is_empty() {
                return Ok(None);
            }
        }

        let doc = self.chunk[self.chunk_pos].clone();
        self.chunk_pos += 1;
        self.total_yielded += 1;
        Ok(Some(Row::from_doc(doc)))
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        self.chunk = Vec::new();
        self.chunk_pos = 0;
        self.cursor = None;
        self.exhausted = false;
        self.total_yielded = 0;
        self.opened = false;
        Ok(())
    }
}

/// Direct key lookup operator
pub struct KeyLookupOp<'a, Ctx: ExecutionContext> {
    ctx: &'a Ctx,
    collection: String,
    key: String,

    // State
    doc: Option<Option<Document>>,
    returned: bool,
}

impl<'a, Ctx: ExecutionContext> KeyLookupOp<'a, Ctx> {
    pub fn new(ctx: &'a Ctx, collection: String, key: String) -> Self {
        Self {
            ctx,
            collection,
            key,
            doc: None,
            returned: false,
        }
    }
}

impl<Ctx: ExecutionContext> Operator for KeyLookupOp<'_, Ctx> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        let doc = self
            .ctx
            .get_document(&self.collection, &self.key)
            .map_err(ExecuteError::Storage)?;
        self.doc = Some(doc);
        self.returned = false;
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        if self.returned {
            return Ok(None);
        }
        self.returned = true;

        let doc = self
            .doc
            .as_ref()
            .ok_or(ExecuteError::OperatorNotInitialized("KeyLookupOp"))?;

        Ok(doc.clone().map(Row::from_doc))
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        self.doc = None;
        Ok(())
    }
}

/// Index scan operator with threshold-based fallback
///
/// Scans an index for matching document IDs, then batch-fetches the documents.
/// If the number of results exceeds the threshold, the `exceeded_threshold`
/// flag is set, which can be used by the planner to fall back to a table scan.
pub struct IndexScanOp<'a, Ctx: ExecutionContext> {
    ctx: &'a Ctx,
    collection: String,
    lookup: IndexLookup,
    threshold: usize,

    // State - pre-fetched documents (batch-fetched for both single and compound indexes)
    docs: Vec<Document>,
    position: usize,
    exceeded_threshold: bool,
}

impl<'a, Ctx: ExecutionContext> IndexScanOp<'a, Ctx> {
    pub fn new(ctx: &'a Ctx, collection: String, _index: IndexRef, lookup: IndexLookup) -> Self {
        Self {
            ctx,
            collection,
            lookup,
            threshold: 1000, // INDEX_RESULT_THRESHOLD
            docs: Vec::new(),
            position: 0,
            exceeded_threshold: false,
        }
    }

    pub fn with_threshold(mut self, threshold: usize) -> Self {
        self.threshold = threshold;
        self
    }

    /// Check if index scan exceeded threshold (for fallback decision)
    pub fn exceeded_threshold(&self) -> bool {
        self.exceeded_threshold
    }
}

impl<Ctx: ExecutionContext> Operator for IndexScanOp<'_, Ctx> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        match &self.lookup {
            IndexLookup::CompoundEq { field_values } => {
                // Compound index: fetch documents directly
                let refs: Vec<(&str, &Value)> =
                    field_values.iter().map(|(f, v)| (f.as_str(), v)).collect();
                let (docs, exceeded) = self
                    .ctx
                    .scan_compound_index_eq_docs(&self.collection, &refs, self.threshold)
                    .map_err(ExecuteError::Storage)?;
                self.docs = docs;
                self.exceeded_threshold = exceeded;
            }
            _ => {
                // Single-field index: get doc_ids, then batch-fetch documents
                let (ids, exceeded) = match &self.lookup {
                    IndexLookup::Eq { field, value } => self
                        .ctx
                        .scan_index_eq(&self.collection, field, value, self.threshold)
                        .map_err(ExecuteError::Storage)?,
                    IndexLookup::Range { field, op, value } => self
                        .ctx
                        .scan_index_range(
                            &self.collection,
                            field,
                            value,
                            op.as_str(),
                            self.threshold,
                        )
                        .map_err(ExecuteError::Storage)?,
                    IndexLookup::RangeBetween {
                        field,
                        lower,
                        lower_inclusive,
                        upper,
                        upper_inclusive,
                    } => self
                        .ctx
                        .scan_index_range_between(
                            &self.collection,
                            field,
                            lower,
                            *lower_inclusive,
                            upper,
                            *upper_inclusive,
                            self.threshold,
                        )
                        .map_err(ExecuteError::Storage)?,
                    IndexLookup::CompoundEq { .. } => unreachable!(),
                    IndexLookup::In { field, values } => self
                        .ctx
                        .scan_index_in(&self.collection, field, values, self.threshold)
                        .map_err(ExecuteError::Storage)?,
                };

                // Batch-fetch all documents at once instead of N+1 individual lookups
                self.docs = self
                    .ctx
                    .get_documents_by_ids(&self.collection, &ids)
                    .map_err(ExecuteError::Storage)?;
                self.exceeded_threshold = exceeded;
            }
        }

        self.position = 0;
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        if self.position >= self.docs.len() {
            return Ok(None);
        }

        let doc = self.docs[self.position].clone();
        self.position += 1;
        Ok(Some(Row::from_doc(doc)))
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        self.docs.clear();
        self.position = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Arc<Database>) {
        let tmp = TempDir::new().unwrap();
        let storage = Arc::new(Database::open(tmp.path()).unwrap());
        (tmp, storage)
    }

    #[test]
    fn test_table_scan_empty() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION test");

        let mut op = TableScanOp::new(&*storage, "test".to_string(), None);
        op.open().unwrap();
        assert!(op.next().unwrap().is_none());
        op.close().unwrap();
    }

    #[test]
    fn test_table_scan_with_data() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION test");
        crate::run_sql!(storage, "INSERT INTO test {id: '1', name: 'Alice'}");
        crate::run_sql!(storage, "INSERT INTO test {id: '2', name: 'Bob'}");

        let mut op = TableScanOp::new(&*storage, "test".to_string(), None);
        op.open().unwrap();

        assert!(op.next().unwrap().is_some());
        assert!(op.next().unwrap().is_some());
        assert!(op.next().unwrap().is_none());

        op.close().unwrap();
    }

    #[test]
    fn test_table_scan_with_limit() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION test");
        crate::run_sql!(storage, "INSERT INTO test {id: '1'}, {id: '2'}, {id: '3'}");

        let mut op = TableScanOp::new(&*storage, "test".to_string(), Some(2));
        op.open().unwrap();

        assert!(op.next().unwrap().is_some());
        assert!(op.next().unwrap().is_some());
        assert!(op.next().unwrap().is_none()); // Limited to 2

        op.close().unwrap();
    }

    #[test]
    fn test_table_scan_many_docs() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION test");

        // Insert 50 documents
        for i in 0..50 {
            crate::run_sql!(
                storage,
                &format!("INSERT INTO test {{id: '{:04}', val: {}}}", i, i)
            );
        }

        let mut op = TableScanOp::new(&*storage, "test".to_string(), None);
        op.open().unwrap();

        let mut count = 0;
        while op.next().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 50);
        op.close().unwrap();
    }

    #[test]
    fn test_table_scan_reopen() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION test");
        crate::run_sql!(storage, "INSERT INTO test {id: '1'}, {id: '2'}");

        let mut op = TableScanOp::new(&*storage, "test".to_string(), None);

        // First scan
        op.open().unwrap();
        let mut count = 0;
        while op.next().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 2);
        op.close().unwrap();

        // Re-open and scan again - should see all docs again
        op.open().unwrap();
        count = 0;
        while op.next().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 2);
        op.close().unwrap();
    }

    #[test]
    fn test_key_lookup_found() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION test");
        crate::run_sql!(storage, "INSERT INTO test {id: 'alice', name: 'Alice'}");

        let mut op = KeyLookupOp::new(&*storage, "test".to_string(), "alice".to_string());
        op.open().unwrap();

        let row = op.next().unwrap();
        assert!(row.is_some());
        assert_eq!(row.unwrap().doc.id, "test:alice");

        assert!(op.next().unwrap().is_none());
        op.close().unwrap();
    }

    #[test]
    fn test_key_lookup_not_found() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION test");

        let mut op = KeyLookupOp::new(&*storage, "test".to_string(), "notfound".to_string());
        op.open().unwrap();
        assert!(op.next().unwrap().is_none());
        op.close().unwrap();
    }

    #[test]
    fn test_index_scan_eq() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(status)");
        crate::run_sql!(
            storage,
            "INSERT INTO users {id: 'alice', status: 'active'}, {id: 'bob', status: 'inactive'}, {id: 'charlie', status: 'active'}"
        );

        let index = IndexRef {
            collection: "users".to_string(),
            fields: vec!["status".to_string()],
        };
        let lookup = IndexLookup::Eq {
            field: "status".to_string(),
            value: Value::String("active".to_string()),
        };

        let mut op = IndexScanOp::new(&*storage, "users".to_string(), index, lookup);
        op.open().unwrap();
        assert!(!op.exceeded_threshold());

        // Should find two active users
        let mut count = 0;
        while op.next().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 2);
        op.close().unwrap();
    }

    #[test]
    fn test_index_scan_threshold() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION items");
        crate::run_sql!(storage, "CREATE INDEX ON items(category)");

        // Insert 10 items with same category
        for i in 0..10 {
            crate::run_sql!(
                storage,
                &format!("INSERT INTO items {{id: 'item{}', category: 'widgets'}}", i)
            );
        }

        let index = IndexRef {
            collection: "items".to_string(),
            fields: vec!["category".to_string()],
        };
        let lookup = IndexLookup::Eq {
            field: "category".to_string(),
            value: Value::String("widgets".to_string()),
        };

        // With threshold of 5, should exceed threshold
        let mut op =
            IndexScanOp::new(&*storage, "items".to_string(), index, lookup).with_threshold(5);
        op.open().unwrap();
        assert!(op.exceeded_threshold());
        op.close().unwrap();
    }

    #[test]
    fn test_index_scan_no_results() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION products");
        crate::run_sql!(storage, "CREATE INDEX ON products(type)");

        let index = IndexRef {
            collection: "products".to_string(),
            fields: vec!["type".to_string()],
        };
        let lookup = IndexLookup::Eq {
            field: "type".to_string(),
            value: Value::String("nonexistent".to_string()),
        };

        let mut op = IndexScanOp::new(&*storage, "products".to_string(), index, lookup);
        op.open().unwrap();
        assert!(!op.exceeded_threshold());
        assert!(op.next().unwrap().is_none());
        op.close().unwrap();
    }

    #[test]
    fn test_index_scan_in() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(status)");
        crate::run_sql!(
            storage,
            "INSERT INTO users {id: 'a', status: 'active'}, {id: 'b', status: 'pending'}, {id: 'c', status: 'inactive'}, {id: 'd', status: 'active'}"
        );

        let index = IndexRef {
            collection: "users".to_string(),
            fields: vec!["status".to_string()],
        };
        let lookup = IndexLookup::In {
            field: "status".to_string(),
            values: vec![
                Value::String("active".to_string()),
                Value::String("pending".to_string()),
            ],
        };

        let mut op = IndexScanOp::new(&*storage, "users".to_string(), index, lookup);
        op.open().unwrap();

        let mut count = 0;
        while op.next().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 3); // 2 active + 1 pending
        op.close().unwrap();
    }
}
