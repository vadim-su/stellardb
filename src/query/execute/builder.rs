//! Operator tree builder from physical plans
//!
//! Transforms PhysicalOp trees into executable Operator trees.

use std::cell::RefCell;
use std::rc::Rc;

use crate::query::error::ExecuteError;
use crate::query::execute::context::ExecutionContext;
use crate::query::execute::operators::aggregate::AggregateOp;
use crate::query::execute::operators::distinct::DistinctOp;
use crate::query::execute::operators::filter::FilterOp;
use crate::query::execute::operators::fts_scan::FtsIndexScanOp;
use crate::query::execute::operators::group_aggregate::GroupAggregateOp;
use crate::query::execute::operators::limit::LimitOp;
use crate::query::execute::operators::offset::OffsetOp;
use crate::query::execute::operators::operator::{Operator, Row};
use crate::query::execute::operators::project::ProjectOp;
use crate::query::execute::operators::scan::{IndexScanOp, KeyLookupOp, TableScanOp};
use crate::query::execute::operators::sort::SortOp;
use crate::query::execute::operators::value::ValueOp;
use crate::query::execute::operators::vector_scan::LazyVectorIndexScanOp;
use crate::query::plan::{OperatorStats, PhysicalOp};

/// Builds operator tree from PhysicalOp
///
/// Transforms a physical execution plan (PhysicalOp tree) into an executable
/// operator tree using dynamic dispatch. Each PhysicalOp variant is mapped
/// to its corresponding operator implementation.
pub struct OperatorBuilder<'a, Ctx: ExecutionContext> {
    ctx: &'a Ctx,
}

