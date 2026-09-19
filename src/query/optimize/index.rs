// src/query/optimize/index.rs

use crate::document::Value;
use crate::query::ast::expr::{FtsOperator, FtsTarget};
use crate::query::ast::{BinaryOp, Expr};
use crate::query::execute::ExecutionContext;
use crate::query::plan::{IndexLookup, IndexRef, RangeOp};

/// Extracted KNN expression info
#[derive(Debug, Clone)]
pub struct KnnInfo {
    /// Field containing the vectors
    pub field: String,
    /// Query vector expression
    pub vector: Expr,
    /// Number of nearest neighbors to return
    pub k: usize,
    /// Optional ef_search parameter for accuracy/speed tradeoff
    pub effort: Option<usize>,
}

/// Result of analyzing a predicate for KNN expressions
#[derive(Debug)]
pub struct KnnAnalysis {
    /// KNN expressions found in the predicate
    pub knn_exprs: Vec<KnnInfo>,
    /// Remaining predicate after removing KNN expressions
    pub remaining: Option<Expr>,
}

/// Result of analyzing a predicate for index usage
#[derive(Debug)]
pub enum IndexChoice {
    /// Use index
    Use {
        index_ref: IndexRef,
        lookup: Box<IndexLookup>,
        /// Remaining predicate that couldn't be pushed to index
        remaining: Option<Expr>,
    },
    /// Don't use index
    Skip { reason: SkipReason },
}

/// Extracted FTS expression info
#[derive(Debug, Clone)]
pub struct FtsInfo {
    pub target: FtsTarget,
    pub operator: FtsOperator,
    pub query: String,
}

/// Result of analyzing a predicate for FTS expressions
#[derive(Debug)]
pub struct FtsAnalysis {
    /// FTS expressions found in the predicate
    pub fts_exprs: Vec<FtsInfo>,
    /// Remaining predicate after removing FTS expressions
    pub remaining: Option<Expr>,
}

#[derive(Debug)]
pub enum SkipReason {
    NoIndex,
    OrCondition,
    NotSupported,
}

/// Extract field name, operator, and value from a simple comparison expression.
fn as_simple_condition(expr: &Expr) -> Option<(&str, BinaryOp, &crate::document::Value)> {
    match expr {
        Expr::BinaryOp { left, op, right } => match (left.as_ref(), right.as_ref()) {
            (Expr::Field(f), Expr::Literal(v)) => Some((f.as_str(), *op, v)),
            _ => None,
        },
        _ => None,
    }
}

/// Analyze a filter expression to find index usage opportunities
pub fn analyze_for_index<Ctx: ExecutionContext>(
    predicate: &Expr,
    ctx: &Ctx,
    collection: &str,
) -> IndexChoice {
    // For AND expressions, try specialized optimizations first
    if matches!(
        predicate,
        Expr::BinaryOp {
            op: BinaryOp::And,
            ..
        }
    ) {
        // 1. Try range BETWEEN optimization (field >= A AND field <= B)
        if let Some(choice) = try_range_between(predicate, ctx, collection) {
            return choice;
        }

        // 2. Try compound index for multiple equality conditions
        let mut conditions = Vec::new();
        collect_equality_conditions(predicate, &mut conditions);

        if conditions.len() >= 2 {
            // Try to find a compound index
            for i in 0..conditions.len() {
                for j in 0..conditions.len() {
                    if i != j {
                        let fields: Vec<&str> = vec![&conditions[i].0, &conditions[j].0];
                        if let Some(index_fields) = ctx.find_compound_index(collection, &fields) {
                            // Reorder field_values to match index field order
                            let mut ordered_field_values = Vec::new();
                            for idx_field in &index_fields {
                                for cond in &conditions {
                                    if &cond.0 == idx_field {
                                        ordered_field_values.push(cond.clone());
                                        break;
                                    }
                                }
                            }
                            // Compute remaining conditions not covered by the compound index
                            let remaining = compute_compound_remaining(predicate, &index_fields);
                            return IndexChoice::Use {
                                index_ref: IndexRef {
                                    collection: collection.to_string(),
                                    fields: index_fields,
                                },
                                lookup: Box::new(IndexLookup::CompoundEq {
                                    field_values: ordered_field_values,
                                }),
                                remaining,
                            };
                        }
                    }
                }
            }
        }
    }

    // Fall back to single-field index
    analyze_single_field(predicate, ctx, collection)
}

