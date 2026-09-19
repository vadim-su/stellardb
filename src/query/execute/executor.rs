use crate::Database;
use crate::query::error::ExecuteError;
use crate::query::plan::LogicalPlan;

use super::Row;
use super::ddl::{
    execute_create_index, execute_define_collection, execute_describe_collection,
    execute_describe_collections, execute_drop_collection, execute_drop_index, execute_reindex,
};

/// Execute a logical plan (DDL operations only).
///
/// DML operations (Scan, Insert, Create, Update, Delete) are handled by `execute`
/// in engine.rs and should never reach this function.
pub fn execute_ddl(plan: LogicalPlan, storage: &Database) -> Result<Vec<Row>, ExecuteError> {
    match plan {
        // DML should not reach here - routed to execute in engine.rs
        LogicalPlan::Scan { .. }
        | LogicalPlan::Aggregate { .. }
        | LogicalPlan::GroupAggregate { .. }
        | LogicalPlan::InMemoryScan { .. }
        | LogicalPlan::Insert { .. }
        | LogicalPlan::Create { .. }
        | LogicalPlan::Update { .. }
        | LogicalPlan::Upsert { .. }
        | LogicalPlan::Delete { .. }
        | LogicalPlan::DeleteEdge { .. } => {
            unreachable!("DML operations should be routed to execute in engine.rs")
        }

        // Transaction control
        LogicalPlan::Begin => Err(ExecuteError::TransactionNotSupported("BEGIN")),
        LogicalPlan::Commit => Err(ExecuteError::TransactionNotSupported("COMMIT")),
        LogicalPlan::Rollback => Err(ExecuteError::TransactionNotSupported("ROLLBACK")),

        // DDL operations
        LogicalPlan::DefineCollection {
            name,
            mode,
            fields,
            indexes,
        } => execute_define_collection(storage, name, mode, fields, indexes),
        LogicalPlan::DropCollection { name, cascade } => {
            execute_drop_collection(storage, &name, cascade)
        }
        LogicalPlan::DescribeCollection(name) => execute_describe_collection(storage, &name),
        LogicalPlan::DescribeCollections => execute_describe_collections(storage),

        // Index operations
        LogicalPlan::CreateIndex {
            collection,
            fields,
            unique,
            index_type,
            hnsw_params,
            analyzer,
        } => execute_create_index(
            storage,
            &collection,
            fields,
            unique,
            index_type,
            hnsw_params,
            analyzer,
        ),
        LogicalPlan::DropIndex { name, collection } => {
            execute_drop_index(storage, &collection, &name)
        }

        // EXPLAIN is handled in engine.rs, not DDL executor
        LogicalPlan::Explain { .. } => {
            unreachable!("EXPLAIN should be handled in engine.rs")
        }

        // LET is handled at session/engine level
        LogicalPlan::Let { .. } => {
            unreachable!("LET should be handled at session level")
        }

        // REINDEX
        LogicalPlan::Reindex { name, collection } => execute_reindex(storage, &collection, &name),
    }
}
