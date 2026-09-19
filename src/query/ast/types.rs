use super::expr::{Expr, ProjectionItem};
use crate::document::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub collection: String,
    pub key: Option<String>,
}

/// Postfix operation applied to a data source value.
#[derive(Debug, Clone, PartialEq)]
pub enum PostfixOp {
    FieldAccess(String),
    Index(Box<Expr>),
    Slice {
        start: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
    },
}

/// Data source for SELECT queries.
#[derive(Debug, Clone, PartialEq)]
pub enum DataSource {
    /// No FROM clause - scalar query
    None,
    /// FROM collection / FROM collection:key - storage scan with index support
    Collection(Target),
    /// `FROM collection:key.field.path[postfix]` - document field access with optional postfix ops
    FieldPath {
        target: Target,
        path: Vec<String>,
        postfix: Vec<PostfixOp>,
    },
    /// `FROM <expr>` - any expression: subquery, array, function, variable, range
    Expr(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Projection {
    All,
    Items(Vec<ProjectionItem>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    pub path: Vec<String>,
    pub expr: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectLiteral {
    pub fields: Vec<(String, Value)>,
}