fn analyze_single_field<Ctx: ExecutionContext>(
    predicate: &Expr,
    ctx: &Ctx,
    collection: &str,
) -> IndexChoice {
    // Try to extract a simple field op value condition
    if let Some((field, op, value)) = as_simple_condition(predicate) {
        if let Some(index_fields) = ctx.find_index_for_field(collection, field) {
            let index_ref = IndexRef {
                collection: collection.to_string(),
                fields: index_fields,
            };

            match op {
                BinaryOp::Eq => IndexChoice::Use {
                    index_ref,
                    lookup: Box::new(IndexLookup::Eq {
                        field: field.to_string(),
                        value: value.clone(),
                    }),
                    remaining: None,
                },
                BinaryOp::Gt => IndexChoice::Use {
                    index_ref,
                    lookup: Box::new(IndexLookup::Range {
                        field: field.to_string(),
                        op: RangeOp::Gt,
                        value: value.clone(),
                    }),
                    remaining: None,
                },
                BinaryOp::Gte => IndexChoice::Use {
                    index_ref,
                    lookup: Box::new(IndexLookup::Range {
                        field: field.to_string(),
                        op: RangeOp::Gte,
                        value: value.clone(),
                    }),
                    remaining: None,
                },
                BinaryOp::Lt => IndexChoice::Use {
                    index_ref,
                    lookup: Box::new(IndexLookup::Range {
                        field: field.to_string(),
                        op: RangeOp::Lt,
                        value: value.clone(),
                    }),
                    remaining: None,
                },
                BinaryOp::Lte => IndexChoice::Use {
                    index_ref,
                    lookup: Box::new(IndexLookup::Range {
                        field: field.to_string(),
                        op: RangeOp::Lte,
                        value: value.clone(),
                    }),
                    remaining: None,
                },
                BinaryOp::Ne => IndexChoice::Skip {
                    reason: SkipReason::NotSupported,
                },
                _ => IndexChoice::Skip {
                    reason: SkipReason::NotSupported,
                },
            }
        } else {
            IndexChoice::Skip {
                reason: SkipReason::NoIndex,
            }
        }
    } else {
        match predicate {
            Expr::BinaryOp {
                left,
                op: BinaryOp::And,
                right,
            } => {
                // Try left first, then right
                // When one side uses an index, the other side becomes the remaining predicate
                let left_choice = analyze_single_field(left, ctx, collection);
                match left_choice {
                    IndexChoice::Use {
                        index_ref,
                        lookup,
                        remaining,
                    } => {
                        // Combine existing remaining with right side
                        let new_remaining = match remaining {
                            Some(rem) => Some(Expr::BinaryOp {
                                left: Box::new(rem),
                                op: BinaryOp::And,
                                right: right.clone(),
                            }),
                            None => Some((**right).clone()),
                        };
                        IndexChoice::Use {
                            index_ref,
                            lookup,
                            remaining: new_remaining,
                        }
                    }
                    IndexChoice::Skip { .. } => {
                        // Try right side
                        let right_choice = analyze_single_field(right, ctx, collection);
                        match right_choice {
                            IndexChoice::Use {
                                index_ref,
                                lookup,
                                remaining,
                            } => {
                                // Combine existing remaining with left side
                                let new_remaining = match remaining {
                                    Some(rem) => Some(Expr::BinaryOp {
                                        left: left.clone(),
                                        op: BinaryOp::And,
                                        right: Box::new(rem),
                                    }),
                                    None => Some((**left).clone()),
                                };
                                IndexChoice::Use {
                                    index_ref,
                                    lookup,
                                    remaining: new_remaining,
                                }
                            }
                            skip => skip,
                        }
                    }
                }
            }
            Expr::BinaryOp {
                op: BinaryOp::Or, ..
            } => IndexChoice::Skip {
                reason: SkipReason::OrCondition,
            },
            _ => IndexChoice::Skip {
                reason: SkipReason::NotSupported,
            },
        }
    }
}

/// Extract all equality conditions from an AND expression
fn collect_equality_conditions(expr: &Expr, conditions: &mut Vec<(String, Value)>) {
    if let Some((field, BinaryOp::Eq, value)) = as_simple_condition(expr) {
        conditions.push((field.to_string(), value.clone()));
    } else if let Expr::BinaryOp {
        left,
        op: BinaryOp::And,
        right,
    } = expr
    {
        collect_equality_conditions(left, conditions);
        collect_equality_conditions(right, conditions);
    }
}

/// Compute remaining predicate after extracting compound index conditions.
/// Removes equality conditions on the specified fields and returns what's left.
fn compute_compound_remaining(expr: &Expr, used_fields: &[String]) -> Option<Expr> {
    // Check if this is an equality condition on one of the used fields
    if let Some((field, BinaryOp::Eq, _)) = as_simple_condition(expr)
        && used_fields.iter().any(|f| f == field)
    {
        // This condition is covered by the compound index, remove it
        return None;
    }

    match expr {
        // AND expression: recursively process both sides
        Expr::BinaryOp {
            left,
            op: BinaryOp::And,
            right,
        } => {
            let left_remaining = compute_compound_remaining(left, used_fields);
            let right_remaining = compute_compound_remaining(right, used_fields);

            match (left_remaining, right_remaining) {
                (None, None) => None,
                (Some(l), None) => Some(l),
                (None, Some(r)) => Some(r),
                (Some(l), Some(r)) => Some(Expr::BinaryOp {
                    left: Box::new(l),
                    op: BinaryOp::And,
                    right: Box::new(r),
                }),
            }
        }
        // Any other expression: keep as-is (not covered by compound index)
        other => Some(other.clone()),
    }
}

