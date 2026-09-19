#[cfg(test)]
mod tests {
    use crate::Database;
    use crate::query::{ExecuteContext, Query};
    use crate::session::{SessionConfig, SessionManager};
    use std::sync::Arc;
    use tempfile::tempdir;

    fn setup() -> (Arc<Database>, Arc<SessionManager>, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let storage = Arc::new(Database::open(dir.path()).unwrap());
        let sessions = Arc::new(SessionManager::new(SessionConfig::default()));
        (storage, sessions, dir)
    }

    fn run_batch_result(
        sql: &str,
        storage: &Arc<Database>,
        sessions: &SessionManager,
        session_id: Option<&str>,
    ) -> Result<crate::query::BatchResult, crate::query::ExecuteError> {
        let ctx = ExecuteContext::with_session(sessions, session_id);
        Query::parse(sql)
            .expect("parse failed")
            .bind(storage.clone())
            .expect("bind failed")
            .execute(&ctx)
    }

    fn run_batch(
        sql: &str,
        storage: &Arc<Database>,
        sessions: &SessionManager,
        session_id: Option<&str>,
    ) -> crate::query::BatchResult {
        run_batch_result(sql, storage, sessions, session_id).expect("execute failed")
    }

    #[test]
    fn test_batch_multiple_inserts() {
        let (storage, sessions, _dir) = setup();

        // First create the collection
        let _ = run_batch("DEFINE COLLECTION user", &storage, &sessions, None);

        let result = run_batch(
            "INSERT INTO user {id: 'alice', name: 'Alice'}; INSERT INTO user {id: 'bob', name: 'Bob'}",
            &storage,
            &sessions,
            None,
        );

        assert_eq!(result.completed, 2);
        assert!(result.error.is_none());
        assert_eq!(result.results.len(), 2);
    }

    #[test]
    fn test_batch_stop_on_error() {
        let (storage, sessions, _dir) = setup();

        // Create collection first (separate batch - DDL must be bound/executed before DML can reference it)
        run_batch("DEFINE COLLECTION user", &storage, &sessions, None);
        // Then insert alice
        run_batch(
            "INSERT INTO user {id: 'alice', name: 'Alice'}",
            &storage,
            &sessions,
            None,
        );

        // Second batch: bob succeeds, alice duplicate fails, select never runs
        let result = run_batch(
            "INSERT INTO user {id: 'bob', name: 'Bob'}; INSERT INTO user {id: 'alice', name: 'Duplicate'}; SELECT * FROM user",
            &storage,
            &sessions,
            None,
        );

        assert_eq!(result.completed, 1);
        assert!(result.error.is_some());
        assert_eq!(result.error.as_ref().unwrap().statement_index, 1);
        assert_eq!(result.results.len(), 1);
    }

    #[test]
    fn test_batch_with_transaction() {
        let (storage, sessions, _dir) = setup();

        // Create collection first (outside transaction, DDL not allowed in tx)
        let _ = run_batch("DEFINE COLLECTION user", &storage, &sessions, None);

        let result = run_batch(
            "BEGIN; INSERT INTO user {id: 'alice', name: 'Alice'}; COMMIT",
            &storage,
            &sessions,
            None,
        );

        assert_eq!(result.completed, 3);
        assert!(result.error.is_none());
        assert_eq!(result.results[0].statement_type, "BEGIN");
        assert!(result.results[0].session_id.is_some());
        assert_eq!(result.results[2].statement_type, "COMMIT");
    }

    #[test]
    fn transaction_planned_error_keeps_execute_error_type() {
        let (storage, sessions, _dir) = setup();
        run_batch("DEFINE COLLECTION user", &storage, &sessions, None);
        run_batch("INSERT INTO user {id: 'alice'}", &storage, &sessions, None);

        let result = run_batch(
            "BEGIN; INSERT INTO user {id: 'alice'}",
            &storage,
            &sessions,
            None,
        );
        let error = result.error.expect("duplicate insert must fail");
        assert_eq!(error.statement_index, 1);
        assert_eq!(error.code, "SDB-QE002");
        assert_eq!(error.class, crate::error::ErrorClass::AlreadyExists);
        assert!(error.message.contains("already exists"));
    }

