use std::sync::Arc;

use crate::Database;
use crate::document::Value;
use crate::query::ExecuteError;
use crate::query::ast::Statement;
use crate::query::execute::{self, BatchContext, Row};
use crate::query::result::{BatchResult, StatementResult};

use super::ExecuteContext;
use super::batch_result;
use super::state::BatchState;
use super::{map_session_access_error, map_session_error, map_session_operation_error};

pub(super) enum DirectOutcome {
    Handled,
    NotHandled(Box<Statement>),
    Error(BatchResult),
}

pub(super) fn try_handle_tx_or_set(
    index: usize,
    stmt: Statement,
    storage: Arc<Database>,
    ctx: &ExecuteContext<'_>,
    state: &mut BatchState,
) -> Result<DirectOutcome, ExecuteError> {
    match stmt {
        Statement::Begin(_) => {
            let sessions = ctx.sessions.ok_or(ExecuteError::SessionRequired)?;
            if state.batch_tx.is_some() || ctx.session_id.is_some() {
                return Err(ExecuteError::TransactionNotSupported(
                    "BEGIN requires no active transaction",
                ));
            }
            let id = sessions.begin_transaction(storage, ctx.session_owner());
            state.batch_tx = Some(id.clone());
            state.results.push(StatementResult::begin(index, id));
            Ok(DirectOutcome::Handled)
        }
        Statement::Commit(_) => {
            let sessions = ctx.sessions.ok_or(ExecuteError::SessionRequired)?;
            let tx_id = state.batch_tx.as_ref().or(ctx.session_id.as_ref()).ok_or(
                ExecuteError::TransactionNotSupported("COMMIT requires active transaction"),
            )?;
            sessions
                .commit(tx_id, &ctx.session_owner())
                .map_err(map_session_error)?;
            state.batch_tx = None;
            state.results.push(StatementResult::commit(index));
            Ok(DirectOutcome::Handled)
        }
        Statement::Rollback(_) => {
            let sessions = ctx.sessions.ok_or(ExecuteError::SessionRequired)?;
            let tx_id = state.batch_tx.as_ref().or(ctx.session_id.as_ref()).ok_or(
                ExecuteError::TransactionNotSupported("ROLLBACK requires active transaction"),
            )?;
            sessions
                .rollback(tx_id, &ctx.session_owner())
                .map_err(map_session_error)?;
            state.batch_tx = None;
            state.results.push(StatementResult::rollback(index));
            Ok(DirectOutcome::Handled)
        }
        Statement::Set(set_ast) => {
            let timeout = if set_ast.value_nanos == 0 {
                None
            } else {
                Some(std::time::Duration::from_nanos(set_ast.value_nanos))
            };
            if let Some(sessions) = ctx.sessions
                && let Some(sid) = state.batch_tx.as_ref().or(ctx.session_id.as_ref())
            {
                sessions
                    .set_query_timeout(sid, &ctx.session_owner(), timeout)
                    .map_err(map_session_access_error)?;
            }
            state.batch_query_timeout = Some(timeout);
            state.results.push(StatementResult::data(
                index,
                "SET",
                execute::status_row(vec![
                    ("key", Value::String(set_ast.key)),
                    ("status", Value::String("ok".to_string())),
                ]),
            ));
            Ok(DirectOutcome::Handled)
        }
        other => Ok(DirectOutcome::NotHandled(Box::new(other))),
    }
}

