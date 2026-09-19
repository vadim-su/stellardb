use std::collections::HashMap;

use crate::Database;
use crate::document::{Document, Value};
use crate::query::ast::{Block, Expr, Statement};
use crate::query::error::ExecuteError;
use crate::query::execute::eval::eval_expr;
use crate::query::execute::operators::operator::{QueryDeadline, check_deadline};
use crate::query::execute::subquery::resolve_subqueries_in_expr;
use crate::query::execute::var_scope::VarScope;
use crate::query::execute::{BatchContext, ExecutionContext, Row};

/// Result of executing a statement or block within control flow.
#[derive(Debug)]
pub enum ControlFlow {
    /// Normal completion with a value.
    Value(Value),
    /// BREAK — exit the innermost FOR loop.
    Break,
    /// CONTINUE — skip to next iteration of the innermost FOR loop.
    Continue,
}

/// Execute a block of statements, returning the value of the last expression.
/// Pushes/pops a scope frame for block-level variable scoping.
pub fn execute_block<Ctx: ExecutionContext>(
    block: &Block,
    vars: &mut VarScope,
    db: &Database,
    storage: &Ctx,
    deadline: QueryDeadline,
) -> Result<ControlFlow, ExecuteError> {
    vars.push();
    let result = execute_block_inner(block, vars, db, storage, deadline);
    vars.pop();
    result
}

/// Inner block execution without push/pop (used by FOR which manages its own frames).
fn execute_block_inner<Ctx: ExecutionContext>(
    block: &Block,
    vars: &mut VarScope,
    db: &Database,
    storage: &Ctx,
    deadline: QueryDeadline,
) -> Result<ControlFlow, ExecuteError> {
    let mut last_value = Value::Null;

    for stmt in &block.statements {
        check_deadline(deadline)?;
        match execute_statement_cf(stmt, vars, db, storage, deadline)? {
            ControlFlow::Value(v) => last_value = v,
            cf @ ControlFlow::Break | cf @ ControlFlow::Continue => return Ok(cf),
        }
    }

    Ok(ControlFlow::Value(last_value))
}

/// Execute a single statement, returning ControlFlow.
/// Intercepts IF/FOR/Break/Continue; delegates everything else.
fn execute_statement_cf<Ctx: ExecutionContext>(
    stmt: &Statement,
    vars: &mut VarScope,
    db: &Database,
    storage: &Ctx,
    deadline: QueryDeadline,
) -> Result<ControlFlow, ExecuteError> {
    match stmt {
        Statement::Begin(_) | Statement::Commit(_) | Statement::Rollback(_) => {
            Err(ExecuteError::TransactionNotSupported(
                "Transaction control is not allowed inside control flow",
            ))
        }
        Statement::Expr(Expr::If {
            branches,
            else_block,
        }) => eval_if(branches, else_block, vars, db, storage, deadline),
        Statement::Expr(Expr::For {
            binding,
            iterable,
            body,
        }) => eval_for(binding, iterable, body, vars, db, storage, deadline),
        Statement::Expr(Expr::Break) => Ok(ControlFlow::Break),
        Statement::Expr(Expr::Continue) => Ok(ControlFlow::Continue),

        // LET — execute and bind variable in current scope
        Statement::Let(let_ast) => {
            let batch_ctx = BatchContext::new(storage, &*vars);
            let resolved = resolve_subqueries_in_expr(&let_ast.expr, &batch_ctx)?;

            // If the RHS is a control flow expression, evaluate it through control flow
            let value = if matches!(&resolved, Expr::If { .. } | Expr::For { .. }) {
                let block = Block {
                    statements: vec![Statement::Expr(resolved)],
                };
                match execute_block(&block, vars, db, storage, deadline)? {
                    ControlFlow::Value(v) => v,
                    ControlFlow::Break => {
                        return Err(ExecuteError::UnexpectedExpression("Break in LET"));
                    }
                    ControlFlow::Continue => {
                        return Err(ExecuteError::UnexpectedExpression("Continue in LET"));
                    }
                }
            } else {
                let empty_row = empty_row();
                eval_expr(&resolved, &empty_row, None, Some(&*vars), &batch_ctx)?
            };

            vars.set(let_ast.name.clone(), value);
            Ok(ControlFlow::Value(Value::Null))
        }

        // Expression statement — evaluate and return value
        Statement::Expr(expr) => {
            let batch_ctx = BatchContext::new(storage, &*vars);
            let resolved = resolve_subqueries_in_expr(expr, &batch_ctx)?;
            let empty_row = empty_row();
            let value = eval_expr(&resolved, &empty_row, None, Some(&*vars), &batch_ctx)?;
            Ok(ControlFlow::Value(value))
        }

        // SELECT — execute through the normal pipeline
        Statement::Select(_) => {
            let rows = execute_dml_statement(stmt.clone(), vars, db, storage, deadline)?;
            Ok(ControlFlow::Value(rows_to_value(rows)))
        }

        // Other DML (CREATE, UPDATE, DELETE, INSERT, RELATE)
        Statement::Create(_)
        | Statement::Update(_)
        | Statement::Upsert(_)
        | Statement::Delete(_)
        | Statement::Insert(_)
        | Statement::Relate(_) => {
            let rows = execute_dml_statement(stmt.clone(), vars, db, storage, deadline)?;
            Ok(ControlFlow::Value(rows_to_value(rows)))
        }
        _ => Err(ExecuteError::InvalidOperation(
            "DDL, EXPLAIN, and session configuration are not supported inside control flow".into(),
        )),
    }
}

