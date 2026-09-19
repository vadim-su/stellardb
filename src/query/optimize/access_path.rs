//! Access path generation and scoring
//!
//! Generates candidate access paths from classified predicates and scores them.

use crate::document::Value;
use crate::query::ast::{BinaryOp, Expr};
use crate::query::execute::ExecutionContext;
use crate::query::optimize::predicate::{IndexValue, IndexableOp, PredicatePart};
use crate::query::plan::{IndexLookup, IndexRef, RangeOp};

/// Candidate data access method
#[derive(Debug, Clone)]
pub struct AccessPath {
    pub method: AccessMethod,
    pub score: u32,
    pub remaining_filter: Option<Expr>,
}

/// How to access data
#[derive(Debug, Clone)]
pub enum AccessMethod {
    TableScan,
    IndexScan {
        index: IndexRef,
        lookup: IndexLookup,
    },
    Union {
        branches: Vec<(IndexRef, IndexLookup)>,
        needs_dedup: bool,
    },
}

/// Scoring constants
pub mod scores {
    pub const TABLE_SCAN: u32 = 1000;
    pub const INDEX_EQ: u32 = 1;
    pub const INDEX_IN_BASE: u32 = 5;
    pub const INDEX_IN_PER_VALUE: u32 = 2;
    pub const INDEX_BETWEEN: u32 = 15;
    pub const INDEX_RANGE: u32 = 20;
    pub const UNION_PER_BRANCH: u32 = 10;
    pub const FILTER_PENALTY: u32 = 2;
}

impl AccessPath {
    /// Create a table scan path
    pub fn table_scan(remaining: Option<Expr>) -> Self {
        let filter_penalty = remaining
            .as_ref()
            .map(|e| count_predicates(e) * scores::FILTER_PENALTY)
            .unwrap_or(0);

        Self {
            method: AccessMethod::TableScan,
            score: scores::TABLE_SCAN + filter_penalty,
            remaining_filter: remaining,
        }
    }

    /// Create an index scan path
    pub fn index_scan(
        collection: &str,
        field: &str,
        op: IndexableOp,
        value: &IndexValue,
        remaining: Option<Expr>,
    ) -> Self {
        let index = IndexRef {
            collection: collection.to_string(),
            fields: vec![field.to_string()],
        };

        let (lookup, base_score) = match (op, value) {
            (IndexableOp::Eq, IndexValue::Single(v)) => (
                IndexLookup::Eq {
                    field: field.to_string(),
                    value: v.clone(),
                },
                scores::INDEX_EQ,
            ),
            (IndexableOp::In, IndexValue::List(vals)) => (
                IndexLookup::In {
                    field: field.to_string(),
                    values: vals.clone(),
                },
                scores::INDEX_IN_BASE
                    + (vals.len().saturating_sub(1) as u32) * scores::INDEX_IN_PER_VALUE,
            ),
            (IndexableOp::Gt, IndexValue::Single(v)) => (
                IndexLookup::Range {
                    field: field.to_string(),
                    op: RangeOp::Gt,
                    value: v.clone(),
                },
                scores::INDEX_RANGE,
            ),
            (IndexableOp::Gte, IndexValue::Single(v)) => (
                IndexLookup::Range {
                    field: field.to_string(),
                    op: RangeOp::Gte,
                    value: v.clone(),
                },
                scores::INDEX_RANGE,
            ),
            (IndexableOp::Lt, IndexValue::Single(v)) => (
                IndexLookup::Range {
                    field: field.to_string(),
                    op: RangeOp::Lt,
                    value: v.clone(),
                },
                scores::INDEX_RANGE,
            ),
            (IndexableOp::Lte, IndexValue::Single(v)) => (
                IndexLookup::Range {
                    field: field.to_string(),
                    op: RangeOp::Lte,
                    value: v.clone(),
                },
                scores::INDEX_RANGE,
            ),
            _ => unreachable!("Invalid op/value combination"),
        };

        let filter_penalty = remaining
            .as_ref()
            .map(|e| count_predicates(e) * scores::FILTER_PENALTY)
            .unwrap_or(0);

        Self {
            method: AccessMethod::IndexScan { index, lookup },
            score: base_score + filter_penalty,
            remaining_filter: remaining,
        }
    }

