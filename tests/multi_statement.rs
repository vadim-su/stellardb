use std::sync::Arc;
use stellardb::Database;
use stellardb::query::{ExecuteContext, Query};
use stellardb::session::{SessionConfig, SessionManager};
use tempfile::tempdir;

fn setup() -> (Arc<Database>, Arc<SessionManager>, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Database::open(dir.path()).unwrap());
    let sessions = Arc::new(SessionManager::new(SessionConfig::default()));
    (storage, sessions, dir)
}

fn run_batch(
    sql: &str,
    storage: &Arc<Database>,
    sessions: &SessionManager,
    session_id: Option<&str>,
) -> stellardb::query::BatchResult {
    let ctx = ExecuteContext::with_session(sessions, session_id);
    Query::parse(sql)
        .expect("parse failed")
        .bind(storage.clone())
        .expect("bind failed")
        .execute(&ctx)
        .expect("execute failed")
}

#[test]
fn test_multi_insert_and_select() {
    let (storage, sessions, _dir) = setup();

    // First create the collection
    run_batch("DEFINE COLLECTION user", &storage, &sessions, None);

    let result = run_batch(
        r#"
        INSERT INTO user {id: 'alice', name: 'Alice'};
        INSERT INTO user {id: 'bob', name: 'Bob'};
        SELECT * FROM user
        "#,
        &storage,
        &sessions,
        None,
    );

    assert_eq!(result.completed, 3);
    assert!(result.error.is_none());

    // Check SELECT result
    let select_result = &result.results[2];
    assert_eq!(select_result.statement_type, "SELECT");
    assert_eq!(select_result.rows.len(), 2);
}

#[test]
fn test_transaction_in_batch() {
    let (storage, sessions, _dir) = setup();

    // Create collection first (DDL not allowed in transaction)
    run_batch("DEFINE COLLECTION account", &storage, &sessions, None);

    let result = run_batch(
        r#"
        BEGIN;
        INSERT INTO account {id: 'alice', balance: 100};
        INSERT INTO account {id: 'bob', balance: 50};
        COMMIT
        "#,
        &storage,
        &sessions,
        None,
    );

    assert_eq!(result.completed, 4);
    assert!(result.error.is_none());

    // Verify data was committed
    let verify = run_batch("SELECT * FROM account", &storage, &sessions, None);

    assert_eq!(verify.results[0].rows.len(), 2);
}

#[test]
fn test_transaction_rollback_on_error() {
    let (storage, sessions, _dir) = setup();

    // Create collection first (separate batch - DDL must complete before DML can reference it)
    run_batch("DEFINE COLLECTION account", &storage, &sessions, None);
    // Then insert alice
    run_batch(
        "INSERT INTO account {id: 'alice', balance: 100}",
        &storage,
        &sessions,
        None,
    );

    // Try batch with duplicate - should fail and rollback
    let result = run_batch(
        r#"
        BEGIN;
        INSERT INTO account {id: 'bob', balance: 50};
        INSERT INTO account {id: 'alice', balance: 200};
        COMMIT
        "#,
        &storage,
        &sessions,
        None,
    );

    assert!(result.error.is_some());
    assert_eq!(result.error.as_ref().unwrap().statement_index, 2);

    // Verify bob was NOT inserted (rollback)
    let verify = run_batch("SELECT * FROM account", &storage, &sessions, None);

    // Only alice should exist
    assert_eq!(verify.results[0].rows.len(), 1);
}

#[test]
fn test_semicolon_in_string() {
    let (storage, sessions, _dir) = setup();

    // Create collection first
    run_batch("DEFINE COLLECTION user", &storage, &sessions, None);

    let result = run_batch(
        r#"INSERT INTO user {id: 'test', bio: 'Hello; world'}; SELECT * FROM user"#,
        &storage,
        &sessions,
        None,
    );

    assert_eq!(result.completed, 2);
    assert!(result.error.is_none());
}

#[test]
fn test_multiple_selects() {
    let (storage, sessions, _dir) = setup();

    // Create collections first
    run_batch(
        "DEFINE COLLECTION a; DEFINE COLLECTION b",
        &storage,
        &sessions,
        None,
    );

    run_batch(
        "INSERT INTO a {id: '1'}; INSERT INTO b {id: '2'}",
        &storage,
        &sessions,
        None,
    );

    let result = run_batch(
        "SELECT * FROM a; SELECT * FROM b",
        &storage,
        &sessions,
        None,
    );

    assert_eq!(result.completed, 2);
    assert_eq!(result.results[0].rows.len(), 1);
    assert_eq!(result.results[1].rows.len(), 1);
}