/// Evaluate IF expression.
fn eval_if<Ctx: ExecutionContext>(
    branches: &[(Box<Expr>, Block)],
    else_block: &Option<Block>,
    vars: &mut VarScope,
    db: &Database,
    storage: &Ctx,
    deadline: QueryDeadline,
) -> Result<ControlFlow, ExecuteError> {
    let empty_row = empty_row();

    for (condition, block) in branches {
        check_deadline(deadline)?;
        let batch_ctx = BatchContext::new(storage, &*vars);
        let resolved = resolve_subqueries_in_expr(condition, &batch_ctx)?;
        let cond_val = eval_expr(&resolved, &empty_row, None, Some(&*vars), &batch_ctx)?;
        if cond_val.is_truthy() {
            return execute_block(block, vars, db, storage, deadline);
        }
    }

    match else_block {
        Some(block) => execute_block(block, vars, db, storage, deadline),
        None => Ok(ControlFlow::Value(Value::Null)),
    }
}

/// Evaluate FOR expression. Returns Value::Array of collected iteration results.
fn eval_for<Ctx: ExecutionContext>(
    binding: &str,
    iterable: &Expr,
    body: &Block,
    vars: &mut VarScope,
    db: &Database,
    storage: &Ctx,
    deadline: QueryDeadline,
) -> Result<ControlFlow, ExecuteError> {
    let empty_row = empty_row();
    let batch_ctx = BatchContext::new(storage, &*vars);
    let resolved_iter = resolve_subqueries_in_expr(iterable, &batch_ctx)?;
    let iter_value = eval_expr(&resolved_iter, &empty_row, None, Some(&*vars), &batch_ctx)?;
    let items = value_to_iterator(iter_value)?;
    let mut results = Vec::new();

    for item in items {
        check_deadline(deadline)?;
        vars.push();
        vars.set(binding.to_string(), item);

        let result = execute_block_inner(body, vars, db, storage, deadline);
        vars.pop();
        match result? {
            ControlFlow::Value(v) => results.push(v),
            ControlFlow::Break => {
                break;
            }
            ControlFlow::Continue => {
                continue;
            }
        }
    }

    Ok(ControlFlow::Value(Value::Array(results)))
}

/// Convert a Value into an iterable Vec.
fn value_to_iterator(value: Value) -> Result<Vec<Value>, ExecuteError> {
    match value {
        Value::Array(items) => Ok(items),
        Value::Null => Ok(vec![]),
        Value::Range { start, end } => match (*start, *end) {
            (Value::Int(s), Value::Int(e)) => Ok((s..=e).map(Value::Int).collect()),
            _ => Err(ExecuteError::Internal(
                "Range iteration only supports integer bounds".into(),
            )),
        },
        other => Err(ExecuteError::Internal(format!(
            "Cannot iterate over {}",
            other.type_name()
        ))),
    }
}

/// Execute a DML statement through the existing pipeline.
fn execute_dml_statement<Ctx: ExecutionContext>(
    stmt: Statement,
    vars: &VarScope,
    db: &Database,
    storage: &Ctx,
    deadline: QueryDeadline,
) -> Result<Vec<Row>, ExecuteError> {
    // Bind the statement
    let fn_registry = crate::query::function::global_registry();
    let stmt = crate::query::bind::bind_statement(stmt, db.schema_registry(), fn_registry)
        .map_err(|e| ExecuteError::Internal(format!("Bind error in control flow: {}", e)))?;

    let batch_ctx = BatchContext::new(storage, vars);
    crate::query::execute::engine::execute_statement(stmt, &batch_ctx, deadline)
        .map(|result| result.rows)
}

/// Convert rows to a Value for use as a block result.
fn rows_to_value(rows: Vec<Row>) -> Value {
    let values: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            if let Some(scalar) = row.scalar_value {
                scalar
            } else {
                let mut obj = HashMap::new();
                if !row.doc.id.is_empty() {
                    obj.insert("id".to_string(), Value::String(row.doc.id));
                }
                for (k, v) in row.doc.fields {
                    obj.insert(k, v);
                }
                Value::Object(obj)
            }
        })
        .collect();
    Value::Array(values)
}

fn empty_row() -> Row {
    Row::from_doc(Document {
        id: String::new(),
        fields: HashMap::new(),
    })
}
