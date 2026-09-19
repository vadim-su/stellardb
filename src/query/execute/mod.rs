use std::collections::HashMap;

use crate::document::{Document, Value};

pub mod builder;
pub mod context;
pub mod control_flow;
pub mod ddl;
pub mod engine;
pub mod eval;
pub mod eval_result;
pub mod executor;
pub mod fetch;
pub mod field_path;
pub mod operators;
pub mod parent_context;
pub mod relate;
pub mod subquery;
pub mod var_scope;

pub use builder::OperatorBuilder;
pub use context::{BatchContext, DocumentOps, EdgeOps, ExecutionContext, IndexOps};
pub use engine::{execute, execute_statement, plan_from_statement};
pub use eval_result::{EvalResult, expand_value_to_rows};
pub use executor::execute_ddl;
pub use operators::scan::{IndexScanOp, KeyLookupOp, TableScanOp};
pub use operators::{
    AggregateOp, DeleteOp, FilterOp, GroupAggregateOp, InMemoryScanOp, InsertOp,
    InstrumentedOperator, LimitOp, Operator, ProjectOp, Row, SortOp, UnionOp, ValueOp, collect_all,
    document_to_json, execute_to_json,
};
pub use parent_context::ParentContext;
pub use var_scope::VarScope;

/// Result of query execution containing rows (not JSON).
#[derive(Debug)]
pub struct ExecuteResult {
    pub rows: Vec<Row>,
}

impl ExecuteResult {
    pub fn new(rows: Vec<Row>) -> Self {
        Self { rows }
    }

    pub fn empty() -> Self {
        Self { rows: Vec::new() }
    }
}

/// Create a status row with the given fields.
/// Used by DDL operations to return structured status information.
pub fn status_row(fields: Vec<(&str, Value)>) -> Vec<Row> {
    let mut map = HashMap::new();
    for (k, v) in fields {
        map.insert(k.to_string(), v);
    }
    vec![Row::from_doc(Document {
        id: String::new(),
        fields: map,
    })]
}
