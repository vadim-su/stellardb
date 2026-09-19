mod common;

use std::path::Path;

use fjall::{KeyspaceCreateOptions, OptimisticTxDatabase, PersistMode};
use rkyv::rancor;
use serde_json::json;
use stellardb::{Document, Value};

async fn sql(
    client: &reqwest::Client,
    addr: std::net::SocketAddr,
    query: &str,
) -> serde_json::Value {
    let response = client
        .post(format!("http://{addr}/sql"))
        .json(&json!({ "query": query }))
        .send()
        .await
        .unwrap();
    response.json().await.unwrap()
}

fn assert_success(body: &serde_json::Value) {
    assert!(body["error"].is_null(), "query failed: {body:?}");
}

fn inject_committed_update_with_pending_external_index_job(
    data_dir: &Path,
    collection: &str,
    old_doc: &Document,
    new_doc: &Document,
) {
    let storage_path = data_dir
        .join("databases")
        .join(common::TEST_DB)
        .join(stellardb::storage::DIR_STORAGE);
    let db = OptimisticTxDatabase::builder(&storage_path).open().unwrap();
    let docs = db
        .keyspace(
            &format!("docs_{collection}"),
            KeyspaceCreateOptions::default,
        )
        .unwrap();
    let exists = db
        .keyspace(
            &format!("exists_{collection}"),
            KeyspaceCreateOptions::default,
        )
        .unwrap();
    let jobs = db
        .keyspace("_index_jobs", KeyspaceCreateOptions::default)
        .unwrap();

    let mut tx = db.write_tx().unwrap();
    let bytes = rkyv::to_bytes::<rancor::Error>(new_doc).unwrap();
    tx.insert(&docs, new_doc.key().as_bytes(), bytes.as_slice());
    tx.insert(&exists, new_doc.key().as_bytes(), []);
    tx.insert(
        &jobs,
        format!("{collection}:{}", new_doc.key()).as_bytes(),
        serde_json::to_vec(&json!({
            "collection": collection,
            "key": new_doc.key(),
            "old_doc": old_doc,
            "new_doc": new_doc,
        }))
        .unwrap(),
    );
    tx.commit().unwrap().unwrap();
    db.persist(PersistMode::SyncAll).unwrap();
}

#[tokio::test]
async fn server_restart_after_abrupt_stop_preserves_docs_and_fts() {
    let tmp = tempfile::tempdir().unwrap();
    let client = common::test_client();

    let first = common::spawn_server_on_data_dir(tmp.path()).await;
    assert_success(&sql(&client, first.addr, "DEFINE COLLECTION articles").await);
    assert_success(
        &sql(
            &client,
            first.addr,
            "CREATE INDEX ON articles(title) FULLTEXT",
        )
        .await,
    );
    assert_success(
        &sql(
            &client,
            first.addr,
            r#"INSERT INTO articles {id: "restart", title: "Abrupt Restart Recovery"}"#,
        )
        .await,
    );

    first.abort().await;

    let second = common::spawn_server_on_data_dir(tmp.path()).await;
    let by_id = sql(
        &client,
        second.addr,
        r#"SELECT title FROM articles:restart"#,
    )
    .await;
    assert_success(&by_id);
    assert_eq!(
        by_id["results"][0]["data"][0]["title"],
        "Abrupt Restart Recovery"
    );

    let by_fts = sql(
        &client,
        second.addr,
        r#"SELECT title FROM articles WHERE title @@ "recovery""#,
    )
    .await;
    assert_success(&by_fts);
    assert_eq!(
        by_fts["results"][0]["data"][0]["title"],
        "Abrupt Restart Recovery"
    );

    second.abort().await;
}

#[tokio::test]
async fn server_restart_replays_pending_fts_update_job() {
    let tmp = tempfile::tempdir().unwrap();
    let client = common::test_client();

    let first = common::spawn_server_on_data_dir(tmp.path()).await;
    assert_success(&sql(&client, first.addr, "DEFINE COLLECTION articles").await);
    assert_success(
        &sql(
            &client,
            first.addr,
            "CREATE INDEX ON articles(title) FULLTEXT",
        )
        .await,
    );
    assert_success(
        &sql(
            &client,
            first.addr,
            r#"INSERT INTO articles {id: "replay_update", title: "Obsolete Crash Token"}"#,
        )
        .await,
    );
    let old_search = sql(
        &client,
        first.addr,
        r#"SELECT title FROM articles WHERE title @@ "obsolete""#,
    )
    .await;
    assert_success(&old_search);
    assert_eq!(
        old_search["results"][0]["data"].as_array().unwrap().len(),
        1
    );

    first.abort().await;

    let old_doc = Document {
        id: "articles:replay_update".to_string(),
        fields: [(
            "title".to_string(),
            Value::String("Obsolete Crash Token".to_string()),
        )]
        .into_iter()
        .collect(),
    };
    let new_doc = Document {
        id: "articles:replay_update".to_string(),
        fields: [(
            "title".to_string(),
            Value::String("Recovered Durable Token".to_string()),
        )]
        .into_iter()
        .collect(),
    };
    inject_committed_update_with_pending_external_index_job(
        tmp.path(),
        "articles",
        &old_doc,
        &new_doc,
    );

    let second = common::spawn_server_on_data_dir(tmp.path()).await;
    let by_new_fts = sql(
        &client,
        second.addr,
        r#"SELECT title FROM articles WHERE title @@ "recovered""#,
    )
    .await;
    assert_success(&by_new_fts);
    assert_eq!(
        by_new_fts["results"][0]["data"][0]["title"],
        "Recovered Durable Token"
    );

    let by_old_fts = sql(
        &client,
        second.addr,
        r#"SELECT title FROM articles WHERE title @@ "obsolete""#,
    )
    .await;
    assert_success(&by_old_fts);
    assert!(
        by_old_fts["results"][0]["data"]
            .as_array()
            .unwrap()
            .is_empty(),
        "replayed update job must remove stale FTS terms: {by_old_fts:?}"
    );

    second.abort().await;
}

