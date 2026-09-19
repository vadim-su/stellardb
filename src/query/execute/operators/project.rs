// src/query/execute/project.rs

use crate::document::Document;
use crate::query::ast::{AggregateArg, BinaryOp, Expr, Projection, UnaryOp};
use crate::query::error::ExecuteError;
use crate::query::execute::VarScope;
use crate::query::execute::context::ExecutionContext;
use crate::query::execute::eval::eval_expr;
use crate::query::execute::operators::operator::{Operator, Row};
use crate::query::execute::subquery::{expr_has_subquery, expr_has_traversal};
use std::collections::HashMap;

/// Project operator - selects specific fields
pub struct ProjectOp<'a, I: Operator, Ctx: ExecutionContext> {
    input: I,
    fields: Projection,
    vars: Option<&'a VarScope>,
    ctx: &'a Ctx,
}

impl<'a, I: Operator, Ctx: ExecutionContext> ProjectOp<'a, I, Ctx> {
    pub fn new(input: I, fields: Projection, vars: Option<&'a VarScope>, ctx: &'a Ctx) -> Self {
        Self {
            input,
            fields,
            vars,
            ctx,
        }
    }
}

impl<'a, I: Operator, Ctx: ExecutionContext> Operator for ProjectOp<'a, I, Ctx> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        self.input.open()
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        if let Some(row) = self.input.next()? {
            // For scalar rows with SELECT *, pass through unchanged
            if row.is_scalar() && matches!(self.fields, Projection::All) {
                return Ok(Some(row));
            }
            let projected = project_row(&row, &self.fields, self.vars, self.ctx)?;
            Ok(Some(Row::from_doc(projected)))
        } else {
            Ok(None)
        }
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        self.input.close()
    }
}

fn project_row<Ctx: ExecutionContext>(
    row: &Row,
    projection: &Projection,
    vars: Option<&VarScope>,
    ctx: &Ctx,
) -> Result<Document, ExecuteError> {
    match projection {
        Projection::All => Ok(row.doc.clone()),
        Projection::Items(items) => {
            let mut projected_fields = HashMap::new();

            for item in items {
                // Aggregates are handled by AggregateOp, not ProjectOp
                // Subqueries (including wrapped: (SELECT...)[0]) are handled by post_process_correlated_subqueries
                // Traversals are handled by post_process_traversals
                if matches!(item.expr, Expr::Aggregate(_))
                    || expr_has_subquery(&item.expr)
                    || expr_has_traversal(&item.expr)
                {
                    continue;
                }

                let name = match &item.alias {
                    Some(alias) => alias.clone(),
                    None => expr_display_name(&item.expr),
                };

                // id is always in doc.id, skip unless aliased
                if name == "id" && item.alias.is_none() {
                    continue;
                }

                let value = eval_expr(&item.expr, row, None, vars, ctx)?;
                projected_fields.insert(name, value);
            }

            Ok(Document {
                id: row.doc.id.clone(),
                fields: projected_fields,
            })
        }
    }
}

