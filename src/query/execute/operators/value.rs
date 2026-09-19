//! Value operator - extracts single value from each row for SELECT VALUE

use std::collections::HashSet;

use crate::query::ast::Expr;
use crate::query::error::ExecuteError;
use crate::query::execute::VarScope;
use crate::query::execute::context::ExecutionContext;
use crate::query::execute::eval::eval_expr;
use crate::query::execute::operators::accumulator::serialize_value;
use crate::query::execute::operators::operator::{Operator, Row};

/// ValueOp extracts a single value from each row.
/// Used for SELECT VALUE expr - returns array of values instead of documents.
pub struct ValueOp<'a, I: Operator, Ctx: ExecutionContext> {
    input: I,
    expr: Expr,
    distinct: bool,
    vars: Option<&'a VarScope>,
    ctx: &'a Ctx,
    seen: HashSet<String>,
    opened: bool,
}

impl<'a, I: Operator, Ctx: ExecutionContext> ValueOp<'a, I, Ctx> {
    pub fn new(
        input: I,
        expr: Expr,
        distinct: bool,
        vars: Option<&'a VarScope>,
        ctx: &'a Ctx,
    ) -> Self {
        Self {
            input,
            expr,
            distinct,
            vars,
            ctx,
            seen: HashSet::new(),
            opened: false,
        }
    }
}

impl<I: Operator, Ctx: ExecutionContext> Operator for ValueOp<'_, I, Ctx> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        self.input.open()?;
        self.seen.clear();
        self.opened = true;
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        while let Some(row) = self.input.next()? {
            let value = eval_expr(&self.expr, &row, None, self.vars, self.ctx)?;

            if self.distinct {
                let key = serialize_value(&value);
                if !self.seen.insert(key) {
                    continue; // Skip duplicate
                }
            }

            // Return a scalar row with the extracted value
            return Ok(Some(Row::scalar(value)));
        }
        Ok(None)
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        self.seen.clear();
        self.opened = false;
        self.input.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Value;
    use crate::query::execute::context::NullContext;
    use crate::query::execute::operators::operator::collect_all;
    use crate::query::execute::operators::operator::test_util::{MockOp, make_doc};

    #[test]
    fn test_value_extracts_field() {
        let docs = vec![
            make_doc(vec![("name", Value::String("Alice".into()))]),
            make_doc(vec![("name", Value::String("Bob".into()))]),
        ];
        let mock = MockOp::new(docs);
        let mut value_op = ValueOp::new(
            mock,
            Expr::Field("name".to_string()),
            false,
            None,
            &NullContext,
        );

        let rows = collect_all(&mut value_op).unwrap();
        assert_eq!(rows.len(), 2);
        // ValueOp returns scalar rows
        assert!(rows[0].is_scalar());
        assert_eq!(rows[0].scalar_value, Some(Value::String("Alice".into())));
        assert_eq!(rows[1].scalar_value, Some(Value::String("Bob".into())));
    }

    #[test]
    fn test_value_distinct() {
        let docs = vec![
            make_doc(vec![("city", Value::String("NYC".into()))]),
            make_doc(vec![("city", Value::String("LA".into()))]),
            make_doc(vec![("city", Value::String("NYC".into()))]),
        ];
        let mock = MockOp::new(docs);
        let mut value_op = ValueOp::new(
            mock,
            Expr::Field("city".to_string()),
            true, // distinct
            None,
            &NullContext,
        );

        let rows = collect_all(&mut value_op).unwrap();
        assert_eq!(rows.len(), 2); // NYC and LA only
    }

    #[test]
    fn test_value_empty_input() {
        let mock = MockOp::new(vec![]);
        let mut value_op = ValueOp::new(
            mock,
            Expr::Field("name".to_string()),
            false,
            None,
            &NullContext,
        );

        let rows = collect_all(&mut value_op).unwrap();
        assert!(rows.is_empty());
    }
}
