// src/query/builder.rs

use std::collections::HashMap;
use std::sync::Arc;

use crate::Database;
use crate::document::Value;
use crate::query::ast::Statement;
use crate::query::result::BatchResult;
use crate::query::{ExecuteError, ParseError, parse};
use crate::schema::BindError;
use crate::session::SessionManager;

#[path = "builder/authz.rs"]
mod authz;
#[path = "builder/result.rs"]
mod batch_result;
#[path = "builder/direct.rs"]
mod direct;
#[path = "builder/planned.rs"]
mod planned;
#[path = "builder/state.rs"]
mod state;
#[cfg(test)]
use authz::statement_to_action;

/// Alias for ergonomic API
pub type Query = ParsedQuery;

/// Parsed but not yet bound statements
pub struct ParsedQuery {
    pub(crate) statements: Vec<Statement>,
}

/// Bound and validated statements ready for execution
pub struct BoundQuery {
    pub(crate) statements: Vec<Statement>,
    pub(crate) storage: Arc<Database>,
}

/// Execution context (session, variables)
pub struct ExecuteContext<'a> {
    pub(crate) sessions: Option<&'a SessionManager>,
    pub(crate) session_id: Option<String>,
    pub(crate) query_timeout: Option<std::time::Duration>,
    pub(crate) params: HashMap<String, Value>,
    pub(crate) query_timeout_explicit: bool,
    pub(crate) execution_started_at: std::time::Instant,
    pub(crate) server_db_deadline: Option<std::time::Instant>,
    pub(crate) request_deadline: Option<std::time::Instant>,
    pub(crate) database: Option<String>,
    pub(crate) authorization: Option<&'a dyn crate::auth::AuthorizationProvider>,
    #[cfg(feature = "auth")]
    pub(crate) auth: Option<&'a crate::auth::AuthService>,
    pub(crate) subject: Option<crate::auth::Subject>,
    pub(crate) principal_identity: Option<String>,
}

impl ParsedQuery {
    /// Parse SQL input into statements
    pub fn parse(input: &str) -> Result<Self, ParseError> {
        let statements = parse::parse(input)?;
        if statements.is_empty() {
            return Err(ParseError::EmptyInput);
        }
        Ok(Self { statements })
    }

    /// Whether this batch contains DDL, whose blocking storage work cannot be
    /// safely interrupted once it has started.
    pub fn contains_ddl(&self) -> bool {
        self.statements.iter().any(is_ddl)
    }

    /// Return the CREATE INDEX payload when this request contains exactly one
    /// statement. The HTTP server uses this to route potentially expensive
    /// index builds through the durable DDL job runner without changing the
    /// synchronous embedded API.
    #[cfg(feature = "server")]
    pub fn single_create_index(&self) -> Option<crate::query::ast::CreateIndexAst> {
        match self.statements.as_slice() {
            [Statement::CreateIndex(ast)] => Some(ast.clone()),
            _ => None,
        }
    }

    /// Bind statements against schema, apply defaults, validate types
    ///
    /// Note: Binding happens just-in-time during execution to support
    /// multi-statement batches where later statements depend on earlier ones
    /// (e.g., DEFINE COLLECTION followed by INSERT).
    pub fn bind(self, storage: Arc<Database>) -> Result<BoundQuery, BindError> {
        // Don't bind upfront - bind JIT during execute to support multi-statement batches
        // where later statements depend on earlier ones (e.g., DEFINE COLLECTION + INSERT)
        Ok(BoundQuery {
            statements: self.statements,
            storage,
        })
    }
}

impl ExecuteContext<'_> {
    /// Create context without session (for tests, simple queries)
    pub fn none() -> ExecuteContext<'static> {
        ExecuteContext {
            sessions: None,
            session_id: None,
            params: HashMap::new(),
            query_timeout: None,
            query_timeout_explicit: false,
            execution_started_at: std::time::Instant::now(),
            server_db_deadline: None,
            request_deadline: None,
            database: None,
            authorization: None,
            #[cfg(feature = "auth")]
            auth: None,
            subject: None,
            principal_identity: None,
        }
    }
}

impl<'a> ExecuteContext<'a> {
    /// Create context with session manager and optional session ID
    pub fn with_session(sessions: &'a SessionManager, id: Option<&str>) -> Self {
        ExecuteContext {
            sessions: Some(sessions),
            session_id: id.map(String::from),
            params: HashMap::new(),
            query_timeout: None,
            query_timeout_explicit: false,
            execution_started_at: std::time::Instant::now(),
            server_db_deadline: None,
            request_deadline: None,
            database: None,
            authorization: None,
            #[cfg(feature = "auth")]
            auth: None,
            subject: None,
            principal_identity: None,
        }
    }

    /// Set typed parameters available as `$name` in each executed batch.
    ///
    /// Map keys are variable names without the leading `$`. Values are passed
    /// directly to the evaluator, never interpolated into SQL. Calling this
    /// method replaces any previously configured parameters.
    ///
    /// Each batch starts with a fresh copy of these values. `LET` can read a
    /// parameter and then shadow it for the remainder of its scope, without
    /// changing this context or the parameters seen by a subsequent execution.
    pub fn with_params(mut self, params: HashMap<String, Value>) -> Self {
        self.params = params;
        self
    }

    /// Set query timeout on context (builder pattern)
    pub fn with_query_timeout(mut self, timeout: Option<std::time::Duration>) -> Self {
        self.query_timeout = timeout;
        self
    }

    /// Mark a transport-provided timeout as explicit. It takes priority over a
    /// stored session timeout, while the server DB deadline remains a hard cap.
    pub fn with_explicit_query_timeout(mut self, timeout: Option<std::time::Duration>) -> Self {
        self.query_timeout = timeout;
        self.query_timeout_explicit = true;
        self
    }

    /// Set absolute request/batch timing bounds supplied by the server.
    pub fn with_server_db_deadline(
        mut self,
        started_at: std::time::Instant,
        deadline: std::time::Instant,
    ) -> Self {
        self.execution_started_at = started_at;
        self.server_db_deadline = Some(deadline);
        self
    }

    /// Set the transport request deadline. Unlike the query timeout this is an
    /// outer RPC/request lifetime cap, but it participates in cooperative
    /// cancellation so dropped async handlers cannot leave DB work running.
    pub fn with_request_deadline(mut self, deadline: Option<std::time::Instant>) -> Self {
        self.request_deadline = deadline;
        self
    }