    #[cfg(feature = "hnsw")]
    #[test]
    fn transaction_commit_keeps_vector_dimension_storage_error_type() {
        let (storage, sessions, _dir) = setup();
        run_batch("DEFINE COLLECTION vectors", &storage, &sessions, None);
        run_batch(
            "CREATE INDEX ON vectors(embedding) HNSW DIMENSION 3 DIST COSINE",
            &storage,
            &sessions,
            None,
        );

        let result = run_batch(
            "BEGIN; INSERT INTO vectors {id: 'bad', embedding: [1.0, 0.0, 0.0, 0.0]}; COMMIT",
            &storage,
            &sessions,
            None,
        );
        let error = result
            .error
            .expect("invalid vector dimension must fail commit");
        assert_eq!(error.statement_index, 2);
        assert_eq!(error.code, "SDB-ST008");
        assert_eq!(error.class, crate::error::ErrorClass::InvalidArgument);
        assert!(error.message.contains("Vector dimension mismatch"));
        assert!(storage.get_document("vectors", "bad").unwrap().is_none());
    }

    #[test]
    fn unknown_or_expired_session_never_falls_back_to_autocommit() {
        use crate::query::ExecuteError;
        use crate::session::SessionOwner;
        use std::time::Duration;

        let (storage, _, _dir) = setup();
        let sessions = SessionManager::new(SessionConfig {
            transaction_timeout: Duration::ZERO,
            cleanup_interval: Duration::from_secs(1),
        });
        run_batch("DEFINE COLLECTION user", &storage, &sessions, None);

        let unknown = run_batch_result(
            "INSERT INTO user {id: 'unknown'}",
            &storage,
            &sessions,
            Some("missing-session"),
        );
        assert!(matches!(unknown, Err(ExecuteError::SessionExpired)));
        assert!(storage.get_document("user", "unknown").unwrap().is_none());

        let set = run_batch_result(
            "SET QUERY_TIMEOUT = 5s",
            &storage,
            &sessions,
            Some("missing-session"),
        );
        assert!(matches!(set, Err(ExecuteError::SessionExpired)));

        let owner = SessionOwner::anonymous("");
        let expired = sessions.begin_transaction(storage.clone(), owner);
        assert_eq!(sessions.cleanup_timed_out(), 1);
        let after_cleanup = run_batch_result(
            "INSERT INTO user {id: 'expired'}",
            &storage,
            &sessions,
            Some(&expired),
        );
        assert!(matches!(after_cleanup, Err(ExecuteError::SessionExpired)));
        assert!(storage.get_document("user", "expired").unwrap().is_none());
    }

    #[test]
    fn repeated_begin_rolls_back_batch_transaction_without_orphan() {
        let (storage, sessions, _dir) = setup();
        let result = run_batch("BEGIN; BEGIN", &storage, &sessions, None);
        let error = result.error.expect("second BEGIN must fail");
        assert_eq!(error.statement_index, 1);
        assert_eq!(error.code, "SDB-QE005");
        assert_eq!(sessions.active_count(), 0);
    }

    #[test]
    fn begin_with_active_external_session_does_not_create_another_session() {
        let (storage, sessions, _dir) = setup();
        let begin = run_batch("BEGIN", &storage, &sessions, None);
        let session_id = begin.results[0].session_id.clone().unwrap();
        assert_eq!(sessions.active_count(), 1);

        let result = run_batch("BEGIN", &storage, &sessions, Some(&session_id));
        assert_eq!(result.error.as_ref().unwrap().code, "SDB-QE005");
        assert_eq!(sessions.active_count(), 1);

        run_batch("ROLLBACK", &storage, &sessions, Some(&session_id));
        assert_eq!(sessions.active_count(), 0);
    }

    #[test]
    fn test_let_with_subquery() {
        let (storage, sessions, _dir) = setup();

        // Create collection with test data
        run_batch("DEFINE COLLECTION items", &storage, &sessions, None);
        run_batch(
            "INSERT INTO items {id: '1', name: 'A'}, {id: '2', name: 'B'}",
            &storage,
            &sessions,
            None,
        );

        // LET with subquery - this should resolve the subquery before storing
        let result = run_batch(
            "LET names = (SELECT name FROM items); SELECT $names AS result",
            &storage,
            &sessions,
            None,
        );

        assert_eq!(result.completed, 2, "Both statements should complete");
        assert!(result.error.is_none(), "No error expected");

        // The second statement should have the subquery results
        let select_result = &result.results[1];
        assert_eq!(select_result.statement_type, "SELECT");
    }

