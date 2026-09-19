use crate::query::ast::{
    AggregateArg, AggregateCall, Assignment, DataSource, DeleteTarget, DescribeAst, Expr,
    FetchItem, ObjectLiteral, OrderItem, Projection, ProjectionItem, Statement,
};
use crate::query::function::AggregateFunction;
use crate::schema::{FieldDef, HnswParams, IndexDef, IndexType, SchemaMode};

#[derive(Debug, Clone, PartialEq)]
pub enum LogicalPlan {
    Scan {
        collection: String,
        key: Option<String>,
        filter: Option<Expr>,
        projection: Projection,
        order: Option<Vec<OrderItem>>,
        limit: Option<usize>,
        offset: Option<usize>,
        distinct: bool,
        value_mode: bool,
        value_expr: Option<Expr>,
        fetch: Option<Vec<FetchItem>>,
    },

    /// Aggregate query: SELECT COUNT(*), SUM(score) FROM collection [WHERE ...]
    Aggregate {
        collection: String,
        filter: Option<Expr>,
        aggregates: Vec<ResolvedAggregate>,
        /// Post-aggregation projection for expressions like SUM(age)/COUNT(*).
        /// When `Some`, the aggregate results are fed into this projection
        /// which evaluates the rewritten expressions (with aggregates replaced by field refs).
        post_projection: Option<Projection>,
    },

    /// Grouped aggregate: SELECT status, COUNT(*) FROM collection GROUP status
    GroupAggregate {
        collection: String,
        filter: Option<Expr>,
        group_fields: Vec<String>,
        aggregates: Vec<ResolvedAggregate>,
        post_projection: Option<Projection>,
        order: Option<Vec<OrderItem>>,
    },

    /// In-memory scan over pre-materialized rows (from subquery or field path)
    InMemoryScan {
        rows: Vec<crate::query::execute::operators::operator::Row>,
        filter: Option<Expr>,
        projection: Projection,
        order: Option<Vec<OrderItem>>,
        limit: Option<usize>,
        offset: Option<usize>,
        scalar_output: bool,
        distinct: bool,
        value_mode: bool,
        value_expr: Option<Expr>,
        fetch: Option<Vec<FetchItem>>,
    },

    Insert {
        collection: String,
        documents: Vec<ObjectLiteral>,
    },

    Create {
        collection: String,
        key: Option<String>,
        assignments: Vec<Assignment>,
    },

    Update {
        collection: String,
        key: Option<String>,
        filter: Option<Expr>,
        assignments: Vec<Assignment>,
    },

    Upsert {
        collection: String,
        key: String,
        assignments: Vec<Assignment>,
        replace: bool,
    },

    Delete {
        collection: String,
        key: Option<String>,
        filter: Option<Expr>,
    },

    /// Delete edge(s)
    DeleteEdge {
        from: String,
        label: String,
        to: Option<String>,
    },

    Begin,
    Commit,
    Rollback,

    /// Define a new collection with schema
    DefineCollection {
        name: String,
        mode: SchemaMode,
        fields: Vec<FieldDef>,
        indexes: Vec<IndexDef>,
    },

    /// Drop a collection
    DropCollection {
        name: String,
        cascade: bool,
    },

    /// Describe a specific collection
    DescribeCollection(String),

    /// Describe all collections
    DescribeCollections,

    /// Create an index on a collection
    CreateIndex {
        collection: String,
        fields: Vec<String>,
        unique: bool,
        index_type: IndexType,
        hnsw_params: Option<HnswParams>,
        analyzer: Option<String>,
    },

    /// Drop an index from a collection
    DropIndex {
        /// Index name
        name: String,
        /// Collection name
        collection: String,
    },

    /// Rebuild an index
    Reindex {
        /// Index name
        name: String,
        /// Collection name
        collection: String,
    },

    /// Show query execution plan
    Explain {
        analyze: bool,
        plan: Box<LogicalPlan>,
        /// Time spent materializing subquery/field-path data source (if any)
        materialize_time_micros: Option<u64>,
    },

    /// Variable binding: LET name = expr
    Let {
        name: String,
        expr: Expr,
    },
}

/// A resolved aggregate function call (after bind stage)
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAggregate {
    pub function: AggregateFunction,
    pub arg: AggregateArg,
    /// Display name used as key in result: "count(*)", "sum(score)"
    pub alias: String,
    /// Whether DISTINCT modifier is applied
    pub distinct: bool,
}