    /// Set the target database identity on context (builder pattern)
    pub fn with_database(mut self, database: impl Into<String>) -> Self {
        self.database = Some(database.into());
        self
    }

    /// Set the authorization provider used by the core query engine.
    pub fn with_authorization(
        mut self,
        authorization: Option<&'a dyn crate::auth::AuthorizationProvider>,
    ) -> Self {
        self.authorization = authorization;
        self
    }

    /// Set the full authentication service used by server and auth-management queries.
    #[cfg(feature = "auth")]
    pub fn with_auth(mut self, auth: Option<&'a crate::auth::AuthService>) -> Self {
        self.authorization = auth.map(|service| service as &dyn crate::auth::AuthorizationProvider);
        self.auth = auth;
        self
    }

    /// Set authenticated subject on context (builder pattern)
    pub fn with_subject(mut self, subject: Option<crate::auth::Subject>) -> Self {
        self.subject = subject;
        self.principal_identity = None;
        self
    }

    #[cfg(feature = "server")]
    pub(crate) fn with_resolved_subject(
        mut self,
        subject: Option<crate::auth::ResolvedSubject>,
    ) -> Self {
        self.principal_identity = subject
            .as_ref()
            .map(|resolved| resolved.principal_identity().to_string());
        self.subject = subject.map(crate::auth::ResolvedSubject::into_subject);
        self
    }

    pub(crate) fn session_owner(&self) -> crate::session::SessionOwner {
        let database = self.database.clone().unwrap_or_default();
        match &self.subject {
            Some(subject) => crate::session::SessionOwner::authenticated(
                self.principal_identity
                    .clone()
                    .unwrap_or_else(|| subject.user_id.clone()),
                database,
            ),
            None => crate::session::SessionOwner::anonymous(database),
        }
    }
}

pub(crate) fn map_session_access_error(error: crate::session::SessionAccessError) -> ExecuteError {
    match error {
        crate::session::SessionAccessError::PrincipalMismatch => ExecuteError::SessionAccessDenied,
        crate::session::SessionAccessError::DatabaseMismatch => {
            ExecuteError::SessionDatabaseMismatch
        }
        crate::session::SessionAccessError::Expired => ExecuteError::SessionExpired,
    }
}

pub(crate) fn map_session_operation_error(
    error: crate::session::SessionOperationError<ExecuteError>,
) -> ExecuteError {
    match error {
        crate::session::SessionOperationError::Access(error) => map_session_access_error(error),
        crate::session::SessionOperationError::Transaction(error) => {
            map_session_error(crate::session::SessionError::Transaction(error))
        }
        crate::session::SessionOperationError::Operation(error) => error,
    }
}

pub(crate) fn map_session_error(error: crate::session::SessionError) -> ExecuteError {
    match error {
        crate::session::SessionError::Access(error) => map_session_access_error(error),
        crate::session::SessionError::Transaction(error) => match error {
            crate::transaction::TransactionError::Conflict => ExecuteError::TransactionConflict,
            crate::transaction::TransactionError::Timeout => {
                ExecuteError::QueryTimeout("transaction deadline exceeded".to_string())
            }
            crate::transaction::TransactionError::NoActiveTransaction => {
                ExecuteError::TransactionNotSupported("No active transaction")
            }
            crate::transaction::TransactionError::AlreadyCompleted => {
                ExecuteError::TransactionNotSupported("Transaction already completed")
            }
            crate::transaction::TransactionError::Storage(error) => ExecuteError::Storage(error),
            crate::transaction::TransactionError::Internal(error) => ExecuteError::Internal(error),
        },
    }
}

impl BoundQuery {
    /// Execute all statements in the batch
    pub fn execute(self, ctx: &ExecuteContext<'_>) -> Result<BatchResult, ExecuteError> {
        execute_batch(self.statements, self.storage, ctx)
    }
}

fn is_ddl(stmt: &Statement) -> bool {
    matches!(
        stmt,
        Statement::DefineCollection(_)
            | Statement::DropCollection(_)
            | Statement::Describe(_)
            | Statement::CreateIndex(_)
            | Statement::DropIndex(_)
            | Statement::Reindex(_)
            | Statement::DefineAnalyzer(_)
            | Statement::DropAnalyzer(_)
            | Statement::CreateUser(_)
            | Statement::DropUser(_)
            | Statement::AlterUser(_)
            | Statement::CreateApiKey(_)
            | Statement::DropApiKey(_)
            | Statement::CreatePolicy(_)
            | Statement::DropPolicy(_)
            | Statement::AlterCollectionAttrs(_)
    )
}

fn auto_rollback(batch_tx: &Option<String>, ctx: &ExecuteContext<'_>) {
    if let Some(tx_id) = batch_tx
        && let Some(sessions) = ctx.sessions
    {
        let _ = sessions.rollback(tx_id, &ctx.session_owner());
    }
}

fn execute_batch(
    statements: Vec<Statement>,
    storage: Arc<Database>,
    ctx: &ExecuteContext<'_>,
) -> Result<BatchResult, ExecuteError> {
    let db: &Database = &storage;
    let mut state = state::BatchState::new(ctx.params.clone());
    let session_owner = ctx.session_owner();

    if let (Some(sessions), Some(session_id)) = (ctx.sessions, ctx.session_id.as_ref()) {
        sessions
            .validate_access(session_id, &session_owner)
            .map_err(map_session_access_error)?;
    }

    for (index, stmt) in statements.into_iter().enumerate() {
        // --- Per-statement authorization ---
        if let Err(message) = authz::authorize_statement(
            ctx.authorization,
            ctx.subject.as_ref(),
            ctx.database.as_deref().unwrap_or_default(),
            &stmt,
        ) {
            auto_rollback(&state.batch_tx, ctx);
            let error = crate::query::result::BatchError::public(
                index,
                "SDB-AC002",
                crate::error::ErrorClass::Forbidden,
                message,
            );
            return Ok(batch_result::error_result(state.results, index, error));
        }

        // Determine active transaction (batch-level or external session)
        // An explicitly supplied session is an obligatory transaction context.
        // Never discard it and fall back to autocommit if cleanup removes it.
        let active_tx_id = state.batch_tx.as_ref().or(ctx.session_id.as_ref()).cloned();

        let stmt = match direct::try_handle_tx_or_set(index, stmt, storage.clone(), ctx, &mut state)
        {
            Ok(direct::DirectOutcome::Handled) => continue,
            Ok(direct::DirectOutcome::NotHandled(stmt)) => *stmt,
            Ok(direct::DirectOutcome::Error(result)) => return Ok(result),
            Err(error) => {
                auto_rollback(&state.batch_tx, ctx);
                return Ok(batch_result::query_error_result(
                    std::mem::take(&mut state.results),
                    index,
                    error.into(),
                ));
            }
        };

        let deadline = execution_deadline(ctx, &state);
        if let Err(error) = crate::query::execute::operators::operator::check_deadline(deadline) {
            auto_rollback(&state.batch_tx, ctx);
            return Ok(batch_result::query_error_result(
                std::mem::take(&mut state.results),
                index,
                error.into(),
            ));
        }

        let stmt = match direct::try_handle_statement(
            index,
            stmt,
            db,
            active_tx_id.as_ref(),
            ctx,
            &mut state,
            deadline,
        ) {
            direct::DirectOutcome::Handled => continue,
            direct::DirectOutcome::NotHandled(stmt) => *stmt,
            direct::DirectOutcome::Error(result) => return Ok(result),
        };

        let (resolved_stmt, result) = match planned::execute_planned_statement(
            index,
            stmt,
            db,
            active_tx_id.as_ref(),
            ctx,
            &mut state,
            deadline,
        ) {
            Ok(value) => value,
            Err(result) => return Ok(result),
        };

        state.results.push(batch_result::make_statement_result(
            index,
            &resolved_stmt,
            result,
        ));
    }

    let completed = state.results.len();
    Ok(BatchResult {
        results: state.results,
        completed,
        error: None,
    })
}