impl<'a, Ctx: ExecutionContext + 'a> OperatorBuilder<'a, Ctx> {
    /// Create a new builder with the given execution context
    pub fn new(ctx: &'a Ctx) -> Self {
        Self { ctx }
    }

    /// Build operator tree from physical plan
    ///
    /// Recursively transforms a PhysicalOp tree into a boxed Operator tree.
    /// Returns boxed operator for dynamic dispatch because different PhysicalOp
    /// variants produce different concrete types.
    pub fn build(&self, plan: PhysicalOp) -> Box<dyn Operator + 'a> {
        match plan {
            PhysicalOp::TableScan { collection, limit } => {
                Box::new(TableScanOp::new(self.ctx, collection, limit))
            }

            PhysicalOp::KeyLookup { collection, key } => {
                Box::new(KeyLookupOp::new(self.ctx, collection, key))
            }

            PhysicalOp::IndexScan {
                collection,
                index,
                lookup,
            } => Box::new(IndexScanOp::new(self.ctx, collection, index, lookup)),

            PhysicalOp::Filter { predicate, input } => {
                let child = self.build(*input);
                Box::new(FilterOp::new(child, predicate, self.ctx))
            }

            PhysicalOp::Project { fields, input } => {
                let child = self.build(*input);
                Box::new(ProjectOp::new(child, fields, self.ctx.vars(), self.ctx))
            }

            PhysicalOp::Limit { count, input } => {
                let child = self.build(*input);
                Box::new(LimitOp::new(child, count))
            }

            PhysicalOp::Offset { count, input } => {
                let child = self.build(*input);
                Box::new(OffsetOp::new(child, count))
            }

            PhysicalOp::Sort { items, input } => {
                let child = self.build(*input);
                Box::new(SortOp::new(child, items, self.ctx.vars(), self.ctx))
            }

            PhysicalOp::Aggregate { aggregates, input } => {
                let child = self.build(*input);
                Box::new(AggregateOp::new(child, aggregates))
            }

            PhysicalOp::GroupAggregate {
                group_fields,
                aggregates,
                input,
            } => {
                let child = self.build(*input);
                Box::new(GroupAggregateOp::new(child, group_fields, aggregates))
            }

            PhysicalOp::InMemoryScan { rows } => Box::new(
                crate::query::execute::operators::in_memory_scan::InMemoryScanOp::new(rows),
            ),

            PhysicalOp::FtsScan {
                collection,
                target,
                operator,
                query,
                limit,
            } => Box::new(FtsIndexScanOp::new(
                self.ctx, collection, target, operator, query, limit,
            )),

            PhysicalOp::KnnScan {
                collection,
                field,
                vector,
                k,
                effort,
            } => Box::new(LazyVectorIndexScanOp::new(
                self.ctx, collection, field, vector, k, effort,
            )),

            PhysicalOp::Union {
                inputs,
                needs_dedup,
            } => {
                let children: Vec<Box<dyn Operator + 'a>> =
                    inputs.into_iter().map(|input| self.build(input)).collect();
                Box::new(crate::query::execute::operators::union::UnionOp::new(
                    children,
                    needs_dedup,
                ))
            }

            PhysicalOp::Distinct { input } => {
                let child = self.build(*input);
                Box::new(DistinctOp::new(child))
            }

            PhysicalOp::Value {
                expr,
                distinct,
                input,
            } => {
                let child = self.build(*input);
                Box::new(ValueOp::new(
                    child,
                    expr,
                    distinct,
                    self.ctx.vars(),
                    self.ctx,
                ))
            }

            // DML operations are handled by DDL executor, return placeholder
            PhysicalOp::Insert { .. }
            | PhysicalOp::Create { .. }
            | PhysicalOp::Update { .. }
            | PhysicalOp::Upsert { .. }
            | PhysicalOp::Delete { .. }
            | PhysicalOp::DeleteEdge { .. } => {
                // DML is handled directly in engine/mod.rs, not through operator tree
                Box::new(EmptyOp)
            }
        }
    }

    /// Build operator tree with instrumentation for EXPLAIN ANALYZE
    /// Returns the root operator and a list of stats collectors in pre-order traversal order
    pub fn build_instrumented(
        &self,
        plan: PhysicalOp,
    ) -> (Box<dyn Operator + 'a>, Vec<Rc<RefCell<OperatorStats>>>) {
        let mut stats_list = Vec::new();
        let op = self.build_instrumented_inner(plan, &mut stats_list);
        (op, stats_list)
    }

    fn build_instrumented_inner(
        &self,
        plan: PhysicalOp,
        stats_list: &mut Vec<Rc<RefCell<OperatorStats>>>,
    ) -> Box<dyn Operator + 'a> {
        match plan {
            PhysicalOp::TableScan { collection, limit } => {
                let op = TableScanOp::new(self.ctx, collection, limit);
                let stats = Rc::new(RefCell::new(OperatorStats::default()));
                stats_list.push(Rc::clone(&stats));
                Box::new(StatsCollectingOp::new(op, stats))
            }

            PhysicalOp::KeyLookup { collection, key } => {
                let op = KeyLookupOp::new(self.ctx, collection, key);
                let stats = Rc::new(RefCell::new(OperatorStats::default()));
                stats_list.push(Rc::clone(&stats));
                Box::new(StatsCollectingOp::new(op, stats))
            }

            PhysicalOp::IndexScan {
                collection,
                index,
                lookup,
            } => {
                let op = IndexScanOp::new(self.ctx, collection, index, lookup);
                let stats = Rc::new(RefCell::new(OperatorStats::default()));
                stats_list.push(Rc::clone(&stats));
                Box::new(StatsCollectingOp::new(op, stats))
            }

            PhysicalOp::Filter { predicate, input } => {
                let child = self.build_instrumented_inner(*input, stats_list);
                let op = FilterOp::new(child, predicate, self.ctx);
                let stats = Rc::new(RefCell::new(OperatorStats::default()));
                stats_list.push(Rc::clone(&stats));
                Box::new(StatsCollectingOp::new(op, stats))
            }

            PhysicalOp::Project { fields, input } => {
                let child = self.build_instrumented_inner(*input, stats_list);
                let op = ProjectOp::new(child, fields, self.ctx.vars(), self.ctx);
                let stats = Rc::new(RefCell::new(OperatorStats::default()));
                stats_list.push(Rc::clone(&stats));
                Box::new(StatsCollectingOp::new(op, stats))
            }

            PhysicalOp::Limit { count, input } => {
                let child = self.build_instrumented_inner(*input, stats_list);
                let op = LimitOp::new(child, count);
                let stats = Rc::new(RefCell::new(OperatorStats::default()));
                stats_list.push(Rc::clone(&stats));
                Box::new(StatsCollectingOp::new(op, stats))
            }

            PhysicalOp::Offset { count, input } => {
                let child = self.build_instrumented_inner(*input, stats_list);
                let op = OffsetOp::new(child, count);
                let stats = Rc::new(RefCell::new(OperatorStats::default()));
                stats_list.push(Rc::clone(&stats));
                Box::new(StatsCollectingOp::new(op, stats))
            }

            PhysicalOp::Sort { items, input } => {
                let child = self.build_instrumented_inner(*input, stats_list);
                let op = SortOp::new(child, items, self.ctx.vars(), self.ctx);
                let stats = Rc::new(RefCell::new(OperatorStats::default()));
                stats_list.push(Rc::clone(&stats));
                Box::new(StatsCollectingOp::new(op, stats))
            }

            PhysicalOp::Aggregate { aggregates, input } => {
                let child = self.build_instrumented_inner(*input, stats_list);
                let op = AggregateOp::new(child, aggregates);
                let stats = Rc::new(RefCell::new(OperatorStats::default()));
                stats_list.push(Rc::clone(&stats));
                Box::new(StatsCollectingOp::new(op, stats))
            }

            PhysicalOp::GroupAggregate {
                group_fields,
                aggregates,
                input,
            } => {
                let child = self.build_instrumented_inner(*input, stats_list);
                let op = GroupAggregateOp::new(child, group_fields, aggregates);
                let stats = Rc::new(RefCell::new(OperatorStats::default()));
                stats_list.push(Rc::clone(&stats));
                Box::new(StatsCollectingOp::new(op, stats))
            }
            PhysicalOp::InMemoryScan { rows } => {
                let op =
                    crate::query::execute::operators::in_memory_scan::InMemoryScanOp::new(rows);
                let stats = Rc::new(RefCell::new(OperatorStats::default()));
                stats_list.push(Rc::clone(&stats));
                Box::new(StatsCollectingOp::new(op, stats))
            }

            PhysicalOp::FtsScan {
                collection,
                target,
                operator,
                query,
                limit,
            } => {
                let op = FtsIndexScanOp::new(self.ctx, collection, target, operator, query, limit);
                let stats = Rc::new(RefCell::new(OperatorStats::default()));
                stats_list.push(Rc::clone(&stats));
                Box::new(StatsCollectingOp::new(op, stats))
            }

            PhysicalOp::KnnScan {
                collection,
                field,
                vector,
                k,
                effort,
            } => {
                let op = LazyVectorIndexScanOp::new(self.ctx, collection, field, vector, k, effort);
                let stats = Rc::new(RefCell::new(OperatorStats::default()));
                stats_list.push(Rc::clone(&stats));
                Box::new(StatsCollectingOp::new(op, stats))
            }

            PhysicalOp::Union {
                inputs,
                needs_dedup,
            } => {
                let children: Vec<Box<dyn Operator + 'a>> = inputs
                    .into_iter()
                    .map(|input| self.build_instrumented_inner(input, stats_list))
                    .collect();
                let op =
                    crate::query::execute::operators::union::UnionOp::new(children, needs_dedup);
                let stats = Rc::new(RefCell::new(OperatorStats::default()));
                stats_list.push(Rc::clone(&stats));
                Box::new(StatsCollectingOp::new(op, stats))
            }

            PhysicalOp::Distinct { input } => {
                let child = self.build_instrumented_inner(*input, stats_list);
                let op = DistinctOp::new(child);
                let stats = Rc::new(RefCell::new(OperatorStats::default()));
                stats_list.push(Rc::clone(&stats));
                Box::new(StatsCollectingOp::new(op, stats))
            }

            PhysicalOp::Value {
                expr,
                distinct,
                input,
            } => {
                let child = self.build_instrumented_inner(*input, stats_list);
                let op = ValueOp::new(child, expr, distinct, self.ctx.vars(), self.ctx);
                let stats = Rc::new(RefCell::new(OperatorStats::default()));
                stats_list.push(Rc::clone(&stats));
                Box::new(StatsCollectingOp::new(op, stats))
            }

            PhysicalOp::Insert { .. }
            | PhysicalOp::Create { .. }
            | PhysicalOp::Update { .. }
            | PhysicalOp::Upsert { .. }
            | PhysicalOp::Delete { .. }
            | PhysicalOp::DeleteEdge { .. } => Box::new(EmptyOp),
        }
    }
}

