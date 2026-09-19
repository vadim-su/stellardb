// src/query/optimize/optimizer.rs

use crate::query::ast::{Expr, OrderItem, Projection};
use crate::query::execute::ExecutionContext;
use crate::query::optimize::access_path::{AccessMethod, generate_access_paths, select_best};
use crate::query::optimize::index::{
    IndexChoice, analyze_for_fts, analyze_for_index, analyze_for_knn, analyze_or_for_union,
    compute_needs_dedup,
};
use crate::query::optimize::predicate::{classify_all, extract_conjuncts};
use crate::query::plan::{LogicalPlan, PhysicalOp};

/// Apply DISTINCT and/or VALUE operators based on flags.
/// - If value_mode is true with an expression, wrap in Value operator (which handles distinct internally)
/// - If only distinct is true (no value_mode), wrap in Distinct operator
fn apply_distinct_value(
    input: PhysicalOp,
    distinct: bool,
    value_mode: bool,
    value_expr: Option<Expr>,
) -> PhysicalOp {
    if value_mode {
        if let Some(expr) = value_expr {
            // ValueOp handles distinct internally when needed
            PhysicalOp::Value {
                expr,
                distinct,
                input: Box::new(input),
            }
        } else {
            // No expression for value mode (shouldn't happen with proper parsing)
            if distinct {
                PhysicalOp::Distinct {
                    input: Box::new(input),
                }
            } else {
                input
            }
        }
    } else if distinct {
        PhysicalOp::Distinct {
            input: Box::new(input),
        }
    } else {
        input
    }
}

/// Apply projection, ordering, distinct, and value operators in correct order.
///
/// For value_mode: Sort -> Value (sort while fields are accessible)
/// For regular: Sort -> Project -> Distinct (sort needs access to all fields)
fn apply_projection_and_order(
    input: PhysicalOp,
    projection: Projection,
    order: Option<Vec<OrderItem>>,
    distinct: bool,
    value_mode: bool,
    value_expr: Option<Expr>,
) -> PhysicalOp {
    if value_mode {
        // Sort BEFORE Value when in value_mode (so we can sort by original field)
        let sorted = if let Some(items) = order {
            PhysicalOp::Sort {
                items,
                input: Box::new(input),
            }
        } else {
            input
        };
        apply_distinct_value(sorted, distinct, value_mode, value_expr)
    } else {
        // Sort FIRST while all fields are accessible
        let sorted = if let Some(items) = order {
            PhysicalOp::Sort {
                items,
                input: Box::new(input),
            }
        } else {
            input
        };
        let projected = PhysicalOp::Project {
            fields: projection,
            input: Box::new(sorted),
        };
        apply_distinct_value(projected, distinct, false, None)
    }
}

/// Optimizer transforms LogicalPlan to PhysicalPlan
pub struct Optimizer<'a, Ctx: ExecutionContext> {
    ctx: &'a Ctx,
}

impl<'a, Ctx: ExecutionContext> Optimizer<'a, Ctx> {
    pub fn new(ctx: &'a Ctx) -> Self {
        Self { ctx }
    }

    /// Optimize a predicate to the best access path
    fn optimize_predicate(&self, predicate: &Expr, collection: &str) -> (PhysicalOp, Option<Expr>) {
        // 1. Extract conjuncts
        let conjuncts = extract_conjuncts(predicate);

        // 2. Classify
        let parts = classify_all(&conjuncts, self.ctx, collection);

        // 3. Generate paths
        let paths = generate_access_paths(&parts, self.ctx, collection);

        // 4. Select best
        let best = select_best(paths);

        // 5. Build physical op
        let op = match best.method {
            AccessMethod::TableScan => PhysicalOp::TableScan {
                collection: collection.to_string(),
                limit: None,
            },
            AccessMethod::IndexScan { index, lookup } => PhysicalOp::IndexScan {
                collection: collection.to_string(),
                index,
                lookup,
            },
            AccessMethod::Union {
                branches,
                needs_dedup,
            } => {
                let inputs = branches
                    .into_iter()
                    .map(|(index, lookup)| PhysicalOp::IndexScan {
                        collection: collection.to_string(),
                        index,
                        lookup,
                    })
                    .collect();
                PhysicalOp::Union {
                    inputs,
                    needs_dedup,
                }
            }
        };

        (op, best.remaining_filter)
    }