    /// Create a compound index scan path
    pub fn compound_index_scan(
        collection: &str,
        index_fields: Vec<String>,
        field_values: Vec<(String, Value)>,
        remaining: Option<Expr>,
    ) -> Self {
        let index = IndexRef {
            collection: collection.to_string(),
            fields: index_fields,
        };

        let num_fields = field_values.len();
        let lookup = IndexLookup::CompoundEq { field_values };

        // Compound indexes are very efficient - better score for more fields
        let base_score = scores::INDEX_EQ.saturating_sub(num_fields as u32 / 2);

        let filter_penalty = remaining
            .as_ref()
            .map(|e| count_predicates(e) * scores::FILTER_PENALTY)
            .unwrap_or(0);

        Self {
            method: AccessMethod::IndexScan { index, lookup },
            score: base_score + filter_penalty,
            remaining_filter: remaining,
        }
    }

    /// Create a union path from OR branches
    pub fn union(
        branches: Vec<(IndexRef, IndexLookup)>,
        needs_dedup: bool,
        remaining: Option<Expr>,
    ) -> Self {
        let branch_score: u32 = branches
            .iter()
            .map(|(_, lookup)| score_lookup(lookup))
            .sum();
        let union_overhead = branches.len() as u32 * scores::UNION_PER_BRANCH;

        let filter_penalty = remaining
            .as_ref()
            .map(|e| count_predicates(e) * scores::FILTER_PENALTY)
            .unwrap_or(0);

        Self {
            method: AccessMethod::Union {
                branches,
                needs_dedup,
            },
            score: branch_score + union_overhead + filter_penalty,
            remaining_filter: remaining,
        }
    }
}

fn score_lookup(lookup: &IndexLookup) -> u32 {
    match lookup {
        IndexLookup::Eq { .. } => scores::INDEX_EQ,
        IndexLookup::In { values, .. } => {
            scores::INDEX_IN_BASE
                + (values.len().saturating_sub(1) as u32) * scores::INDEX_IN_PER_VALUE
        }
        IndexLookup::RangeBetween { .. } => scores::INDEX_BETWEEN,
        IndexLookup::Range { .. } => scores::INDEX_RANGE,
        IndexLookup::CompoundEq { field_values } => {
            scores::INDEX_EQ.saturating_sub(field_values.len() as u32 / 2)
        }
    }
}

/// Count predicates in an expression (for filter penalty)
fn count_predicates(expr: &Expr) -> u32 {
    match expr {
        Expr::BinaryOp {
            left,
            op: BinaryOp::And | BinaryOp::Or,
            right,
        } => count_predicates(left) + count_predicates(right),
        _ => 1,
    }
}

/// Combine expressions with AND
pub fn combine_exprs(exprs: Vec<Expr>) -> Option<Expr> {
    exprs.into_iter().reduce(|a, b| Expr::BinaryOp {
        left: Box::new(a),
        op: BinaryOp::And,
        right: Box::new(b),
    })
}

/// Select the best access path from candidates
pub fn select_best(paths: Vec<AccessPath>) -> AccessPath {
    paths
        .into_iter()
        .min_by_key(|p| p.score)
        .expect("paths should contain at least TableScan")
}

/// Generate all reasonable access paths from classified predicates
pub fn generate_access_paths<Ctx: ExecutionContext>(
    parts: &[PredicatePart],
    ctx: &Ctx,
    collection: &str,
) -> Vec<AccessPath> {
    let mut paths = vec![];

    // 1. Always add TableScan as fallback
    let all_filter = combine_parts_to_filter(parts);
    paths.push(AccessPath::table_scan(all_filter));

    // 2. For each Indexable part, create an IndexScan path
    for (i, part) in parts.iter().enumerate() {
        if let PredicatePart::Indexable {
            field, op, value, ..
        } = part
        {
            let remaining = combine_parts_except(parts, i);
            paths.push(AccessPath::index_scan(
                collection, field, *op, value, remaining,
            ));
        }
    }

    // 3. Try compound index for multiple equality conditions
    if let Some(compound_path) = try_build_compound_index(parts, ctx, collection) {
        paths.push(compound_path);
    }

    // 4. For DisjunctionGroups, try to create Union paths
    for (i, part) in parts.iter().enumerate() {
        if let PredicatePart::DisjunctionGroup { branches, .. } = part
            && let Some(union_path) = try_build_union(branches, parts, i, collection)
        {
            paths.push(union_path);
        }
    }

    paths
}

