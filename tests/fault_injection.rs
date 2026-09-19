#![cfg(feature = "fault-injection")]

use std::io::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use fjall::{KeyspaceCreateOptions, OptimisticTxDatabase};
use parking_lot::Mutex;
use serde_json::{Value as JsonValue, json};
use stellardb::fault_injection::FaultSession;
use stellardb::namespace::Namespace;
use stellardb::schema::IndexDef;
use stellardb::server::create_router;
use stellardb::storage::{DIR_STORAGE, WriteOp};
use stellardb::{Database, Document, IndexBuildConfig, Value};

const TEST_DB: &str = "test";
const CHILD_ENV: &str = "STELLARDB_FAULT_TEST_CHILD";
const DATA_ENV: &str = "STELLARDB_FAULT_TEST_DATA";
const PORT_ENV: &str = "STELLARDB_FAULT_TEST_PORT";
const FAULT_DIR_ENV: &str = "STELLARDB_FAULT_DIR";
const READY_FILE: &str = "server.ready";

fn document(
    collection: &str,
    key: impl ToString,
    fields: impl IntoIterator<Item = (&'static str, Value)>,
) -> Document {
    Document {
        id: format!("{collection}:{}", key.to_string()),
        fields: fields
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect(),
    }
}

struct FaultController {
    directory: tempfile::TempDir,
}

impl FaultController {
    fn new() -> Self {
        Self {
            directory: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self) -> &Path {
        self.directory.path()
    }

    fn action_path(&self, point: &str) -> PathBuf {
        self.path().join(format!("{point}.action"))
    }

    fn reached_path(&self, point: &str) -> PathBuf {
        self.path().join(format!("{point}.reached"))
    }

    fn arm(&self, point: &str, action: &str) {
        let _ = std::fs::remove_file(self.reached_path(point));
        std::fs::write(self.action_path(point), action).unwrap();
    }

    fn release(&self, point: &str) {
        match std::fs::remove_file(self.action_path(point)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to release {point}: {error}"),
        }
    }

    async fn wait_reached(&self, point: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !self.reached_path(point).exists() {
            assert!(
                Instant::now() < deadline,
                "fault point '{point}' was not reached"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn wait_reached_blocking(&self, point: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !self.reached_path(point).exists() {
            assert!(
                Instant::now() < deadline,
                "fault point '{point}' was not reached"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

struct ChildServer {
    child: Option<Child>,
    addr: SocketAddr,
}

impl ChildServer {
    fn spawn(data_dir: &Path, faults: &FaultController) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let ready = faults.path().join(READY_FILE);
        let _ = std::fs::remove_file(&ready);

        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "fault_server_child",
                "--test-threads=1",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            .env(DATA_ENV, data_dir)
            .env(PORT_ENV, addr.port().to_string())
            .env(FAULT_DIR_ENV, faults.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(20);
        while !ready.exists() {
            assert!(
                Instant::now() < deadline,
                "fault test server did not become ready"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        Self {
            child: Some(child),
            addr,
        }
    }

    fn kill(mut self) {
        self.terminate();
    }

    fn terminate(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for ChildServer {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[test]
#[ignore]
fn fault_server_child() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    let data_dir = PathBuf::from(std::env::var_os(DATA_ENV).unwrap());
    let port: u16 = std::env::var(PORT_ENV).unwrap().parse().unwrap();
    let fault_dir = PathBuf::from(std::env::var_os(FAULT_DIR_ENV).unwrap());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let namespace = std::sync::Arc::new(
            Namespace::open_with_index_build_config(
                &data_dir,
                IndexBuildConfig {
                    chunk_size: 32,
                    memory_budget_bytes: 2_048,
                    worker_count: 2,
                },
            )
            .unwrap(),
        );
        if !namespace.database_exists(TEST_DB) {
            namespace.create_database(TEST_DB).unwrap();
        }
        let (app, _state) = create_router(namespace, None, None);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .unwrap();
        std::fs::write(fault_dir.join(READY_FILE), []).unwrap();
        axum::serve(listener, app).await.unwrap();
    });
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().build().unwrap()
}

async fn try_sql(
    client: &reqwest::Client,
    addr: SocketAddr,
    query: &str,
) -> Result<JsonValue, reqwest::Error> {
    client
        .post(format!("http://{addr}/sql"))
        .header("X-Database", TEST_DB)
        .json(&json!({"query": query}))
        .send()
        .await?
        .json()
        .await
}

async fn sql(client: &reqwest::Client, addr: SocketAddr, query: &str) -> JsonValue {
    try_sql(client, addr, query).await.unwrap()
}

fn assert_success(body: &JsonValue) {
    assert!(body["error"].is_null(), "query failed: {body:#}");
}

async fn submit_index(
    client: &reqwest::Client,
    addr: SocketAddr,
    query: &str,
    operation_id: &str,
) -> JsonValue {
    let response = client
        .post(format!("http://{addr}/v1/query"))
        .header("X-Database", TEST_DB)
        .json(&json!({"query": query, "operation_id": operation_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
    response.json().await.unwrap()
}

async fn operation(client: &reqwest::Client, addr: SocketAddr, operation_id: &str) -> JsonValue {
    client
        .get(format!("http://{addr}/v1/operations/{operation_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn wait_terminal(
    client: &reqwest::Client,
    addr: SocketAddr,
    operation_id: &str,
) -> JsonValue {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = operation(client, addr, operation_id).await;
        if matches!(
            status["state"].as_str(),
            Some("READY" | "FAILED" | "CANCELLED")
        ) {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "operation did not finish: {status:#}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn prepare_btree_data(path: &Path, count: usize, duplicate_values: bool) {
    let namespace = Namespace::open(path).unwrap();
    let database = namespace.create_database(TEST_DB).unwrap();
    database.get_or_create_collection("items").unwrap();
    database
        .atomic_write_batch(
            (0..count)
                .map(|number| {
                    WriteOp::InsertDoc(document(
                        "items",
                        number,
                        [(
                            "category",
                            if duplicate_values {
                                Value::String(format!("duplicate_{}", number % 2))
                            } else {
                                Value::String(format!("category_{}", number % 17))
                            },
                        )],
                    ))
                })
                .collect(),
        )
        .unwrap();
    database.sync().unwrap();
}

fn raw_counts(path: &Path, collection: &str) -> (usize, usize) {
    let storage = path.join("databases").join(TEST_DB).join(DIR_STORAGE);
    let database = OptimisticTxDatabase::builder(&storage).open().unwrap();
    let docs = database
        .keyspace(
            &format!("docs_{collection}"),
            KeyspaceCreateOptions::default,
        )
        .unwrap();
    let jobs = database
        .keyspace("_index_jobs", KeyspaceCreateOptions::default)
        .unwrap();
    (docs.inner().iter().count(), jobs.inner().iter().count())
}

fn assert_no_orphaned_resources(path: &Path) {
    let storage = path.join("databases").join(TEST_DB).join(DIR_STORAGE);
    let database = OptimisticTxDatabase::builder(&storage).open().unwrap();
    assert!(
        database
            .list_keyspace_names()
            .into_iter()
            .all(|name| !name.to_string().contains("__build_")),
        "temporary Fjall index keyspace survived recovery"
    );
    drop(database);

    fn inspect(directory: &Path) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries {
            let entry = entry.unwrap();
            let name = entry.file_name();
            assert!(
                !name.to_string_lossy().contains("__build_"),
                "temporary index resource survived recovery: {}",
                entry.path().display()
            );
            if entry.file_type().unwrap().is_dir() {
                inspect(&entry.path());
            }
        }
    }
    inspect(&path.join("databases").join(TEST_DB).join("indexes"));
}

fn assert_ready_status(status: &JsonValue, operation_id: &str, total: usize) {
    assert_eq!(status["operation_id"], operation_id);
    assert_eq!(status["state"], "READY", "{status:#}");
    assert_eq!(status["phase"], "COMPLETE");
    assert_eq!(status["processed_items"], total);
    assert_eq!(status["total_items"], total);
    assert_eq!(status["progress_percent"], 100.0);
    assert!(status["error"].is_null());
    assert!(status["finished_at"].is_string());
}

async fn assert_btree_query_is_complete(client: &reqwest::Client, addr: SocketAddr) {
    let explain = sql(
        client,
        addr,
        "EXPLAIN SELECT * FROM items WHERE category = 'category_3'",
    )
    .await;
    assert_success(&explain);
    let plan = explain["results"][0]["data"][0].as_str().unwrap();
    assert!(
        plan.contains("IndexScan"),
        "published index not used: {plan}"
    );

    let selected = sql(
        client,
        addr,
        "SELECT * FROM items WHERE category = 'category_3'",
    )
    .await;
    assert_success(&selected);
    assert_eq!(selected["results"][0]["data"].as_array().unwrap().len(), 77);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timeout_during_create_index_keeps_a_durable_truthful_job() {
    let data = tempfile::tempdir().unwrap();
    prepare_btree_data(data.path(), 1_300, false);
    let faults = FaultController::new();
    let server = ChildServer::spawn(data.path(), &faults);
    let operation_id = "timeout-create-index";
    faults.arm("ddl_after_enqueue", "pause");

    let timed_client = reqwest::Client::builder()
        .timeout(Duration::from_millis(80))
        .build()
        .unwrap();
    let request = tokio::spawn(async move {
        timed_client
            .post(format!("http://{}/v1/query", server.addr))
            .header("X-Database", TEST_DB)
            .json(&json!({
                "query": "CREATE INDEX ON items(category)",
                "operation_id": operation_id,
            }))
            .send()
            .await
    });
    faults.wait_reached("ddl_after_enqueue").await;
    assert!(
        request.await.unwrap().is_err(),
        "request should time out after job acceptance"
    );
    faults.release("ddl_after_enqueue");

    let client = client();
    let retry = submit_index(
        &client,
        server.addr,
        "CREATE INDEX ON items(category)",
        operation_id,
    )
    .await;
    assert_eq!(retry["operation_id"], operation_id);
    assert_eq!(retry["created"], false);
    let status = wait_terminal(&client, server.addr, operation_id).await;
    assert_ready_status(&status, operation_id, 1_300);
    assert_btree_query_is_complete(&client, server.addr).await;

    server.kill();
    assert_no_orphaned_resources(data.path());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_disconnect_after_job_acceptance_does_not_cancel_it() {
    let data = tempfile::tempdir().unwrap();
    prepare_btree_data(data.path(), 1_300, false);
    let faults = FaultController::new();
    let server = ChildServer::spawn(data.path(), &faults);
    let operation_id = "disconnected-create-index";
    faults.arm("ddl_after_enqueue", "pause");

    let body = serde_json::to_string(&json!({
        "query": "CREATE INDEX ON items(category)",
        "operation_id": operation_id,
    }))
    .unwrap();
    let mut stream = std::net::TcpStream::connect(server.addr).unwrap();
    write!(
        stream,
        "POST /v1/query HTTP/1.1\r\nHost: {}\r\nX-Database: {TEST_DB}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        server.addr,
        body.len()
    )
    .unwrap();
    stream.flush().unwrap();
    faults.wait_reached("ddl_after_enqueue").await;
    drop(stream);
    faults.release("ddl_after_enqueue");

    let client = client();
    let status = wait_terminal(&client, server.addr, operation_id).await;
    assert_ready_status(&status, operation_id, 1_300);
    let retry = submit_index(
        &client,
        server.addr,
        "CREATE INDEX ON items(category)",
        operation_id,
    )
    .await;
    assert_eq!(retry["created"], false);
    assert_btree_query_is_complete(&client, server.addr).await;

    server.kill();
    assert_no_orphaned_resources(data.path());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_stop_during_btree_backfill_restarts_without_exposing_partial_index() {
    let data = tempfile::tempdir().unwrap();
    prepare_btree_data(data.path(), 1_300, false);
    let faults = FaultController::new();
    let server = ChildServer::spawn(data.path(), &faults);
    let operation_id = "backfill-crash";
    faults.arm("btree_backfill_chunk", "pause");

    let accepted = submit_index(
        &client(),
        server.addr,
        "CREATE INDEX ON items(category)",
        operation_id,
    )
    .await;
    assert_eq!(accepted["created"], true);
    faults.wait_reached("btree_backfill_chunk").await;
    let active = operation(&client(), server.addr, operation_id).await;
    assert_eq!(active["state"], "BUILDING");
    server.kill();
    faults.release("btree_backfill_chunk");

    {
        let namespace = Namespace::open(data.path()).unwrap();
        let database = namespace.get_database(TEST_DB).unwrap();
        assert_eq!(database.count_documents("items").unwrap(), 1_300);
        assert!(database.get_indexes("items").is_empty());
        assert!(!database.has_index("items", &["category".to_string()]));
    }

    let restarted = ChildServer::spawn(data.path(), &faults);
    let client = client();
    let status = wait_terminal(&client, restarted.addr, operation_id).await;
    assert_ready_status(&status, operation_id, 1_300);
    assert_btree_query_is_complete(&client, restarted.addr).await;
    restarted.kill();
    assert_no_orphaned_resources(data.path());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_stop_after_index_flush_before_schema_publication_reconciles_orphan() {
    let data = tempfile::tempdir().unwrap();
    prepare_btree_data(data.path(), 1_300, false);
    let faults = FaultController::new();
    let server = ChildServer::spawn(data.path(), &faults);
    let operation_id = "publish-window-crash";
    faults.arm("ddl_after_index_promote", "pause");

    submit_index(
        &client(),
        server.addr,
        "CREATE INDEX ON items(category)",
        operation_id,
    )
    .await;
    faults.wait_reached("ddl_after_index_promote").await;
    server.kill();
    faults.release("ddl_after_index_promote");

    {
        let namespace = Namespace::open(data.path()).unwrap();
        let database = namespace.get_database(TEST_DB).unwrap();
        assert_eq!(database.count_documents("items").unwrap(), 1_300);
        assert!(database.get_indexes("items").is_empty());
        assert!(!database.has_index("items", &["category".to_string()]));
    }

    let restarted = ChildServer::spawn(data.path(), &faults);
    let client = client();
    let status = wait_terminal(&client, restarted.addr, operation_id).await;
    assert_ready_status(&status, operation_id, 1_300);
    assert_btree_query_is_complete(&client, restarted.addr).await;
    restarted.kill();
    assert_no_orphaned_resources(data.path());
}

async fn setup_fts_server(data: &tempfile::TempDir, faults: &FaultController) -> ChildServer {
    let server = ChildServer::spawn(data.path(), faults);
    let client = client();
    let define = sql(&client, server.addr, "DEFINE COLLECTION articles").await;
    assert_success(&define);
    let create = sql(
        &client,
        server.addr,
        "CREATE INDEX ON articles(body) FULLTEXT",
    )
    .await;
    assert_success(&create);
    server
}

async fn assert_recovered_fts(data: &tempfile::TempDir, faults: &FaultController, token: &str) {
    let restarted = ChildServer::spawn(data.path(), faults);
    let client = client();
    let primary = sql(&client, restarted.addr, "SELECT body FROM articles:crash").await;
    assert_success(&primary);
    assert_eq!(primary["results"][0]["data"].as_array().unwrap().len(), 1);
    let search = sql(
        &client,
        restarted.addr,
        &format!("SELECT body FROM articles WHERE body @@ '{token}'"),
    )
    .await;
    assert_success(&search);
    assert_eq!(search["results"][0]["data"].as_array().unwrap().len(), 1);
    restarted.kill();
    assert_eq!(raw_counts(data.path(), "articles"), (1, 0));
    assert_no_orphaned_resources(data.path());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_stop_before_fts_commit_replays_recovery_job() {
    let data = tempfile::tempdir().unwrap();
    let faults = FaultController::new();
    let server = setup_fts_server(&data, &faults).await;
    faults.arm("external_before_commit", "pause");

    let addr = server.addr;
    let insert = tokio::spawn(async move {
        try_sql(
            &client(),
            addr,
            "INSERT INTO articles {id: 'crash', body: 'beforecommit token'}",
        )
        .await
    });
    faults.wait_reached("external_before_commit").await;
    server.kill();
    faults.release("external_before_commit");
    let _ = insert.await;
    assert_eq!(raw_counts(data.path(), "articles"), (1, 1));

    assert_recovered_fts(&data, &faults, "beforecommit").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_stop_after_fts_commit_before_job_delete_replays_idempotently() {
    let data = tempfile::tempdir().unwrap();
    let faults = FaultController::new();
    let server = setup_fts_server(&data, &faults).await;
    faults.arm("external_after_commit", "pause");

    let addr = server.addr;
    let insert = tokio::spawn(async move {
        try_sql(
            &client(),
            addr,
            "INSERT INTO articles {id: 'crash', body: 'aftercommit token'}",
        )
        .await
    });
    faults.wait_reached("external_after_commit").await;
    server.kill();
    faults.release("external_after_commit");
    let _ = insert.await;
    assert_eq!(raw_counts(data.path(), "articles"), (1, 1));

    assert_recovered_fts(&data, &faults, "aftercommit").await;
}

fn init_fts_index(database: &Database, collection: &str) -> IndexDef {
    database.get_or_create_collection(collection).unwrap();
    let index = IndexDef::fulltext(collection, vec!["body".to_string()]);
    database
        .init_fts_index(collection, &index.name, &index.fields, None)
        .unwrap();
    database.build_index(collection, &index).unwrap();
    database
        .publish_ready_index(collection, index.clone())
        .unwrap();
    index
}

fn text_doc(key: &str, body: &str) -> Document {
    document("articles", key, [("body", Value::String(body.to_string()))])
}

#[test]
fn repeated_updates_of_one_document_are_coalesced_before_group_commit() {
    let data = tempfile::tempdir().unwrap();
    let database = Database::open(data.path()).unwrap();
    let index = init_fts_index(&database, "articles");
    database
        .set_document(&text_doc("same", "original marker"))
        .unwrap();

    database
        .atomic_write_batch(vec![
            WriteOp::InsertDoc(text_doc("same", "first pending marker")),
            WriteOp::InsertDoc(text_doc("same", "second pending marker")),
            WriteOp::InsertDoc(text_doc("same", "final durable marker")),
        ])
        .unwrap();

    for stale in ["original", "first", "second"] {
        assert!(
            database
                .fts_search("articles", &index.name, stale, 10)
                .unwrap()
                .is_empty(),
            "stale term '{stale}' survived group commit"
        );
    }
    let matches = database
        .fts_search("articles", &index.name, "final", 10)
        .unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].0, "articles:same");
    assert_eq!(
        database.get_document("articles", "same").unwrap(),
        Some(text_doc("same", "final durable marker"))
    );
}

#[test]
fn delete_after_pending_insert_or_update_leaves_no_primary_or_fts_ghost() {
    let data = tempfile::tempdir().unwrap();
    let database = Database::open(data.path()).unwrap();
    let index = init_fts_index(&database, "articles");
    database
        .set_document(&text_doc("existing", "old existing marker"))
        .unwrap();

    database
        .atomic_write_batch(vec![
            WriteOp::InsertDoc(text_doc("new", "pending insert marker")),
            WriteOp::InsertDoc(text_doc("new", "pending update marker")),
            WriteOp::DeleteDoc {
                collection: "articles".to_string(),
                key: "new".to_string(),
            },
            WriteOp::InsertDoc(text_doc("existing", "pending existing marker")),
            WriteOp::DeleteDoc {
                collection: "articles".to_string(),
                key: "existing".to_string(),
            },
        ])
        .unwrap();

    assert!(database.get_document("articles", "new").unwrap().is_none());
    assert!(
        database
            .get_document("articles", "existing")
            .unwrap()
            .is_none()
    );
    for term in ["insert", "update", "existing", "old"] {
        assert!(
            database
                .fts_search("articles", &index.name, term, 10)
                .unwrap()
                .is_empty(),
            "deleted pending document remained searchable for '{term}'"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_value_during_unique_index_build_fails_without_publication() {
    let data = tempfile::tempdir().unwrap();
    prepare_btree_data(data.path(), 1_300, true);
    let faults = FaultController::new();
    let server = ChildServer::spawn(data.path(), &faults);
    let client = client();
    let operation_id = "duplicate-unique";

    let accepted = submit_index(
        &client,
        server.addr,
        "CREATE INDEX ON items(category) UNIQUE",
        operation_id,
    )
    .await;
    assert_eq!(accepted["created"], true);
    let failed = wait_terminal(&client, server.addr, operation_id).await;
    assert_eq!(failed["state"], "FAILED", "{failed:#}");
    assert_eq!(failed["phase"], "COMPLETE");
    assert!(
        failed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Duplicate value")
    );

    let retry = submit_index(
        &client,
        server.addr,
        "CREATE INDEX ON items(category) UNIQUE",
        operation_id,
    )
    .await;
    assert_eq!(retry["created"], false);
    assert_eq!(retry["state"], "FAILED");
    let count = sql(&client, server.addr, "SELECT COUNT(*) AS total FROM items").await;
    assert_success(&count);
    assert_eq!(count["results"][0]["data"][0]["total"], 1_300);
    let explain = sql(
        &client,
        server.addr,
        "EXPLAIN SELECT * FROM items WHERE category = 'duplicate_1'",
    )
    .await;
    assert_success(&explain);
    assert!(
        explain["results"][0]["data"][0]
            .as_str()
            .unwrap()
            .contains("TableScan")
    );

    server.kill();
    assert_no_orphaned_resources(data.path());
}

fn loader_batch(start: usize, count: usize) -> String {
    (start..start + count)
        .map(|number| {
            format!("UPSERT loaded:doc_{number} SET ordinal = {number}, payload = 'value_{number}'")
        })
        .collect::<Vec<_>>()
        .join(";")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resumable_loader_replays_unacknowledged_batch_after_restart() {
    let data = tempfile::tempdir().unwrap();
    let faults = FaultController::new();
    let server = ChildServer::spawn(data.path(), &faults);
    let http = client();
    assert_success(&sql(&http, server.addr, "DEFINE COLLECTION loaded").await);
    assert_success(&sql(&http, server.addr, &loader_batch(0, 20)).await);

    faults.arm("primary_after_commit", "pause");
    let addr = server.addr;
    let interrupted_batch = loader_batch(20, 20);
    let pending_query = interrupted_batch.clone();
    let pending = tokio::spawn(async move { try_sql(&client(), addr, &pending_query).await });
    faults.wait_reached("primary_after_commit").await;
    server.kill();
    faults.release("primary_after_commit");
    let _ = pending.await;
    assert_eq!(raw_counts(data.path(), "loaded").0, 21);

    let restarted = ChildServer::spawn(data.path(), &faults);
    let http = client();
    assert_success(&sql(&http, restarted.addr, &interrupted_batch).await);
    assert_success(&sql(&http, restarted.addr, &loader_batch(40, 20)).await);
    let count = sql(
        &http,
        restarted.addr,
        "SELECT COUNT(*) AS total FROM loaded",
    )
    .await;
    assert_success(&count);
    assert_eq!(count["results"][0]["data"][0]["total"], 60);
    let last = sql(&http, restarted.addr, "SELECT ordinal FROM loaded:doc_59").await;
    assert_success(&last);
    assert_eq!(last["results"][0]["data"][0]["ordinal"], 59);
    restarted.kill();
    assert_eq!(raw_counts(data.path(), "loaded"), (60, 0));
}

static IN_PROCESS_FAULT_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[test]
fn flush_waits_for_inflight_background_commit_without_splitting_visibility() {
    let _serial = IN_PROCESS_FAULT_LOCK.lock();
    let data = tempfile::tempdir().unwrap();
    let faults = FaultController::new();
    let _session = FaultSession::activate(faults.path());
    let database = Database::open(data.path()).unwrap();
    let index = init_fts_index(&database, "articles");
    let doc = text_doc("flush", "concurrent flush marker");

    faults.arm("fts_background_commit", "pause");
    database
        .indexes()
        .update_fts_hnsw("articles", None, Some(&doc))
        .unwrap();
    faults.wait_reached_blocking("fts_background_commit");

    let fts = database.indexes().fts_backend().clone();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let flush = std::thread::spawn(move || {
        let result = fts.flush();
        done_tx.send(()).unwrap();
        result
    });
    assert!(done_rx.recv_timeout(Duration::from_millis(50)).is_err());
    faults.release("fts_background_commit");
    flush.join().unwrap().unwrap();
    let matches = database
        .fts_search("articles", &index.name, "concurrent", 10)
        .unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].0, "articles:flush");
}

#[test]
fn fjall_and_tantivy_commit_errors_preserve_primary_and_recovery_contracts() {
    let _serial = IN_PROCESS_FAULT_LOCK.lock();
    let data = tempfile::tempdir().unwrap();
    let faults = FaultController::new();
    let _session = FaultSession::activate(faults.path());
    let namespace = Namespace::open(data.path()).unwrap();
    let database = namespace.create_database(TEST_DB).unwrap();
    database.get_or_create_collection("plain").unwrap();
    let plain = document(
        "plain",
        "retry",
        [("value", Value::String("durable".to_string()))],
    );

    faults.arm("fjall_before_commit", "error");
    let error = database
        .atomic_write_batch(vec![WriteOp::InsertDoc(plain.clone())])
        .unwrap_err();
    assert!(error.to_string().contains("fjall_before_commit"));
    assert!(database.get_document("plain", "retry").unwrap().is_none());
    database
        .atomic_write_batch(vec![WriteOp::InsertDoc(plain.clone())])
        .unwrap();
    assert_eq!(
        database.get_document("plain", "retry").unwrap(),
        Some(plain)
    );

    let index = init_fts_index(&database, "articles");
    let committed = text_doc("commit_error", "tantivy recovery marker");
    faults.arm("tantivy_commit", "error");
    database.set_document(&committed).unwrap();
    assert_eq!(
        database.get_document("articles", "commit_error").unwrap(),
        Some(committed.clone())
    );
    drop(database);
    drop(namespace);
    assert_eq!(raw_counts(data.path(), "articles"), (1, 1));

    let reopened_namespace = Namespace::open(data.path()).unwrap();
    let reopened = reopened_namespace.get_database(TEST_DB).unwrap();
    let matches = reopened
        .fts_search("articles", &index.name, "recovery", 10)
        .unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].0, "articles:commit_error");
    drop(reopened);
    drop(reopened_namespace);
    assert_eq!(raw_counts(data.path(), "articles"), (1, 0));
}