/// Operator wrapper that writes stats to shared RefCell.
///
/// Accumulates time per `open()`/`next()`/`close()` call rather than
/// measuring wall-clock from open to close. In a pull-based pipeline,
/// each parent's `next()` includes calling child `next()`, so totals
/// naturally include child time. `compute_self_times()` then subtracts
/// children to get per-operator self time.
struct StatsCollectingOp<O: Operator> {
    inner: O,
    stats: Rc<RefCell<OperatorStats>>,
    accumulated_micros: u64,
    rows_out: u64,
}

impl<O: Operator> StatsCollectingOp<O> {
    fn new(inner: O, stats: Rc<RefCell<OperatorStats>>) -> Self {
        Self {
            inner,
            stats,
            accumulated_micros: 0,
            rows_out: 0,
        }
    }
}

impl<O: Operator> Operator for StatsCollectingOp<O> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        let start = std::time::Instant::now();
        let result = self.inner.open();
        self.accumulated_micros += start.elapsed().as_micros() as u64;
        result
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        let start = std::time::Instant::now();
        let row = self.inner.next()?;
        self.accumulated_micros += start.elapsed().as_micros() as u64;
        if row.is_some() {
            self.rows_out += 1;
        }
        Ok(row)
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        let start = std::time::Instant::now();
        let result = self.inner.close();
        self.accumulated_micros += start.elapsed().as_micros() as u64;
        let mut stats = self.stats.borrow_mut();
        stats.rows_out = self.rows_out;
        stats.time_micros = self.accumulated_micros;
        result
    }
}