fn execution_deadline(
    ctx: &ExecuteContext<'_>,
    state: &state::BatchState,
) -> crate::query::execute::operators::operator::QueryDeadline {
    let effective_timeout = if ctx.query_timeout_explicit {
        ctx.query_timeout
    } else {
        state.batch_query_timeout.unwrap_or_else(|| {
            ctx.sessions
                .and_then(|sessions| {
                    state
                        .batch_tx
                        .as_ref()
                        .or(ctx.session_id.as_ref())
                        .and_then(|id| {
                            sessions
                                .get_query_timeout(id, &ctx.session_owner())
                                .ok()
                                .flatten()
                        })
                })
                .or(ctx.query_timeout)
        })
    };
    use crate::query::execute::operators::operator::DeadlineSource;
    let query_deadline = effective_timeout.and_then(|duration| {
        ctx.execution_started_at
            .checked_add(duration)
            .map(|deadline| (deadline, DeadlineSource::QueryExecution))
    });
    [
        query_deadline,
        ctx.server_db_deadline
            .map(|deadline| (deadline, DeadlineSource::ServerDb)),
        ctx.request_deadline
            .map(|deadline| (deadline, DeadlineSource::Request)),
    ]
    .into_iter()
    .flatten()
    .min_by_key(|(deadline, _)| *deadline)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Action;
    use crate::query::ast::*;

    // ─── statement_to_action mapping tests ───

    fn make_target(coll: &str) -> Target {
        Target {
            collection: coll.to_string(),
            key: None,
        }
    }

    #[test]
    fn select_from_collection_maps_to_select_with_collection() {
        let stmt = Statement::Select(SelectAst {
            projection: Projection::All,
            distinct: false,
            value_mode: false,
            source: DataSource::Collection(make_target("users")),
            filter: None,
            group: None,
            order: None,
            limit: None,
            offset: None,
            fetch: None,
        });
        let (action, coll) = statement_to_action(&stmt).unwrap();
        assert_eq!(action, Action::Select);
        assert_eq!(coll, Some("users".to_string()));
    }

    #[test]
    fn select_from_field_path_maps_to_select_with_collection() {
        let stmt = Statement::Select(SelectAst {
            projection: Projection::All,
            distinct: false,
            value_mode: false,
            source: DataSource::FieldPath {
                target: make_target("orders"),
                path: vec!["items".to_string()],
                postfix: vec![],
            },
            filter: None,
            group: None,
            order: None,
            limit: None,
            offset: None,
            fetch: None,
        });
        let (action, coll) = statement_to_action(&stmt).unwrap();
        assert_eq!(action, Action::Select);
        assert_eq!(coll, Some("orders".to_string()));
    }

    #[test]
    fn select_from_expr_maps_to_select_no_collection() {
        let stmt = Statement::Select(SelectAst {
            projection: Projection::All,
            distinct: false,
            value_mode: false,
            source: DataSource::Expr(Box::new(Expr::Literal(crate::document::Value::Null))),
            filter: None,
            group: None,
            order: None,
            limit: None,
            offset: None,
            fetch: None,
        });
        let (action, coll) = statement_to_action(&stmt).unwrap();
        assert_eq!(action, Action::Select);
        assert_eq!(coll, None);
    }

    #[test]
    fn select_no_source_maps_to_select_no_collection() {
        let stmt = Statement::Select(SelectAst {
            projection: Projection::All,
            distinct: false,
            value_mode: false,
            source: DataSource::None,
            filter: None,
            group: None,
            order: None,
            limit: None,
            offset: None,
            fetch: None,
        });
        let (action, coll) = statement_to_action(&stmt).unwrap();
        assert_eq!(action, Action::Select);
        assert_eq!(coll, None);
    }

    #[test]
    fn insert_maps_to_insert_with_collection() {
        let stmt = Statement::Insert(InsertAst {
            collection: "users".to_string(),
            objects: vec![],
        });
        let (action, coll) = statement_to_action(&stmt).unwrap();
        assert_eq!(action, Action::Insert);
        assert_eq!(coll, Some("users".to_string()));
    }

    #[test]
    fn create_maps_to_insert_with_collection() {
        let stmt = Statement::Create(CreateAst {
            target: make_target("users"),
            assignments: vec![],
        });
        let (action, coll) = statement_to_action(&stmt).unwrap();
        assert_eq!(action, Action::Insert);
        assert_eq!(coll, Some("users".to_string()));
    }

    #[test]
    fn update_maps_to_update_with_collection() {
        let stmt = Statement::Update(UpdateAst {
            target: make_target("users"),
            filter: None,
            assignments: vec![],
        });
        let (action, coll) = statement_to_action(&stmt).unwrap();
        assert_eq!(action, Action::Update);
        assert_eq!(coll, Some("users".to_string()));
    }

    #[test]
    fn upsert_maps_to_update_with_collection() {
        let stmt = Statement::Upsert(UpsertAst {
            target: make_target("users"),
            assignments: vec![],
            replace: false,
        });
        let (action, coll) = statement_to_action(&stmt).unwrap();
        assert_eq!(action, Action::Update);
        assert_eq!(coll, Some("users".to_string()));
    }

    #[test]
    fn delete_document_maps_to_delete_with_collection() {
        let stmt = Statement::Delete(DeleteAst {
            target: DeleteTarget::Document(make_target("users")),
            filter: None,
        });
        let (action, coll) = statement_to_action(&stmt).unwrap();
        assert_eq!(action, Action::Delete);
        assert_eq!(coll, Some("users".to_string()));
    }

    #[test]
    fn delete_edge_maps_to_delete_with_from_collection() {
        let stmt = Statement::Delete(DeleteAst {
            target: DeleteTarget::Edge {
                from: make_target("user"),
                label: "follows".to_string(),
                to: Some(make_target("user")),
            },
            filter: None,
        });
        let (action, coll) = statement_to_action(&stmt).unwrap();
        assert_eq!(action, Action::Delete);
        assert_eq!(coll, Some("user".to_string()));
    }

    #[test]
    fn relate_maps_to_traverse() {
        let stmt = Statement::Relate(RelateAst {
            from: RelateSource::Record(make_target("user")),
            label: "follows".to_string(),
            to: RelateSource::Record(make_target("user")),
            data: None,
            return_clause: ReturnClause::default(),
        });
        let (action, coll) = statement_to_action(&stmt).unwrap();
        assert_eq!(action, Action::Traverse);
        assert_eq!(coll, None);
    }

    #[test]
    fn ddl_statements_map_to_create_or_drop() {
        let define = Statement::DefineCollection(DefineCollectionAst {
            name: "test".to_string(),
            mode: crate::schema::SchemaMode::Flexible,
            fields: vec![],
            indexes: vec![],
        });
        assert_eq!(statement_to_action(&define).unwrap().0, Action::Create);

        let drop = Statement::DropCollection(DropCollectionAst {
            name: "test".to_string(),
            cascade: false,
        });
        assert_eq!(statement_to_action(&drop).unwrap().0, Action::Drop);

        let create_idx = Statement::CreateIndex(CreateIndexAst {
            collection: "test".to_string(),
            fields: vec!["name".to_string()],
            unique: false,
            index_type: crate::schema::IndexType::BTree,
            hnsw_params: None,
            analyzer: None,
        });
        assert_eq!(statement_to_action(&create_idx).unwrap().0, Action::Create);

        let drop_idx = Statement::DropIndex(DropIndexAst {
            name: "idx".to_string(),
            collection: "test".to_string(),
        });
        assert_eq!(statement_to_action(&drop_idx).unwrap().0, Action::Drop);

        let reindex = Statement::Reindex(ReindexAst {
            name: "idx".to_string(),
            collection: "test".to_string(),
        });
        assert_eq!(statement_to_action(&reindex).unwrap().0, Action::Manage);
    }

    #[test]
    fn analyzer_statements_map_to_create_drop() {
        let define = Statement::DefineAnalyzer(DefineAnalyzerAst {
            name: "custom".to_string(),
            tokenizer: crate::schema::TokenizerConfig::Standard,
            filters: vec![],
        });
        assert_eq!(statement_to_action(&define).unwrap().0, Action::Create);

        let drop = Statement::DropAnalyzer(DropAnalyzerAst {
            name: "custom".to_string(),
        });
        assert_eq!(statement_to_action(&drop).unwrap().0, Action::Drop);
    }

    #[test]
    fn auth_management_statements_map_to_manage() {
        let stmts = vec![
            Statement::CreateUser(CreateUserAst {
                username: "alice".to_string(),
                password: "pass".to_string(),
            }),
            Statement::DropUser(DropUserAst {
                username: "alice".to_string(),
            }),
            Statement::AlterUser(AlterUserAst::Password {
                username: "alice".to_string(),
                password: "new".to_string(),
            }),
            Statement::CreateApiKey(CreateApiKeyAst {
                name: "key1".to_string(),
                username: "alice".to_string(),
            }),
            Statement::DropApiKey(DropApiKeyAst {
                name: "key1".to_string(),
            }),
            Statement::CreatePolicy(CreatePolicyAst {
                name: "pol".to_string(),
                conditions: vec![],
                effect: PolicyEffectAst::Allow,
            }),
            Statement::DropPolicy(DropPolicyAst {
                name: "pol".to_string(),
            }),
            Statement::AlterCollectionAttrs(AlterCollectionAttrsAst::Set {
                collection: "test".to_string(),
                attributes: vec![],
            }),
        ];
        for stmt in stmts {
            let (action, coll) = statement_to_action(&stmt)
                .unwrap_or_else(|| panic!("Expected Some for {:?}", stmt));
            assert_eq!(action, Action::Manage, "Failed for {:?}", stmt);
            assert_eq!(coll, None);
        }
    }

    #[test]
    fn control_flow_statements_return_none() {
        let stmts: Vec<Statement> = vec![
            Statement::Begin(BeginAst),
            Statement::Commit(CommitAst),
            Statement::Rollback(RollbackAst),
            Statement::Describe(DescribeAst::Collections),
            Statement::Let(LetAst {
                name: "x".to_string(),
                expr: Expr::Literal(crate::document::Value::Null),
            }),
            Statement::Set(SetAst {
                key: "timeout".to_string(),
                value_nanos: 0,
            }),
            Statement::Expr(Expr::Literal(crate::document::Value::Int(1))),
        ];
        for stmt in stmts {
            assert!(
                statement_to_action(&stmt).is_none(),
                "Expected None for {:?}",
                stmt
            );
        }
    }

    // ─── Authorization enforcement tests ───

    #[test]
    fn no_auth_no_subject_allows_all_statements() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());
        let ctx = ExecuteContext::none();
        // SELECT 1 should succeed without auth
        let result = ParsedQuery::parse("SELECT 1")
            .unwrap()
            .bind(db)
            .unwrap()
            .execute(&ctx)
            .unwrap();
        assert!(result.error.is_none());
        assert_eq!(result.completed, 1);
    }

    #[test]
    fn custom_authorization_provider_does_not_require_auth_service() {
        struct DenyAll;

        impl crate::auth::AuthorizationProvider for DenyAll {
            fn authorize(
                &self,
                _subject: &crate::auth::Subject,
                _action: crate::auth::Action,
                _resource: &crate::auth::Resource,
            ) -> crate::auth::AuthzDecision {
                crate::auth::AuthzDecision::Denied {
                    policy: Some("custom-deny".to_string()),
                }
            }
        }

        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());
        let provider = DenyAll;
        let subject = crate::auth::Subject {
            user_id: "embedded-user".to_string(),
            attributes: std::collections::HashMap::new(),
        };
        let ctx = ExecuteContext::none()
            .with_authorization(Some(&provider))
            .with_subject(Some(subject));

        let result = ParsedQuery::parse("SELECT 1")
            .unwrap()
            .bind(db)
            .unwrap()
            .execute(&ctx)
            .unwrap();
        let error = result.error.expect("custom provider must be enforced");
        assert_eq!(error.code, "SDB-AC002");
        assert!(error.message.contains("custom-deny"));
    }

    #[cfg(not(feature = "auth"))]
    #[test]
    fn auth_management_statement_reports_missing_build_capability() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());

        let result = ParsedQuery::parse("CREATE USER 'alice' PASSWORD 'secret'")
            .unwrap()
            .bind(db)
            .unwrap()
            .execute(&ExecuteContext::none())
            .unwrap();
        let error = result.error.expect("auth DDL must not run without auth");
        assert!(error.message.contains("--features auth"));
    }

    #[test]
    #[cfg(feature = "auth")]
    fn auth_with_no_subject_denies_authorized_statements() {
        let tmp_sys = tempfile::tempdir().unwrap();
        let system_db = Arc::new(Database::open(tmp_sys.path()).unwrap());
        let (auth_service, _) =
            crate::auth::AuthService::init(system_db, b"test-secret-32-bytes-minimum!!", 3600)
                .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());

        let ctx = ExecuteContext::none().with_auth(Some(&auth_service));
        // Once auth is configured, authorization-relevant statements fail closed.
        let result = ParsedQuery::parse("SELECT 1")
            .unwrap()
            .bind(db)
            .unwrap()
            .execute(&ctx)
            .unwrap();
        let error = result.error.expect("missing subject must be denied");
        assert_eq!(error.code, "SDB-AC002");
        assert!(error.message.contains("authenticated subject required"));
    }

    #[test]
    #[cfg(feature = "auth")]
    fn auth_with_subject_denied_by_default_deny() {
        let tmp_sys = tempfile::tempdir().unwrap();
        let system_db = Arc::new(Database::open(tmp_sys.path()).unwrap());
        let (auth_service, _) =
            crate::auth::AuthService::init(system_db, b"test-secret-32-bytes-minimum!!", 3600)
                .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());

        let subject = crate::auth::Subject {
            user_id: "alice".to_string(),
            attributes: std::collections::HashMap::new(),
        };

        let ctx = ExecuteContext::none()
            .with_auth(Some(&auth_service))
            .with_subject(Some(subject));

        // Non-root user, no policies → default deny for SELECT (which needs authorization)
        let result = ParsedQuery::parse("SELECT * FROM users")
            .unwrap()
            .bind(db)
            .unwrap()
            .execute(&ctx)
            .unwrap();
        assert!(result.error.is_some());
        assert!(result.error.unwrap().message.contains("Access denied"));
    }

    #[test]
    #[cfg(feature = "auth")]
    fn root_user_bypasses_all_policies() {
        let tmp_sys = tempfile::tempdir().unwrap();
        let system_db = Arc::new(Database::open(tmp_sys.path()).unwrap());
        let (auth_service, _) =
            crate::auth::AuthService::init(system_db, b"test-secret-32-bytes-minimum!!", 3600)
                .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());

        let subject = crate::auth::Subject {
            user_id: "root".to_string(),
            attributes: std::collections::HashMap::new(),
        };

        let ctx = ExecuteContext::none()
            .with_auth(Some(&auth_service))
            .with_subject(Some(subject));

        // Root bypasses all → SELECT 1 succeeds
        let result = ParsedQuery::parse("SELECT 1")
            .unwrap()
            .bind(db)
            .unwrap()
            .execute(&ctx)
            .unwrap();
        assert!(result.error.is_none());
    }

    #[test]
    #[cfg(feature = "auth")]
    fn policy_allows_matching_action() {
        let tmp_sys = tempfile::tempdir().unwrap();
        let system_db = Arc::new(Database::open(tmp_sys.path()).unwrap());
        let (auth_service, _) =
            crate::auth::AuthService::init(system_db, b"test-secret-32-bytes-minimum!!", 3600)
                .unwrap();

        // Create a policy that allows SELECT for department=analytics
        auth_service
            .create_policy(&crate::auth::Policy {
                name: "analytics_read".to_string(),
                conditions: vec![
                    crate::auth::Condition {
                        field: "subject.department".to_string(),
                        op: crate::auth::ConditionOp::Eq,
                        value: crate::auth::ConditionValue::Single("analytics".to_string()),
                    },
                    crate::auth::Condition {
                        field: "action".to_string(),
                        op: crate::auth::ConditionOp::Eq,
                        value: crate::auth::ConditionValue::Single("SELECT".to_string()),
                    },
                ],
                effect: crate::auth::Effect::Allow,
            })
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());

        let subject = crate::auth::Subject {
            user_id: "alice".to_string(),
            attributes: std::collections::HashMap::from([(
                "department".to_string(),
                "analytics".to_string(),
            )]),
        };

        let ctx = ExecuteContext::none()
            .with_auth(Some(&auth_service))
            .with_subject(Some(subject));

        // SELECT allowed
        let result = ParsedQuery::parse("SELECT 1")
            .unwrap()
            .bind(db.clone())
            .unwrap()
            .execute(&ctx)
            .unwrap();
        assert!(result.error.is_none(), "SELECT should be allowed");

        // INSERT denied (no policy for INSERT)
        let result = ParsedQuery::parse("INSERT INTO test {name: 'a'}")
            .unwrap()
            .bind(db)
            .unwrap()
            .execute(&ctx)
            .unwrap();
        assert!(result.error.is_some(), "INSERT should be denied");
        assert!(result.error.unwrap().message.contains("Access denied"));
    }

    #[test]
    #[cfg(feature = "auth")]
    fn deny_policy_overrides_allow() {
        let tmp_sys = tempfile::tempdir().unwrap();
        let system_db = Arc::new(Database::open(tmp_sys.path()).unwrap());
        let (auth_service, _) =
            crate::auth::AuthService::init(system_db, b"test-secret-32-bytes-minimum!!", 3600)
                .unwrap();

        // Allow all SELECTs
        auth_service
            .create_policy(&crate::auth::Policy {
                name: "allow_select".to_string(),
                conditions: vec![crate::auth::Condition {
                    field: "action".to_string(),
                    op: crate::auth::ConditionOp::Eq,
                    value: crate::auth::ConditionValue::Single("SELECT".to_string()),
                }],
                effect: crate::auth::Effect::Allow,
            })
            .unwrap();

        // But deny for interns
        auth_service
            .create_policy(&crate::auth::Policy {
                name: "deny_interns".to_string(),
                conditions: vec![
                    crate::auth::Condition {
                        field: "action".to_string(),
                        op: crate::auth::ConditionOp::Eq,
                        value: crate::auth::ConditionValue::Single("SELECT".to_string()),
                    },
                    crate::auth::Condition {
                        field: "subject.role".to_string(),
                        op: crate::auth::ConditionOp::Eq,
                        value: crate::auth::ConditionValue::Single("intern".to_string()),
                    },
                ],
                effect: crate::auth::Effect::Deny,
            })
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());

        // Regular user allowed
        let regular = crate::auth::Subject {
            user_id: "bob".to_string(),
            attributes: std::collections::HashMap::from([(
                "role".to_string(),
                "engineer".to_string(),
            )]),
        };
        let ctx = ExecuteContext::none()
            .with_auth(Some(&auth_service))
            .with_subject(Some(regular));

        let result = ParsedQuery::parse("SELECT 1")
            .unwrap()
            .bind(db.clone())
            .unwrap()
            .execute(&ctx)
            .unwrap();
        assert!(result.error.is_none(), "engineer should be allowed");

        // Intern denied (deny wins)
        let intern = crate::auth::Subject {
            user_id: "charlie".to_string(),
            attributes: std::collections::HashMap::from([(
                "role".to_string(),
                "intern".to_string(),
            )]),
        };
        let ctx = ExecuteContext::none()
            .with_auth(Some(&auth_service))
            .with_subject(Some(intern));

        let result = ParsedQuery::parse("SELECT 1")
            .unwrap()
            .bind(db)
            .unwrap()
            .execute(&ctx)
            .unwrap();
        assert!(result.error.is_some(), "intern should be denied");
        assert!(
            result.error.unwrap().message.contains("deny_interns"),
            "should reference denying policy name"
        );
    }

    #[test]
    #[cfg(feature = "auth")]
    fn describe_and_explain_skip_authorization() {
        let tmp_sys = tempfile::tempdir().unwrap();
        let system_db = Arc::new(Database::open(tmp_sys.path()).unwrap());
        let (auth_service, _) =
            crate::auth::AuthService::init(system_db, b"test-secret-32-bytes-minimum!!", 3600)
                .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());

        // Non-root with no policies — but DESCRIBE shouldn't need authorization
        let subject = crate::auth::Subject {
            user_id: "alice".to_string(),
            attributes: std::collections::HashMap::new(),
        };

        let ctx = ExecuteContext::none()
            .with_auth(Some(&auth_service))
            .with_subject(Some(subject));

        let result = ParsedQuery::parse("DESCRIBE COLLECTIONS")
            .unwrap()
            .bind(db)
            .unwrap()
            .execute(&ctx)
            .unwrap();
        assert!(result.error.is_none(), "DESCRIBE should skip auth check");
    }

    #[test]
    #[cfg(feature = "auth")]
    fn batch_stops_at_first_denied_statement() {
        let tmp_sys = tempfile::tempdir().unwrap();
        let system_db = Arc::new(Database::open(tmp_sys.path()).unwrap());
        let (auth_service, _) =
            crate::auth::AuthService::init(system_db, b"test-secret-32-bytes-minimum!!", 3600)
                .unwrap();

        // Allow only SELECT
        auth_service
            .create_policy(&crate::auth::Policy {
                name: "select_only".to_string(),
                conditions: vec![crate::auth::Condition {
                    field: "action".to_string(),
                    op: crate::auth::ConditionOp::Eq,
                    value: crate::auth::ConditionValue::Single("SELECT".to_string()),
                }],
                effect: crate::auth::Effect::Allow,
            })
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());

        let subject = crate::auth::Subject {
            user_id: "alice".to_string(),
            attributes: std::collections::HashMap::new(),
        };
        let ctx = ExecuteContext::none()
            .with_auth(Some(&auth_service))
            .with_subject(Some(subject));

        // Batch: SELECT succeeds, INSERT fails at authorization
        let result = ParsedQuery::parse("SELECT 1; INSERT INTO test {name: 'a'}")
            .unwrap()
            .bind(db)
            .unwrap()
            .execute(&ctx)
            .unwrap();

        assert_eq!(result.completed, 1, "should stop after first statement");
        assert_eq!(result.results.len(), 1, "should have 1 successful result");
        assert!(result.error.is_some());
        assert_eq!(result.error.as_ref().unwrap().statement_index, 1);
        assert!(result.error.unwrap().message.contains("Access denied"));
    }

    // ─── ALTER COLLECTION attributes tests ───

    #[test]
    #[cfg(feature = "auth")]
    fn alter_collection_set_attributes_via_sql() {
        let tmp_sys = tempfile::tempdir().unwrap();
        let system_db = Arc::new(Database::open(tmp_sys.path()).unwrap());
        let (auth_service, _) =
            crate::auth::AuthService::init(system_db, b"test-secret-32-bytes-minimum!!", 3600)
                .unwrap();

        // Allow Manage for root
        let subject = crate::auth::Subject {
            user_id: "root".to_string(),
            attributes: std::collections::HashMap::new(),
        };

        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());

        let ctx = ExecuteContext::none()
            .with_auth(Some(&auth_service))
            .with_subject(Some(subject));

        // SET attributes
        let result =
            ParsedQuery::parse("ALTER COLLECTION users SET sensitivity = 'pii', team = 'backend'")
                .unwrap()
                .bind(db.clone())
                .unwrap()
                .execute(&ctx)
                .unwrap();
        assert!(
            result.error.is_none(),
            "ALTER COLLECTION SET should succeed"
        );
        assert_eq!(result.completed, 1);

        // Verify the attributes were stored
        let attrs = auth_service.get_collection_attributes("", "users");
        assert_eq!(attrs.get("sensitivity"), Some(&"pii".to_string()));
        assert_eq!(attrs.get("team"), Some(&"backend".to_string()));
    }

    #[test]
    #[cfg(feature = "auth")]
    fn alter_collection_remove_attributes_via_sql() {
        let tmp_sys = tempfile::tempdir().unwrap();
        let system_db = Arc::new(Database::open(tmp_sys.path()).unwrap());
        let (auth_service, _) =
            crate::auth::AuthService::init(system_db, b"test-secret-32-bytes-minimum!!", 3600)
                .unwrap();

        // Pre-set some attributes
        auth_service
            .set_collection_attributes(
                "",
                "users",
                std::collections::HashMap::from([
                    ("sensitivity".to_string(), "pii".to_string()),
                    ("team".to_string(), "backend".to_string()),
                ]),
            )
            .unwrap();

        let subject = crate::auth::Subject {
            user_id: "root".to_string(),
            attributes: std::collections::HashMap::new(),
        };

        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());

        let ctx = ExecuteContext::none()
            .with_auth(Some(&auth_service))
            .with_subject(Some(subject));

        // REMOVE attributes
        let result = ParsedQuery::parse("ALTER COLLECTION users REMOVE sensitivity")
            .unwrap()
            .bind(db)
            .unwrap()
            .execute(&ctx)
            .unwrap();
        assert!(
            result.error.is_none(),
            "ALTER COLLECTION REMOVE should succeed"
        );

        // Verify sensitivity was removed but team remains
        let attrs = auth_service.get_collection_attributes("", "users");
        assert_eq!(attrs.get("sensitivity"), None);
        assert_eq!(attrs.get("team"), Some(&"backend".to_string()));
    }

    #[test]
    #[cfg(feature = "auth")]
    fn collection_attributes_used_in_authorization() {
        let tmp_sys = tempfile::tempdir().unwrap();
        let system_db = Arc::new(Database::open(tmp_sys.path()).unwrap());
        let (auth_service, _) =
            crate::auth::AuthService::init(system_db, b"test-secret-32-bytes-minimum!!", 3600)
                .unwrap();

        // Set collection attribute: users.sensitivity = pii
        auth_service
            .set_collection_attributes(
                "",
                "users",
                std::collections::HashMap::from([("sensitivity".to_string(), "pii".to_string())]),
            )
            .unwrap();

        // Create an allow-all-SELECT policy
        auth_service
            .create_policy(&crate::auth::Policy {
                name: "allow_select".to_string(),
                conditions: vec![crate::auth::Condition {
                    field: "action".to_string(),
                    op: crate::auth::ConditionOp::Eq,
                    value: crate::auth::ConditionValue::Single("SELECT".to_string()),
                }],
                effect: crate::auth::Effect::Allow,
            })
            .unwrap();

        // Create a deny policy for resources with sensitivity=pii
        auth_service
            .create_policy(&crate::auth::Policy {
                name: "deny_pii".to_string(),
                conditions: vec![
                    crate::auth::Condition {
                        field: "action".to_string(),
                        op: crate::auth::ConditionOp::Eq,
                        value: crate::auth::ConditionValue::Single("SELECT".to_string()),
                    },
                    crate::auth::Condition {
                        field: "resource.sensitivity".to_string(),
                        op: crate::auth::ConditionOp::Eq,
                        value: crate::auth::ConditionValue::Single("pii".to_string()),
                    },
                ],
                effect: crate::auth::Effect::Deny,
            })
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());

        let subject = crate::auth::Subject {
            user_id: "alice".to_string(),
            attributes: std::collections::HashMap::new(),
        };

        let ctx = ExecuteContext::none()
            .with_auth(Some(&auth_service))
            .with_subject(Some(subject));

        // SELECT from users should be denied because resource.sensitivity=pii
        let result = ParsedQuery::parse("SELECT * FROM users")
            .unwrap()
            .bind(db.clone())
            .unwrap()
            .execute(&ctx)
            .unwrap();
        assert!(
            result.error.is_some(),
            "SELECT from pii collection should be denied"
        );
        assert!(
            result.error.as_ref().unwrap().message.contains("deny_pii"),
            "Should reference the deny_pii policy, got: {}",
            result.error.as_ref().unwrap().message
        );

        // SELECT 1 (no collection) should succeed since no collection attributes apply
        let result = ParsedQuery::parse("SELECT 1")
            .unwrap()
            .bind(db)
            .unwrap()
            .execute(&ctx)
            .unwrap();
        assert!(
            result.error.is_none(),
            "SELECT without collection should be allowed"
        );
    }

    fn execute_test_batch(db: &Arc<Database>, sql: &str, ctx: &ExecuteContext<'_>) -> BatchResult {
        Query::parse(sql)
            .unwrap()
            .bind(db.clone())
            .unwrap()
            .execute(ctx)
            .unwrap()
    }

    #[test]
    fn typed_parameters_preserve_strings_and_nested_arrays() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());
        let text = Value::String("quotes '\"; LET injected = 1; -- \n Unicode: λ".into());
        let items = Value::Array(vec![
            text.clone(),
            Value::Int(42),
            Value::Bool(true),
            Value::Null,
            Value::Array(vec![Value::Int(7)]),
        ]);
        let ctx = ExecuteContext::none().with_params(HashMap::from([
            ("text".into(), text.clone()),
            ("items".into(), items.clone()),
        ]));
        let result = execute_test_batch(&db, "SELECT VALUE $text; SELECT VALUE $items", &ctx);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.completed, 2);
        assert_eq!(result.results[0].rows[0].scalar_value, Some(text));
        assert_eq!(result.results[1].rows[0].scalar_value, Some(items));
    }

    #[test]
    fn parameters_allow_let_shadowing_without_leaking_between_batches() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());
        let ctx = ExecuteContext::none().with_params(HashMap::from([("n".into(), Value::Int(4))]));
        let result = execute_test_batch(
            &db,
            "LET n = $n + 1; IF true THEN LET n = 9; $n END; $n; LET local = 12",
            &ctx,
        );
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.results[1].rows[0].scalar_value, Some(Value::Int(9)));
        assert_eq!(result.results[2].rows[0].scalar_value, Some(Value::Int(5)));

        let next = execute_test_batch(&db, "$n; $local", &ctx);
        assert_eq!(next.results[0].rows[0].scalar_value, Some(Value::Int(4)));
        assert_eq!(next.completed, 1);
        assert_eq!(next.error.unwrap().code, "SDB-QE026");

        let independent = execute_test_batch(&db, "$n", &ExecuteContext::none());
        assert_eq!(independent.completed, 0);
        assert_eq!(independent.error.unwrap().code, "SDB-QE026");
    }

    #[test]
    fn missing_parameter_reports_undefined_variable_and_stops_batch() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());
        let ctx =
            ExecuteContext::none().with_params(HashMap::from([("present".into(), Value::Null)]));
        let result = execute_test_batch(
            &db,
            "SELECT VALUE $present; SELECT VALUE $missing; SELECT VALUE 42",
            &ctx,
        );
        assert_eq!(result.results[0].rows[0].scalar_value, Some(Value::Null));
        assert_eq!(result.completed, 1);
        let error = result.error.unwrap();
        assert_eq!(error.statement_index, 1);
        assert_eq!(error.code, "SDB-QE026");
    }

    #[test]
    fn transaction_subqueries_let_and_control_flow_share_transaction_context() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());
        let setup = execute_test_batch(
            &db,
            "DEFINE COLLECTION item (n int)",
            &ExecuteContext::none(),
        );
        assert!(setup.error.is_none(), "{:?}", setup.error);
        let sessions = SessionManager::new(crate::session::SessionConfig::default());
        let owner = crate::session::SessionOwner::anonymous("");
        let session_id = sessions.begin_transaction(db.clone(), owner.clone());
        let ctx = ExecuteContext::with_session(&sessions, Some(&session_id))
            .with_params(HashMap::from([("n".into(), Value::Int(7))]));
        let insert = execute_test_batch(&db, "CREATE item:pending SET n = $n", &ctx);
        assert!(insert.error.is_none(), "{:?}", insert.error);
        assert!(db.get_document("item", "pending").unwrap().is_none());

        let reads = execute_test_batch(
            &db,
            "SELECT VALUE (SELECT VALUE n FROM item:pending); \
             LET snapshot = (SELECT VALUE n FROM item:pending); $snapshot; \
             (SELECT VALUE n FROM item:pending); \
             SELECT VALUE n FROM (SELECT * FROM item:pending)",
            &ctx,
        );
        assert!(reads.error.is_none(), "{:?}", reads.error);
        let expected = Some(Value::Array(vec![Value::Int(7)]));
        assert_eq!(reads.results[0].rows[0].scalar_value, expected);
        assert_eq!(reads.results[2].rows[0].scalar_value, expected);
        assert_eq!(reads.results[3].rows[0].scalar_value, expected);
        assert_eq!(reads.results[4].rows[0].scalar_value, Some(Value::Int(7)));

        let writes = execute_test_batch(
            &db,
            "LET made = IF true THEN CREATE item:bound SET n = 8; \
             SELECT VALUE n FROM item:bound END; $made; \
             IF true THEN CREATE item:branch SET n = 9 END; \
             FOR $n IN [10] DO CREATE item:loop SET n = $n END",
            &ctx,
        );
        assert!(writes.error.is_none(), "{:?}", writes.error);
        assert_eq!(
            writes.results[1].rows[0].scalar_value,
            Some(Value::Array(vec![Value::Int(8)])),
        );
        for key in ["pending", "bound", "branch", "loop"] {
            assert!(db.get_document("item", key).unwrap().is_none());
        }
        let nested_commit = execute_test_batch(&db, "IF true THEN COMMIT END", &ctx);
        assert_eq!(nested_commit.error.unwrap().code, "SDB-QE005");
        assert!(
            sessions
                .has_active_transaction(&session_id, &owner)
                .unwrap()
        );
        sessions.rollback(&session_id, &owner).unwrap();
        for key in ["pending", "bound", "branch", "loop"] {
            assert!(db.get_document("item", key).unwrap().is_none());
        }
    }

    #[test]
    fn expired_timeout_prevents_direct_control_flow_mutation() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());
        let setup = execute_test_batch(
            &db,
            "DEFINE COLLECTION item (n int)",
            &ExecuteContext::none(),
        );
        assert!(setup.error.is_none(), "{:?}", setup.error);
        let ctx =
            ExecuteContext::none().with_explicit_query_timeout(Some(std::time::Duration::ZERO));
        let result = execute_test_batch(
            &db,
            "SET query_timeout = 0; IF true THEN CREATE item:late SET n = 1 END",
            &ctx,
        );
        assert_eq!(result.error.unwrap().code, "SDB-QE014");
        assert!(db.get_document("item", "late").unwrap().is_none());
    }

    #[test]
    fn typed_parameters_survive_create_update_upsert_and_filters() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());
        let payload = Value::Object(HashMap::from([
            ("bytes".into(), Value::Bytes(vec![0, 128, 255])),
            ("reference".into(), Value::Reference("item:target".into())),
            (
                "array".into(),
                Value::Array(vec![Value::Int(4), Value::Null]),
            ),
        ]));
        let text = Value::String("'\"; DELETE item; --".into());
        let ctx = ExecuteContext::none().with_params(HashMap::from([
            ("payload".into(), payload.clone()),
            ("text".into(), text),
            ("n".into(), Value::Int(5)),
        ]));
        let created = execute_test_batch(
            &db,
            "DEFINE COLLECTION item; \
             CREATE item:a SET text = $text, payload = $payload, n = $n; \
             SELECT VALUE payload FROM item WHERE text = $text",
            &ctx,
        );
        assert!(created.error.is_none(), "{:?}", created.error);
        assert_eq!(
            created.results[2].rows[0].scalar_value,
            Some(payload.clone()),
        );
        let updated = execute_test_batch(
            &db,
            "UPDATE item SET n = $n + 1 WHERE text = $text; \
             UPSERT item:b SET payload = $payload, n = $n; \
             SELECT VALUE n FROM item ORDER n * $n",
            &ctx,
        );
        assert!(updated.error.is_none(), "{:?}", updated.error);
        let values: Vec<_> = updated.results[2]
            .rows
            .iter()
            .map(|row| row.scalar_value.clone())
            .collect();
        assert_eq!(values, vec![Some(Value::Int(5)), Some(Value::Int(6))]);
        assert_eq!(
            db.get_document("item", "b").unwrap().unwrap().fields["payload"],
            payload,
        );
        let missing = execute_test_batch(&db, "CREATE item:missing SET payload = $missing", &ctx);
        assert_eq!(missing.error.unwrap().code, "SDB-QE026");
        assert!(db.get_document("item", "missing").unwrap().is_none());

        let deleted = execute_test_batch(&db, "DELETE item WHERE text = $text", &ctx);
        assert!(deleted.error.is_none(), "{:?}", deleted.error);
        assert!(db.get_document("item", "a").unwrap().is_none());
        assert!(db.get_document("item", "b").unwrap().is_some());
    }
}
