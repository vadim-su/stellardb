//! Predicate analysis for query optimization
//!
//! Extracts and classifies predicate components to enable smart index selection.

use crate::document::Value;
use crate::query::ast::{BinaryOp, Expr};
use crate::query::execute::ExecutionContext;

/// A conjunct extracted from an AND expression
#[derive(Debug, Clone)]
pub enum Conjunct {
    /// Single atomic expression
    Atom(Expr),
    /// OR group within AND
    Disjunction(Vec<Conjunct>),
}

/// Classified predicate part with indexability info
#[derive(Debug, Clone)]
pub enum PredicatePart {
    /// Can be used for index lookup
    Indexable {
        field: String,
        op: IndexableOp,
        value: IndexValue,
        original: Expr,
    },
    /// OR group - may be partially or fully indexable
    DisjunctionGroup {
        branches: Vec<PredicatePart>,
        original: Expr,
    },
    /// Cannot use index - must filter
    FilterOnly(Expr),
}

/// Type of indexable operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexableOp {
    Eq,
    Gt,
    Gte,
    Lt,
    Lte,
    In,
}

/// Value(s) for index lookup
#[derive(Debug, Clone)]
pub enum IndexValue {
    Single(Value),
    List(Vec<Value>),
}

/// Extract top-level conjuncts from an AND expression
pub fn extract_conjuncts(expr: &Expr) -> Vec<Conjunct> {
    match expr {
        Expr::BinaryOp {
            left,
            op: BinaryOp::And,
            right,
        } => {
            let mut result = extract_conjuncts(left);
            result.extend(extract_conjuncts(right));
            result
        }
        Expr::BinaryOp {
            op: BinaryOp::Or, ..
        } => {
            vec![Conjunct::Disjunction(extract_disjuncts(expr))]
        }
        other => vec![Conjunct::Atom(other.clone())],
    }
}

/// Extract disjuncts from an OR expression
fn extract_disjuncts(expr: &Expr) -> Vec<Conjunct> {
    match expr {
        Expr::BinaryOp {
            left,
            op: BinaryOp::Or,
            right,
        } => {
            let mut result = extract_disjuncts(left);
            result.extend(extract_disjuncts(right));
            result
        }
        other => vec![Conjunct::Atom(other.clone())],
    }
}

impl PredicatePart {
    /// Check if this part can use an index
    pub fn is_indexable(&self) -> bool {
        matches!(self, PredicatePart::Indexable { .. })
    }

    /// Check if this is a disjunction group
    pub fn is_disjunction(&self) -> bool {
        matches!(self, PredicatePart::DisjunctionGroup { .. })
    }

    /// Get the original expression
    pub fn original_expr(&self) -> &Expr {
        match self {
            PredicatePart::Indexable { original, .. } => original,
            PredicatePart::DisjunctionGroup { original, .. } => original,
            PredicatePart::FilterOnly(expr) => expr,
        }
    }
}

/// Classify a conjunct based on available indexes
pub fn classify_conjunct<Ctx: ExecutionContext>(
    conjunct: &Conjunct,
    ctx: &Ctx,
    collection: &str,
) -> PredicatePart {
    match conjunct {
        Conjunct::Atom(expr) => classify_atom(expr, ctx, collection),
        Conjunct::Disjunction(branches) => {
            let classified: Vec<PredicatePart> = branches
                .iter()
                .map(|b| classify_conjunct(b, ctx, collection))
                .collect();
            PredicatePart::DisjunctionGroup {
                branches: classified,
                original: reconstruct_or(branches),
            }
        }
    }
}

