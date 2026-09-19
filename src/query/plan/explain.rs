//! EXPLAIN plan representation and formatting

use crate::document::Value;
use crate::query::ast::{BinaryOp, Expr, OrderDirection, Projection, UnaryOp};
use crate::query::plan::physical::{IndexLookup, PhysicalOp, RangeOp};

/// Statistics collected during EXPLAIN ANALYZE
#[derive(Debug, Clone, Default)]
pub struct OperatorStats {
    pub rows_out: u64,
    pub time_micros: u64,
    pub self_time_micros: u64,
}

/// Format Value for human-readable output
fn format_value(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Decimal(d) => d.as_decimal().to_string(),
        Value::String(s) => format!("\"{}\"", s),
        Value::Array(arr) => {
            let items: Vec<_> = arr.iter().map(format_value).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Object(_) => "{...}".to_string(),
        Value::Reference(id) => format!("ref:{}", id),
        Value::Datetime(ms) => chrono::DateTime::from_timestamp_millis(*ms)
            .map(|dt| format!("d\"{}\"", dt.to_rfc3339()))
            .unwrap_or_else(|| format!("d\"{}\"", ms)),
        Value::Duration(ns) => format!("{}ns", ns),
        Value::Bytes(_) => "[bytes]".to_string(),
        Value::Range { start, end } => format!("{}..{}", format_value(start), format_value(end)),
    }
}