    #[cfg(feature = "hnsw")]
    #[test]
    fn test_let_with_subquery_used_in_knn() {
        let (storage, sessions, _dir) = setup();

        // Create collection with HNSW index
        run_batch("DEFINE COLLECTION vectors", &storage, &sessions, None);
        run_batch(
            "CREATE INDEX ON vectors(emb) HNSW DIMENSION 3 DIST EUCLIDEAN",
            &storage,
            &sessions,
            None,
        );
        run_batch(
            "INSERT INTO vectors {id: '1', emb: [1.0, 0.0, 0.0]}, {id: '2', emb: [0.0, 1.0, 0.0]}",
            &storage,
            &sessions,
            None,
        );

        // LET with subquery to get a vector, then use it in KNN
        // Using SELECT VALUE to get just the emb array value, then index [0] to get first result
        let result = run_batch(
            "LET vec = (SELECT VALUE emb FROM vectors:1); SELECT id FROM vectors WHERE emb <|2|> $vec[0]",
            &storage,
            &sessions,
            None,
        );

        assert_eq!(
            result.completed, 2,
            "Both statements should complete: {:?}",
            result.error
        );
        assert!(
            result.error.is_none(),
            "No error expected: {:?}",
            result.error
        );
    }

    #[cfg(all(feature = "fts", feature = "hnsw"))]
    #[test]
    fn test_rrf_with_subquery_arguments() {
        let (storage, sessions, _dir) = setup();

        // Create collection with both FTS and HNSW indexes
        run_batch("DEFINE COLLECTION products", &storage, &sessions, None);
        run_batch(
            "CREATE INDEX ON products(name) FULLTEXT",
            &storage,
            &sessions,
            None,
        );
        run_batch(
            "CREATE INDEX ON products(embedding) HNSW DIMENSION 3 DIST EUCLIDEAN",
            &storage,
            &sessions,
            None,
        );
        run_batch(
            "INSERT INTO products {id: 'laptop-pro', name: 'laptop gaming pro', embedding: [1.0, 0.0, 0.0]}, \
             {id: 'laptop-air', name: 'laptop ultra air', embedding: [0.9, 0.1, 0.0]}, \
             {id: 'desktop', name: 'desktop tower gaming', embedding: [0.0, 1.0, 0.0]}",
            &storage,
            &sessions,
            None,
        );

        // Test: LET with subquery, then search::rrf with subqueries in array argument
        let result = run_batch(
            "LET sample_vec = (SELECT VALUE embedding FROM products:laptop-pro); \
             SELECT search::rrf([ \
                 (SELECT id, name FROM products WHERE embedding <|5|> $sample_vec[0]), \
                 (SELECT id, name FROM products WHERE name @@ 'laptop') \
             ], 10)",
            &storage,
            &sessions,
            None,
        );

        assert_eq!(
            result.completed, 2,
            "Both statements should complete: {:?}",
            result.error
        );
        assert!(
            result.error.is_none(),
            "No error expected: {:?}",
            result.error
        );

        // The second result should be the RRF output
        let rrf_result = &result.results[1];
        assert_eq!(rrf_result.statement_type, "SELECT");
    }

    #[cfg(feature = "hnsw")]
    #[test]
    fn test_chained_let_with_subquery() {
        let (storage, sessions, _dir) = setup();

        // Create collection with HNSW index
        run_batch("DEFINE COLLECTION vectors", &storage, &sessions, None);
        run_batch(
            "CREATE INDEX ON vectors(emb) HNSW DIMENSION 3 DIST EUCLIDEAN",
            &storage,
            &sessions,
            None,
        );
        run_batch(
            "INSERT INTO vectors {id: '1', emb: [1.0, 0.0, 0.0]}, {id: '2', emb: [0.0, 1.0, 0.0]}",
            &storage,
            &sessions,
            None,
        );

        // Test: Chain of LET statements where second LET uses variable from first
        let result = run_batch(
            "LET vec = (SELECT VALUE emb FROM vectors:1)[0]; \
             LET results = (SELECT id FROM vectors WHERE emb <|2|> $vec); \
             SELECT $results AS found",
            &storage,
            &sessions,
            None,
        );

        assert_eq!(
            result.completed, 3,
            "All three statements should complete: {:?}",
            result.error
        );
        assert!(
            result.error.is_none(),
            "No error expected: {:?}",
            result.error
        );
    }
}
