//! Volcano-style pull-based execution operators
//!
//! Each operator implements the `Operator` trait (open/next/close)
//! and composes into execution trees via dynamic dispatch.

pub mod accumulator;
pub mod aggregate;
pub mod distinct;
pub mod dml;
pub mod filter;
pub mod fts_scan;
pub mod group_aggregate;
pub mod in_memory_scan;
pub mod instrumented;
pub mod limit;
pub mod offset;
pub mod operator;
pub mod project;
pub mod scan;
pub mod sort;
pub mod traversal;
pub mod union;
pub mod value;
pub mod vector_scan;

pub use aggregate::AggregateOp;
pub use distinct::DistinctOp;
pub use dml::{DeleteOp, InsertOp, UpdateOp};
pub use filter::FilterOp;
pub use fts_scan::FtsIndexScanOp;
pub use group_aggregate::GroupAggregateOp;
pub use in_memory_scan::InMemoryScanOp;
pub use instrumented::InstrumentedOperator;
pub use limit::LimitOp;
pub use offset::OffsetOp;
pub use operator::{
    Operator, Row, collect_all, document_to_json, execute_to_json, row_to_json, rows_to_json,
};
pub use project::ProjectOp;
pub use scan::{IndexScanOp, KeyLookupOp, TableScanOp};
pub use sort::SortOp;
pub use traversal::TraversalOp;
pub use union::UnionOp;
pub use value::ValueOp;
pub use vector_scan::{LazyVectorIndexScanOp, VectorIndexScanOp};