/// Try to build a compound index scan from multiple equality conditions
fn try_build_compound_index<Ctx: ExecutionContext>(
    parts: &[PredicatePart],
    ctx: &Ctx,
    collection: &str,
) -> Option<AccessPath> {
    // Collect all equality conditions (field, value, part_index)
    // We need to look at ALL parts, not just Indexable ones, because
    // non-first fields of compound indexes are classified as FilterOnly
    let eq_conditions: Vec<(String, Value, usize)> = parts
        .iter()
        .enumerate()
        .filter_map(|(i, part)| {
            match part {
                PredicatePart::Indexable {
                    field,
                    op: IndexableOp::Eq,
                    value: IndexValue::Single(v),
                    ..
                } => Some((field.clone(), v.clone(), i)),
                PredicatePart::FilterOnly(expr) => {
                    // Check if this is a field = value expression
                    extract_eq_condition(expr).map(|(field, value)| (field, value, i))
                }
                _ => None,
            }
        })
        .collect();

    // Need at least 2 equality conditions for compound index
    if eq_conditions.len() < 2 {
        return None;
    }

    // Try all combinations of 2+ fields to find a compound index
    // Start with the most fields for best selectivity
    for size in (2..=eq_conditions.len()).rev() {
        for indices in combinations(eq_conditions.len(), size) {
            let fields: Vec<&str> = indices
                .iter()
                .map(|&i| eq_conditions[i].0.as_str())
                .collect();

            if let Some(index_fields) = ctx.find_compound_index(collection, &fields) {
                // Found a compound index! Build field_values in index field order
                let mut field_values = Vec::new();
                let mut used_part_indices = Vec::new();

                for idx_field in &index_fields {
                    for (field, value, part_idx) in &eq_conditions {
                        if field == idx_field {
                            field_values.push((field.clone(), value.clone()));
                            used_part_indices.push(*part_idx);
                            break;
                        }
                    }
                }

                // Remaining filter: parts not used by compound index
                let remaining = combine_parts_except_multiple(parts, &used_part_indices);

                return Some(AccessPath::compound_index_scan(
                    collection,
                    index_fields,
                    field_values,
                    remaining,
                ));
            }
        }
    }

    None
}

/// Generate all combinations of `k` items from `n` items
fn combinations(n: usize, k: usize) -> Vec<Vec<usize>> {
    let mut result = Vec::new();
    let mut combo = vec![0; k];

    fn generate(
        combo: &mut Vec<usize>,
        start: usize,
        depth: usize,
        k: usize,
        n: usize,
        result: &mut Vec<Vec<usize>>,
    ) {
        if depth == k {
            result.push(combo.clone());
            return;
        }
        for i in start..=n - (k - depth) {
            combo[depth] = i;
            generate(combo, i + 1, depth + 1, k, n, result);
        }
    }

    generate(&mut combo, 0, 0, k, n, &mut result);
    result
}

/// Combine all parts except multiple indices into a filter expression
fn combine_parts_except_multiple(
    parts: &[PredicatePart],
    except_indices: &[usize],
) -> Option<Expr> {
    let exprs: Vec<Expr> = parts
        .iter()
        .enumerate()
        .filter(|(i, _)| !except_indices.contains(i))
        .map(|(_, p)| p.original_expr().clone())
        .collect();
    combine_exprs(exprs)
}

/// Extract field = value from an expression (for compound index detection)
fn extract_eq_condition(expr: &Expr) -> Option<(String, Value)> {
    if let Expr::BinaryOp {
        left,
        op: BinaryOp::Eq,
        right,
    } = expr
        && let (Expr::Field(field), Expr::Literal(value)) = (left.as_ref(), right.as_ref())
    {
        return Some((field.clone(), value.clone()));
    }
    None
}

