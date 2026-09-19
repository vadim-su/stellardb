use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use stellardb::namespace::Namespace;
use stellardb::server::create_router;
use stellardb::storage::WriteOp;
use stellardb::{Document, Value};

const TEST_DB: &str = "test";

#[tokio::test]
async fn large_create_index_is_durable_observable_and_deduplicated() {
    let tmp = tempfile::tempdir().unwrap();
    let namespace = Arc::new(Namespace::open(tmp.path()).unwrap());
    let database = namespace.create_database(TEST_DB).unwrap();
    database.get_or_create_collection("items").unwrap();
    database.get_or_create_collection("dupes").unwrap();

    let writes = (0..5_000)
        .map(|number| {
            WriteOp::InsertDoc(Document {
                id: format!("items:{number}"),
                fields: HashMap::from([(
                    "category".to_string(),
                    Value::String(format!("category_{}", number % 20)),
                )]),
            })
        })
        .collect();
    database.atomic_write_batch(writes).unwrap();
    database
        .atomic_write_batch(
            (0..1_000)
                .map(|number| {
                    WriteOp::InsertDoc(Document {
                        id: format!("dupes:{number}"),
                        fields: HashMap::from([(
                            "code".to_string(),
                            Value::String(format!("duplicate_{}", number % 2)),
                        )]),
                    })
                })
                .collect(),
        )
        .unwrap();

    let (app, _state) = create_router(namespace, None, None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let create = || {
        client
            .post(format!("http://{addr}/v1/query"))
            .header("X-Database", TEST_DB)
            .json(&json!({"query": "CREATE INDEX ON items(category)"}))
            .send()
    };

    let (first, second) = tokio::join!(create(), create());
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.status(), reqwest::StatusCode::ACCEPTED);
    assert_eq!(second.status(), reqwest::StatusCode::ACCEPTED);
    let first: serde_json::Value = first.json().await.unwrap();
    let second: serde_json::Value = second.json().await.unwrap();
    assert_eq!(first["operation_id"], second["operation_id"]);
    assert!(
        first["status_url"]
            .as_str()
            .unwrap()
            .starts_with("/v1/operations/")
    );

    let operation_id = first["operation_id"].as_str().unwrap();
    let mut terminal = None;
    for _ in 0..200 {
        let status: serde_json::Value = client
            .get(format!("http://{addr}/v1/operations/{operation_id}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(status["operation_id"], operation_id);
        assert_eq!(status["operation_type"], "CREATE_INDEX");
        assert_eq!(status["database"], TEST_DB);
        assert_eq!(status["collection"], "items");
        assert_eq!(status["index_type"], "BTREE");
        assert_eq!(status["total_items"], 5_000);
        assert!(status["processed_items"].as_u64().unwrap() <= 5_000);
        if matches!(
            status["state"].as_str(),
            Some("READY" | "FAILED" | "CANCELLED")
        ) {
            terminal = Some(status);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let terminal = terminal.expect("index job did not finish");
    assert_eq!(terminal["state"], "READY", "{terminal:?}");
    assert_eq!(terminal["processed_items"], 5_000);
    assert_eq!(terminal["progress_percent"], 100.0);
    assert!(terminal["finished_at"].is_string());
    assert!(terminal["error"].is_null());

    let list: serde_json::Value = client
        .get(format!("http://{addr}/v1/operations?limit=10"))
        .header("X-Database", TEST_DB)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["count"], 1);
    assert_eq!(list["operations"][0]["operation_id"], operation_id);

    let failed = client
        .post(format!("http://{addr}/v1/query"))
        .header("X-Database", TEST_DB)
        .json(&json!({"query": "CREATE INDEX ON dupes(code) UNIQUE"}))
        .send()
        .await
        .unwrap();
    assert_eq!(failed.status(), reqwest::StatusCode::ACCEPTED);
    let failed: serde_json::Value = failed.json().await.unwrap();
    let failed_id = failed["operation_id"].as_str().unwrap();
    let mut failure = None;
    for _ in 0..200 {
        let status: serde_json::Value = client
            .get(format!("http://{addr}/v1/operations/{failed_id}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if status["state"] == "FAILED" {
            failure = Some(status);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let failure = failure.expect("failing unique build did not retain its error");
    assert!(failure["error"]["code"].is_string());
    assert!(
        failure["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("Duplicate"))
    );

    server.abort();
    let _ = server.await;
}

#[cfg(feature = "grpc")]
fn grpc_request<T>(value: T) -> tonic::Request<T> {
    let mut request = tonic::Request::new(value);
    request
        .metadata_mut()
        .insert("x-database", "test".parse().unwrap());
    request
}

#[cfg(feature = "grpc")]
#[tokio::test]
async fn grpc_large_create_index_exposes_durable_operation_api() {
    use stellardb::server::grpc::GrpcServer;
    use stellardb::server::grpc::proto;
    use stellardb::server::grpc::proto::query_service_client::QueryServiceClient;
    use tonic::transport::Channel;

    let tmp = tempfile::tempdir().unwrap();
    let namespace = Arc::new(Namespace::open(tmp.path()).unwrap());
    let database = namespace.create_database(TEST_DB).unwrap();
    database.get_or_create_collection("grpc_items").unwrap();
    database
        .get_or_create_collection("grpc_stream_items")
        .unwrap();
    database.get_or_create_collection("grpc_dupes").unwrap();

    let mut writes = Vec::new();
    for collection in ["grpc_items", "grpc_stream_items"] {
        writes.extend((0..5_000).map(|number| {
            WriteOp::InsertDoc(Document {
                id: format!("{collection}:{number}"),
                fields: HashMap::from([(
                    "category".to_string(),
                    Value::String(format!("category_{}", number % 20)),
                )]),
            })
        }));
    }
    writes.extend((0..1_000).map(|number| {
        WriteOp::InsertDoc(Document {
            id: format!("grpc_dupes:{number}"),
            fields: HashMap::from([(
                "code".to_string(),
                Value::String(format!("duplicate_{}", number % 2)),
            )]),
        })
    }));
    database.atomic_write_batch(writes).unwrap();

    let (_app, state) = create_router(namespace, None, None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let server = tokio::spawn(async move { GrpcServer::new(state).start(addr).await.unwrap() });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let channel = Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut first_client = QueryServiceClient::new(channel.clone());
    let mut second_client = QueryServiceClient::new(channel.clone());
    let execute = || {
        grpc_request(proto::ExecuteRequest {
            query: "CREATE INDEX ON grpc_items(category)".to_string(),
            session_id: None,
            query_timeout: None,
        })
    };
    let (first, second) = tokio::join!(
        first_client.execute(execute()),
        second_client.execute(execute())
    );
    let first = first.unwrap().into_inner().operation.unwrap();
    let second = second.unwrap().into_inner().operation.unwrap();
    assert_eq!(first.operation_id, second.operation_id);
    assert_eq!(first.operation_type, "CREATE_INDEX");
    assert_eq!(first.total_items, 5_000);

    let mut client = QueryServiceClient::new(channel.clone());
    let operation_id = first.operation_id;
    let mut terminal = None;
    for _ in 0..400 {
        let operation = client
            .get_operation(tonic::Request::new(proto::GetOperationRequest {
                operation_id: operation_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();
        if matches!(operation.state.as_str(), "READY" | "FAILED" | "CANCELLED") {
            terminal = Some(operation);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let terminal = terminal.expect("gRPC DDL operation did not finish");
    assert_eq!(terminal.state, "READY", "{terminal:?}");
    assert_eq!(terminal.processed_items, 5_000);
    assert_eq!(terminal.progress_percent, 100.0);

    let listed = client
        .list_operations(grpc_request(proto::ListOperationsRequest {
            active_only: false,
            limit: Some(10),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed.count, 1);
    assert_eq!(listed.operations[0].operation_id, operation_id);

    let cancel_error = client
        .cancel_operation(tonic::Request::new(proto::CancelOperationRequest {
            operation_id,
        }))
        .await
        .unwrap_err();
    assert_eq!(cancel_error.code(), tonic::Code::FailedPrecondition);

    let failed = client
        .execute(grpc_request(proto::ExecuteRequest {
            query: "CREATE INDEX ON grpc_dupes(code) UNIQUE".to_string(),
            session_id: None,
            query_timeout: None,
        }))
        .await
        .unwrap()
        .into_inner()
        .operation
        .unwrap();
    let mut retained_failure = None;
    for _ in 0..400 {
        let operation = client
            .get_operation(tonic::Request::new(proto::GetOperationRequest {
                operation_id: failed.operation_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();
        if operation.state == "FAILED" {
            retained_failure = operation.error;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let retained_failure = retained_failure.expect("gRPC failed job did not retain its error");
    assert!(!retained_failure.code.is_empty());
    assert!(retained_failure.message.contains("Duplicate"));

    let mut stream = client
        .execute_stream(grpc_request(proto::ExecuteRequest {
            query: "CREATE INDEX ON grpc_stream_items(category)".to_string(),
            session_id: None,
            query_timeout: None,
        }))
        .await
        .unwrap()
        .into_inner();
    let launched = stream.message().await.unwrap().unwrap();
    assert!(launched.payload.is_none());
    let stream_operation = launched.operation.unwrap();
    assert_eq!(stream_operation.total_items, 5_000);
    assert!(stream.message().await.unwrap().is_none());

    let mut stream_finished = false;
    for _ in 0..400 {
        let operation = client
            .get_operation(tonic::Request::new(proto::GetOperationRequest {
                operation_id: stream_operation.operation_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();
        if matches!(operation.state.as_str(), "READY" | "FAILED" | "CANCELLED") {
            assert_eq!(operation.state, "READY", "{operation:?}");
            stream_finished = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        stream_finished,
        "stream-launched DDL operation did not finish"
    );

    server.abort();
    let _ = server.await;
}