/// Generate a display name for an expression (used when no alias is provided).
pub fn expr_display_name(expr: &Expr) -> String {
    match expr {
        Expr::Field(f) => f.clone(),
        Expr::Literal(v) => format!("{:?}", v),
        Expr::BinaryOp { left, op, right } => {
            let op_sym = match op {
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
            format!(
                "{} {} {}",
                expr_display_name(left),
                op_sym,
                expr_display_name(right)
            )
        }
        Expr::UnaryOp { op, expr: inner } => match op {
            UnaryOp::Neg => format!("-{}", expr_display_name(inner)),
            UnaryOp::Not => format!("NOT {}", expr_display_name(inner)),
            UnaryOp::IsNull => format!("{} IS NULL", expr_display_name(inner)),
            UnaryOp::IsNotNull => format!("{} IS NOT NULL", expr_display_name(inner)),
            UnaryOp::IsNone => format!("{} IS NONE", expr_display_name(inner)),
            UnaryOp::IsNotNone => format!("{} IS NOT NONE", expr_display_name(inner)),
        },
        Expr::FunctionCall { name, args, .. } => {
            let arg_strs: Vec<String> = args.iter().map(expr_display_name).collect();
            format!("{}({})", name.to_lowercase(), arg_strs.join(", "))
        }
        Expr::Aggregate(agg) => {
            let arg_str = match &agg.arg {
                AggregateArg::Wildcard => "*".to_string(),
                AggregateArg::Field(f) => f.clone(),
            };
            format!("{}({})", agg.function.to_lowercase(), arg_str)
        }
        Expr::Array(_) => "[...]".to_string(),
        Expr::Object(_) => "{...}".to_string(),
        Expr::InList { .. } => "in_expr".to_string(),
        Expr::InSubquery { .. } => "in_subquery".to_string(),
        Expr::InExpr { .. } => "in_expr".to_string(),
        Expr::Subquery(_) => "(SELECT ...)".to_string(),
        Expr::ParentRef(p) => {
            let parents = "parent.".repeat(p.depth);
            format!("$parent.{}{}", parents, p.field)
        }
        Expr::ValueRef(None) => "$value".to_string(),
        Expr::ValueRef(Some(path)) => format!("$value.{}", path),
        Expr::Index { base, index } => {
            format!("{}[{}]", expr_display_name(base), expr_display_name(index))
        }
        Expr::Slice { base, start, end } => {
            let start_str = start
                .as_ref()
                .map(|e| expr_display_name(e))
                .unwrap_or_default();
            let end_str = end
                .as_ref()
                .map(|e| expr_display_name(e))
                .unwrap_or_default();
            format!("{}[{}..{}]", expr_display_name(base), start_str, end_str)
        }
        Expr::FieldAccess { base, field } => {
            format!("{}.{}", expr_display_name(base), field)
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
            format!("{}..{}", expr_display_name(start), expr_display_name(end))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Value;
    use crate::query::ast::{Expr, ProjectionItem};
    use crate::query::execute::context::NullContext;
    use crate::query::execute::operators::operator::collect_all;
    use crate::query::execute::operators::operator::test_util::MockOp;

    fn make_test_doc(id: &str, name: &str, age: i64, city: &str) -> Document {
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), Value::String(name.to_string()));
        fields.insert("age".to_string(), Value::Int(age));
        fields.insert("city".to_string(), Value::String(city.to_string()));
        Document {
            id: format!("test:{}", id),
            fields,
        }
    }

    fn field_item(name: &str) -> ProjectionItem {
        ProjectionItem {
            expr: Expr::Field(name.to_string()),
            alias: None,
        }
    }

    #[test]
    fn test_project_all() {
        let docs = vec![make_test_doc("1", "Alice", 30, "NYC")];
        let mock = MockOp::new(docs.clone());
        let mut project = ProjectOp::new(mock, Projection::All, None, &NullContext);

        let rows = collect_all(&mut project).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].doc.id, "test:1");
        assert!(rows[0].doc.fields.contains_key("name"));
        assert!(rows[0].doc.fields.contains_key("age"));
        assert!(rows[0].doc.fields.contains_key("city"));
    }

    #[test]
    fn test_project_specific_fields() {
        let docs = vec![make_test_doc("1", "Alice", 30, "NYC")];
        let mock = MockOp::new(docs);
        let mut project = ProjectOp::new(
            mock,
            Projection::Items(vec![field_item("name")]),
            None,
            &NullContext,
        );

        let rows = collect_all(&mut project).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].doc.fields.contains_key("name"));
        assert!(!rows[0].doc.fields.contains_key("age"));
        assert!(!rows[0].doc.fields.contains_key("city"));
        // ID is always included
        assert_eq!(rows[0].doc.id, "test:1");
    }

    #[test]
    fn test_project_multiple_fields() {
        let docs = vec![make_test_doc("1", "Alice", 30, "NYC")];
        let mock = MockOp::new(docs);
        let mut project = ProjectOp::new(
            mock,
            Projection::Items(vec![field_item("name"), field_item("city")]),
            None,
            &NullContext,
        );

        let rows = collect_all(&mut project).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].doc.fields.contains_key("name"));
        assert!(rows[0].doc.fields.contains_key("city"));
        assert!(!rows[0].doc.fields.contains_key("age"));
    }

    #[test]
    fn test_project_nonexistent_field() {
        let docs = vec![make_test_doc("1", "Alice", 30, "NYC")];
        let mock = MockOp::new(docs);
        let mut project = ProjectOp::new(
            mock,
            Projection::Items(vec![field_item("nonexistent")]),
            None,
            &NullContext,
        );

        let rows = collect_all(&mut project).unwrap();
        assert_eq!(rows.len(), 1);
        // eval_expr returns Null for missing fields, so the field is present with Null value
        assert_eq!(rows[0].doc.fields.get("nonexistent"), Some(&Value::Null));
    }

    #[test]
    fn test_project_empty_input() {
        let mock = MockOp::new(vec![]);
        let mut project = ProjectOp::new(mock, Projection::All, None, &NullContext);

        let rows = collect_all(&mut project).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_project_multiple_docs() {
        let docs = vec![
            make_test_doc("1", "Alice", 30, "NYC"),
            make_test_doc("2", "Bob", 25, "LA"),
            make_test_doc("3", "Charlie", 35, "Chicago"),
        ];
        let mock = MockOp::new(docs);
        let mut project = ProjectOp::new(
            mock,
            Projection::Items(vec![field_item("name")]),
            None,
            &NullContext,
        );

        let rows = collect_all(&mut project).unwrap();
        assert_eq!(rows.len(), 3);
        for row in &rows {
            assert!(row.doc.fields.contains_key("name"));
            assert!(!row.doc.fields.contains_key("age"));
            assert!(!row.doc.fields.contains_key("city"));
        }
    }

    #[test]
    fn test_project_with_alias() {
        let docs = vec![make_test_doc("1", "Alice", 30, "NYC")];
        let mock = MockOp::new(docs);
        let mut project = ProjectOp::new(
            mock,
            Projection::Items(vec![ProjectionItem {
                expr: Expr::Field("name".to_string()),
                alias: Some("full_name".to_string()),
            }]),
            None,
            &NullContext,
        );

        let rows = collect_all(&mut project).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].doc.fields.contains_key("full_name"));
        assert!(!rows[0].doc.fields.contains_key("name"));
        assert_eq!(
            rows[0].doc.fields.get("full_name"),
            Some(&Value::String("Alice".to_string()))
        );
    }

    #[test]
    fn test_project_expression() {
        let docs = vec![make_test_doc("1", "Alice", 30, "NYC")];
        let mock = MockOp::new(docs);
        let mut project = ProjectOp::new(
            mock,
            Projection::Items(vec![ProjectionItem {
                expr: Expr::BinaryOp {
                    left: Box::new(Expr::Field("age".to_string())),
                    op: BinaryOp::Add,
                    right: Box::new(Expr::Literal(Value::Int(1))),
                },
                alias: Some("next_age".to_string()),
            }]),
            None,
            &NullContext,
        );

        let rows = collect_all(&mut project).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].doc.fields.get("next_age"), Some(&Value::Int(31)));
    }

    #[test]
    fn test_expr_display_name_field() {
        assert_eq!(expr_display_name(&Expr::Field("name".to_string())), "name");
    }

    #[test]
    fn test_expr_display_name_binary_op() {
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::Field("a".to_string())),
            op: BinaryOp::Add,
            right: Box::new(Expr::Field("b".to_string())),
        };
        assert_eq!(expr_display_name(&expr), "a + b");
    }

    #[test]
    fn test_expr_display_name_function() {
        let expr = Expr::FunctionCall {
            name: "UPPER".to_string(),
            namespace: None,
            args: vec![Expr::Field("name".to_string())],
        };
        assert_eq!(expr_display_name(&expr), "upper(name)");
    }

    #[test]
    fn test_expr_display_name_aggregate() {
        use crate::query::ast::AggregateCall;
        let expr = Expr::Aggregate(AggregateCall {
            namespace: None,
            function: "COUNT".to_string(),
            arg: AggregateArg::Wildcard,
            distinct: false,
        });
        assert_eq!(expr_display_name(&expr), "count(*)");
    }

    #[test]
    fn test_expr_display_name_unary() {
        let neg = Expr::UnaryOp {
            op: UnaryOp::Neg,
            expr: Box::new(Expr::Field("x".to_string())),
        };
        assert_eq!(expr_display_name(&neg), "-x");

        let not = Expr::UnaryOp {
            op: UnaryOp::Not,
            expr: Box::new(Expr::Field("flag".to_string())),
        };
        assert_eq!(expr_display_name(&not), "NOT flag");
    }
}