/// Try to build a Union path from a disjunction where all branches are indexable
fn try_build_union(
    branches: &[PredicatePart],
    all_parts: &[PredicatePart],
    disj_index: usize,
    collection: &str,
) -> Option<AccessPath> {
    // Check all branches are indexable
    let lookups: Option<Vec<(IndexRef, IndexLookup)>> = branches
        .iter()
        .map(|branch| {
            if let PredicatePart::Indexable {
                field, op, value, ..
            } = branch
            {
                let index = IndexRef {
                    collection: collection.to_string(),
                    fields: vec![field.clone()],
                };
                let lookup = match (op, value) {
                    (IndexableOp::Eq, IndexValue::Single(v)) => Some(IndexLookup::Eq {
                        field: field.clone(),
                        value: v.clone(),
                    }),
                    (IndexableOp::In, IndexValue::List(vals)) => Some(IndexLookup::In {
                        field: field.clone(),
                        values: vals.clone(),
                    }),
                    (IndexableOp::Gt, IndexValue::Single(v)) => Some(IndexLookup::Range {
                        field: field.clone(),
                        op: RangeOp::Gt,
                        value: v.clone(),
                    }),
                    (IndexableOp::Gte, IndexValue::Single(v)) => Some(IndexLookup::Range {
                        field: field.clone(),
                        op: RangeOp::Gte,
                        value: v.clone(),
                    }),
                    (IndexableOp::Lt, IndexValue::Single(v)) => Some(IndexLookup::Range {
                        field: field.clone(),
                        op: RangeOp::Lt,
                        value: v.clone(),
                    }),
                    (IndexableOp::Lte, IndexValue::Single(v)) => Some(IndexLookup::Range {
                        field: field.clone(),
                        op: RangeOp::Lte,
                        value: v.clone(),
                    }),
                    _ => None,
                };
                lookup.map(|l| (index, l))
            } else {
                None
            }
        })
        .collect();

    let lookups = lookups?;

    // Compute needs_dedup
    let needs_dedup = compute_union_needs_dedup(&lookups);

    // Remaining filter is other parts (not this disjunction)
    let remaining = combine_parts_except(all_parts, disj_index);

    Some(AccessPath::union(lookups, needs_dedup, remaining))
}