pub(super) fn try_handle_statement(
    index: usize,
    stmt: Statement,
    db: &Database,
    active_tx_id: Option<&String>,
    ctx: &ExecuteContext<'_>,
    state: &mut BatchState,
    deadline: crate::query::execute::operators::operator::QueryDeadline,
) -> DirectOutcome {
    if super::is_ddl(&stmt) && active_tx_id.is_some() {
        return fail_execute(index, ExecuteError::DdlNotAllowedInTransaction, ctx, state);
    }

    match stmt {
        Statement::DefineAnalyzer(ast) => {
            match execute::ddl::execute_define_analyzer(db, ast.name, ast.tokenizer, ast.filters) {
                Ok(rows) => {
                    state
                        .results
                        .push(StatementResult::data(index, "DEFINE ANALYZER", rows));
                    DirectOutcome::Handled
                }
                Err(error) => fail_execute(index, error, ctx, state),
            }
        }
        Statement::DropAnalyzer(ast) => match execute::ddl::execute_drop_analyzer(db, &ast.name) {
            Ok(rows) => {
                state
                    .results
                    .push(StatementResult::data(index, "DROP ANALYZER", rows));
                DirectOutcome::Handled
            }
            Err(error) => fail_execute(index, error, ctx, state),
        },
        #[cfg(feature = "auth")]
        Statement::CreateUser(ast) => {
            let Some(auth) = ctx.auth else {
                return fail(index, "Auth not configured", ctx, state);
            };
            match auth.create_user_typed(&ast.username, &ast.password) {
                Ok(()) => {
                    state.results.push(StatementResult::data(
                        index,
                        "CREATE USER",
                        execute::status_row(vec![
                            ("status", Value::String("created".into())),
                            ("user", Value::String(ast.username)),
                        ]),
                    ));
                    DirectOutcome::Handled
                }
                Err(error) => fail_auth(index, error, ctx, state),
            }
        }
        #[cfg(feature = "auth")]
        Statement::DropUser(ast) => {
            let Some(auth) = ctx.auth else {
                return fail(index, "Auth not configured", ctx, state);
            };
            match auth.drop_user_typed(&ast.username) {
                Ok(()) => {
                    state.results.push(StatementResult::data(
                        index,
                        "DROP USER",
                        execute::status_row(vec![
                            ("status", Value::String("dropped".into())),
                            ("user", Value::String(ast.username)),
                        ]),
                    ));
                    DirectOutcome::Handled
                }
                Err(error) => fail_auth(index, error, ctx, state),
            }
        }
        #[cfg(feature = "auth")]
        Statement::AlterUser(ast) => {
            let Some(auth) = ctx.auth else {
                return fail(index, "Auth not configured", ctx, state);
            };
            let result = match ast {
                crate::query::ast::AlterUserAst::Password { username, password } => auth
                    .alter_user_password_typed(&username, &password)
                    .map(|()| {
                        execute::status_row(vec![
                            ("status", Value::String("password_changed".into())),
                            ("user", Value::String(username)),
                        ])
                    }),
                crate::query::ast::AlterUserAst::Set {
                    username,
                    attributes,
                } => {
                    let attrs = attributes.into_iter().collect();
                    auth.alter_user_set_typed(&username, attrs).map(|()| {
                        execute::status_row(vec![
                            ("status", Value::String("attributes_set".into())),
                            ("user", Value::String(username)),
                        ])
                    })
                }
                crate::query::ast::AlterUserAst::Remove {
                    username,
                    attributes,
                } => auth
                    .alter_user_remove_typed(&username, &attributes)
                    .map(|()| {
                        execute::status_row(vec![
                            ("status", Value::String("attributes_removed".into())),
                            ("user", Value::String(username)),
                        ])
                    }),
            };
            push_auth_rows(index, "ALTER USER", result, ctx, state)
        }
        #[cfg(feature = "auth")]
        Statement::CreateApiKey(ast) => {
            let Some(auth) = ctx.auth else {
                return fail(index, "Auth not configured", ctx, state);
            };
            match auth.create_api_key_typed(&ast.name, &ast.username) {
                Ok(key) => {
                    state.results.push(StatementResult::data(
                        index,
                        "CREATE API KEY",
                        execute::status_row(vec![
                            ("status", Value::String("created".into())),
                            ("name", Value::String(ast.name)),
                            ("user", Value::String(ast.username)),
                            ("key", Value::String(key)),
                        ]),
                    ));
                    DirectOutcome::Handled
                }
                Err(error) => fail_auth(index, error, ctx, state),
            }
        }
        #[cfg(feature = "auth")]
        Statement::DropApiKey(ast) => {
            let Some(auth) = ctx.auth else {
                return fail(index, "Auth not configured", ctx, state);
            };
            match auth.drop_api_key_typed(&ast.name) {
                Ok(()) => {
                    state.results.push(StatementResult::data(
                        index,
                        "DROP API KEY",
                        execute::status_row(vec![
                            ("status", Value::String("dropped".into())),
                            ("name", Value::String(ast.name)),
                        ]),
                    ));
                    DirectOutcome::Handled
                }
                Err(error) => fail_auth(index, error, ctx, state),
            }
        }
        #[cfg(feature = "auth")]
        Statement::CreatePolicy(ast) => {
            let Some(auth) = ctx.auth else {
                return fail(index, "Auth not configured", ctx, state);
            };
            use crate::auth::{Condition, ConditionOp, ConditionValue, Effect, Policy};
            use crate::query::ast::PolicyEffectAst;

            let conditions: Vec<Condition> = ast
                .conditions
                .into_iter()
                .map(|c| {
                    let op = match c.op.as_str() {
                        "=" => ConditionOp::Eq,
                        "!=" => ConditionOp::Neq,
                        "IN" => ConditionOp::In,
                        _ => ConditionOp::Eq,
                    };
                    let value = if c.values.len() == 1 {
                        ConditionValue::Single(c.values[0].clone())
                    } else {
                        ConditionValue::List(c.values)
                    };
                    Condition {
                        field: c.field,
                        op,
                        value,
                    }
                })
                .collect();

            let effect = match ast.effect {
                PolicyEffectAst::Allow => Effect::Allow,
                PolicyEffectAst::Deny => Effect::Deny,
            };
            let name = ast.name;
            let policy = Policy {
                name: name.clone(),
                conditions,
                effect,
            };
            match auth.create_policy_typed(&policy) {
                Ok(()) => {
                    state.results.push(StatementResult::data(
                        index,
                        "CREATE POLICY",
                        execute::status_row(vec![
                            ("status", Value::String("created".into())),
                            ("policy", Value::String(name)),
                        ]),
                    ));
                    DirectOutcome::Handled
                }
                Err(error) => fail_auth(index, error, ctx, state),
            }
        }
        #[cfg(feature = "auth")]
        Statement::DropPolicy(ast) => {
            let Some(auth) = ctx.auth else {
                return fail(index, "Auth not configured", ctx, state);
            };
            match auth.drop_policy_typed(&ast.name) {
                Ok(()) => {
                    state.results.push(StatementResult::data(
                        index,
                        "DROP POLICY",
                        execute::status_row(vec![
                            ("status", Value::String("dropped".into())),
                            ("policy", Value::String(ast.name)),
                        ]),
                    ));
                    DirectOutcome::Handled
                }
                Err(error) => fail_auth(index, error, ctx, state),
            }
        }
        #[cfg(feature = "auth")]
        Statement::AlterCollectionAttrs(ast) => {
            let Some(auth) = ctx.auth else {
                return fail(index, "Auth not configured", ctx, state);
            };
            let database = ctx.database.as_deref().unwrap_or_default();
            let result = match ast {
                crate::query::ast::AlterCollectionAttrsAst::Set {
                    collection,
                    attributes,
                } => {
                    let attrs = attributes.into_iter().collect();
                    auth.set_collection_attributes(database, &collection, attrs)
                        .map(|()| {
                            execute::status_row(vec![
                                ("status", Value::String("attributes_set".into())),
                                ("collection", Value::String(collection)),
                            ])
                        })
                }
                crate::query::ast::AlterCollectionAttrsAst::Remove {
                    collection,
                    attributes,
                } => auth
                    .remove_collection_attributes(database, &collection, &attributes)
                    .map(|()| {
                        execute::status_row(vec![
                            ("status", Value::String("attributes_removed".into())),
                            ("collection", Value::String(collection)),
                        ])
                    }),
            };
            push_internal_rows(index, "ALTER COLLECTION", result, ctx, state)
        }
        #[cfg(not(feature = "auth"))]
        Statement::CreateUser(_)
        | Statement::DropUser(_)
        | Statement::AlterUser(_)
        | Statement::CreateApiKey(_)
        | Statement::DropApiKey(_)
        | Statement::CreatePolicy(_)
        | Statement::DropPolicy(_)
        | Statement::AlterCollectionAttrs(_) => fail(
            index,
            "auth support is not enabled in this build; rebuild with --features auth",
            ctx,
            state,
        ),
        Statement::Relate(ast) => {
            let exec_result = if let Some(tx_id) = active_tx_id {
                ctx.sessions
                    .expect("sessions required for transaction context")
                    .with_transaction_operation(tx_id, &ctx.session_owner(), |tx| {
                        let batch_ctx = BatchContext::new(tx, &state.vars);
                        execute::relate::execute_relate(&ast, &batch_ctx)
                    })
                    .map_err(map_session_operation_error)
            } else {
                let batch_ctx = BatchContext::new(db, &state.vars);
                execute::relate::execute_relate(&ast, &batch_ctx)
            };
            match exec_result {
                Ok(rows) => {
                    state
                        .results
                        .push(StatementResult::data(index, "RELATE", rows));
                    DirectOutcome::Handled
                }
                Err(error) => fail_execute(index, error, ctx, state),
            }
        }
        Statement::Expr(expr) => {
            use crate::query::execute::control_flow::{self, ControlFlow};
            let block = crate::query::ast::Block {
                statements: vec![Statement::Expr(expr)],
            };
            let result = if let Some(tx_id) = active_tx_id {
                ctx.sessions
                    .expect("sessions required for transaction context")
                    .with_transaction_operation(tx_id, &ctx.session_owner(), |tx| {
                        control_flow::execute_block(&block, &mut state.vars, db, tx, deadline)
                    })
                    .map_err(map_session_operation_error)
            } else {
                control_flow::execute_block(&block, &mut state.vars, db, db, deadline)
            };
            match result {
                Ok(ControlFlow::Value(value)) => {
                    let rows = vec![Row::scalar(value)];
                    state
                        .results
                        .push(StatementResult::data(index, "EXPR", rows));
                    DirectOutcome::Handled
                }
                Ok(ControlFlow::Break) => fail(index, "BREAK outside of FOR loop", ctx, state),
                Ok(ControlFlow::Continue) => {
                    fail(index, "CONTINUE outside of FOR loop", ctx, state)
                }
                Err(error) => fail_execute(index, error, ctx, state),
            }
        }
        other => DirectOutcome::NotHandled(Box::new(other)),
    }
}