#[tokio::test]
async fn server_restart_after_abrupt_stop_preserves_hnsw_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let client = common::test_client();

    let first = common::spawn_server_on_data_dir(tmp.path()).await;
    assert_success(&sql(&client, first.addr, "DEFINE COLLECTION items").await);
    assert_success(
        &sql(
            &client,
            first.addr,
            "CREATE INDEX ON items(embedding) HNSW DIMENSION 3 DIST COSINE",
        )
        .await,
    );
    assert_success(
        &sql(
            &client,
            first.addr,
            r#"INSERT INTO items {id: "near", name: "Near", embedding: [1.0, 0.0, 0.0]}"#,
        )
        .await,
    );
    assert_success(
        &sql(
            &client,
            first.addr,
            r#"INSERT INTO items {id: "far", name: "Far", embedding: [0.0, 1.0, 0.0]}"#,
        )
        .await,
    );
    assert_success(&sql(&client, first.addr, "DELETE items:near").await);

    first.abort().await;

    let second = common::spawn_server_on_data_dir(tmp.path()).await;
    let by_vector = sql(
        &client,
        second.addr,
        r#"SELECT id, name FROM items WHERE embedding <|2|> [1.0, 0.0, 0.0]"#,
    )
    .await;
    assert_success(&by_vector);

    let rows = by_vector["results"][0]["data"].as_array().unwrap();
    assert!(
        rows.iter().all(|row| row["id"] != "items:near"),
        "deleted document must not reappear in HNSW results after restart: {rows:?}"
    );
    assert!(
        rows.iter().any(|row| row["id"] == "items:far"),
        "remaining document should still be searchable after restart: {rows:?}"
    );

    second.abort().await;
}

#[tokio::test]
async fn server_restart_replays_pending_hnsw_update_job() {
    let tmp = tempfile::tempdir().unwrap();
    let client = common::test_client();

    let first = common::spawn_server_on_data_dir(tmp.path()).await;
    assert_success(&sql(&client, first.addr, "DEFINE COLLECTION items").await);
    assert_success(
        &sql(
            &client,
            first.addr,
            "CREATE INDEX ON items(embedding) HNSW DIMENSION 3 DIST COSINE",
        )
        .await,
    );
    assert_success(
        &sql(
            &client,
            first.addr,
            r#"INSERT INTO items {id: "anchor", name: "Anchor", embedding: [1.0, 0.0, 0.0]}"#,
        )
        .await,
    );
    assert_success(
        &sql(
            &client,
            first.addr,
            r#"INSERT INTO items {id: "moved", name: "Moved", embedding: [1.0, 0.0, 0.0]}"#,
        )
        .await,
    );

    first.abort().await;

    let old_doc = Document {
        id: "items:moved".to_string(),
        fields: [
            ("name".to_string(), Value::String("Moved".to_string())),
            (
                "embedding".to_string(),
                Value::Array(vec![
                    Value::Float(1.0),
                    Value::Float(0.0),
                    Value::Float(0.0),
                ]),
            ),
        ]
        .into_iter()
        .collect(),
    };
    let new_doc = Document {
        id: "items:moved".to_string(),
        fields: [
            ("name".to_string(), Value::String("Moved".to_string())),
            (
                "embedding".to_string(),
                Value::Array(vec![
                    Value::Float(0.0),
                    Value::Float(1.0),
                    Value::Float(0.0),
                ]),
            ),
        ]
        .into_iter()
        .collect(),
    };
    inject_committed_update_with_pending_external_index_job(
        tmp.path(),
        "items",
        &old_doc,
        &new_doc,
    );

    let second = common::spawn_server_on_data_dir(tmp.path()).await;
    let by_new_vector = sql(
        &client,
        second.addr,
        r#"SELECT id, name FROM items WHERE embedding <|1|> [0.0, 1.0, 0.0]"#,
    )
    .await;
    assert_success(&by_new_vector);
    assert_eq!(by_new_vector["results"][0]["data"][0]["id"], "items:moved");

    let by_old_vector = sql(
        &client,
        second.addr,
        r#"SELECT id, name FROM items WHERE embedding <|1|> [1.0, 0.0, 0.0]"#,
    )
    .await;
    assert_success(&by_old_vector);
    assert_eq!(
        by_old_vector["results"][0]["data"][0]["id"], "items:anchor",
        "replayed update job must remove the stale HNSW vector: {by_old_vector:?}"
    );

    second.abort().await;
}