/// Classify a single atomic expression
fn classify_atom<Ctx: ExecutionContext>(expr: &Expr, ctx: &Ctx, collection: &str) -> PredicatePart {
    match expr {
        // field = value
        Expr::BinaryOp { left, op, right } => {
            // Check if left is a field and right is a literal
            if let (Expr::Field(field), Expr::Literal(value)) = (left.as_ref(), right.as_ref()) {
                let indexable_op = match op {
                    BinaryOp::Eq => Some(IndexableOp::Eq),
                    BinaryOp::Gt => Some(IndexableOp::Gt),
                    BinaryOp::Gte => Some(IndexableOp::Gte),
                    BinaryOp::Lt => Some(IndexableOp::Lt),
                    BinaryOp::Lte => Some(IndexableOp::Lte),
                    _ => None,
                };

                if let Some(iop) = indexable_op
                    && ctx.find_index_for_field(collection, field).is_some()
                {
                    return PredicatePart::Indexable {
                        field: field.clone(),
                        op: iop,
                        value: IndexValue::Single(value.clone()),
                        original: expr.clone(),
                    };
                }
            }
            PredicatePart::FilterOnly(expr.clone())
        }

        // field IN (v1, v2, ...)
        Expr::InList {
            expr: inner_expr,
            list,
            negated,
        } => {
            if *negated {
                return PredicatePart::FilterOnly(expr.clone());
            }
            if let Expr::Field(field) = inner_expr.as_ref() {
                // Check all items are literals
                let values: Option<Vec<Value>> = list
                    .iter()
                    .map(|e| match e {
                        Expr::Literal(v) => Some(v.clone()),
                        _ => None,
                    })
                    .collect();

                if let Some(vals) = values
                    && ctx.find_index_for_field(collection, field).is_some()
                {
                    return PredicatePart::Indexable {
                        field: field.clone(),
                        op: IndexableOp::In,
                        value: IndexValue::List(vals),
                        original: expr.clone(),
                    };
                }
            }
            PredicatePart::FilterOnly(expr.clone())
        }

        _ => PredicatePart::FilterOnly(expr.clone()),
    }
}

/// Reconstruct OR expression from disjuncts
fn reconstruct_or(branches: &[Conjunct]) -> Expr {
    let exprs: Vec<Expr> = branches
        .iter()
        .map(|b| match b {
            Conjunct::Atom(e) => e.clone(),
            Conjunct::Disjunction(inner) => reconstruct_or(inner),
        })
        .collect();

    exprs
        .into_iter()
        .reduce(|a, b| Expr::BinaryOp {
            left: Box::new(a),
            op: BinaryOp::Or,
            right: Box::new(b),
        })
        .unwrap_or(Expr::Literal(Value::Bool(false)))
}