/// Format Expr for human-readable output
fn format_expr(e: &Expr) -> String {
    match e {
        Expr::Literal(v) => format_value(v),
        Expr::Field(f) => f.clone(),
        Expr::BinaryOp { left, op, right } => {
            let op_str = match op {
                BinaryOp::Add => "+",
                BinaryOp::Sub => "-",
                BinaryOp::Mul => "*",
                BinaryOp::Div => "/",
                BinaryOp::Eq => "=",
                BinaryOp::Ne => "!=",
                BinaryOp::Gt => ">",
                BinaryOp::Gte => ">=",
                BinaryOp::Lt => "<",
                BinaryOp::Lte => "<=",
                BinaryOp::And => "AND",
                BinaryOp::Or => "OR",
                BinaryOp::Coalesce => "??",
            };
            if matches!(op, BinaryOp::Or) {
                format!("({} {} {})", format_expr(left), op_str, format_expr(right))
            } else {
                format!("{} {} {}", format_expr(left), op_str, format_expr(right))
            }
        }
        Expr::UnaryOp { op, expr } => match op {
            UnaryOp::Neg => format!("-{}", format_expr(expr)),
            UnaryOp::Not => format!("NOT {}", format_expr(expr)),
            UnaryOp::IsNull => format!("{} IS NULL", format_expr(expr)),
            UnaryOp::IsNotNull => format!("{} IS NOT NULL", format_expr(expr)),
            UnaryOp::IsNone => format!("{} IS NONE", format_expr(expr)),
            UnaryOp::IsNotNone => format!("{} IS NOT NONE", format_expr(expr)),
        },
        Expr::FunctionCall { name, args, .. } => {
            let args_str: Vec<String> = args.iter().map(format_expr).collect();
            format!("{}({})", name.to_uppercase(), args_str.join(", "))
        }
        Expr::Aggregate(a) => match &a.arg {
            crate::query::ast::AggregateArg::Wildcard => format!("{}(*)", a.function),
            crate::query::ast::AggregateArg::Field(f) => format!("{}({})", a.function, f),
        },
        Expr::InList {
            expr,
            list,
            negated,
        } => {
            let items: Vec<String> = list.iter().map(format_expr).collect();
            let not = if *negated { "NOT " } else { "" };
            format!("{} {}IN [{}]", format_expr(expr), not, items.join(", "))
        }
        Expr::InSubquery { expr, negated, .. } => {
            let not = if *negated { "NOT " } else { "" };
            format!("{} {}IN (SELECT ...)", format_expr(expr), not)
        }
        Expr::InExpr {
            expr,
            target,
            negated,
        } => {
            let not = if *negated { "NOT " } else { "" };
            format!("{} {}IN {}", format_expr(expr), not, format_expr(target))
        }
        Expr::Subquery(_) => "(SELECT ...)".to_string(),
        Expr::ParentRef(p) => {
            let parents = "parent.".repeat(p.depth);
            format!("$parent.{}{}", parents, p.field)
        }
        Expr::ValueRef(None) => "$value".to_string(),
        Expr::ValueRef(Some(path)) => format!("$value.{}", path),
        Expr::Array(items) => {
            let parts: Vec<String> = items.iter().map(format_expr).collect();
            format!("[{}]", parts.join(", "))
        }
        Expr::Object(pairs) => {
            let parts: Vec<String> = pairs
                .iter()
                .map(|(k, v)| format!("{}: {}", k, format_expr(v)))
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
        Expr::Index { base, index } => {
            format!("{}[{}]", format_expr(base), format_expr(index))
        }
        Expr::Slice { base, start, end } => {
            let start_str = start.as_ref().map(|e| format_expr(e)).unwrap_or_default();
            let end_str = end.as_ref().map(|e| format_expr(e)).unwrap_or_default();
            format!("{}[{}..{}]", format_expr(base), start_str, end_str)
        }
        Expr::FieldAccess { base, field } => {
            format!("{}.{}", format_expr(base), field)
        }
        Expr::Constant { namespace, name } => format!("{}::{}", namespace, name),
        Expr::Variable(name) => format!("${}", name),
        Expr::Fts { target, query, .. } => {
            let target_str = match target {
                crate::query::ast::FtsTarget::Field(f) => f.clone(),
                crate::query::ast::FtsTarget::Match { .. } => "MATCH(...)".to_string(),
            };
            format!("{} @@ \"{}\"", target_str, query)
        }
        Expr::KnnSearch { field, k, .. } => {
            format!("{} <|{}|> [...]", field, k)
        }
        Expr::Traversal(_) => "->...".to_string(),
        Expr::If { .. } => "IF...END".to_string(),
        Expr::For { binding, .. } => format!("FOR ${}...END", binding),
        Expr::Break => "BREAK".to_string(),
        Expr::Continue => "CONTINUE".to_string(),
        Expr::Range { start, end } => {
            format!("{}..{}", format_expr(start), format_expr(end))
        }
    }
}

/// Node in the explain tree for display
#[derive(Debug, Clone)]
pub struct ExplainNode {
    pub operator: String,
    pub details: Vec<(String, String)>,
    pub children: Vec<ExplainNode>,
    pub stats: Option<OperatorStats>,
}

impl ExplainNode {
    /// Create ExplainNode from PhysicalOp
    pub fn from_physical(plan: &PhysicalOp) -> Self {
        match plan {
            PhysicalOp::TableScan { collection, limit } => ExplainNode {
                operator: "TableScan".into(),
                details: {
                    let mut d = vec![("collection".into(), collection.clone())];
                    if let Some(l) = limit {
                        d.push(("limit".into(), l.to_string()));
                    }
                    d
                },
                children: vec![],
                stats: None,
            },

            PhysicalOp::KeyLookup { collection, key } => ExplainNode {
                operator: "KeyLookup".into(),
                details: vec![
                    ("collection".into(), collection.clone()),
                    ("key".into(), key.clone()),
                ],
                children: vec![],
                stats: None,
            },

            PhysicalOp::IndexScan {
                collection,
                index,
                lookup,
            } => {
                let lookup_str = match lookup {
                    IndexLookup::Eq { field, value } => {
                        format!("{} = {}", field, format_value(value))
                    }
                    IndexLookup::Range { field, op, value } => {
                        let op_str = match op {
                            RangeOp::Gt => ">",
                            RangeOp::Gte => ">=",
                            RangeOp::Lt => "<",
                            RangeOp::Lte => "<=",
                        };
                        format!("{} {} {}", field, op_str, format_value(value))
                    }
                    IndexLookup::RangeBetween {
                        field,
                        lower,
                        lower_inclusive,
                        upper,
                        upper_inclusive,
                    } => {
                        let lower_op = if *lower_inclusive { ">=" } else { ">" };
                        let upper_op = if *upper_inclusive { "<=" } else { "<" };
                        format!(
                            "{} {} {} AND {} {} {}",
                            field,
                            lower_op,
                            format_value(lower),
                            field,
                            upper_op,
                            format_value(upper)
                        )
                    }
                    IndexLookup::CompoundEq { field_values } => field_values
                        .iter()
                        .map(|(f, v)| format!("{} = {}", f, format_value(v)))
                        .collect::<Vec<_>>()
                        .join(" AND "),
                    IndexLookup::In { field, values } => {
                        let vals: Vec<String> = values.iter().map(format_value).collect();
                        format!("{} IN [{}]", field, vals.join(", "))
                    }
                };
                ExplainNode {
                    operator: "IndexScan".into(),
                    details: vec![
                        ("collection".into(), collection.clone()),
                        ("index".into(), format!("[{}]", index.fields.join(", "))),
                        ("lookup".into(), lookup_str),
                    ],
                    children: vec![],
                    stats: None,
                }
            }

            PhysicalOp::Filter { predicate, input } => ExplainNode {
                operator: "Filter".into(),
                details: vec![("predicate".into(), format_expr(predicate))],
                children: vec![Self::from_physical(input)],
                stats: None,
            },

            PhysicalOp::Project { fields, input } => {
                let fields_str = match fields {
                    Projection::All => "*".to_string(),
                    Projection::Items(items) => {
                        let names: Vec<String> = items
                            .iter()
                            .map(|i| {
                                let name = format_expr(&i.expr);
                                match &i.alias {
                                    Some(a) => format!("{} AS {}", name, a),
                                    None => name,
                                }
                            })
                            .collect();
                        format!("[{}]", names.join(", "))
                    }
                };
                ExplainNode {
                    operator: "Project".into(),
                    details: vec![("fields".into(), fields_str)],
                    children: vec![Self::from_physical(input)],
                    stats: None,
                }
            }

            PhysicalOp::Limit { count, input } => ExplainNode {
                operator: "Limit".into(),
                details: vec![("count".into(), count.to_string())],
                children: vec![Self::from_physical(input)],
                stats: None,
            },

            PhysicalOp::Sort { items, input } => {
                let fields: Vec<String> = items
                    .iter()
                    .map(|i| {
                        let dir = match i.direction {
                            OrderDirection::Asc => "ASC",
                            OrderDirection::Desc => "DESC",
                        };
                        format!("{:?} {}", i.expr, dir)
                    })
                    .collect();
                ExplainNode {
                    operator: "Sort".into(),
                    details: vec![("by".into(), fields.join(", "))],
                    children: vec![Self::from_physical(input)],
                    stats: None,
                }
            }

            PhysicalOp::Aggregate { aggregates, input } => {
                let agg_str = aggregates
                    .iter()
                    .map(|a| a.alias.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                ExplainNode {
                    operator: "Aggregate".into(),
                    details: vec![("functions".into(), agg_str)],
                    children: vec![Self::from_physical(input)],
                    stats: None,
                }
            }

            PhysicalOp::GroupAggregate {
                group_fields,
                aggregates,
                input,
            } => {
                let agg_str = aggregates
                    .iter()
                    .map(|a| a.alias.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                let group_str = group_fields.join(", ");
                ExplainNode {
                    operator: "GroupAggregate".into(),
                    details: vec![
                        ("group_by".into(), group_str),
                        ("functions".into(), agg_str),
                    ],
                    children: vec![Self::from_physical(input)],
                    stats: None,
                }
            }

            PhysicalOp::Insert {
                collection,
                documents,
            } => ExplainNode {
                operator: "Insert".into(),
                details: vec![
                    ("collection".into(), collection.clone()),
                    ("documents".into(), documents.len().to_string()),
                ],
                children: vec![],
                stats: None,
            },

            PhysicalOp::Create {
                collection, key, ..
            } => ExplainNode {
                operator: "Create".into(),
                details: vec![
                    ("collection".into(), collection.clone()),
                    (
                        "key".into(),
                        key.clone().unwrap_or_else(|| "generated".into()),
                    ),
                ],
                children: vec![],
                stats: None,
            },

            PhysicalOp::Update {
                collection,
                assignments,
                input,
            } => ExplainNode {
                operator: "Update".into(),
                details: vec![
                    ("collection".into(), collection.clone()),
                    (
                        "assignments".into(),
                        assignments
                            .iter()
                            .map(|a| format!("{} = {}", a.path.join("."), format_expr(&a.expr)))
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ],
                children: vec![Self::from_physical(input)],
                stats: None,
            },

            PhysicalOp::Delete { collection, input } => ExplainNode {
                operator: "Delete".into(),
                details: vec![("collection".into(), collection.clone())],
                children: vec![Self::from_physical(input)],
                stats: None,
            },

            PhysicalOp::InMemoryScan { rows } => ExplainNode {
                operator: "InMemoryScan".into(),
                details: vec![("rows".into(), rows.len().to_string())],
                children: vec![],
                stats: None,
            },

            PhysicalOp::FtsScan {
                collection,
                target,
                operator: _,
                query,
                limit,
            } => {
                let target_str = match target {
                    crate::query::ast::FtsTarget::Field(f) => f.clone(),
                    crate::query::ast::FtsTarget::Match { fields, .. } => {
                        let field_names: Vec<String> = fields
                            .iter()
                            .map(|f| {
                                if let Some(boost) = f.boost {
                                    format!("{}^{}", f.name, boost)
                                } else {
                                    f.name.clone()
                                }
                            })
                            .collect();
                        format!("MATCH({})", field_names.join(", "))
                    }
                };
                let mut details = vec![
                    ("collection".into(), collection.clone()),
                    ("target".into(), target_str),
                    ("query".into(), format!("\"{}\"", query)),
                ];
                if let Some(l) = limit {
                    details.push(("limit".into(), l.to_string()));
                }
                ExplainNode {
                    operator: "FtsScan".into(),
                    details,
                    children: vec![],
                    stats: None,
                }
            }

            PhysicalOp::KnnScan {
                collection,
                field,
                vector,
                k,
                effort,
            } => {
                let mut details = vec![
                    ("collection".into(), collection.clone()),
                    ("field".into(), field.clone()),
                    ("vector".into(), format_expr(vector)),
                    ("k".into(), k.to_string()),
                ];
                if let Some(ef) = effort {
                    details.push(("effort".into(), ef.to_string()));
                }
                ExplainNode {
                    operator: "KnnScan".into(),
                    details,
                    children: vec![],
                    stats: None,
                }
            }

            PhysicalOp::Union {
                inputs,
                needs_dedup,
            } => ExplainNode {
                operator: "Union".into(),
                details: vec![("needs_dedup".into(), needs_dedup.to_string())],
                children: inputs.iter().map(Self::from_physical).collect(),
                stats: None,
            },

            PhysicalOp::Distinct { input } => ExplainNode {
                operator: "Distinct".into(),
                details: vec![],
                children: vec![Self::from_physical(input)],
                stats: None,
            },

            PhysicalOp::Value {
                expr,
                distinct,
                input,
            } => {
                let mut details = vec![("expr".into(), format_expr(expr))];
                if *distinct {
                    details.push(("distinct".into(), "true".to_string()));
                }
                ExplainNode {
                    operator: "Value".into(),
                    details,
                    children: vec![Self::from_physical(input)],
                    stats: None,
                }
            }

            PhysicalOp::Offset { count, input } => ExplainNode {
                operator: "Offset".into(),
                details: vec![("count".into(), count.to_string())],
                children: vec![Self::from_physical(input)],
                stats: None,
            },

            PhysicalOp::Upsert {
                collection,
                key,
                assignments,
                replace,
            } => {
                let mode = if *replace { "replace" } else { "merge" };
                ExplainNode {
                    operator: "Upsert".into(),
                    details: vec![
                        ("target".into(), format!("{}:{}", collection, key)),
                        ("mode".into(), mode.to_string()),
                        (
                            "assignments".into(),
                            assignments
                                .iter()
                                .map(|a| format!("{} = {}", a.path.join("."), format_expr(&a.expr)))
                                .collect::<Vec<_>>()
                                .join(", "),
                        ),
                    ],
                    children: vec![],
                    stats: None,
                }
            }

            PhysicalOp::DeleteEdge { from, label, to } => {
                let mut details = vec![
                    ("from".into(), from.clone()),
                    ("label".into(), label.clone()),
                ];
                if let Some(t) = to {
                    details.push(("to".into(), t.clone()));
                }
                ExplainNode {
                    operator: "DeleteEdge".into(),
                    details,
                    children: vec![],
                    stats: None,
                }
            }
        }
    }

    /// Recursively compute self_time_micros for each node.
    /// self_time = total_time - sum(children's total_time)
    pub fn compute_self_times(&mut self) {
        for child in &mut self.children {
            child.compute_self_times();
        }

        if let Some(stats) = &mut self.stats {
            let children_time: u64 = self
                .children
                .iter()
                .filter_map(|c| c.stats.as_ref())
                .map(|s| s.time_micros)
                .sum();
            stats.self_time_micros = stats.time_micros.saturating_sub(children_time);
        }
    }

    /// Format as tree text
    pub fn format(&self, indent: usize) -> String {
        let mut result = String::new();

        // Build prefix
        let prefix = if indent == 0 {
            String::new()
        } else {
            format!("{}└─ ", "   ".repeat(indent - 1))
        };

        // Operator and details
        let details = self
            .details
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect::<Vec<_>>()
            .join(", ");

        result.push_str(&format!("{}{}({})", prefix, self.operator, details));

        // Stats if present
        if let Some(stats) = &self.stats {
            result.push_str(&format!(
                " [rows: {}, self: {:.2}ms, total: {:.2}ms]",
                stats.rows_out,
                stats.self_time_micros as f64 / 1000.0,
                stats.time_micros as f64 / 1000.0
            ));
        }

        result.push('\n');

        // Children
        for child in &self.children {
            result.push_str(&child.format(indent + 1));
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Value;
    use crate::query::plan::physical::IndexRef;

    #[test]
    fn test_explain_node_table_scan() {
        let plan = PhysicalOp::TableScan {
            collection: "users".to_string(),
            limit: None,
        };
        let node = ExplainNode::from_physical(&plan);
        assert_eq!(node.operator, "TableScan");
        assert!(node.format(0).contains("collection: users"));
    }

    #[test]
    fn test_explain_node_with_limit() {
        let plan = PhysicalOp::Limit {
            count: 10,
            input: Box::new(PhysicalOp::TableScan {
                collection: "users".to_string(),
                limit: None,
            }),
        };
        let node = ExplainNode::from_physical(&plan);
        let text = node.format(0);
        assert!(text.contains("Limit(count: 10)"));
        assert!(text.contains("TableScan"));
    }

    #[test]
    fn test_explain_node_index_scan() {
        let plan = PhysicalOp::IndexScan {
            collection: "users".to_string(),
            index: IndexRef {
                collection: "users".to_string(),
                fields: vec!["age".to_string()],
            },
            lookup: IndexLookup::Eq {
                field: "age".to_string(),
                value: Value::Int(25),
            },
        };
        let node = ExplainNode::from_physical(&plan);
        let text = node.format(0);
        assert!(text.contains("IndexScan"));
        assert!(text.contains("age = "));
    }

    #[test]
    fn test_explain_format_nested() {
        let plan = PhysicalOp::Limit {
            count: 5,
            input: Box::new(PhysicalOp::Filter {
                predicate: crate::query::ast::Expr::BinaryOp {
                    left: Box::new(crate::query::ast::Expr::Field("x".to_string())),
                    op: crate::query::ast::BinaryOp::Eq,
                    right: Box::new(crate::query::ast::Expr::Literal(Value::Int(1))),
                },
                input: Box::new(PhysicalOp::TableScan {
                    collection: "test".to_string(),
                    limit: None,
                }),
            }),
        };
        let node = ExplainNode::from_physical(&plan);
        let text = node.format(0);

        assert!(text.contains("Limit"));
        assert!(text.contains("└─ Filter"));
        assert!(text.contains("└─ TableScan"));
    }

    #[test]
    fn test_explain_with_stats() {
        let node = ExplainNode {
            operator: "TableScan".to_string(),
            details: vec![("collection".to_string(), "users".to_string())],
            children: vec![],
            stats: Some(OperatorStats {
                rows_out: 100,
                time_micros: 1500,
                self_time_micros: 1500,
            }),
        };
        let text = node.format(0);
        assert!(text.contains("[rows: 100, self: 1.50ms, total: 1.50ms]"));
    }
}