#[cfg(feature = "auth")]
fn push_auth_rows(
    index: usize,
    statement_type: &'static str,
    result: Result<Vec<Row>, crate::auth::AuthError>,
    ctx: &ExecuteContext<'_>,
    state: &mut BatchState,
) -> DirectOutcome {
    match result {
        Ok(rows) => {
            state
                .results
                .push(StatementResult::data(index, statement_type, rows));
            DirectOutcome::Handled
        }
        Err(error) => fail_auth(index, error, ctx, state),
    }
}

#[cfg(feature = "auth")]
fn push_internal_rows(
    index: usize,
    statement_type: &'static str,
    result: Result<Vec<Row>, String>,
    ctx: &ExecuteContext<'_>,
    state: &mut BatchState,
) -> DirectOutcome {
    match result {
        Ok(rows) => {
            state
                .results
                .push(StatementResult::data(index, statement_type, rows));
            DirectOutcome::Handled
        }
        Err(error) => fail_internal(index, error, ctx, state),
    }
}

fn fail(
    index: usize,
    message: impl Into<String>,
    ctx: &ExecuteContext<'_>,
    state: &mut BatchState,
) -> DirectOutcome {
    let error = crate::query::result::BatchError::public(
        index,
        "SDB-QE000",
        crate::error::ErrorClass::InvalidArgument,
        message,
    );
    finish_failure(index, error, ctx, state)
}