/// Check if union results need deduplication
fn compute_union_needs_dedup(branches: &[(IndexRef, IndexLookup)]) -> bool {
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

/// Combine all parts into a single filter expression
fn combine_parts_to_filter(parts: &[PredicatePart]) -> Option<Expr> {
    let exprs: Vec<Expr> = parts.iter().map(|p| p.original_expr().clone()).collect();
    combine_exprs(exprs)
}

/// Combine all parts except one into a filter expression
fn combine_parts_except(parts: &[PredicatePart], except_index: usize) -> Option<Expr> {
    let exprs: Vec<Expr> = parts
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != except_index)
        .map(|(_, p)| p.original_expr().clone())
        .collect();
    combine_exprs(exprs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Value;

    #[test]
    fn test_table_scan_score() {
        let path = AccessPath::table_scan(None);
        assert_eq!(path.score, 1000);
    }

    #[test]
    fn test_index_eq_score() {
        let path = AccessPath::index_scan(
            "users",
            "status",
            IndexableOp::Eq,
            &IndexValue::Single(Value::String("active".to_string())),
            None,
        );
        assert_eq!(path.score, 1); // INDEX_EQ
    }

    #[test]
    fn test_index_in_score() {
        let path = AccessPath::index_scan(
            "users",
            "status",
            IndexableOp::In,
            &IndexValue::List(vec![
                Value::String("a".to_string()),
                Value::String("b".to_string()),
                Value::String("c".to_string()),
            ]),
            None,
        );
        // INDEX_IN_BASE (5) + 2 * INDEX_IN_PER_VALUE (2) = 9
        assert_eq!(path.score, 9);
    }

    #[test]
    fn test_index_range_score() {
        let path = AccessPath::index_scan(
            "users",
            "age",
            IndexableOp::Gt,
            &IndexValue::Single(Value::Int(18)),
            None,
        );
        assert_eq!(path.score, 20); // INDEX_RANGE
    }

    #[test]
    fn test_select_best() {
        let paths = vec![
            AccessPath::table_scan(None),
            AccessPath::index_scan(
                "users",
                "status",
                IndexableOp::Eq,
                &IndexValue::Single(Value::String("active".to_string())),
                None,
            ),
        ];
        let best = select_best(paths);
        assert_eq!(best.score, 1);
    }

    #[test]
    fn test_generate_paths_simple() {
        use crate::Database;
        use crate::query::optimize::predicate::{classify_all, extract_conjuncts};
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let storage = std::sync::Arc::new(Database::open(tmp.path()).unwrap());
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(status)");

        // status = 'active'
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::Field("status".to_string())),
            op: BinaryOp::Eq,
            right: Box::new(Expr::Literal(Value::String("active".to_string()))),
        };

        let conjuncts = extract_conjuncts(&expr);
        let parts = classify_all(&conjuncts, &*storage, "users");
        let paths = generate_access_paths(&parts, &*storage, "users");

        // Should have TableScan and IndexScan
        assert_eq!(paths.len(), 2);

        let best = select_best(paths);
        assert!(matches!(best.method, AccessMethod::IndexScan { .. }));
    }

    #[test]
    fn test_generate_paths_or_union() {
        use crate::Database;
        use crate::query::optimize::predicate::{classify_all, extract_conjuncts};
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let storage = std::sync::Arc::new(Database::open(tmp.path()).unwrap());
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(role)");

        // role = 'admin' OR role = 'mod'
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("role".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Value::String("admin".to_string()))),
            }),
            op: BinaryOp::Or,
            right: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("role".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Value::String("mod".to_string()))),
            }),
        };

        let conjuncts = extract_conjuncts(&expr);
        let parts = classify_all(&conjuncts, &*storage, "users");
        let paths = generate_access_paths(&parts, &*storage, "users");

        // Should have TableScan and Union
        assert!(paths.len() >= 2);
        assert!(
            paths
                .iter()
                .any(|p| matches!(p.method, AccessMethod::Union { .. }))
        );
    }

    #[test]
    fn test_union_with_range_cross_field() {
        use crate::Database;
        use crate::query::optimize::predicate::{classify_all, extract_conjuncts};
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let storage = std::sync::Arc::new(Database::open(tmp.path()).unwrap());
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(name)");
        crate::run_sql!(storage, "CREATE INDEX ON users(age)");

        // name = 'alice' OR age > 75
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("name".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Value::String("alice".to_string()))),
            }),
            op: BinaryOp::Or,
            right: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("age".to_string())),
                op: BinaryOp::Gt,
                right: Box::new(Expr::Literal(Value::Int(75))),
            }),
        };

        let conjuncts = extract_conjuncts(&expr);
        let parts = classify_all(&conjuncts, &*storage, "users");
        let paths = generate_access_paths(&parts, &*storage, "users");

        // Should have Union path
        let union_path = paths
            .iter()
            .find(|p| matches!(p.method, AccessMethod::Union { .. }));
        assert!(
            union_path.is_some(),
            "Should generate Union path for OR with range"
        );
    }

    #[test]
    fn test_union_different_fields_needs_dedup() {
        use crate::Database;
        use crate::query::optimize::predicate::{classify_all, extract_conjuncts};
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let storage = std::sync::Arc::new(Database::open(tmp.path()).unwrap());
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(name)");
        crate::run_sql!(storage, "CREATE INDEX ON users(age)");

        // name = 'alice' OR age > 75
        // Different fields still need dedup: one doc can match both conditions
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("name".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Value::String("alice".to_string()))),
            }),
            op: BinaryOp::Or,
            right: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("age".to_string())),
                op: BinaryOp::Gt,
                right: Box::new(Expr::Literal(Value::Int(75))),
            }),
        };

        let conjuncts = extract_conjuncts(&expr);
        let parts = classify_all(&conjuncts, &*storage, "users");
        let paths = generate_access_paths(&parts, &*storage, "users");

        let union_path = paths
            .iter()
            .find(|p| matches!(p.method, AccessMethod::Union { .. }));
        assert!(union_path.is_some());

        if let AccessMethod::Union { needs_dedup, .. } = &union_path.unwrap().method {
            assert!(
                needs_dedup,
                "Different fields need dedup - one doc can match both"
            );
        }
    }

    #[test]
    fn test_union_same_field_range_needs_dedup() {
        use crate::Database;
        use crate::query::optimize::predicate::{classify_all, extract_conjuncts};
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let storage = std::sync::Arc::new(Database::open(tmp.path()).unwrap());
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(age)");

        // age < 20 OR age > 80 (same field with range = needs dedup)
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("age".to_string())),
                op: BinaryOp::Lt,
                right: Box::new(Expr::Literal(Value::Int(20))),
            }),
            op: BinaryOp::Or,
            right: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::Field("age".to_string())),
                op: BinaryOp::Gt,
                right: Box::new(Expr::Literal(Value::Int(80))),
            }),
        };

        let conjuncts = extract_conjuncts(&expr);
        let parts = classify_all(&conjuncts, &*storage, "users");
        let paths = generate_access_paths(&parts, &*storage, "users");

        let union_path = paths
            .iter()
            .find(|p| matches!(p.method, AccessMethod::Union { .. }));
        assert!(union_path.is_some());

        if let AccessMethod::Union { needs_dedup, .. } = &union_path.unwrap().method {
            assert!(needs_dedup, "Same field with range should need dedup");
        }
    }
}