/// Range condition info for BETWEEN detection
struct RangeBound {
    field: String,
    value: Value,
    inclusive: bool,
    is_lower: bool, // true = lower bound (>, >=), false = upper bound (<, <=)
}

/// Try to detect BETWEEN pattern: field >= A AND field <= B
fn try_range_between<Ctx: ExecutionContext>(
    predicate: &Expr,
    ctx: &Ctx,
    collection: &str,
) -> Option<IndexChoice> {
    let mut bounds = Vec::new();
    collect_range_bounds(predicate, &mut bounds);

    // Group bounds by field
    let mut field_bounds: std::collections::HashMap<&str, Vec<&RangeBound>> =
        std::collections::HashMap::new();
    for bound in &bounds {
        field_bounds.entry(&bound.field).or_default().push(bound);
    }

    // Find a field with both lower and upper bounds
    for (field, field_bounds) in field_bounds {
        let lower = field_bounds.iter().find(|b| b.is_lower);
        let upper = field_bounds.iter().find(|b| !b.is_lower);

        if let (Some(lower), Some(upper)) = (lower, upper) {
            // Found BETWEEN pattern! Check if we have an index
            if let Some(index_fields) = ctx.find_index_for_field(collection, field) {
                return Some(IndexChoice::Use {
                    index_ref: IndexRef {
                        collection: collection.to_string(),
                        fields: index_fields,
                    },
                    lookup: Box::new(IndexLookup::RangeBetween {
                        field: field.to_string(),
                        lower: lower.value.clone(),
                        lower_inclusive: lower.inclusive,
                        upper: upper.value.clone(),
                        upper_inclusive: upper.inclusive,
                    }),
                    remaining: None,
                });
            }
        }
    }

    None
}

/// Collect range bounds from AND expression
fn collect_range_bounds(expr: &Expr, bounds: &mut Vec<RangeBound>) {
    if let Some((field, op, value)) = as_simple_condition(expr) {
        match op {
            BinaryOp::Gt => bounds.push(RangeBound {
                field: field.to_string(),
                value: value.clone(),
                inclusive: false,
                is_lower: true,
            }),
            BinaryOp::Gte => bounds.push(RangeBound {
                field: field.to_string(),
                value: value.clone(),
                inclusive: true,
                is_lower: true,
            }),
            BinaryOp::Lt => bounds.push(RangeBound {
                field: field.to_string(),
                value: value.clone(),
                inclusive: false,
                is_lower: false,
            }),
            BinaryOp::Lte => bounds.push(RangeBound {
                field: field.to_string(),
                value: value.clone(),
                inclusive: true,
                is_lower: false,
            }),
            _ => {}
        }
    } else if let Expr::BinaryOp {
        left,
        op: BinaryOp::And,
        right,
    } = expr
    {
        collect_range_bounds(left, bounds);
        collect_range_bounds(right, bounds);
    }
}

/// Analyze a top-level OR expression for union-based index optimization.
/// Returns Some(branches) if all OR branches can use an index, None otherwise.
pub fn analyze_or_for_union<Ctx: ExecutionContext>(
    expr: &Expr,
    ctx: &Ctx,
    collection: &str,
) -> Option<Vec<(IndexRef, IndexLookup)>> {
    // Only applies to OR expressions
    let mut flat = Vec::new();
    if !flatten_or(expr, &mut flat) {
        return None;
    }

    let mut branches = Vec::new();
    for branch in &flat {
        match analyze_for_index(branch, ctx, collection) {
            IndexChoice::Use {
                index_ref, lookup, ..
            } => {
                branches.push((index_ref, *lookup));
            }
            IndexChoice::Skip { .. } => return None,
        }
    }

    Some(branches)
}

/// Flatten nested OR into a list of non-OR branches.
/// Returns false if expr is not an OR at all.
fn flatten_or<'a>(expr: &'a Expr, out: &mut Vec<&'a Expr>) -> bool {
    match expr {
        Expr::BinaryOp {
            left,
            op: BinaryOp::Or,
            right,
        } => {
            flatten_or(left, out);
            flatten_or(right, out);
            true
        }
        _ => {
            out.push(expr);
            false
        }
    }
}

/// Analyze a predicate for FTS expressions.
///
/// Extracts all FTS expressions from AND-connected predicates and returns:
/// - The list of FTS expressions found
/// - The remaining non-FTS predicate (if any)
///
/// FTS expressions in OR predicates are not extracted (the whole predicate must be evaluated).
pub fn analyze_for_fts(predicate: &Expr) -> FtsAnalysis {
    let mut fts_exprs = Vec::new();
    let remaining = extract_fts_from_and(predicate, &mut fts_exprs);
    FtsAnalysis {
        fts_exprs,
        remaining,
    }
}

