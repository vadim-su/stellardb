use crate::Database;
use crate::document::{Document, Value};
use crate::query::ast::{Expr, Statement};
use crate::query::execute::eval::eval_expr;
use crate::query::execute::subquery::{
    projection_has_non_correlated_subquery, resolve_subqueries_in_expr,
    resolve_subqueries_in_projection,
};
use crate::query::execute::{self, BatchContext, ExecuteResult, ExecutionContext, Row};
use crate::query::result::BatchResult;
use crate::query::{ExecuteError, LogicalPlan, bind, function};

use super::batch_result;
use super::state::BatchState;
use super::{ExecuteContext, is_ddl, map_session_operation_error};

pub(super) fn execute_planned_statement(
    index: usize,
    stmt: Statement,
    db: &Database,
    active_tx_id: Option<&String>,
    ctx: &ExecuteContext<'_>,
    state: &mut BatchState,
    deadline: crate::query::execute::operators::operator::QueryDeadline,
) -> Result<(Statement, ExecuteResult), BatchResult> {
    if is_ddl(&stmt) && active_tx_id.is_some() {
        return Err(fail(
            index,
            ExecuteError::DdlNotAllowedInTransaction.into(),
            ctx,
            state,
        ));
    }

    let fn_registry = function::global_registry();
    let stmt = bind::bind_statement(stmt, db.schema_registry(), fn_registry)
        .map_err(|error| fail(index, crate::query::QueryError::Bind(error), ctx, state))?;

    let result = if let Some(tx_id) = active_tx_id {
        ctx.sessions
            .expect("sessions required for transaction context")
            .with_transaction_operation(tx_id, &ctx.session_owner(), |tx| {
                execute_bound_statement(stmt, db, tx, state, deadline, true)
            })
            .map_err(map_session_operation_error)
    } else {
        execute_bound_statement(stmt, db, db, state, deadline, false)
    };
    result.map_err(|error| fail(index, error.into(), ctx, state))
}

fn execute_bound_statement<Ctx: ExecutionContext>(
    stmt: Statement,
    db: &Database,
    storage: &Ctx,
    state: &mut BatchState,
    deadline: crate::query::execute::operators::operator::QueryDeadline,
    in_transaction: bool,
) -> Result<(Statement, ExecuteResult), ExecuteError> {
    let resolved_stmt = match stmt {
        Statement::Select(mut select)
            if projection_has_non_correlated_subquery(&select.projection) =>
        {
            let batch_ctx = BatchContext::new(storage, &state.vars);
            select.projection = resolve_subqueries_in_projection(&select.projection, &batch_ctx)?;
            Statement::Select(select)
        }
        other => other,
    };
    let batch_ctx = BatchContext::new(storage, &state.vars);
    let plan = execute::engine::plan_from_statement(resolved_stmt.clone(), &batch_ctx)?;
    let result = execute_plan(
        &resolved_stmt,
        plan,
        db,
        storage,
        state,
        deadline,
        in_transaction,
    )?;
    Ok((resolved_stmt, result))
}

fn execute_plan<Ctx: ExecutionContext>(
    resolved_stmt: &Statement,
    plan: LogicalPlan,
    db: &Database,
    storage: &Ctx,
    state: &mut BatchState,
    deadline: crate::query::execute::operators::operator::QueryDeadline,
    in_transaction: bool,
) -> Result<ExecuteResult, ExecuteError> {
    match &plan {
        LogicalPlan::DefineCollection { .. }
        | LogicalPlan::DropCollection { .. }
        | LogicalPlan::DescribeCollection(_)
        | LogicalPlan::DescribeCollections
        | LogicalPlan::CreateIndex { .. }
        | LogicalPlan::DropIndex { .. }
        | LogicalPlan::Reindex { .. } => execute::execute_ddl(plan, db).map(ExecuteResult::new),

        LogicalPlan::Explain { .. } => {
            if in_transaction {
                return Err(ExecuteError::Internal(
                    "EXPLAIN not allowed in transactions".to_string(),
                ));
            }
            let _query_guard = db.acquire_query_guard();
            execute::execute(plan, db, deadline).map(ExecuteResult::new)
        }

        LogicalPlan::Let { name, expr } => execute_let(name, expr, db, storage, state, deadline),

        LogicalPlan::Begin | LogicalPlan::Commit | LogicalPlan::Rollback => {
            unreachable!("TX control handled above")
        }

        _ => {
            let _query_guard = db.acquire_query_guard();
            let batch_ctx = BatchContext::new(storage, &state.vars);
            execute::engine::execute_statement(resolved_stmt.clone(), &batch_ctx, deadline)
        }
    }
}

fn execute_let<Ctx: ExecutionContext>(
    name: &str,
    expr: &Expr,
    db: &Database,
    storage: &Ctx,
    state: &mut BatchState,
    deadline: crate::query::execute::operators::operator::QueryDeadline,
) -> Result<ExecuteResult, ExecuteError> {
    let resolved_expr = {
        let batch_ctx = BatchContext::new(storage, &state.vars);
        resolve_subqueries_in_expr(expr, &batch_ctx)?
    };

    if matches!(&resolved_expr, Expr::If { .. } | Expr::For { .. }) {
        use crate::query::execute::control_flow::{self, ControlFlow};
        let block = crate::query::ast::Block {
            statements: vec![Statement::Expr(resolved_expr)],
        };
        match control_flow::execute_block(&block, &mut state.vars, db, storage, deadline) {
            Ok(ControlFlow::Value(value)) => bind_variable_result(name, value, state),
            Ok(_) => Err(ExecuteError::Internal(
                "BREAK/CONTINUE in LET expression".into(),
            )),
            Err(e) => Err(e),
        }
    } else {
        let empty_row = Row::from_doc(Document {
            id: String::new(),
            fields: std::collections::HashMap::new(),
        });
        let value = {
            let batch_ctx = BatchContext::new(storage, &state.vars);
            eval_expr(
                &resolved_expr,
                &empty_row,
                None,
                Some(&state.vars),
                &batch_ctx,
            )?
        };
        bind_variable_result(name, value, state)
    }
}

fn bind_variable_result(
    name: &str,
    value: Value,
    state: &mut BatchState,
) -> Result<ExecuteResult, ExecuteError> {
    state.vars.set(name.to_owned(), value.clone());
    let rows = execute::status_row(vec![
        ("variable", Value::String(name.to_owned())),
        ("value", value),
        ("status", Value::String("bound".to_string())),
    ]);
    Ok(ExecuteResult::new(rows))
}

fn fail(
    index: usize,
    error: crate::query::QueryError,
    ctx: &ExecuteContext<'_>,
    state: &mut BatchState,
) -> BatchResult {
    super::auto_rollback(&state.batch_tx, ctx);
    batch_result::query_error_result(std::mem::take(&mut state.results), index, error)
}
