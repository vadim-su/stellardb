use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::document::{Document, Value};
use crate::error::StorageError;
use crate::query::execute::{DocumentOps, Row};
use crate::query::{
    BatchError, BatchResult, ExecuteContext, Query, QueryError, Statement, StatementResult,
};
use crate::session::{SessionConfig, SessionError, SessionManager, SessionOwner};
use crate::storage::{Database, StorageStats};
use crate::transaction::Transaction;

/// Synchronous, in-process database handle. Clones share the same storage.
///
/// Run blocking operations off an application's event-loop thread. Releasing a
/// handle does not invalidate other clones, transactions, or `storage()` handles.
#[derive(Clone)]
pub struct EmbeddedDatabase {
    storage: Arc<Database>,
}

/// Per-call values and execution limits. Parameter names omit the `$` prefix.
#[derive(Debug, Default)]
pub struct EmbeddedQueryOptions {
    pub params: HashMap<String, Value>,
    /// Cooperative execution timeout, including preceding statements in a batch.
    /// `Some(Duration::ZERO)` expires immediately; `None` allows StellarQL SET.
    /// Parsing and blocking storage/DDL work cannot be interrupted mid-operation.
    pub timeout: Option<Duration>,
}

#[derive(Debug, thiserror::Error)]
pub enum EmbeddedError {
    #[error("{0}")]
    Storage(#[from] StorageError),
    #[error("{0}")]
    Query(#[from] QueryError),
    #[error("{0}")]
    Session(#[from] SessionError),
    /// Earlier results are diagnostic, not proof of commit inside a transaction.
    #[error("batch error at statement {}: {}", .error.statement_index, .error.message)]
    Batch {
        error: BatchError,
        results: Vec<StatementResult>,
        completed: usize,
    },
    #[error("expected a single statement, got {actual}")]
    ExpectedSingleResult { actual: usize },
    #[error("use the transaction handle's commit/rollback methods, not transaction control SQL")]
    TransactionControl,
}

impl EmbeddedDatabase {
    /// Open or create a local database directory. No server or async runtime is started.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, EmbeddedError> {
        Ok(Self {
            storage: Arc::new(Database::open(path.as_ref())?),
        })
    }

    /// Shared low-level handle. Its lifetime can extend beyond this handle's close.
    pub fn storage(&self) -> Arc<Database> {
        self.storage.clone()
    }

    /// Execute an autocommit batch. On failure, earlier successful writes remain.
    /// Use `transaction()` when the batch must be atomic.
    pub fn execute(&self, sql: &str) -> Result<BatchResult, EmbeddedError> {
        self.execute_with_options(sql, EmbeddedQueryOptions::default())
    }

    pub fn execute_with_params(
        &self,
        sql: &str,
        params: HashMap<String, Value>,
    ) -> Result<BatchResult, EmbeddedError> {
        self.execute_with_options(
            sql,
            EmbeddedQueryOptions {
                params,
                ..Default::default()
            },
        )
    }

    pub fn execute_with_options(
        &self,
        sql: &str,
        options: EmbeddedQueryOptions,
    ) -> Result<BatchResult, EmbeddedError> {
        run(
            self.storage.clone(),
            sql,
            context(ExecuteContext::none(), options),
            false,
            false,
        )
    }

    /// Execute exactly one statement. Multiple statements are rejected before execution.
    pub fn query(&self, sql: &str) -> Result<Vec<Row>, EmbeddedError> {
        self.query_with_options(sql, EmbeddedQueryOptions::default())
    }

    pub fn query_with_params(
        &self,
        sql: &str,
        params: HashMap<String, Value>,
    ) -> Result<Vec<Row>, EmbeddedError> {
        self.query_with_options(
            sql,
            EmbeddedQueryOptions {
                params,
                ..Default::default()
            },
        )
    }

    pub fn query_with_options(
        &self,
        sql: &str,
        options: EmbeddedQueryOptions,
    ) -> Result<Vec<Row>, EmbeddedError> {
        run(
            self.storage.clone(),
            sql,
            context(ExecuteContext::none(), options),
            true,
            false,
        )
        .map(single_rows)
    }

    /// Begin a buffered transaction with read-your-writes and atomic commit.
    /// This is not a repeatable-read snapshot: concurrent committed writes can be
    /// observed. DDL is not transactional. Dropping the handle rolls back.
    pub fn transaction(&self) -> EmbeddedTransaction {
        let sessions = SessionManager::new(SessionConfig::default());
        let owner = SessionOwner::anonymous(String::new());
        let id = sessions.begin_transaction(self.storage.clone(), owner.clone());
        EmbeddedTransaction {
            storage: self.storage.clone(),
            sessions,
            owner,
            id,
            finished: false,
        }
    }

    pub fn get_document(
        &self,
        collection: &str,
        key: &str,
    ) -> Result<Option<Document>, EmbeddedError> {
        Ok(self.storage.get_document(collection, key)?)
    }

    pub fn create_document(&self, doc: &Document) -> Result<bool, EmbeddedError> {
        Ok(self.storage.create_document(doc)?)
    }

    pub fn set_document(&self, doc: &Document) -> Result<(), EmbeddedError> {
        Ok(self.storage.set_document(doc)?)
    }

    pub fn update_document(&self, doc: &Document) -> Result<bool, EmbeddedError> {
        Ok(self.storage.update_document(doc)?)
    }

    pub fn delete_document(&self, collection: &str, key: &str) -> Result<bool, EmbeddedError> {
        Ok(self.storage.delete_document(collection, key)?)
    }

    pub fn list_documents(
        &self,
        collection: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Document>, Option<String>), EmbeddedError> {
        Ok(self.storage.list_documents(collection, after, limit)?)
    }

    pub fn list_collections(&self) -> Vec<String> {
        self.storage.list_collections()
    }

    pub fn stats(&self) -> StorageStats {
        self.storage.stats()
    }

    /// Commit search indexes and persist the storage journal using SyncAll.
    /// Does not commit pending transactions. Quiesce concurrent writers when a
    /// stable application checkpoint is required. Propagates persistence errors.
    pub fn sync(&self) -> Result<(), EmbeddedError> {
        Ok(self.storage.sync()?)
    }

    /// Sync committed data and release this handle (also on error).
    /// Storage closes only after the final shared handle is released. Ordinary
    /// Drop cannot report durability errors; use sync/close when they matter.
    pub fn close(self) -> Result<(), EmbeddedError> {
        self.sync()
    }
}

/// An owned transaction. All SQL and document operations share its write buffer.
///
/// Statement errors leave prior buffered writes pending; explicitly roll back or
/// drop the handle to discard them. Commit/rollback consume the handle. Query
/// parameters and LET variables are local to each call, not the transaction.
pub struct EmbeddedTransaction {
    storage: Arc<Database>,
    sessions: SessionManager,
    owner: SessionOwner,
    id: String,
    finished: bool,
}

impl EmbeddedTransaction {
    pub fn execute(&self, sql: &str) -> Result<BatchResult, EmbeddedError> {
        self.execute_with_options(sql, EmbeddedQueryOptions::default())
    }

    pub fn execute_with_params(
        &self,
        sql: &str,
        params: HashMap<String, Value>,
    ) -> Result<BatchResult, EmbeddedError> {
        self.execute_with_options(
            sql,
            EmbeddedQueryOptions {
                params,
                ..Default::default()
            },
        )
    }

    pub fn execute_with_options(
        &self,
        sql: &str,
        options: EmbeddedQueryOptions,
    ) -> Result<BatchResult, EmbeddedError> {
        run(
            self.storage.clone(),
            sql,
            context(
                ExecuteContext::with_session(&self.sessions, Some(&self.id)),
                options,
            ),
            false,
            true,
        )
    }

    pub fn query(&self, sql: &str) -> Result<Vec<Row>, EmbeddedError> {
        self.query_with_options(sql, EmbeddedQueryOptions::default())
    }

    pub fn query_with_params(
        &self,
        sql: &str,
        params: HashMap<String, Value>,
    ) -> Result<Vec<Row>, EmbeddedError> {
        self.query_with_options(
            sql,
            EmbeddedQueryOptions {
                params,
                ..Default::default()
            },
        )
    }

    pub fn query_with_options(
        &self,
        sql: &str,
        options: EmbeddedQueryOptions,
    ) -> Result<Vec<Row>, EmbeddedError> {
        run(
            self.storage.clone(),
            sql,
            context(
                ExecuteContext::with_session(&self.sessions, Some(&self.id)),
                options,
            ),
            true,
            true,
        )
        .map(single_rows)
    }

    fn document_operation<T>(
        &self,
        operation: impl FnOnce(&Transaction) -> Result<T, StorageError>,
    ) -> Result<T, EmbeddedError> {
        Ok(self
            .sessions
            .with_transaction(&self.id, &self.owner, |tx| Ok(operation(tx)?))?)
    }

    pub fn get_document(
        &self,
        collection: &str,
        key: &str,
    ) -> Result<Option<Document>, EmbeddedError> {
        self.document_operation(|tx| tx.get_document(collection, key))
    }

    pub fn create_document(&self, doc: &Document) -> Result<bool, EmbeddedError> {
        self.document_operation(|tx| tx.create_document(doc))
    }

    pub fn set_document(&self, doc: &Document) -> Result<(), EmbeddedError> {
        self.document_operation(|tx| tx.set_document(doc))
    }

    pub fn update_document(&self, doc: &Document) -> Result<bool, EmbeddedError> {
        self.document_operation(|tx| {
            if tx.get_document(doc.collection(), doc.key())?.is_none() {
                return Ok(false);
            }
            tx.set_document(doc)?;
            Ok(true)
        })
    }

    pub fn delete_document(&self, collection: &str, key: &str) -> Result<bool, EmbeddedError> {
        self.document_operation(|tx| tx.delete_document(collection, key))
    }

    pub fn list_documents(
        &self,
        collection: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Document>, Option<String>), EmbeddedError> {
        self.document_operation(|tx| tx.list_documents_paged(collection, after, limit))
    }

    /// Atomically publish buffered writes. Call database.sync() for an explicit
    /// durability checkpoint. On failure this handle is dropped and rolled back.
    pub fn commit(mut self) -> Result<(), EmbeddedError> {
        self.sessions.commit(&self.id, &self.owner)?;
        self.finished = true;
        Ok(())
    }

    pub fn rollback(mut self) -> Result<(), EmbeddedError> {
        self.sessions.rollback(&self.id, &self.owner)?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for EmbeddedTransaction {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.sessions.rollback(&self.id, &self.owner);
        }
    }
}

fn context(ctx: ExecuteContext<'_>, options: EmbeddedQueryOptions) -> ExecuteContext<'_> {
    let ctx = ctx.with_params(options.params);
    if options.timeout.is_some() {
        ctx.with_explicit_query_timeout(options.timeout)
    } else {
        ctx
    }
}

fn run(
    storage: Arc<Database>,
    sql: &str,
    ctx: ExecuteContext<'_>,
    single: bool,
    managed: bool,
) -> Result<BatchResult, EmbeddedError> {
    let parsed = Query::parse(sql).map_err(QueryError::from)?;
    if single && parsed.statements.len() != 1 {
        return Err(EmbeddedError::ExpectedSingleResult {
            actual: parsed.statements.len(),
        });
    }
    if managed
        && parsed.statements.iter().any(|stmt| {
            matches!(
                stmt,
                Statement::Begin(_) | Statement::Commit(_) | Statement::Rollback(_)
            )
        })
    {
        return Err(EmbeddedError::TransactionControl);
    }
    let mut batch = parsed
        .bind(storage)
        .map_err(QueryError::from)?
        .execute(&ctx)
        .map_err(QueryError::from)?;
    if let Some(error) = batch.error.take() {
        return Err(EmbeddedError::Batch {
            error,
            results: batch.results,
            completed: batch.completed,
        });
    }
    Ok(batch)
}

fn single_rows(mut batch: BatchResult) -> Vec<Row> {
    // Statement count is validated before execution, including mutating statements.
    batch.results.remove(0).rows
}