/// Recursively extract FTS expressions from AND-connected predicates.
/// Returns the remaining predicate after removing FTS expressions.
fn extract_fts_from_and(expr: &Expr, fts_exprs: &mut Vec<FtsInfo>) -> Option<Expr> {
    match expr {
        // Direct FTS expression
        Expr::Fts {
            target,
            operator,
            query,
        } => {
            fts_exprs.push(FtsInfo {
                target: target.clone(),
                operator: operator.clone(),
                query: query.clone(),
            });
            None // No remaining predicate
        }
        // AND expression: recursively process both sides
        Expr::BinaryOp {
            left,
            op: BinaryOp::And,
            right,
        } => {
            let left_remaining = extract_fts_from_and(left, fts_exprs);
            let right_remaining = extract_fts_from_and(right, fts_exprs);

            match (left_remaining, right_remaining) {
                (None, None) => None,
                (Some(l), None) => Some(l),
                (None, Some(r)) => Some(r),
                (Some(l), Some(r)) => Some(Expr::BinaryOp {
                    left: Box::new(l),
                    op: BinaryOp::And,
                    right: Box::new(r),
                }),
            }
        }
        // Any other expression: keep as-is
        other => Some(other.clone()),
    }
}

/// Analyze a predicate for KNN expressions.
///
/// Extracts all KNN expressions from AND-connected predicates and returns:
/// - The list of KNN expressions found
/// - The remaining non-KNN predicate (if any)
///
/// KNN expressions in OR predicates are not extracted (the whole predicate must be evaluated).
pub fn analyze_for_knn(predicate: &Expr) -> KnnAnalysis {
    let mut knn_exprs = Vec::new();
    let remaining = extract_knn_from_and(predicate, &mut knn_exprs);
    KnnAnalysis {
        knn_exprs,
        remaining,
    }
}

/// Recursively extract KNN expressions from AND-connected predicates.
/// Returns the remaining predicate after removing KNN expressions.
fn extract_knn_from_and(expr: &Expr, knn_exprs: &mut Vec<KnnInfo>) -> Option<Expr> {
    match expr {
        // Direct KNN expression
        Expr::KnnSearch {
            field,
            vector,
            k,
            effort,
        } => {
            knn_exprs.push(KnnInfo {
                field: field.clone(),
                vector: (**vector).clone(),
                k: *k,
                effort: *effort,
            });
            None // No remaining predicate
        }
        // AND expression: recursively process both sides
        Expr::BinaryOp {
            left,
            op: BinaryOp::And,
            right,
        } => {
            let left_remaining = extract_knn_from_and(left, knn_exprs);
            let right_remaining = extract_knn_from_and(right, knn_exprs);

            match (left_remaining, right_remaining) {
                (None, None) => None,
                (Some(l), None) => Some(l),
                (None, Some(r)) => Some(r),
                (Some(l), Some(r)) => Some(Expr::BinaryOp {
                    left: Box::new(l),
                    op: BinaryOp::And,
                    right: Box::new(r),
                }),
            }
        }
        // Any other expression: keep as-is
        other => Some(other.clone()),
    }
}

