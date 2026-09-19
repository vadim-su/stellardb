//! Index backend trait and types

use fjall::OptimisticTxKeyspace;

use crate::document::{Document, Value};
use crate::error::StorageError;

/// Result of an index scan operation
#[derive(Debug, Clone)]
pub struct ScanResult {
    /// Document IDs matching the query
    pub doc_ids: Vec<String>,
    /// True if more results exist beyond the limit
    pub exceeded_limit: bool,
}

impl ScanResult {
    /// Create a new ScanResult
    pub fn new(doc_ids: Vec<String>, exceeded_limit: bool) -> Self {
        Self {
            doc_ids,
            exceeded_limit,
        }
    }

    /// Create an empty ScanResult
    pub fn empty() -> Self {
        Self {
            doc_ids: vec![],
            exceeded_limit: false,
        }
    }
}

/// Range operation for index scans
#[derive(Debug, Clone)]
pub enum RangeOp {
    /// Greater than
    Gt,
    /// Greater than or equal
    Gte,
    /// Less than
    Lt,
    /// Less than or equal
    Lte,
    /// Between two values (inclusive/exclusive controlled by flags)
    Between {
        lower: Value,
        upper: Value,
        lower_inclusive: bool,
        upper_inclusive: bool,
    },
}

impl RangeOp {
    /// Convert string operator to RangeOp
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(op: &str) -> Option<Self> {
        match op {
            "gt" => Some(RangeOp::Gt),
            "gte" => Some(RangeOp::Gte),
            "lt" => Some(RangeOp::Lt),
            "lte" => Some(RangeOp::Lte),
            _ => None,
        }
    }

    /// Get string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            RangeOp::Gt => "gt",
            RangeOp::Gte => "gte",
            RangeOp::Lt => "lt",
            RangeOp::Lte => "lte",
            RangeOp::Between { .. } => "between",
        }
    }
}

/// Type of index backend
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum IndexType {
    /// B-tree index for scalar values (current implementation)
    #[default]
    BTree,
    /// HNSW index for vector similarity search (future)
    Hnsw,
    /// Full-text search index (future)
    FullText,
}

/// Backend for secondary indexes.
///
/// Implementors provide index operations for specific index types (B-tree, HNSW, FTS, etc.).
/// Each backend manages its own key encoding and scan logic.
pub trait IndexBackend: Send + Sync {
    /// Get the name of this index backend (for logging/metrics)
    fn name(&self) -> &'static str;

    /// Get the index type
    fn index_type(&self) -> IndexType;

    /// Build index entries for a document.
    /// Called for each document when building an index from scratch.
    fn index_document(
        &self,
        keyspace: &OptimisticTxKeyspace,
        fields: &[String],
        doc: &Document,
    ) -> Result<(), StorageError>;

    /// Update index when a document changes.
    /// Removes old entries and adds new ones.
    fn update_document(
        &self,
        keyspace: &OptimisticTxKeyspace,
        fields: &[String],
        old_doc: Option<&Document>,
        new_doc: Option<&Document>,
    ) -> Result<(), StorageError>;

    /// Scan index for equality: field = value
    fn scan_eq(
        &self,
        keyspace: &OptimisticTxKeyspace,
        field: &str,
        value: &Value,
        limit: usize,
    ) -> Result<ScanResult, StorageError>;

    /// Scan index for range: field > value, field <= value, etc.
    fn scan_range(
        &self,
        keyspace: &OptimisticTxKeyspace,
        field: &str,
        op: &RangeOp,
        value: &Value,
        limit: usize,
    ) -> Result<ScanResult, StorageError>;

    /// Scan index for range between two values
    #[allow(clippy::too_many_arguments)]
    fn scan_range_between(
        &self,
        keyspace: &OptimisticTxKeyspace,
        field: &str,
        lower: &Value,
        lower_inclusive: bool,
        upper: &Value,
        upper_inclusive: bool,
        limit: usize,
    ) -> Result<ScanResult, StorageError>;

    /// Scan compound index for equality on multiple fields
    fn scan_compound_eq(
        &self,
        keyspace: &OptimisticTxKeyspace,
        field_values: &[(&str, &Value)],
        limit: usize,
    ) -> Result<ScanResult, StorageError>;

    /// Commit any pending changes.
    ///
    /// Called after bulk indexing operations to ensure all changes are persisted.
    /// Default implementation does nothing (for backends that commit immediately).
    fn commit(&self) -> Result<(), StorageError> {
        Ok(())
    }
}