impl LogicalPlan {
    pub fn from_ast(stmt: Statement) -> Self {
        match stmt {
            Statement::Select(s) => {
                // Extract collection target (subqueries/field paths are materialized before from_ast)
                let (collection, key) = match &s.source {
                    DataSource::Collection(t) => (t.collection.clone(), t.key.clone()),
                    _ => {
                        unreachable!("Subquery/FieldPath sources are materialized before from_ast")
                    }
                };

                // If GROUP is present, build GroupAggregate
                if let Some(group_fields) = s.group.clone() {
                    let mut aggregates: Vec<ResolvedAggregate> = Vec::new();
                    if let Projection::Items(ref items) = s.projection {
                        let mut needs_post_projection = false;

                        // Post-projection is needed when any item is not a bare
                        // aggregate and not a group field (e.g. variables, literals,
                        // expressions containing aggregates).
                        for item in items {
                            if !matches!(&item.expr, Expr::Aggregate(_))
                                && !matches!(&item.expr, Expr::Field(f) if group_fields.contains(f))
                            {
                                needs_post_projection = true;
                                break;
                            }
                        }

                        if needs_post_projection {
                            let mut post_items = Vec::new();
                            for item in items {
                                if matches!(&item.expr, Expr::Field(f) if group_fields.contains(f))
                                {
                                    post_items.push(item.clone());
                                    continue;
                                }
                                let (_, rewritten) =
                                    extract_aggregates(&item.expr, &mut aggregates);
                                let display = item.alias.clone().unwrap_or_else(|| {
                                    crate::query::execute::operators::project::expr_display_name(
                                        &item.expr,
                                    )
                                });
                                post_items.push(ProjectionItem {
                                    expr: rewritten,
                                    alias: Some(display),
                                });
                            }
                            return LogicalPlan::GroupAggregate {
                                collection,
                                filter: s.filter,
                                group_fields,
                                aggregates,
                                post_projection: Some(Projection::Items(post_items)),
                                order: s.order,
                            };
                        }

                        // Simple case: bare aggregates + group fields
                        for item in items {
                            if let Expr::Aggregate(ref call) = item.expr
                                && let Some(func) = AggregateFunction::from_name(&call.function)
                            {
                                let alias = item.alias.clone().unwrap_or_else(|| agg_alias(call));
                                aggregates.push(ResolvedAggregate {
                                    function: func,
                                    arg: call.arg.clone(),
                                    alias,
                                    distinct: call.distinct,
                                });
                            }
                        }
                    }

                    return LogicalPlan::GroupAggregate {
                        collection,
                        filter: s.filter,
                        group_fields,
                        aggregates,
                        post_projection: None,
                        order: s.order,
                    };
                }

                // Check if this is an aggregate query (top-level or nested in expressions)
                if let Projection::Items(ref items) = s.projection {
                    let has_agg = items.iter().any(|i| i.expr.contains_aggregate());
                    if has_agg {
                        let mut aggregates: Vec<ResolvedAggregate> = Vec::new();
                        let mut needs_post_projection = false;

                        // Post-projection is needed when any item is not a bare
                        // aggregate (e.g. variables, literals, expressions).
                        for item in items {
                            if !matches!(&item.expr, Expr::Aggregate(_)) {
                                needs_post_projection = true;
                                break;
                            }
                        }

                        if needs_post_projection {
                            // Extract aggregates from expressions and build post-projection
                            let mut post_items = Vec::new();

                            for item in items {
                                let (_extracted, rewritten) =
                                    extract_aggregates(&item.expr, &mut aggregates);

                                let display = item.alias.clone().unwrap_or_else(|| {
                                    crate::query::execute::operators::project::expr_display_name(
                                        &item.expr,
                                    )
                                });

                                post_items.push(ProjectionItem {
                                    expr: rewritten,
                                    alias: Some(display),
                                });
                            }

                            return LogicalPlan::Aggregate {
                                collection,
                                filter: s.filter,
                                aggregates,
                                post_projection: Some(Projection::Items(post_items)),
                            };
                        }

                        // All items are bare aggregates (existing path)
                        let aggregates = items
                            .iter()
                            .filter_map(|item| {
                                if let Expr::Aggregate(ref call) = item.expr {
                                    let func = AggregateFunction::from_name(&call.function)?;
                                    let alias =
                                        item.alias.clone().unwrap_or_else(|| agg_alias(call));
                                    Some(ResolvedAggregate {
                                        function: func,
                                        arg: call.arg.clone(),
                                        alias,
                                        distinct: call.distinct,
                                    })
                                } else {
                                    None
                                }
                            })
                            .collect();

                        return LogicalPlan::Aggregate {
                            collection,
                            filter: s.filter,
                            aggregates,
                            post_projection: None,
                        };
                    }
                }

                let value_expr = if s.value_mode {
                    extract_value_expr(&s.projection)
                } else {
                    None
                };

                LogicalPlan::Scan {
                    collection,
                    key,
                    filter: s.filter,
                    projection: s.projection,
                    order: s.order,
                    limit: s.limit,
                    offset: s.offset,
                    distinct: s.distinct,
                    value_mode: s.value_mode,
                    value_expr,
                    fetch: s.fetch,
                }
            }
            Statement::Insert(i) => LogicalPlan::Insert {
                collection: i.collection,
                documents: i.objects,
            },
            Statement::Create(c) => LogicalPlan::Create {
                collection: c.target.collection,
                key: c.target.key,
                assignments: c.assignments,
            },
            Statement::Update(u) => LogicalPlan::Update {
                collection: u.target.collection,
                key: u.target.key,
                filter: u.filter,
                assignments: u.assignments,
            },
            Statement::Upsert(u) => LogicalPlan::Upsert {
                collection: u.target.collection,
                key: u.target.key.expect("UPSERT requires key"),
                assignments: u.assignments,
                replace: u.replace,
            },
            Statement::Delete(d) => match d.target {
                DeleteTarget::Document(target) => LogicalPlan::Delete {
                    collection: target.collection,
                    key: target.key,
                    filter: d.filter,
                },
                DeleteTarget::Edge { from, label, to } => {
                    // Convert targets to document IDs
                    let from_id = if let Some(key) = from.key {
                        format!("{}:{}", from.collection, key)
                    } else {
                        from.collection
                    };
                    let to_id = to.map(|t| {
                        if let Some(key) = t.key {
                            format!("{}:{}", t.collection, key)
                        } else {
                            t.collection
                        }
                    });
                    LogicalPlan::DeleteEdge {
                        from: from_id,
                        label,
                        to: to_id,
                    }
                }
            },
            Statement::Begin(_) => LogicalPlan::Begin,
            Statement::Commit(_) => LogicalPlan::Commit,
            Statement::Rollback(_) => LogicalPlan::Rollback,
            Statement::DefineCollection(ast) => {
                // Convert FieldDefAst to schema::FieldDef
                let fields = ast
                    .fields
                    .into_iter()
                    .map(|f| FieldDef {
                        name: f.name,
                        field_type: f.field_type,
                        required: f.required,
                        default: f.default,
                    })
                    .collect();

                LogicalPlan::DefineCollection {
                    name: ast.name,
                    mode: ast.mode,
                    fields,
                    indexes: ast.indexes,
                }
            }
            Statement::DropCollection(ast) => LogicalPlan::DropCollection {
                name: ast.name,
                cascade: ast.cascade,
            },
            Statement::Describe(ast) => match ast {
                DescribeAst::Collection(name) => LogicalPlan::DescribeCollection(name),
                DescribeAst::Collections => LogicalPlan::DescribeCollections,
            },
            Statement::CreateIndex(ast) => LogicalPlan::CreateIndex {
                collection: ast.collection,
                fields: ast.fields,
                unique: ast.unique,
                index_type: ast.index_type,
                hnsw_params: ast.hnsw_params,
                analyzer: ast.analyzer,
            },
            Statement::DropIndex(ast) => LogicalPlan::DropIndex {
                name: ast.name,
                collection: ast.collection,
            },
            Statement::Explain(ast) => {
                let inner_plan = LogicalPlan::from_ast(*ast.statement);
                LogicalPlan::Explain {
                    analyze: ast.analyze,
                    plan: Box::new(inner_plan),
                    materialize_time_micros: None,
                }
            }
            Statement::Let(let_ast) => LogicalPlan::Let {
                name: let_ast.name.clone(),
                expr: let_ast.expr.clone(),
            },
            Statement::Reindex(ast) => LogicalPlan::Reindex {
                name: ast.name,
                collection: ast.collection,
            },
            Statement::DefineAnalyzer(_) | Statement::DropAnalyzer(_) => {
                // DDL statements for analyzers are handled directly by the executor
                // and don't need a query plan
                unimplemented!("Analyzer DDL statements are handled directly by the executor")
            }
            Statement::Relate(_) => {
                // RELATE statements are handled directly by the executor
                unimplemented!("RELATE statements are handled directly by the executor")
            }
            Statement::Set(_) => {
                unreachable!("SET handled in execute_batch")
            }
            Statement::Expr(_) => {
                // Expression statements are handled directly by the executor
                unimplemented!("Expression statements are handled directly by the executor")
            }
            Statement::CreateUser(_)
            | Statement::DropUser(_)
            | Statement::AlterUser(_)
            | Statement::CreateApiKey(_)
            | Statement::DropApiKey(_)
            | Statement::CreatePolicy(_)
            | Statement::DropPolicy(_)
            | Statement::AlterCollectionAttrs(_) => {
                // Auth DDL statements are handled directly by the executor
                unimplemented!("Auth statements are handled directly by the executor")
            }
        }
    }
}