/// Determine if union results need deduplication.
/// No dedup needed when: all branches use the same single field with Eq lookups and distinct values.
pub fn compute_needs_dedup(branches: &[(IndexRef, IndexLookup)]) -> bool {
    if branches.len() < 2 {
        return false;
    }

    // Extract field from each lookup
    let fields: Vec<&str> = branches
        .iter()
        .map(|(_, lookup)| match lookup {
            IndexLookup::Eq { field, .. } => field.as_str(),
            IndexLookup::In { field, .. } => field.as_str(),
            IndexLookup::Range { field, .. } => field.as_str(),
            IndexLookup::RangeBetween { field, .. } => field.as_str(),
            IndexLookup::CompoundEq { field_values } => {
                field_values.first().map(|(f, _)| f.as_str()).unwrap_or("")
            }
        })
        .collect();

    // Same field: only safe if all are Eq with distinct values
    // NOTE: Different fields still need dedup because one document
    // can match multiple conditions (e.g., name='Alice' AND city='Moscow')
    let first_field = fields[0];
    let all_same_field = fields.iter().all(|f| *f == first_field);

    if all_same_field {
        // Check if all Eq with distinct values
        let mut eq_values: Vec<&Value> = vec![];
        for (_, lookup) in branches {
            match lookup {
                IndexLookup::Eq { value, .. } => {
                    if eq_values.contains(&value) {
                        return true; // Duplicate value
                    }
                    eq_values.push(value);
                }
                _ => return true, // Non-Eq on same field needs dedup
            }
        }
        return false; // All Eq with distinct values
    }

    // Mixed fields with some overlap - need dedup
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use crate::document::Value;
    use crate::query::ast::{BinaryOp, Expr};
    use std::sync::Arc;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Arc<Database>) {
        let tmp = TempDir::new().unwrap();
        let storage = Arc::new(Database::open(tmp.path()).unwrap());
        (tmp, storage)
    }

    #[test]
    fn test_or_union_both_indexed() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(name)");
        crate::run_sql!(storage, "CREATE INDEX ON users(age)");

        let expr = Expr::BinaryOp {
            left: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("name".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Value::String("Alice".to_string()))),
            }),
            op: BinaryOp::Or,
            right: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("age".to_string())),
                op: BinaryOp::Gt,
                right: Box::new(Expr::Literal(Value::Int(25))),
            }),
        };

        let result = analyze_or_for_union(&expr, &*storage, "users");
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 2);
    }

    #[test]
    fn test_or_union_one_not_indexed() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(name)");

        let expr = Expr::BinaryOp {
            left: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("name".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Value::String("Alice".to_string()))),
            }),
            op: BinaryOp::Or,
            right: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("city".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Value::String("NYC".to_string()))),
            }),
        };

        let result = analyze_or_for_union(&expr, &*storage, "users");
        assert!(result.is_none());
    }

    #[test]
    fn test_or_union_flattens_nested() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(name)");

        let expr = Expr::BinaryOp {
            left: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::BinaryOp {
                    left: Box::new(Expr::Field("name".to_string())),
                    op: BinaryOp::Eq,
                    right: Box::new(Expr::Literal(Value::String("A".to_string()))),
                }),
                op: BinaryOp::Or,
                right: Box::new(Expr::BinaryOp {
                    left: Box::new(Expr::Field("name".to_string())),
                    op: BinaryOp::Eq,
                    right: Box::new(Expr::Literal(Value::String("B".to_string()))),
                }),
            }),
            op: BinaryOp::Or,
            right: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("name".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Value::String("C".to_string()))),
            }),
        };

        let result = analyze_or_for_union(&expr, &*storage, "users");
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 3);
    }

    #[test]
    fn test_or_union_not_an_or() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(name)");

        let expr = Expr::BinaryOp {
            left: Box::new(Expr::Field("name".to_string())),
            op: BinaryOp::Eq,
            right: Box::new(Expr::Literal(Value::String("Alice".to_string()))),
        };

        let result = analyze_or_for_union(&expr, &*storage, "users");
        assert!(result.is_none());
    }

    #[test]
    fn test_needs_dedup_same_field_eq() {
        let branches = vec![
            (
                IndexRef {
                    collection: "u".to_string(),
                    fields: vec!["name".to_string()],
                },
                IndexLookup::Eq {
                    field: "name".to_string(),
                    value: Value::String("A".to_string()),
                },
            ),
            (
                IndexRef {
                    collection: "u".to_string(),
                    fields: vec!["name".to_string()],
                },
                IndexLookup::Eq {
                    field: "name".to_string(),
                    value: Value::String("B".to_string()),
                },
            ),
        ];
        assert!(!compute_needs_dedup(&branches));
    }

    #[test]
    fn test_needs_dedup_different_fields() {
        let branches = vec![
            (
                IndexRef {
                    collection: "u".to_string(),
                    fields: vec!["name".to_string()],
                },
                IndexLookup::Eq {
                    field: "name".to_string(),
                    value: Value::String("A".to_string()),
                },
            ),
            (
                IndexRef {
                    collection: "u".to_string(),
                    fields: vec!["age".to_string()],
                },
                IndexLookup::Eq {
                    field: "age".to_string(),
                    value: Value::Int(25),
                },
            ),
        ];
        // Different fields still need dedup - one document can match both conditions
        assert!(compute_needs_dedup(&branches));
    }

    #[test]
    fn test_needs_dedup_same_field_range() {
        let branches = vec![
            (
                IndexRef {
                    collection: "u".to_string(),
                    fields: vec!["age".to_string()],
                },
                IndexLookup::Range {
                    field: "age".to_string(),
                    op: RangeOp::Gt,
                    value: Value::Int(20),
                },
            ),
            (
                IndexRef {
                    collection: "u".to_string(),
                    fields: vec!["age".to_string()],
                },
                IndexLookup::Range {
                    field: "age".to_string(),
                    op: RangeOp::Lt,
                    value: Value::Int(30),
                },
            ),
        ];
        assert!(compute_needs_dedup(&branches));
    }

    #[test]
    fn test_analyze_fts_simple() {
        // Simple FTS expression: title @@ "hello"
        let expr = Expr::Fts {
            target: FtsTarget::Field("title".to_string()),
            operator: FtsOperator::Simple,
            query: "hello".to_string(),
        };

        let analysis = analyze_for_fts(&expr);
        assert_eq!(analysis.fts_exprs.len(), 1);
        assert!(analysis.remaining.is_none());

        let fts_info = &analysis.fts_exprs[0];
        assert_eq!(fts_info.query, "hello");
        assert!(matches!(fts_info.target, FtsTarget::Field(ref f) if f == "title"));
    }

    #[test]
    fn test_analyze_fts_with_and() {
        // FTS AND regular condition: title @@ "hello" AND status = "active"
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::Fts {
                target: FtsTarget::Field("title".to_string()),
                operator: FtsOperator::Simple,
                query: "hello".to_string(),
            }),
            op: BinaryOp::And,
            right: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("status".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Value::String("active".to_string()))),
            }),
        };

        let analysis = analyze_for_fts(&expr);
        assert_eq!(analysis.fts_exprs.len(), 1);
        assert!(analysis.remaining.is_some());

        // The remaining should be the status = "active" condition
        let remaining = analysis.remaining.unwrap();
        assert!(matches!(
            remaining,
            Expr::BinaryOp {
                op: BinaryOp::Eq,
                ..
            }
        ));
    }

    #[test]
    fn test_analyze_fts_no_fts() {
        // No FTS expression: just a regular condition
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::Field("age".to_string())),
            op: BinaryOp::Gt,
            right: Box::new(Expr::Literal(Value::Int(25))),
        };

        let analysis = analyze_for_fts(&expr);
        assert!(analysis.fts_exprs.is_empty());
        assert!(analysis.remaining.is_some());
    }

    #[test]
    fn test_analyze_fts_multiple() {
        // Multiple FTS in AND: title @@ "hello" AND body @@ "world"
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::Fts {
                target: FtsTarget::Field("title".to_string()),
                operator: FtsOperator::Simple,
                query: "hello".to_string(),
            }),
            op: BinaryOp::And,
            right: Box::new(Expr::Fts {
                target: FtsTarget::Field("body".to_string()),
                operator: FtsOperator::Simple,
                query: "world".to_string(),
            }),
        };

        let analysis = analyze_for_fts(&expr);
        assert_eq!(analysis.fts_exprs.len(), 2);
        assert!(analysis.remaining.is_none());
    }

    #[test]
    fn test_analyze_fts_in_or_not_extracted() {
        // FTS in OR: title @@ "hello" OR status = "active"
        // FTS should NOT be extracted (whole predicate stays)
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::Fts {
                target: FtsTarget::Field("title".to_string()),
                operator: FtsOperator::Simple,
                query: "hello".to_string(),
            }),
            op: BinaryOp::Or,
            right: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("status".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Value::String("active".to_string()))),
            }),
        };

        let analysis = analyze_for_fts(&expr);
        // The OR expression is not split, so no FTS extracted
        assert!(analysis.fts_exprs.is_empty());
        assert!(analysis.remaining.is_some());
    }

    #[test]
    fn test_analyze_knn_simple() {
        // Simple KNN expression: embedding <|10|> [0.1, 0.2, 0.3]
        let expr = Expr::KnnSearch {
            field: "embedding".to_string(),
            vector: Box::new(Expr::Array(vec![
                Expr::Literal(Value::Float(0.1)),
                Expr::Literal(Value::Float(0.2)),
                Expr::Literal(Value::Float(0.3)),
            ])),
            k: 10,
            effort: None,
        };

        let analysis = analyze_for_knn(&expr);
        assert_eq!(analysis.knn_exprs.len(), 1);
        assert!(analysis.remaining.is_none());

        let knn_info = &analysis.knn_exprs[0];
        assert_eq!(knn_info.field, "embedding");
        assert_eq!(knn_info.k, 10);
        assert!(knn_info.effort.is_none());
    }

    #[test]
    fn test_analyze_knn_with_effort() {
        // KNN with effort: embedding <|10, 100|> [0.1, 0.2]
        let expr = Expr::KnnSearch {
            field: "embedding".to_string(),
            vector: Box::new(Expr::Array(vec![
                Expr::Literal(Value::Float(0.1)),
                Expr::Literal(Value::Float(0.2)),
            ])),
            k: 10,
            effort: Some(100),
        };

        let analysis = analyze_for_knn(&expr);
        assert_eq!(analysis.knn_exprs.len(), 1);
        let knn_info = &analysis.knn_exprs[0];
        assert_eq!(knn_info.effort, Some(100));
    }

    #[test]
    fn test_analyze_knn_with_and() {
        // KNN AND regular condition: embedding <|10|> [0.1] AND status = "active"
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::KnnSearch {
                field: "embedding".to_string(),
                vector: Box::new(Expr::Array(vec![Expr::Literal(Value::Float(0.1))])),
                k: 10,
                effort: None,
            }),
            op: BinaryOp::And,
            right: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("status".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Value::String("active".to_string()))),
            }),
        };

        let analysis = analyze_for_knn(&expr);
        assert_eq!(analysis.knn_exprs.len(), 1);
        assert!(analysis.remaining.is_some());

        // The remaining should be the status = "active" condition
        let remaining = analysis.remaining.unwrap();
        assert!(matches!(
            remaining,
            Expr::BinaryOp {
                op: BinaryOp::Eq,
                ..
            }
        ));
    }

    #[test]
    fn test_analyze_knn_no_knn() {
        // No KNN expression: just a regular condition
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::Field("age".to_string())),
            op: BinaryOp::Gt,
            right: Box::new(Expr::Literal(Value::Int(25))),
        };

        let analysis = analyze_for_knn(&expr);
        assert!(analysis.knn_exprs.is_empty());
        assert!(analysis.remaining.is_some());
    }

    #[test]
    fn test_analyze_knn_in_or_not_extracted() {
        // KNN in OR: embedding <|10|> [0.1] OR status = "active"
        // KNN should NOT be extracted (whole predicate stays)
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::KnnSearch {
                field: "embedding".to_string(),
                vector: Box::new(Expr::Array(vec![Expr::Literal(Value::Float(0.1))])),
                k: 10,
                effort: None,
            }),
            op: BinaryOp::Or,
            right: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("status".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Value::String("active".to_string()))),
            }),
        };

        let analysis = analyze_for_knn(&expr);
        // The OR expression is not split, so no KNN extracted
        assert!(analysis.knn_exprs.is_empty());
        assert!(analysis.remaining.is_some());
    }

    #[test]
    fn test_and_with_indexed_and_non_indexed_preserves_remaining() {
        // Test: name = "Alice" AND age > 25
        // When only `name` is indexed, `age > 25` should be preserved as remaining predicate
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(name)");
        // Note: no index on age

        // Indexed condition on left, non-indexed on right
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("name".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Value::String("Alice".to_string()))),
            }),
            op: BinaryOp::And,
            right: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("age".to_string())),
                op: BinaryOp::Gt,
                right: Box::new(Expr::Literal(Value::Int(25))),
            }),
        };

        let result = analyze_for_index(&expr, &*storage, "users");
        match result {
            IndexChoice::Use {
                index_ref,
                lookup,
                remaining,
            } => {
                // Should use the name index
                assert_eq!(index_ref.fields, vec!["name".to_string()]);
                assert!(
                    matches!(lookup.as_ref(), IndexLookup::Eq { field, .. } if field == "name")
                );

                // The remaining predicate should be age > 25
                assert!(
                    remaining.is_some(),
                    "remaining predicate should not be None"
                );
                let rem = remaining.unwrap();
                assert!(
                    matches!(
                        rem,
                        Expr::BinaryOp {
                            op: BinaryOp::Gt,
                            ..
                        }
                    ),
                    "remaining should be the age > 25 condition"
                );
            }
            IndexChoice::Skip { reason } => {
                panic!(
                    "Expected IndexChoice::Use, got Skip with reason: {:?}",
                    reason
                );
            }
        }
    }

    #[test]
    fn test_and_with_non_indexed_and_indexed_preserves_remaining() {
        // Test: age > 25 AND name = "Alice"
        // When only `name` is indexed (right side), `age > 25` should be preserved as remaining
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(name)");
        // Note: no index on age

        // Non-indexed on left, indexed condition on right
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("age".to_string())),
                op: BinaryOp::Gt,
                right: Box::new(Expr::Literal(Value::Int(25))),
            }),
            op: BinaryOp::And,
            right: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("name".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Value::String("Alice".to_string()))),
            }),
        };

        let result = analyze_for_index(&expr, &*storage, "users");
        match result {
            IndexChoice::Use {
                index_ref,
                lookup,
                remaining,
            } => {
                // Should use the name index
                assert_eq!(index_ref.fields, vec!["name".to_string()]);
                assert!(
                    matches!(lookup.as_ref(), IndexLookup::Eq { field, .. } if field == "name")
                );

                // The remaining predicate should be age > 25
                assert!(
                    remaining.is_some(),
                    "remaining predicate should not be None"
                );
                let rem = remaining.unwrap();
                assert!(
                    matches!(
                        rem,
                        Expr::BinaryOp {
                            op: BinaryOp::Gt,
                            ..
                        }
                    ),
                    "remaining should be the age > 25 condition"
                );
            }
            IndexChoice::Skip { reason } => {
                panic!(
                    "Expected IndexChoice::Use, got Skip with reason: {:?}",
                    reason
                );
            }
        }
    }

    #[test]
    fn test_and_with_multiple_non_indexed_conditions() {
        // Test: name = "Alice" AND age > 25 AND city = "NYC"
        // When only `name` is indexed, both `age > 25` AND `city = "NYC"` should be preserved
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(name)");

        // (name = "Alice" AND age > 25) AND city = "NYC"
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::BinaryOp {
                    left: Box::new(Expr::Field("name".to_string())),
                    op: BinaryOp::Eq,
                    right: Box::new(Expr::Literal(Value::String("Alice".to_string()))),
                }),
                op: BinaryOp::And,
                right: Box::new(Expr::BinaryOp {
                    left: Box::new(Expr::Field("age".to_string())),
                    op: BinaryOp::Gt,
                    right: Box::new(Expr::Literal(Value::Int(25))),
                }),
            }),
            op: BinaryOp::And,
            right: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("city".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Value::String("NYC".to_string()))),
            }),
        };

        let result = analyze_for_index(&expr, &*storage, "users");
        match result {
            IndexChoice::Use {
                index_ref,
                lookup,
                remaining,
            } => {
                // Should use the name index
                assert_eq!(index_ref.fields, vec!["name".to_string()]);
                assert!(
                    matches!(lookup.as_ref(), IndexLookup::Eq { field, .. } if field == "name")
                );

                // The remaining predicate should contain both age and city conditions
                assert!(
                    remaining.is_some(),
                    "remaining predicate should contain age and city conditions"
                );
                let rem = remaining.unwrap();
                // Should be an AND of the two remaining conditions
                assert!(
                    matches!(
                        rem,
                        Expr::BinaryOp {
                            op: BinaryOp::And,
                            ..
                        }
                    ),
                    "remaining should be AND of age and city conditions"
                );
            }
            IndexChoice::Skip { reason } => {
                panic!(
                    "Expected IndexChoice::Use, got Skip with reason: {:?}",
                    reason
                );
            }
        }
    }

    #[test]
    fn test_compound_index_preserves_remaining() {
        // Test: name = "Alice" AND city = "NYC" AND age > 25
        // With compound index on (name, city), age > 25 should be preserved as remaining
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(name, city)");
        // Note: no index on age

        // (name = "Alice" AND city = "NYC") AND age > 25
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::BinaryOp {
                    left: Box::new(Expr::Field("name".to_string())),
                    op: BinaryOp::Eq,
                    right: Box::new(Expr::Literal(Value::String("Alice".to_string()))),
                }),
                op: BinaryOp::And,
                right: Box::new(Expr::BinaryOp {
                    left: Box::new(Expr::Field("city".to_string())),
                    op: BinaryOp::Eq,
                    right: Box::new(Expr::Literal(Value::String("NYC".to_string()))),
                }),
            }),
            op: BinaryOp::And,
            right: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("age".to_string())),
                op: BinaryOp::Gt,
                right: Box::new(Expr::Literal(Value::Int(25))),
            }),
        };

        let result = analyze_for_index(&expr, &*storage, "users");
        match result {
            IndexChoice::Use {
                index_ref,
                lookup,
                remaining,
            } => {
                // Should use the compound index
                assert_eq!(
                    index_ref.fields,
                    vec!["name".to_string(), "city".to_string()]
                );
                assert!(matches!(lookup.as_ref(), IndexLookup::CompoundEq { .. }));

                // The remaining predicate should be age > 25
                assert!(
                    remaining.is_some(),
                    "remaining predicate should not be None"
                );
                let rem = remaining.unwrap();
                assert!(
                    matches!(
                        rem,
                        Expr::BinaryOp {
                            op: BinaryOp::Gt,
                            ..
                        }
                    ),
                    "remaining should be the age > 25 condition"
                );
            }
            IndexChoice::Skip { reason } => {
                panic!(
                    "Expected IndexChoice::Use, got Skip with reason: {:?}",
                    reason
                );
            }
        }
    }

    #[test]
    fn test_compound_index_no_remaining_when_all_covered() {
        // Test: name = "Alice" AND city = "NYC"
        // With compound index on (name, city), no remaining should exist
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(name, city)");

        // name = "Alice" AND city = "NYC"
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("name".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Value::String("Alice".to_string()))),
            }),
            op: BinaryOp::And,
            right: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("city".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Value::String("NYC".to_string()))),
            }),
        };

        let result = analyze_for_index(&expr, &*storage, "users");
        match result {
            IndexChoice::Use {
                index_ref,
                lookup,
                remaining,
            } => {
                // Should use the compound index
                assert_eq!(
                    index_ref.fields,
                    vec!["name".to_string(), "city".to_string()]
                );
                assert!(matches!(lookup.as_ref(), IndexLookup::CompoundEq { .. }));

                // No remaining predicate when all conditions are covered
                assert!(
                    remaining.is_none(),
                    "remaining should be None when all conditions covered by index"
                );
            }
            IndexChoice::Skip { reason } => {
                panic!(
                    "Expected IndexChoice::Use, got Skip with reason: {:?}",
                    reason
                );
            }
        }
    }
}
