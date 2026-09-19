use crate::query::ast::Expr;
use crate::query::error::ExecuteError;
use crate::query::execute::context::ExecutionContext;
use crate::query::execute::eval::eval_filter;
use crate::query::execute::operators::operator::{Operator, Row};

/// Filter operator - passes through rows matching predicate
pub struct FilterOp<'a, I: Operator, Ctx: ExecutionContext> {
    input: I,
    predicate: Expr,
    ctx: &'a Ctx,
}

impl<'a, I: Operator, Ctx: ExecutionContext> FilterOp<'a, I, Ctx> {
    pub fn new(input: I, predicate: Expr, ctx: &'a Ctx) -> Self {
        Self {
            input,
            predicate,
            ctx,
        }
    }
}

impl<'a, I: Operator, Ctx: ExecutionContext> Operator for FilterOp<'a, I, Ctx> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        self.input.open()
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        // Pull from input until we find a matching row
        while let Some(row) = self.input.next()? {
            if eval_filter(&self.predicate, &row, self.ctx)? {
                return Ok(Some(row));
            }
        }
        Ok(None)
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        self.input.close()
    }
}