/// Generate the default alias for an aggregate call: e.g. "sum(age)", "count(*)".
fn agg_alias(call: &AggregateCall) -> String {
    match &call.arg {
        AggregateArg::Wildcard => format!("{}(*)", call.function.to_lowercase()),
        AggregateArg::Field(f) => format!("{}({})", call.function.to_lowercase(), f),
    }
}

/// Walk an expression tree, collect `Expr::Aggregate` nodes into `aggregates`
/// (deduplicating by alias), and return a rewritten expression where each
/// aggregate is replaced by `Expr::Field(alias)`.
fn extract_aggregates(expr: &Expr, aggregates: &mut Vec<ResolvedAggregate>) -> ((), Expr) {
    match expr {
        Expr::Aggregate(call) => {
            let alias = agg_alias(call);
            // Only add if not already present
            if !aggregates.iter().any(|a| a.alias == alias)
                && let Some(func) = AggregateFunction::from_name(&call.function)
            {
                aggregates.push(ResolvedAggregate {
                    function: func,
                    arg: call.arg.clone(),
                    alias: alias.clone(),
                    distinct: call.distinct,
                });
            }
            ((), Expr::Field(alias))
        }
        Expr::BinaryOp { left, op, right } => {
            let (_, l) = extract_aggregates(left, aggregates);
            let (_, r) = extract_aggregates(right, aggregates);
            (
                (),
                Expr::BinaryOp {
                    left: Box::new(l),
                    op: *op,
                    right: Box::new(r),
                },
            )
        }
        Expr::UnaryOp { op, expr: inner } => {
            let (_, e) = extract_aggregates(inner, aggregates);
            (
                (),
                Expr::UnaryOp {
                    op: *op,
                    expr: Box::new(e),
                },
            )
        }
        Expr::FunctionCall {
            name,
            namespace,
            args,
        } => {
            let new_args: Vec<Expr> = args
                .iter()
                .map(|a| extract_aggregates(a, aggregates).1)
                .collect();
            (
                (),
                Expr::FunctionCall {
                    name: name.clone(),
                    namespace: namespace.clone(),
                    args: new_args,
                },
            )
        }
        Expr::Array(items) => {
            let new_items: Vec<Expr> = items
                .iter()
                .map(|e| extract_aggregates(e, aggregates).1)
                .collect();
            ((), Expr::Array(new_items))
        }
        Expr::Object(pairs) => {
            let new_pairs: Vec<(String, Expr)> = pairs
                .iter()
                .map(|(k, e)| (k.clone(), extract_aggregates(e, aggregates).1))
                .collect();
            ((), Expr::Object(new_pairs))
        }
        // Leaf nodes - no aggregates to extract
        other => ((), other.clone()),
    }
}

/// Extract the expression for SELECT VALUE.
/// For SELECT VALUE, we expect exactly one expression in the projection.
fn extract_value_expr(projection: &Projection) -> Option<Expr> {
    match projection {
        Projection::Items(items) if items.len() == 1 => Some(items[0].expr.clone()),
        Projection::All => None, // SELECT VALUE * doesn't make sense, handled elsewhere
        _ => None,
    }
}
