use std::collections::HashMap;

use stellardb::{Document, EmbeddedDatabase, Value};

#[test]
fn embedded_database_executes_stellarql_and_document_helpers() {
    let dir = tempfile::tempdir().unwrap();
    let db = EmbeddedDatabase::open(dir.path()).unwrap();

    db.execute("DEFINE COLLECTION user").unwrap();
    db.execute(r#"CREATE user:alice SET name = "Alice", age = 30"#)
        .unwrap();

    let rows = db.query("SELECT name, age FROM user:alice").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get_field("name"),
        Some(Value::String("Alice".to_string()))
    );
    assert_eq!(rows[0].get_field("age"), Some(Value::Int(30)));

    let stored = db.get_document("user", "alice").unwrap().unwrap();
    assert_eq!(stored.id, "user:alice");

    let mut fields = HashMap::new();
    fields.insert("name".to_string(), Value::String("Bob".to_string()));
    let created = db
        .create_document(&Document {
            id: "user:bob".to_string(),
            fields,
        })
        .unwrap();
    assert!(created);
    assert_eq!(db.list_documents("user", None, 10).unwrap().0.len(), 2);

    db.close().unwrap();
}

#[test]
fn query_rejects_multiple_statements_before_any_write() {
    let dir = tempfile::tempdir().unwrap();
    let db = EmbeddedDatabase::open(dir.path()).unwrap();
    db.execute("DEFINE COLLECTION item").unwrap();
    assert!(matches!(
        db.query("CREATE item:a SET value = 1; SELECT * FROM item"),
        Err(stellardb::EmbeddedError::ExpectedSingleResult { actual: 2 })
    ));
    assert!(db.get_document("item", "a").unwrap().is_none());
}

#[test]
fn batch_error_preserves_prior_results_without_implicit_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let db = EmbeddedDatabase::open(dir.path()).unwrap();
    db.execute("DEFINE COLLECTION item").unwrap();
    let failure = db
        .execute("CREATE item:a SET value = 1; BEGIN; CREATE item:b SET value = 2")
        .unwrap_err();
    let stellardb::EmbeddedError::Batch {
        error,
        results,
        completed,
    } = failure
    else {
        panic!("expected structured batch failure");
    };
    use stellardb::error::ErrorCode;
    assert_eq!(error.statement_index, 1);
    assert_eq!(
        error.code,
        stellardb::query::ExecuteError::SessionRequired.code()
    );
    assert_eq!(completed, 1);
    assert_eq!(results[0].rows[0].get_field("value"), Some(Value::Int(1)));
    assert!(db.get_document("item", "a").unwrap().is_some());
    assert!(db.get_document("item", "b").unwrap().is_none());
}

#[test]
fn managed_transaction_mixes_document_writes_and_queries_and_rolls_back_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let db = EmbeddedDatabase::open(dir.path()).unwrap();
    db.execute("DEFINE COLLECTION item").unwrap();
    let tx = db.transaction();
    let doc = Document {
        id: "item:a".into(),
        fields: HashMap::from([(
            "text".into(),
            Value::String("quotes ' \" \\\nПривет".into()),
        )]),
    };
    assert!(tx.create_document(&doc).unwrap());
    assert!(!tx.create_document(&doc).unwrap());
    assert!(db.get_document("item", "a").unwrap().is_none());
    let rows = tx
        .query_with_params(
            "SELECT text FROM item WHERE text = $text",
            HashMap::from([("text".into(), doc.fields["text"].clone())]),
        )
        .unwrap();
    assert_eq!(rows[0].get_field("text"), Some(doc.fields["text"].clone()));
    tx.execute("CREATE item:b SET value = 2; RELATE item:a->links->item:b")
        .unwrap();
    assert!(matches!(
        tx.execute("COMMIT"),
        Err(stellardb::EmbeddedError::TransactionControl)
    ));
    assert!(db.get_document("item", "a").unwrap().is_none());
    tx.commit().unwrap();
    assert_eq!(db.get_document("item", "a").unwrap(), Some(doc));
    assert!(
        db.storage()
            .get_edge("item:a", "links", "item:b")
            .unwrap()
            .is_some()
    );
    {
        let tx = db.transaction();
        assert!(tx.delete_document("item", "a").unwrap());
        tx.execute("CREATE item:discard SET value = 3").unwrap();
    }
    assert!(db.get_document("item", "a").unwrap().is_some());
    assert!(db.get_document("item", "discard").unwrap().is_none());
}

#[test]
fn failed_transaction_commit_publishes_neither_documents_nor_edges() {
    let dir = tempfile::tempdir().unwrap();
    let db = EmbeddedDatabase::open(dir.path()).unwrap();
    db.execute("DEFINE COLLECTION item (name string, INDEX ON name UNIQUE)")
        .unwrap();
    db.execute("CREATE item:existing SET name = 'taken'")
        .unwrap();
    let tx = db.transaction();
    tx.execute("CREATE item:a SET name = 'taken'; CREATE item:b SET name = 'free'; RELATE item:a->links->item:b").unwrap();
    assert!(tx.commit().is_err());
    assert!(db.get_document("item", "a").unwrap().is_none());
    assert!(db.get_document("item", "b").unwrap().is_none());
    assert!(
        db.storage()
            .get_edge("item:a", "links", "item:b")
            .unwrap()
            .is_none()
    );
}

#[test]
fn explicit_timeout_cannot_be_disabled_by_sql_and_prevents_mutation() {
    use std::time::Duration;
    use stellardb::{EmbeddedError, EmbeddedQueryOptions};
    let dir = tempfile::tempdir().unwrap();
    let db = EmbeddedDatabase::open(dir.path()).unwrap();
    db.execute("DEFINE COLLECTION item").unwrap();
    let error = db
        .execute_with_options(
            "SET query_timeout = 0; CREATE item:a SET value = 1",
            EmbeddedQueryOptions {
                timeout: Some(Duration::ZERO),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(matches!(error, EmbeddedError::Batch { .. }));
    assert!(db.get_document("item", "a").unwrap().is_none());
}

#[test]
fn nested_ddl_returns_an_error_without_escaping_the_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let db = EmbeddedDatabase::open(dir.path()).unwrap();
    db.execute("DEFINE COLLECTION item").unwrap();
    let tx = db.transaction();
    tx.execute("CREATE item:pending SET value = 1").unwrap();
    assert!(matches!(
        tx.execute("IF true THEN DEFINE COLLECTION forbidden END"),
        Err(stellardb::EmbeddedError::Batch { .. })
    ));
    assert!(!db.list_collections().iter().any(|name| name == "forbidden"));
    tx.rollback().unwrap();
    assert!(db.get_document("item", "pending").unwrap().is_none());
}