fn fail_execute(
    index: usize,
    error: ExecuteError,
    ctx: &ExecuteContext<'_>,
    state: &mut BatchState,
) -> DirectOutcome {
    let error = crate::query::result::BatchError::from_execute_error(index, &error);
    finish_failure(index, error, ctx, state)
}

#[cfg(feature = "auth")]
fn fail_internal(
    index: usize,
    source: impl std::fmt::Display,
    ctx: &ExecuteContext<'_>,
    state: &mut BatchState,
) -> DirectOutcome {
    let error = crate::query::result::BatchError::internal(index, "SDB-QE016", source);
    finish_failure(index, error, ctx, state)
}

#[cfg(feature = "auth")]
fn fail_auth(
    index: usize,
    source: crate::auth::AuthError,
    ctx: &ExecuteContext<'_>,
    state: &mut BatchState,
) -> DirectOutcome {
    let error = crate::query::result::BatchError::from_auth_error(index, &source);
    finish_failure(index, error, ctx, state)
}

fn finish_failure(
    index: usize,
    error: crate::query::result::BatchError,
    ctx: &ExecuteContext<'_>,
    state: &mut BatchState,
) -> DirectOutcome {
    super::auto_rollback(&state.batch_tx, ctx);
    DirectOutcome::Error(batch_result::error_result(
        std::mem::take(&mut state.results),
        index,
        error,
    ))
}
