//! Physical execution plan - specifies HOW to execute

use crate::document::Value;
use crate::query::ast::expr::{FtsOperator, FtsTarget};
use crate::query::ast::{Assignment, Expr, ObjectLiteral, OrderItem, Projection};
use crate::query::plan::logical::ResolvedAggregate;

/// Physical operator - specifies HOW to execute
#[derive(Debug, Clone)]
pub enum PhysicalOp {
    // === Data Access ===
    /// Full table scan
    TableScan {
        collection: String,
        limit: Option<usize>,
    },

    /// Direct key lookup O(1)
    KeyLookup { collection: String, key: String },

    /// Index scan with early termination
    IndexScan {
        collection: String,
        index: IndexRef,
        lookup: IndexLookup,
    },

    /// In-memory scan over pre-materialized rows
    InMemoryScan {
        rows: Vec<crate::query::execute::operators::operator::Row>,
    },

    /// Full-text search index scan
    FtsScan {
        collection: String,
        target: FtsTarget,
        operator: FtsOperator,
        query: String,
        limit: Option<usize>,
    },

    /// KNN vector search scan
    KnnScan {
        collection: String,
        field: String,
        vector: Expr,
        k: usize,
        effort: Option<usize>,
    },

    /// Union of multiple scans (for OR optimization)
    Union {
        inputs: Vec<PhysicalOp>,
        needs_dedup: bool,
    },

    // === Transforms ===
    /// Filter rows by predicate
    Filter {
        predicate: Expr,
        input: Box<PhysicalOp>,
    },

    /// Project specific fields
    Project {
        fields: Projection,
        input: Box<PhysicalOp>,
    },

    /// Limit output rows
    Limit {
        count: usize,
        input: Box<PhysicalOp>,
    },

    /// Skip first N rows
    Offset {
        count: usize,
        input: Box<PhysicalOp>,
    },

    /// Sort rows by fields
    Sort {
        items: Vec<OrderItem>,
        input: Box<PhysicalOp>,
    },

    /// Aggregate: compute aggregate functions over input
    Aggregate {
        aggregates: Vec<ResolvedAggregate>,
        input: Box<PhysicalOp>,
    },

    /// Group aggregate: compute aggregate functions per group
    GroupAggregate {
        group_fields: Vec<String>,
        aggregates: Vec<ResolvedAggregate>,
        input: Box<PhysicalOp>,
    },

    /// Remove duplicate rows based on all fields
    Distinct { input: Box<PhysicalOp> },

    /// Extract single value from each row (SELECT VALUE)
    Value {
        expr: Expr,
        distinct: bool,
        input: Box<PhysicalOp>,
    },

    // === DML ===
    /// Insert documents
    Insert {
        collection: String,
        documents: Vec<ObjectLiteral>,
    },

    /// Create a document after evaluating its assignments.
    Create {
        collection: String,
        key: Option<String>,
        assignments: Vec<Assignment>,
    },

    /// Update documents (input provides docs to update)
    Update {
        collection: String,
        assignments: Vec<Assignment>,
        input: Box<PhysicalOp>,
    },

    /// Upsert document (create or merge/replace)
    Upsert {
        collection: String,
        key: String,
        assignments: Vec<Assignment>,
        replace: bool,
    },

    /// Delete documents (input provides docs to delete)
    Delete {
        collection: String,
        input: Box<PhysicalOp>,
    },

    /// Delete edge(s)
    DeleteEdge {
        from: String,
        label: String,
        to: Option<String>,
    },
}

/// Reference to an index
#[derive(Debug, Clone)]
pub struct IndexRef {
    pub collection: String,
    pub fields: Vec<String>,
}

/// How to look up in the index
#[derive(Debug, Clone)]
pub enum IndexLookup {
    /// Equality: field = value
    Eq { field: String, value: Value },

    /// Range: field > value, field >= value, etc.
    Range {
        field: String,
        op: RangeOp,
        value: Value,
    },

    /// Range between: field >= lower AND field <= upper
    RangeBetween {
        field: String,
        lower: Value,
        lower_inclusive: bool,
        upper: Value,
        upper_inclusive: bool,
    },

    /// Compound equality: f1 = v1 AND f2 = v2
    CompoundEq { field_values: Vec<(String, Value)> },

    /// IN list: field IN (v1, v2, v3, ...)
    In { field: String, values: Vec<Value> },
}

/// Range operation type
#[derive(Debug, Clone, Copy)]
pub enum RangeOp {
    Gt,
    Gte,
    Lt,
    Lte,
}

impl RangeOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            RangeOp::Gt => "gt",
            RangeOp::Gte => "gte",
            RangeOp::Lt => "lt",
            RangeOp::Lte => "lte",
        }
    }
}