/// Placeholder operator for DML operations (handled by DDL executor)
struct EmptyOp;

impl Operator for EmptyOp {
    fn open(&mut self) -> Result<(), ExecuteError> {
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        Ok(None)
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use crate::document::Value;
    use crate::query::ast::{BinaryOp, Expr, Projection, ProjectionItem};
    use crate::query::execute::operators::operator::collect_all;
    use crate::query::plan::{IndexLookup, IndexRef};
    use std::sync::Arc;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Arc<Database>) {
        let tmp = TempDir::new().unwrap();
        let storage = Arc::new(Database::open(tmp.path()).unwrap());
        (tmp, storage)
    }

    #[test]
    fn test_build_table_scan() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "INSERT INTO users {id: 'alice', name: 'Alice'}");
        crate::run_sql!(storage, "INSERT INTO users {id: 'bob', name: 'Bob'}");

        let builder = OperatorBuilder::new(&*storage);
        let plan = PhysicalOp::TableScan {
            collection: "users".to_string(),
            limit: None,
        };

        let mut op = builder.build(plan);
        let rows = collect_all(&mut *op).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn test_build_table_scan_with_limit() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "INSERT INTO users {id: '1'}, {id: '2'}, {id: '3'}");

        let builder = OperatorBuilder::new(&*storage);
        let plan = PhysicalOp::TableScan {
            collection: "users".to_string(),
            limit: Some(2),
        };

        let mut op = builder.build(plan);
        let rows = collect_all(&mut *op).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn test_build_key_lookup() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "INSERT INTO users {id: 'alice', name: 'Alice'}");

        let builder = OperatorBuilder::new(&*storage);
        let plan = PhysicalOp::KeyLookup {
            collection: "users".to_string(),
            key: "alice".to_string(),
        };

        let mut op = builder.build(plan);
        let rows = collect_all(&mut *op).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].doc.id, "users:alice");
    }

    #[test]
    fn test_build_key_lookup_not_found() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");

        let builder = OperatorBuilder::new(&*storage);
        let plan = PhysicalOp::KeyLookup {
            collection: "users".to_string(),
            key: "notfound".to_string(),
        };

        let mut op = builder.build(plan);
        let rows = collect_all(&mut *op).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_build_index_scan() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(status)");
        crate::run_sql!(
            storage,
            "INSERT INTO users {id: 'a', status: 'active'}, {id: 'b', status: 'inactive'}, {id: 'c', status: 'active'}"
        );

        let builder = OperatorBuilder::new(&*storage);
        let plan = PhysicalOp::IndexScan {
            collection: "users".to_string(),
            index: IndexRef {
                collection: "users".to_string(),
                fields: vec!["status".to_string()],
            },
            lookup: IndexLookup::Eq {
                field: "status".to_string(),
                value: Value::String("active".to_string()),
            },
        };

        let mut op = builder.build(plan);
        let rows = collect_all(&mut *op).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn test_build_filter() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(
            storage,
            "INSERT INTO users {id: '1', age: 25}, {id: '2', age: 30}, {id: '3', age: 25}"
        );

        let builder = OperatorBuilder::new(&*storage);
        let plan = PhysicalOp::Filter {
            predicate: Expr::BinaryOp {
                left: Box::new(Expr::Field("age".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Value::Int(25))),
            },
            input: Box::new(PhysicalOp::TableScan {
                collection: "users".to_string(),
                limit: None,
            }),
        };

        let mut op = builder.build(plan);
        let rows = collect_all(&mut *op).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn test_build_project() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(
            storage,
            "INSERT INTO users {id: '1', name: 'Alice', age: 30, city: 'NYC'}"
        );

        let builder = OperatorBuilder::new(&*storage);
        let plan = PhysicalOp::Project {
            fields: Projection::Items(vec![ProjectionItem {
                expr: Expr::Field("name".to_string()),
                alias: None,
            }]),
            input: Box::new(PhysicalOp::TableScan {
                collection: "users".to_string(),
                limit: None,
            }),
        };

        let mut op = builder.build(plan);
        let rows = collect_all(&mut *op).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].doc.fields.contains_key("name"));
        assert!(!rows[0].doc.fields.contains_key("age"));
        assert!(!rows[0].doc.fields.contains_key("city"));
    }

    #[test]
    fn test_build_limit() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(
            storage,
            "INSERT INTO users {id: '1'}, {id: '2'}, {id: '3'}, {id: '4'}, {id: '5'}"
        );

        let builder = OperatorBuilder::new(&*storage);
        let plan = PhysicalOp::Limit {
            count: 3,
            input: Box::new(PhysicalOp::TableScan {
                collection: "users".to_string(),
                limit: None,
            }),
        };

        let mut op = builder.build(plan);
        let rows = collect_all(&mut *op).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn test_build_nested_plan() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(
            storage,
            "INSERT INTO users {id: '1', name: 'Alice', age: 25}, {id: '2', name: 'Bob', age: 30}, {id: '3', name: 'Charlie', age: 25}, {id: '4', name: 'Diana', age: 25}"
        );

        // Build: LIMIT 2 -> FILTER age=25 -> PROJECT name -> TableScan
        let builder = OperatorBuilder::new(&*storage);
        let plan = PhysicalOp::Limit {
            count: 2,
            input: Box::new(PhysicalOp::Project {
                fields: Projection::Items(vec![ProjectionItem {
                    expr: Expr::Field("name".to_string()),
                    alias: None,
                }]),
                input: Box::new(PhysicalOp::Filter {
                    predicate: Expr::BinaryOp {
                        left: Box::new(Expr::Field("age".to_string())),
                        op: BinaryOp::Eq,
                        right: Box::new(Expr::Literal(Value::Int(25))),
                    },
                    input: Box::new(PhysicalOp::TableScan {
                        collection: "users".to_string(),
                        limit: None,
                    }),
                }),
            }),
        };

        let mut op = builder.build(plan);
        let rows = collect_all(&mut *op).unwrap();

        // Should have 2 rows (limited from 3 matching age=25)
        assert_eq!(rows.len(), 2);
        // Each row should only have 'name' field
        for row in &rows {
            assert!(row.doc.fields.contains_key("name"));
            assert!(!row.doc.fields.contains_key("age"));
        }
    }

    #[test]
    fn test_build_dml_placeholder() {
        let (_tmp, storage) = setup();

        let builder = OperatorBuilder::new(&*storage);
        let plan = PhysicalOp::Insert {
            collection: "users".to_string(),
            documents: vec![],
        };

        // DML returns empty operator for now
        let mut op = builder.build(plan);
        let rows = collect_all(&mut *op).unwrap();
        assert!(rows.is_empty());
    }

    #[cfg(feature = "hnsw")]
    #[test]
    fn test_build_knn_scan() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION items");
        // Create HNSW index on embedding field (DIMENSION 3 for our test vectors)
        crate::run_sql!(
            storage,
            "CREATE INDEX ON items(embedding) HNSW DIMENSION 3 DIST EUCLIDEAN"
        );

        // Insert test documents with vectors
        crate::run_sql!(
            storage,
            "INSERT INTO items {id: 'item1', name: 'Item 1', embedding: [1.0, 0.0, 0.0]}"
        );
        crate::run_sql!(
            storage,
            "INSERT INTO items {id: 'item2', name: 'Item 2', embedding: [0.0, 1.0, 0.0]}"
        );
        crate::run_sql!(
            storage,
            "INSERT INTO items {id: 'item3', name: 'Item 3', embedding: [0.0, 0.0, 1.0]}"
        );

        // Build a KNN scan plan
        let builder = OperatorBuilder::new(&*storage);
        let plan = PhysicalOp::KnnScan {
            collection: "items".to_string(),
            field: "embedding".to_string(),
            vector: Expr::Array(vec![
                Expr::Literal(Value::Float(1.0)),
                Expr::Literal(Value::Float(0.0)),
                Expr::Literal(Value::Float(0.0)),
            ]),
            k: 2,
            effort: None,
        };

        let mut op = builder.build(plan);
        let rows = collect_all(&mut *op).unwrap();

        // Should return 2 nearest neighbors
        assert_eq!(rows.len(), 2);

        // Each result should have a $distance field
        for row in &rows {
            assert!(row.doc.fields.contains_key("$distance"));
        }

        // First result should be item1 (exact match, distance ~0)
        let first_row = &rows[0];
        assert!(first_row.doc.id.contains("item1"));
        if let Some(Value::Float(d)) = first_row.doc.fields.get("$distance") {
            assert!(*d < 0.01, "First result should have distance close to 0");
        }
    }
}
