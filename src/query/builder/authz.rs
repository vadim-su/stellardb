use crate::auth::{Action, AuthorizationProvider, Subject};
use crate::query::ast::{DataSource, DeleteTarget, Statement};

pub(super) fn statement_to_action(stmt: &Statement) -> Option<(Action, Option<String>)> {
    match stmt {
        Statement::Select(s) => {
            let coll = match &s.source {
                DataSource::Collection(t) => Some(t.collection.clone()),
                DataSource::FieldPath { target, .. } => Some(target.collection.clone()),
                _ => None,
            };
            Some((Action::Select, coll))
        }
        Statement::Insert(i) => Some((Action::Insert, Some(i.collection.clone()))),
        Statement::Create(c) => Some((Action::Insert, Some(c.target.collection.clone()))),
        Statement::Update(u) => Some((Action::Update, Some(u.target.collection.clone()))),
        Statement::Upsert(u) => Some((Action::Update, Some(u.target.collection.clone()))),
        Statement::Delete(d) => {
            let coll = match &d.target {
                DeleteTarget::Document(t) => Some(t.collection.clone()),
                DeleteTarget::Edge { from, .. } => Some(from.collection.clone()),
            };
            Some((Action::Delete, coll))
        }
        Statement::Relate(_) => Some((Action::Traverse, None)),
        Statement::DefineCollection(_) => Some((Action::Create, None)),
        Statement::DropCollection(_) => Some((Action::Drop, None)),
        Statement::CreateIndex(_) => Some((Action::Create, None)),
        Statement::DropIndex(_) => Some((Action::Drop, None)),
        Statement::Reindex(_) => Some((Action::Manage, None)),
        Statement::DefineAnalyzer(_) => Some((Action::Create, None)),
        Statement::DropAnalyzer(_) => Some((Action::Drop, None)),
        Statement::CreateUser(_)
        | Statement::DropUser(_)
        | Statement::AlterUser(_)
        | Statement::CreateApiKey(_)
        | Statement::DropApiKey(_)
        | Statement::CreatePolicy(_)
        | Statement::DropPolicy(_)
        | Statement::AlterCollectionAttrs(_) => Some((Action::Manage, None)),
        Statement::Begin(_)
        | Statement::Commit(_)
        | Statement::Rollback(_)
        | Statement::Describe(_)
        | Statement::Explain(_)
        | Statement::Let(_)
        | Statement::Set(_)
        | Statement::Expr(_) => None,
    }
}

pub(super) fn authorize_statement(
    authorization: Option<&dyn AuthorizationProvider>,
    subject: Option<&Subject>,
    database: &str,
    stmt: &Statement,
) -> Result<(), String> {
    let Some((action, collection)) = statement_to_action(stmt) else {
        return Ok(());
    };

    crate::auth::authorize(
        authorization,
        subject,
        action,
        database,
        collection.as_deref(),
    )
    .map_err(|error| error.to_string())
}