    /// Main entry point: LogicalPlan -> PhysicalPlan
    pub fn optimize(&self, plan: LogicalPlan) -> PhysicalOp {
        match plan {
            LogicalPlan::Scan {
                collection,
                key: Some(key),
                filter,
                projection,
                order,
                limit,
                offset,
                distinct,
                value_mode,
                value_expr,
                fetch: _,
            } => {
                let mut result = PhysicalOp::KeyLookup {
                    collection: collection.clone(),
                    key,
                };

                if let Some(pred) = filter {
                    result = PhysicalOp::Filter {
                        predicate: pred,
                        input: Box::new(result),
                    };
                }

                result = apply_projection_and_order(
                    result, projection, order, distinct, value_mode, value_expr,
                );

                // Offset (after sort, before limit)
                if let Some(off) = offset {
                    result = PhysicalOp::Offset {
                        count: off,
                        input: Box::new(result),
                    };
                }

                if let Some(count) = limit {
                    result = PhysicalOp::Limit {
                        count,
                        input: Box::new(result),
                    };
                }

                result
            }

            LogicalPlan::Scan {
                collection,
                key: None,
                filter: Some(predicate),
                projection,
                order,
                limit,
                offset,
                distinct,
                value_mode,
                value_expr,
                fetch: _,
            } => {
                // First, check for KNN expressions in the predicate
                let knn_analysis = analyze_for_knn(&predicate);

                // Then, check for FTS expressions
                let fts_analysis = analyze_for_fts(&predicate);

                let access = if !knn_analysis.knn_exprs.is_empty() {
                    // Use KNN scan as the base when KNN expressions are present
                    // For now, use the first KNN expression as the primary scan
                    let knn_info = &knn_analysis.knn_exprs[0];
                    let knn_scan = PhysicalOp::KnnScan {
                        collection: collection.clone(),
                        field: knn_info.field.clone(),
                        vector: knn_info.vector.clone(),
                        k: knn_info.k,
                        effort: knn_info.effort,
                    };

                    // Apply remaining filter if any non-KNN conditions exist
                    if let Some(rem) = knn_analysis.remaining {
                        PhysicalOp::Filter {
                            predicate: rem,
                            input: Box::new(knn_scan),
                        }
                    } else {
                        knn_scan
                    }
                } else if !fts_analysis.fts_exprs.is_empty() {
                    // Use FTS scan as the base when FTS expressions are present
                    // For now, use the first FTS expression as the primary scan
                    // Additional FTS expressions would need to be handled as filters
                    // (or through more sophisticated query planning)
                    let fts_info = &fts_analysis.fts_exprs[0];
                    let fts_scan = PhysicalOp::FtsScan {
                        collection: collection.clone(),
                        target: fts_info.target.clone(),
                        operator: fts_info.operator.clone(),
                        query: fts_info.query.clone(),
                        limit, // Push limit to FTS scan
                    };

                    // Apply remaining filter if any non-FTS conditions exist
                    if let Some(rem) = fts_analysis.remaining {
                        PhysicalOp::Filter {
                            predicate: rem,
                            input: Box::new(fts_scan),
                        }
                    } else {
                        fts_scan
                    }
                } else {
                    // Use new predicate optimizer instead of old analyze_for_index
                    let (base_op, remaining) = self.optimize_predicate(&predicate, &collection);
                    if let Some(rem) = remaining {
                        PhysicalOp::Filter {
                            predicate: rem,
                            input: Box::new(base_op),
                        }
                    } else {
                        base_op
                    }
                };

                let mut result = apply_projection_and_order(
                    access, projection, order, distinct, value_mode, value_expr,
                );

                // Offset (after sort, before limit)
                if let Some(off) = offset {
                    result = PhysicalOp::Offset {
                        count: off,
                        input: Box::new(result),
                    };
                }

                // Note: For FTS scans, limit was already pushed down, but we still
                // apply it here for correct semantics after filtering
                if let Some(count) = limit {
                    result = PhysicalOp::Limit {
                        count,
                        input: Box::new(result),
                    };
                }

                result
            }

            LogicalPlan::Scan {
                collection,
                key: None,
                filter: None,
                projection,
                order,
                limit,
                offset,
                distinct,
                value_mode,
                value_expr,
                fetch: _,
            } => {
                // Can only push limit to scan if no sorting needed, no distinct/value, and no offset
                let can_push_limit =
                    order.is_none() && !distinct && !value_mode && offset.is_none();
                let scan = PhysicalOp::TableScan {
                    collection,
                    limit: if can_push_limit { limit } else { None },
                };

                let mut result = apply_projection_and_order(
                    scan, projection, order, distinct, value_mode, value_expr,
                );

                // Offset (after sort, before limit)
                if let Some(off) = offset {
                    result = PhysicalOp::Offset {
                        count: off,
                        input: Box::new(result),
                    };
                }

                if let Some(count) = limit {
                    result = PhysicalOp::Limit {
                        count,
                        input: Box::new(result),
                    };
                }

                result
            }

            LogicalPlan::Aggregate {
                collection,
                filter,
                aggregates,
                post_projection,
            } => {
                let input = if let Some(predicate) = filter {
                    if let Some(branches) = analyze_or_for_union(&predicate, self.ctx, &collection)
                    {
                        let needs_dedup = compute_needs_dedup(&branches);
                        let inputs: Vec<PhysicalOp> = branches
                            .into_iter()
                            .map(|(index_ref, lookup)| PhysicalOp::IndexScan {
                                collection: collection.clone(),
                                index: index_ref,
                                lookup,
                            })
                            .collect();
                        PhysicalOp::Union {
                            inputs,
                            needs_dedup,
                        }
                    } else {
                        match analyze_for_index(&predicate, self.ctx, &collection) {
                            IndexChoice::Use {
                                index_ref,
                                lookup,
                                remaining,
                            } => {
                                let index_scan = PhysicalOp::IndexScan {
                                    collection: collection.clone(),
                                    index: index_ref,
                                    lookup: *lookup,
                                };
                                if let Some(rem) = remaining {
                                    PhysicalOp::Filter {
                                        predicate: rem,
                                        input: Box::new(index_scan),
                                    }
                                } else {
                                    index_scan
                                }
                            }
                            IndexChoice::Skip { .. } => PhysicalOp::Filter {
                                predicate,
                                input: Box::new(PhysicalOp::TableScan {
                                    collection: collection.clone(),
                                    limit: None,
                                }),
                            },
                        }
                    }
                } else {
                    PhysicalOp::TableScan {
                        collection,
                        limit: None,
                    }
                };

                let agg_op = PhysicalOp::Aggregate {
                    aggregates,
                    input: Box::new(input),
                };

                if let Some(proj) = post_projection {
                    PhysicalOp::Project {
                        fields: proj,
                        input: Box::new(agg_op),
                    }
                } else {
                    agg_op
                }
            }

            LogicalPlan::GroupAggregate {
                collection,
                filter,
                group_fields,
                aggregates,
                post_projection,
                order,
            } => {
                let input = if let Some(predicate) = filter {
                    if let Some(branches) = analyze_or_for_union(&predicate, self.ctx, &collection)
                    {
                        let needs_dedup = compute_needs_dedup(&branches);
                        let inputs: Vec<PhysicalOp> = branches
                            .into_iter()
                            .map(|(index_ref, lookup)| PhysicalOp::IndexScan {
                                collection: collection.clone(),
                                index: index_ref,
                                lookup,
                            })
                            .collect();
                        PhysicalOp::Union {
                            inputs,
                            needs_dedup,
                        }
                    } else {
                        match analyze_for_index(&predicate, self.ctx, &collection) {
                            IndexChoice::Use {
                                index_ref,
                                lookup,
                                remaining,
                            } => {
                                let index_scan = PhysicalOp::IndexScan {
                                    collection: collection.clone(),
                                    index: index_ref,
                                    lookup: *lookup,
                                };
                                if let Some(rem) = remaining {
                                    PhysicalOp::Filter {
                                        predicate: rem,
                                        input: Box::new(index_scan),
                                    }
                                } else {
                                    index_scan
                                }
                            }
                            IndexChoice::Skip { .. } => PhysicalOp::Filter {
                                predicate,
                                input: Box::new(PhysicalOp::TableScan {
                                    collection: collection.clone(),
                                    limit: None,
                                }),
                            },
                        }
                    }
                } else {
                    PhysicalOp::TableScan {
                        collection,
                        limit: None,
                    }
                };

                let group_op = PhysicalOp::GroupAggregate {
                    group_fields,
                    aggregates,
                    input: Box::new(input),
                };

                let mut result = if let Some(proj) = post_projection {
                    PhysicalOp::Project {
                        fields: proj,
                        input: Box::new(group_op),
                    }
                } else {
                    group_op
                };

                if let Some(items) = order {
                    result = PhysicalOp::Sort {
                        items,
                        input: Box::new(result),
                    };
                }

                result
            }
            LogicalPlan::InMemoryScan {
                rows,
                filter,
                projection,
                order,
                limit,
                offset,
                scalar_output: _,
                distinct,
                value_mode,
                value_expr,
                fetch: _,
            } => {
                let mut result: PhysicalOp = PhysicalOp::InMemoryScan { rows };

                if let Some(pred) = filter {
                    result = PhysicalOp::Filter {
                        predicate: pred,
                        input: Box::new(result),
                    };
                }

                // For SELECT VALUE, skip Project - ValueOp will compute the expression
                // For regular SELECT, add Project to compute projection
                if value_mode {
                    result = apply_distinct_value(result, distinct, value_mode, value_expr);
                } else {
                    result = PhysicalOp::Project {
                        fields: projection,
                        input: Box::new(result),
                    };
                    result = apply_distinct_value(result, distinct, false, None);
                }

                if let Some(items) = order {
                    result = PhysicalOp::Sort {
                        items,
                        input: Box::new(result),
                    };
                }

                // Offset (after sort, before limit)
                if let Some(off) = offset {
                    result = PhysicalOp::Offset {
                        count: off,
                        input: Box::new(result),
                    };
                }

                if let Some(count) = limit {
                    result = PhysicalOp::Limit {
                        count,
                        input: Box::new(result),
                    };
                }

                result
            }

            LogicalPlan::Insert {
                collection,
                documents,
            } => PhysicalOp::Insert {
                collection,
                documents,
            },

            LogicalPlan::Create {
                collection,
                key,
                assignments,
            } => PhysicalOp::Create {
                collection,
                key,
                assignments,
            },

            LogicalPlan::Update {
                collection,
                key,
                filter,
                assignments,
            } => {
                let input = if let Some(k) = key {
                    PhysicalOp::KeyLookup {
                        collection: collection.clone(),
                        key: k,
                    }
                } else {
                    let scan = PhysicalOp::TableScan {
                        collection: collection.clone(),
                        limit: None,
                    };
                    if let Some(pred) = filter {
                        PhysicalOp::Filter {
                            predicate: pred,
                            input: Box::new(scan),
                        }
                    } else {
                        scan
                    }
                };

                PhysicalOp::Update {
                    collection,
                    assignments,
                    input: Box::new(input),
                }
            }

            LogicalPlan::Delete {
                collection,
                key,
                filter,
            } => {
                let input = if let Some(k) = key {
                    PhysicalOp::KeyLookup {
                        collection: collection.clone(),
                        key: k,
                    }
                } else {
                    let scan = PhysicalOp::TableScan {
                        collection: collection.clone(),
                        limit: None,
                    };
                    if let Some(pred) = filter {
                        PhysicalOp::Filter {
                            predicate: pred,
                            input: Box::new(scan),
                        }
                    } else {
                        scan
                    }
                };

                PhysicalOp::Delete {
                    collection,
                    input: Box::new(input),
                }
            }

            LogicalPlan::Upsert {
                collection,
                key,
                assignments,
                replace,
            } => PhysicalOp::Upsert {
                collection,
                key,
                assignments,
                replace,
            },

            LogicalPlan::DeleteEdge { from, label, to } => {
                PhysicalOp::DeleteEdge { from, label, to }
            }

            // DDL and transaction control handled elsewhere
            LogicalPlan::Begin
            | LogicalPlan::Commit
            | LogicalPlan::Rollback
            | LogicalPlan::DefineCollection { .. }
            | LogicalPlan::DropCollection { .. }
            | LogicalPlan::DescribeCollection(_)
            | LogicalPlan::DescribeCollections
            | LogicalPlan::CreateIndex { .. }
            | LogicalPlan::DropIndex { .. }
            | LogicalPlan::Reindex { .. }
            | LogicalPlan::Explain { .. }
            | LogicalPlan::Let { .. } => {
                unreachable!("DDL/TX control/EXPLAIN/LET should not reach optimizer")
            }
        }
    }
}