/// Classify all conjuncts
pub fn classify_all<Ctx: ExecutionContext>(
    conjuncts: &[Conjunct],
    ctx: &Ctx,
    collection: &str,
) -> Vec<PredicatePart> {
    conjuncts
        .iter()
        .map(|c| classify_conjunct(c, ctx, collection))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field_eq(field: &str, value: i64) -> Expr {
        Expr::BinaryOp {
            left: Box::new(Expr::Field(field.to_string())),
            op: BinaryOp::Eq,
            right: Box::new(Expr::Literal(Value::Int(value))),
        }
    }

    fn and(left: Expr, right: Expr) -> Expr {
        Expr::BinaryOp {
            left: Box::new(left),
            op: BinaryOp::And,
            right: Box::new(right),
        }
    }

    fn or(left: Expr, right: Expr) -> Expr {
        Expr::BinaryOp {
            left: Box::new(left),
            op: BinaryOp::Or,
            right: Box::new(right),
        }
    }

    #[test]
    fn test_extract_simple_and() {
        // a = 1 AND b = 2
        let expr = and(field_eq("a", 1), field_eq("b", 2));
        let conjuncts = extract_conjuncts(&expr);
        assert_eq!(conjuncts.len(), 2);
        assert!(matches!(&conjuncts[0], Conjunct::Atom(_)));
        assert!(matches!(&conjuncts[1], Conjunct::Atom(_)));
    }

    #[test]
    fn test_extract_nested_and() {
        // a = 1 AND b = 2 AND c = 3
        let expr = and(and(field_eq("a", 1), field_eq("b", 2)), field_eq("c", 3));
        let conjuncts = extract_conjuncts(&expr);
        assert_eq!(conjuncts.len(), 3);
    }

    #[test]
    fn test_extract_or_in_and() {
        // a = 1 AND (b = 2 OR b = 3)
        let expr = and(field_eq("a", 1), or(field_eq("b", 2), field_eq("b", 3)));
        let conjuncts = extract_conjuncts(&expr);
        assert_eq!(conjuncts.len(), 2);
        assert!(matches!(&conjuncts[0], Conjunct::Atom(_)));
        assert!(matches!(&conjuncts[1], Conjunct::Disjunction(branches) if branches.len() == 2));
    }

    #[test]
    fn test_extract_complex() {
        // a = 1 AND (b = 2 OR c = 3) AND d = 4
        let expr = and(
            and(field_eq("a", 1), or(field_eq("b", 2), field_eq("c", 3))),
            field_eq("d", 4),
        );
        let conjuncts = extract_conjuncts(&expr);
        assert_eq!(conjuncts.len(), 3);
    }

    #[test]
    fn test_classify_indexable() {
        use crate::Database;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let storage = std::sync::Arc::new(Database::open(tmp.path()).unwrap());
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(status)");

        let expr = field_eq("status", 1);
        let conjuncts = extract_conjuncts(&expr);
        let parts = classify_all(&conjuncts, &*storage, "users");

        assert_eq!(parts.len(), 1);
        assert!(parts[0].is_indexable());
    }

    #[test]
    fn test_classify_not_indexed() {
        use crate::Database;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let storage = std::sync::Arc::new(Database::open(tmp.path()).unwrap());
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        // No index

        let expr = field_eq("status", 1);
        let conjuncts = extract_conjuncts(&expr);
        let parts = classify_all(&conjuncts, &*storage, "users");

        assert_eq!(parts.len(), 1);
        assert!(!parts[0].is_indexable());
    }

    #[test]
    fn test_classify_range_indexable() {
        use crate::Database;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let storage = std::sync::Arc::new(Database::open(tmp.path()).unwrap());
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(age)");

        // age > 25
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::Field("age".to_string())),
            op: BinaryOp::Gt,
            right: Box::new(Expr::Literal(Value::Int(25))),
        };
        let conjuncts = extract_conjuncts(&expr);
        let parts = classify_all(&conjuncts, &*storage, "users");

        assert_eq!(parts.len(), 1);
        assert!(parts[0].is_indexable());
        if let PredicatePart::Indexable { op, .. } = &parts[0] {
            assert_eq!(*op, IndexableOp::Gt);
        }
    }

    #[test]
    fn test_classify_in_list_indexable() {
        use crate::Database;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let storage = std::sync::Arc::new(Database::open(tmp.path()).unwrap());
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(status)");

        // status IN (1, 2, 3)
        let expr = Expr::InList {
            expr: Box::new(Expr::Field("status".to_string())),
            list: vec![
                Expr::Literal(Value::Int(1)),
                Expr::Literal(Value::Int(2)),
                Expr::Literal(Value::Int(3)),
            ],
            negated: false,
        };
        let conjuncts = extract_conjuncts(&expr);
        let parts = classify_all(&conjuncts, &*storage, "users");

        assert_eq!(parts.len(), 1);
        assert!(parts[0].is_indexable());
        if let PredicatePart::Indexable { op, value, .. } = &parts[0] {
            assert_eq!(*op, IndexableOp::In);
            if let IndexValue::List(vals) = value {
                assert_eq!(vals.len(), 3);
            } else {
                panic!("expected IndexValue::List");
            }
        }
    }

    #[test]
    fn test_classify_negated_in_list_not_indexable() {
        use crate::Database;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let storage = std::sync::Arc::new(Database::open(tmp.path()).unwrap());
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(status)");

        // status NOT IN (1, 2, 3)
        let expr = Expr::InList {
            expr: Box::new(Expr::Field("status".to_string())),
            list: vec![
                Expr::Literal(Value::Int(1)),
                Expr::Literal(Value::Int(2)),
                Expr::Literal(Value::Int(3)),
            ],
            negated: true,
        };
        let conjuncts = extract_conjuncts(&expr);
        let parts = classify_all(&conjuncts, &*storage, "users");

        assert_eq!(parts.len(), 1);
        assert!(!parts[0].is_indexable());
    }

    #[test]
    fn test_classify_disjunction() {
        use crate::Database;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let storage = std::sync::Arc::new(Database::open(tmp.path()).unwrap());
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(status)");

        // status = 1 OR status = 2
        let expr = or(field_eq("status", 1), field_eq("status", 2));
        let conjuncts = extract_conjuncts(&expr);
        let parts = classify_all(&conjuncts, &*storage, "users");

        assert_eq!(parts.len(), 1);
        assert!(parts[0].is_disjunction());
        if let PredicatePart::DisjunctionGroup { branches, .. } = &parts[0] {
            assert_eq!(branches.len(), 2);
            assert!(branches[0].is_indexable());
            assert!(branches[1].is_indexable());
        }
    }

    #[test]
    fn test_classify_mixed_and() {
        use crate::Database;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let storage = std::sync::Arc::new(Database::open(tmp.path()).unwrap());
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(status)");
        // No index on name

        // status = 1 AND name = "Alice" (as a value)
        let expr = and(
            field_eq("status", 1),
            Expr::BinaryOp {
                left: Box::new(Expr::Field("name".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Value::String("Alice".to_string()))),
            },
        );
        let conjuncts = extract_conjuncts(&expr);
        let parts = classify_all(&conjuncts, &*storage, "users");

        assert_eq!(parts.len(), 2);
        // status = 1 is indexable
        assert!(parts[0].is_indexable());
        // name = "Alice" is not indexable (no index)
        assert!(!parts[1].is_indexable());
    }
}