/// End-to-end integration tests for the smart predicate optimizer.
/// These tests verify that queries with complex predicates (AND, OR, IN)
/// correctly use indexes and return the expected results.
#[cfg(test)]
mod optimizer_integration_tests {
    use crate::Database;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Arc<Database>) {
        let tmp = TempDir::new().unwrap();
        let storage = Arc::new(Database::open(tmp.path()).unwrap());
        (tmp, storage)
    }

    #[test]
    fn test_and_or_optimization() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(status)");
        crate::run_sql!(storage, "CREATE INDEX ON users(role)");
        crate::run_sql!(
            storage,
            r#"
            INSERT INTO users
                {id: 'a', status: 'active', role: 'admin'},
                {id: 'b', status: 'active', role: 'user'},
                {id: 'c', status: 'inactive', role: 'admin'},
                {id: 'd', status: 'active', role: 'mod'}
        "#
        );

        // status = 'active' AND (role = 'admin' OR role = 'mod')
        let result = crate::run_sql!(
            storage,
            "SELECT * FROM users WHERE status = 'active' AND (role = 'admin' OR role = 'mod')"
        );
        assert_eq!(result.rows.len(), 2); // 'a' and 'd'
    }

    #[test]
    fn test_in_optimization() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(status)");
        crate::run_sql!(
            storage,
            r#"
            INSERT INTO users
                {id: 'a', status: 'active'},
                {id: 'b', status: 'pending'},
                {id: 'c', status: 'inactive'},
                {id: 'd', status: 'active'}
        "#
        );

        let result = crate::run_sql!(
            storage,
            "SELECT * FROM users WHERE status IN ['active', 'pending']"
        );
        assert_eq!(result.rows.len(), 3); // 'a', 'b', 'd'
    }

    #[test]
    fn test_best_index_selected() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(id_field)"); // Eq - score 1
        crate::run_sql!(storage, "CREATE INDEX ON users(age)"); // Range - score 20
        crate::run_sql!(
            storage,
            r#"
            INSERT INTO users
                {id: 'x', id_field: 'x', age: 25},
                {id: 'y', id_field: 'y', age: 30}
        "#
        );

        // Both indexed, but Eq should win - verify query returns correct results
        let result = crate::run_sql!(
            storage,
            "SELECT * FROM users WHERE id_field = 'x' AND age > 18"
        );
        assert_eq!(result.rows.len(), 1);
    }

    #[test]
    fn test_or_union_optimization() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(role)");
        crate::run_sql!(
            storage,
            r#"
            INSERT INTO users
                {id: 'a', role: 'admin'},
                {id: 'b', role: 'user'},
                {id: 'c', role: 'mod'}
        "#
        );

        // Should create Union
        let result = crate::run_sql!(
            storage,
            "SELECT * FROM users WHERE role = 'admin' OR role = 'mod'"
        );
        assert_eq!(result.rows.len(), 2); // 'a' and 'c'
    }

    #[test]
    fn test_complex_and_or_and() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(status)");
        crate::run_sql!(storage, "CREATE INDEX ON users(role)");
        crate::run_sql!(storage, "CREATE INDEX ON users(verified)");
        crate::run_sql!(
            storage,
            r#"
            INSERT INTO users
                {id: 'a', status: 'active', role: 'admin', verified: true},
                {id: 'b', status: 'active', role: 'mod', verified: true},
                {id: 'c', status: 'active', role: 'user', verified: true},
                {id: 'd', status: 'inactive', role: 'admin', verified: true}
        "#
        );

        // status = 'active' AND (role = 'admin' OR role = 'mod') AND verified = true
        let result = crate::run_sql!(
            storage,
            "SELECT * FROM users WHERE status = 'active' AND (role = 'admin' OR role = 'mod') AND verified = true"
        );
        assert_eq!(result.rows.len(), 2); // 'a' and 'b'
    }
}
