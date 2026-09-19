mod common;

use std::collections::HashMap;

use stellardb::server::grpc::proto;
use stellardb::server::grpc::proto::database_service_client::DatabaseServiceClient;
use stellardb::server::grpc::proto::document_service_client::DocumentServiceClient;
use stellardb::server::grpc::proto::edge_service_client::EdgeServiceClient;
use stellardb::server::grpc::proto::metrics_service_client::MetricsServiceClient;
use stellardb::server::grpc::proto::query_service_client::QueryServiceClient;
use stellardb::server::grpc::proto::value::Kind;

use tonic::Request;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;

/// Helper: create a gRPC channel to the given address.
async fn connect(grpc_addr: std::net::SocketAddr) -> Channel {
    Channel::from_shared(format!("http://{}", grpc_addr))
        .unwrap()
        .connect()
        .await
        .unwrap()
}

/// Helper: attach x-database metadata to a request.
fn with_db<T>(inner: T) -> Request<T> {
    let mut req = Request::new(inner);
    req.metadata_mut()
        .insert("x-database", MetadataValue::from_static("test"));
    req
}

fn with_auth<T>(inner: T, token: &str) -> Request<T> {
    let mut req = Request::new(inner);
    req.metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    req
}

fn with_database_and_auth<T>(inner: T, database: &str, token: &str) -> Request<T> {
    let mut req = with_auth(inner, token);
    req.metadata_mut()
        .insert("x-database", database.parse().unwrap());
    req
}

fn with_db_and_auth<T>(inner: T, token: &str) -> Request<T> {
    with_database_and_auth(inner, "test", token)
}

/// Helper: create a proto Value from a kind.
fn proto_value(kind: Kind) -> proto::Value {
    proto::Value { kind: Some(kind) }
}

/// Helper: run a SQL query via gRPC and return the response.
async fn sql(client: &mut QueryServiceClient<Channel>, query: &str) -> proto::ExecuteResponse {
    client
        .execute(with_db(proto::ExecuteRequest {
            query: query.to_string(),
            session_id: None,
            query_timeout: None,
        }))
        .await
        .unwrap()
        .into_inner()
}

// =============================================================================
// QueryService tests
// =============================================================================

#[tokio::test]
async fn configured_grpc_message_limit_rejects_oversized_requests() {
    let config = stellardb::server::grpc::GrpcConfig {
        max_message_size: 1024,
        ..Default::default()
    };
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc_config(config).await;
    let mut client = QueryServiceClient::new(connect(grpc).await);
    let error = client
        .execute(with_db(proto::ExecuteRequest {
            query: "x".repeat(2 * 1024),
            session_id: None,
            query_timeout: None,
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::OutOfRange);
}

#[tokio::test]
async fn test_query_execute_simple_select() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    let resp = sql(&mut client, "SELECT 1 + 2 AS result").await;

    assert_eq!(resp.completed, 1);
    assert!(resp.error.is_none());
    assert!(!resp.elapsed.is_empty());
    assert_eq!(resp.results.len(), 1);

    let stmt = &resp.results[0];
    assert_eq!(stmt.statement_type, "SELECT");
    assert_eq!(stmt.rows.len(), 1);

    let row = &stmt.rows[0];
    let val = row.fields.get("result").unwrap();
    assert_eq!(val.kind, Some(Kind::IntValue(3)));
}

async fn grpc_timeout_batch_error(
    admission: stellardb::server::admission::AdmissionConfig,
    query_timeout: Option<&str>,
) -> proto::BatchError {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc_admission(admission).await;
    let mut client = QueryServiceClient::new(connect(grpc).await);
    client
        .execute(with_db(proto::ExecuteRequest {
            query: "SELECT VALUE 1".to_string(),
            session_id: None,
            query_timeout: query_timeout.map(str::to_string),
        }))
        .await
        .unwrap()
        .into_inner()
        .error
        .expect("deadline must produce a terminal batch error")
}

#[tokio::test]
async fn grpc_invalid_query_timeout_is_rejected() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);
    let error = client
        .execute(with_db(proto::ExecuteRequest {
            query: "SELECT VALUE 1".to_string(),
            session_id: None,
            query_timeout: Some("tomorrow".to_string()),
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::InvalidArgument);
    assert!(error.message().contains("SDB-ST008"));
}

#[tokio::test]
async fn grpc_shorter_query_timeout_wins() {
    let error = grpc_timeout_batch_error(Default::default(), Some("1ns")).await;
    assert_eq!(error.code, "SDB-QE014");
}

#[tokio::test]
async fn grpc_shorter_db_deadline_wins() {
    let error = grpc_timeout_batch_error(
        stellardb::server::admission::AdmissionConfig {
            db_deadline: std::time::Duration::from_nanos(1),
            ..Default::default()
        },
        Some("1s"),
    )
    .await;
    assert_eq!(error.code, "SDB-SV003");
}

#[tokio::test]
async fn grpc_zero_disables_query_timeout_but_not_db_deadline() {
    let error = grpc_timeout_batch_error(
        stellardb::server::admission::AdmissionConfig {
            db_deadline: std::time::Duration::from_nanos(1),
            ..Default::default()
        },
        Some("0"),
    )
    .await;
    assert_eq!(error.code, "SDB-SV003");
}

#[tokio::test]
async fn grpc_equal_deadlines_use_query_timeout_tie_breaker() {
    let error = grpc_timeout_batch_error(
        stellardb::server::admission::AdmissionConfig {
            db_deadline: std::time::Duration::from_nanos(1),
            ..Default::default()
        },
        Some("1ns"),
    )
    .await;
    assert_eq!(error.code, "SDB-QE014");
}

#[tokio::test]
async fn grpc_explicit_timeout_overrides_session_timeout() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);
    let begin = sql(&mut client, "BEGIN").await;
    let session_id = begin.results[0].session_id.clone().unwrap();

    client
        .execute(with_db(proto::ExecuteRequest {
            query: "SET QUERY_TIMEOUT = 1ns".to_string(),
            session_id: Some(session_id.clone()),
            query_timeout: None,
        }))
        .await
        .unwrap();

    let response = client
        .execute(with_db(proto::ExecuteRequest {
            query: "SELECT VALUE 1".to_string(),
            session_id: Some(session_id.clone()),
            query_timeout: Some("1s".to_string()),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(response.error.is_none(), "{response:?}");

    client
        .execute(with_db(proto::ExecuteRequest {
            query: "ROLLBACK".to_string(),
            session_id: Some(session_id),
            query_timeout: Some("0".to_string()),
        }))
        .await
        .unwrap();
}

#[tokio::test]
async fn grpc_stream_uses_the_same_query_timeout_semantics() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);
    let mut stream = client
        .execute_stream(with_db(proto::ExecuteRequest {
            query: "SELECT VALUE 1".to_string(),
            session_id: None,
            query_timeout: Some("1ns".to_string()),
        }))
        .await
        .unwrap()
        .into_inner();
    let item = stream.message().await.unwrap().expect("batch error row");
    let Some(proto::stream_row::Payload::Error(error)) = item.payload else {
        panic!("expected terminal batch error");
    };
    assert_eq!(error.code, "SDB-QE014");
}

#[tokio::test]
async fn grpc_transport_deadline_routes_ddl_to_operation_contract() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);
    sql(&mut client, "DEFINE COLLECTION grpc_operation").await;

    let mut request = with_db(proto::ExecuteRequest {
        query: "CREATE INDEX ON grpc_operation (name)".to_string(),
        session_id: None,
        query_timeout: None,
    });
    request.set_timeout(std::time::Duration::from_secs(30));
    let response = client.execute(request).await.unwrap().into_inner();
    let operation = response.operation.expect("DDL must return an operation");
    assert!(operation.operation_id.starts_with("op_"));

    let status = client
        .get_operation(with_db(proto::GetOperationRequest {
            operation_id: operation.operation_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(status.operation_id, operation.operation_id);
    assert!(matches!(
        status.state.as_str(),
        "QUEUED" | "BUILDING" | "READY" | "FAILED"
    ));
}

#[tokio::test]
async fn test_query_execute_create_and_insert() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    let resp = sql(
        &mut client,
        "DEFINE COLLECTION users; INSERT INTO users { name: 'Alice', age: 30 }",
    )
    .await;

    assert_eq!(resp.completed, 2);
    assert!(resp.error.is_none());

    let insert_result = &resp.results[1];
    assert_eq!(insert_result.statement_type, "INSERT");
    assert_eq!(insert_result.rows.len(), 1);

    let row = &insert_result.rows[0];
    let name = row.fields.get("name").unwrap();
    assert_eq!(name.kind, Some(Kind::StringValue("Alice".to_string())));
}

#[tokio::test]
async fn test_query_execute_multi_statement_batch() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    let resp = sql(&mut client, "SELECT 1 AS a; SELECT 2 AS b; SELECT 3 AS c").await;

    assert_eq!(resp.completed, 3);
    assert!(resp.error.is_none());
    assert_eq!(resp.results.len(), 3);

    for (i, stmt) in resp.results.iter().enumerate() {
        assert_eq!(stmt.statement_index, i as i32);
        assert_eq!(stmt.rows.len(), 1);
    }
}

#[tokio::test]
async fn test_query_execute_invalid_sql() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    let result = client
        .execute(with_db(proto::ExecuteRequest {
            query: "THIS IS NOT SQL".to_string(),
            session_id: None,
            query_timeout: None,
        }))
        .await;

    assert!(result.is_err());
    let status = result.unwrap_err();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
}

#[tokio::test]
async fn test_query_execute_stream_multiple_rows() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    // Create collection with multiple documents
    sql(
        &mut client,
        "DEFINE COLLECTION items; \
         INSERT INTO items { name: 'a' }; \
         INSERT INTO items { name: 'b' }; \
         INSERT INTO items { name: 'c' }",
    )
    .await;

    // Stream SELECT
    let response = client
        .execute_stream(with_db(proto::ExecuteRequest {
            query: "SELECT * FROM items".to_string(),
            session_id: None,
            query_timeout: None,
        }))
        .await
        .unwrap();

    let mut stream = response.into_inner();
    let mut headers = vec![];
    let mut rows = vec![];
    let mut footers = vec![];

    while let Some(msg) = stream.message().await.unwrap() {
        match msg.payload {
            Some(proto::stream_row::Payload::Header(h)) => headers.push(h),
            Some(proto::stream_row::Payload::Row(r)) => rows.push(r),
            Some(proto::stream_row::Payload::Footer(f)) => footers.push(f),
            Some(proto::stream_row::Payload::Error(_)) => panic!("unexpected stream error"),
            None => {}
        }
    }

    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].statement_type, "SELECT");
    assert_eq!(rows.len(), 3);
    assert_eq!(footers.len(), 1);
    assert_eq!(footers[0].count, Some(3));
    assert!(!footers[0].elapsed.is_empty());
}

#[tokio::test]
async fn test_query_execute_stream_multi_statement() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    let response = client
        .execute_stream(with_db(proto::ExecuteRequest {
            query: "SELECT 1 AS x; SELECT 2 AS y".to_string(),
            session_id: None,
            query_timeout: None,
        }))
        .await
        .unwrap();

    let mut stream = response.into_inner();
    let mut headers = vec![];
    let mut footers = vec![];

    while let Some(msg) = stream.message().await.unwrap() {
        match msg.payload {
            Some(proto::stream_row::Payload::Header(h)) => headers.push(h),
            Some(proto::stream_row::Payload::Footer(f)) => footers.push(f),
            _ => {}
        }
    }

    // Two statements -> two headers + two footers
    assert_eq!(headers.len(), 2);
    assert_eq!(footers.len(), 2);
    assert_eq!(headers[0].statement_index, 0);
    assert_eq!(headers[1].statement_index, 1);
}

#[tokio::test]
async fn test_query_session_support() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    // Create collection outside transaction (DDL doesn't work inside transactions)
    sql(&mut client, "DEFINE COLLECTION session_test").await;

    // BEGIN transaction
    let resp = client
        .execute(with_db(proto::ExecuteRequest {
            query: "BEGIN".to_string(),
            session_id: None,
            query_timeout: None,
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.completed, 1);
    let session_id = resp.results[0].session_id.as_ref().unwrap().clone();
    assert!(!session_id.is_empty());

    // INSERT within transaction
    let resp = client
        .execute(with_db(proto::ExecuteRequest {
            query: "INSERT INTO session_test {id: 'tx_item', name: 'Test'}".to_string(),
            session_id: Some(session_id.clone()),
            query_timeout: None,
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.completed, 1);

    // COMMIT
    let resp = client
        .execute(with_db(proto::ExecuteRequest {
            query: "COMMIT".to_string(),
            session_id: Some(session_id),
            query_timeout: None,
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.completed, 1);
    assert_eq!(resp.results[0].statement_type, "COMMIT");

    // Verify committed data is visible
    let resp = sql(&mut client, "SELECT * FROM session_test:tx_item").await;
    assert_eq!(resp.completed, 1);
    assert_eq!(resp.results[0].rows.len(), 1);
}

// =============================================================================
// DocumentService tests
// =============================================================================

#[tokio::test]
async fn test_document_create_and_get() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut doc_client = DocumentServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    // Create collection first via SQL
    sql(&mut query_client, "DEFINE COLLECTION users").await;

    // Create document
    let mut fields = HashMap::new();
    fields.insert(
        "name".to_string(),
        proto_value(Kind::StringValue("Alice".to_string())),
    );
    fields.insert("age".to_string(), proto_value(Kind::IntValue(30)));

    let created = doc_client
        .create(with_db(proto::CreateDocumentRequest {
            collection: "users".to_string(),
            fields,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(created.id.starts_with("users:"));
    assert_eq!(
        created.fields.get("name").unwrap().kind,
        Some(Kind::StringValue("Alice".to_string()))
    );
    assert_eq!(
        created.fields.get("age").unwrap().kind,
        Some(Kind::IntValue(30))
    );

    // Get the document back
    let key = created.id.split_once(':').unwrap().1.to_string();
    let fetched = doc_client
        .get(with_db(proto::GetDocumentRequest {
            collection: "users".to_string(),
            key,
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(fetched.id, created.id);
    assert_eq!(
        fetched.fields.get("name").unwrap().kind,
        Some(Kind::StringValue("Alice".to_string()))
    );
}

#[tokio::test]
async fn test_document_get_not_found() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = DocumentServiceClient::new(connect(grpc).await);

    let result = client
        .get(with_db(proto::GetDocumentRequest {
            collection: "nonexistent".to_string(),
            key: "nope".to_string(),
        }))
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn test_document_update() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut doc_client = DocumentServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(&mut query_client, "DEFINE COLLECTION items").await;

    let mut fields = HashMap::new();
    fields.insert("value".to_string(), proto_value(Kind::IntValue(1)));
    let created = doc_client
        .create(with_db(proto::CreateDocumentRequest {
            collection: "items".to_string(),
            fields,
        }))
        .await
        .unwrap()
        .into_inner();

    let key = created.id.split_once(':').unwrap().1.to_string();

    // Update
    let mut new_fields = HashMap::new();
    new_fields.insert("value".to_string(), proto_value(Kind::IntValue(42)));
    new_fields.insert(
        "extra".to_string(),
        proto_value(Kind::StringValue("hello".to_string())),
    );

    let updated = doc_client
        .update(with_db(proto::UpdateDocumentRequest {
            collection: "items".to_string(),
            key,
            fields: new_fields,
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        updated.fields.get("value").unwrap().kind,
        Some(Kind::IntValue(42))
    );
    assert_eq!(
        updated.fields.get("extra").unwrap().kind,
        Some(Kind::StringValue("hello".to_string()))
    );
}

#[tokio::test]
async fn test_document_delete() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut doc_client = DocumentServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(&mut query_client, "DEFINE COLLECTION items").await;

    let mut fields = HashMap::new();
    fields.insert("x".to_string(), proto_value(Kind::IntValue(1)));
    let created = doc_client
        .create(with_db(proto::CreateDocumentRequest {
            collection: "items".to_string(),
            fields,
        }))
        .await
        .unwrap()
        .into_inner();

    let key = created.id.split_once(':').unwrap().1.to_string();

    // Delete
    doc_client
        .delete(with_db(proto::DeleteDocumentRequest {
            collection: "items".to_string(),
            key: key.clone(),
        }))
        .await
        .unwrap();

    // Verify it's gone
    let result = doc_client
        .get(with_db(proto::GetDocumentRequest {
            collection: "items".to_string(),
            key,
        }))
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn test_document_list_streaming() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut doc_client = DocumentServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(&mut query_client, "DEFINE COLLECTION items").await;

    // Create 5 documents
    for i in 0..5 {
        let mut fields = HashMap::new();
        fields.insert("idx".to_string(), proto_value(Kind::IntValue(i)));
        doc_client
            .create(with_db(proto::CreateDocumentRequest {
                collection: "items".to_string(),
                fields,
            }))
            .await
            .unwrap();
    }

    // List all
    let response = doc_client
        .list(with_db(proto::ListDocumentsRequest {
            collection: "items".to_string(),
            pagination: Some(proto::PaginationRequest {
                limit: 50,
                after: None,
            }),
        }))
        .await
        .unwrap();

    let mut stream = response.into_inner();
    let mut docs = vec![];
    let mut has_more_received = false;

    while let Some(item) = stream.message().await.unwrap() {
        match item.payload {
            Some(proto::document_list_item::Payload::Document(doc)) => docs.push(doc),
            Some(proto::document_list_item::Payload::HasMore(hm)) => {
                has_more_received = true;
                assert!(!hm); // 5 docs < 50 limit
            }
            None => {}
        }
    }

    assert_eq!(docs.len(), 5);
    assert!(has_more_received);
}

#[tokio::test]
async fn test_document_list_pagination() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut doc_client = DocumentServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(&mut query_client, "DEFINE COLLECTION items").await;

    // Create 5 documents
    for i in 0..5 {
        let mut fields = HashMap::new();
        fields.insert("idx".to_string(), proto_value(Kind::IntValue(i)));
        doc_client
            .create(with_db(proto::CreateDocumentRequest {
                collection: "items".to_string(),
                fields,
            }))
            .await
            .unwrap();
    }

    // List with limit of 2
    let response = doc_client
        .list(with_db(proto::ListDocumentsRequest {
            collection: "items".to_string(),
            pagination: Some(proto::PaginationRequest {
                limit: 2,
                after: None,
            }),
        }))
        .await
        .unwrap();

    let mut stream = response.into_inner();
    let mut docs = vec![];
    let mut has_more_value = None;

    while let Some(item) = stream.message().await.unwrap() {
        match item.payload {
            Some(proto::document_list_item::Payload::Document(doc)) => docs.push(doc),
            Some(proto::document_list_item::Payload::HasMore(hm)) => has_more_value = Some(hm),
            None => {}
        }
    }

    assert_eq!(docs.len(), 2);
    assert_eq!(has_more_value, Some(true));
}

// =============================================================================
// EdgeService tests
// =============================================================================

#[tokio::test]
async fn test_edge_create_and_get() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut edge_client = EdgeServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    // Setup: create collection and documents
    sql(
        &mut query_client,
        r#"DEFINE COLLECTION users;
           CREATE users:alice SET name = "Alice";
           CREATE users:bob SET name = "Bob""#,
    )
    .await;

    // Create edge with fields
    let mut fields = HashMap::new();
    fields.insert("since".to_string(), proto_value(Kind::IntValue(2024)));

    let created = edge_client
        .create(with_db(proto::CreateEdgeRequest {
            from: "users:alice".to_string(),
            label: "follows".to_string(),
            to: "users:bob".to_string(),
            fields,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(created.id.starts_with("follows:"));
    assert_eq!(created.from, "users:alice");
    assert_eq!(created.to, "users:bob");
    assert_eq!(created.label, "follows");
    assert_eq!(
        created.fields.get("since").unwrap().kind,
        Some(Kind::IntValue(2024))
    );

    // Get the edge back
    let fetched = edge_client
        .get(with_db(proto::GetEdgeRequest {
            from: "users:alice".to_string(),
            label: "follows".to_string(),
            to: "users:bob".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(fetched.from, "users:alice");
    assert_eq!(fetched.to, "users:bob");
    assert_eq!(fetched.label, "follows");
}

#[tokio::test]
async fn test_edge_get_not_found() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = EdgeServiceClient::new(connect(grpc).await);

    let result = client
        .get(with_db(proto::GetEdgeRequest {
            from: "a:1".to_string(),
            label: "nope".to_string(),
            to: "b:1".to_string(),
        }))
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn test_edge_delete() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut edge_client = EdgeServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(
        &mut query_client,
        "DEFINE COLLECTION users; \
         CREATE users:alice SET name = 'Alice'; \
         CREATE users:bob SET name = 'Bob'",
    )
    .await;

    // Create edge
    edge_client
        .create(with_db(proto::CreateEdgeRequest {
            from: "users:alice".to_string(),
            label: "follows".to_string(),
            to: "users:bob".to_string(),
            fields: HashMap::new(),
        }))
        .await
        .unwrap();

    // Delete it
    edge_client
        .delete(with_db(proto::DeleteEdgeRequest {
            from: "users:alice".to_string(),
            label: "follows".to_string(),
            to: "users:bob".to_string(),
        }))
        .await
        .unwrap();

    // Verify it's gone
    let result = edge_client
        .get(with_db(proto::GetEdgeRequest {
            from: "users:alice".to_string(),
            label: "follows".to_string(),
            to: "users:bob".to_string(),
        }))
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn test_edge_list_out() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut edge_client = EdgeServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(
        &mut query_client,
        "DEFINE COLLECTION users; \
         CREATE users:alice SET name = 'Alice'; \
         CREATE users:bob SET name = 'Bob'; \
         CREATE users:charlie SET name = 'Charlie'",
    )
    .await;

    // Create outgoing edges from alice
    for to in &["users:bob", "users:charlie"] {
        edge_client
            .create(with_db(proto::CreateEdgeRequest {
                from: "users:alice".to_string(),
                label: "follows".to_string(),
                to: to.to_string(),
                fields: HashMap::new(),
            }))
            .await
            .unwrap();
    }

    // List outgoing edges
    let response = edge_client
        .list_out(with_db(proto::ListEdgesRequest {
            node_id: "users:alice".to_string(),
            label: "follows".to_string(),
            pagination: Some(proto::PaginationRequest {
                limit: 50,
                after: None,
            }),
        }))
        .await
        .unwrap();

    let mut stream = response.into_inner();
    let mut edges = vec![];

    while let Some(item) = stream.message().await.unwrap() {
        if let Some(proto::edge_list_item::Payload::Edge(e)) = item.payload {
            edges.push(e);
        }
    }

    assert_eq!(edges.len(), 2);
    assert!(edges.iter().all(|e| e.from == "users:alice"));
}

#[tokio::test]
async fn test_edge_list_in() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut edge_client = EdgeServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(
        &mut query_client,
        "DEFINE COLLECTION users; \
         CREATE users:alice SET name = 'Alice'; \
         CREATE users:bob SET name = 'Bob'; \
         CREATE users:charlie SET name = 'Charlie'",
    )
    .await;

    // Both bob and charlie follow alice
    for from in &["users:bob", "users:charlie"] {
        edge_client
            .create(with_db(proto::CreateEdgeRequest {
                from: from.to_string(),
                label: "follows".to_string(),
                to: "users:alice".to_string(),
                fields: HashMap::new(),
            }))
            .await
            .unwrap();
    }

    // List incoming edges to alice
    let response = edge_client
        .list_in(with_db(proto::ListEdgesRequest {
            node_id: "users:alice".to_string(),
            label: "follows".to_string(),
            pagination: Some(proto::PaginationRequest {
                limit: 50,
                after: None,
            }),
        }))
        .await
        .unwrap();

    let mut stream = response.into_inner();
    let mut edges = vec![];

    while let Some(item) = stream.message().await.unwrap() {
        if let Some(proto::edge_list_item::Payload::Edge(e)) = item.payload {
            edges.push(e);
        }
    }

    assert_eq!(edges.len(), 2);
    assert!(edges.iter().all(|e| e.to == "users:alice"));
}

// =============================================================================
// DatabaseService tests
// =============================================================================

#[tokio::test]
async fn test_database_list() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = DatabaseServiceClient::new(connect(grpc).await);

    let resp = client.list(Request::new(())).await.unwrap().into_inner();

    assert!(resp.databases.contains(&"test".to_string()));
}

#[tokio::test]
async fn test_database_create() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = DatabaseServiceClient::new(connect(grpc).await);

    let resp = client
        .create(Request::new(proto::CreateDatabaseRequest {
            name: "mydb".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.name, "mydb");
    assert_eq!(resp.status, "created");

    // Verify it appears in list
    let list = client.list(Request::new(())).await.unwrap().into_inner();
    assert!(list.databases.contains(&"mydb".to_string()));
}

#[tokio::test]
async fn test_database_create_duplicate() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = DatabaseServiceClient::new(connect(grpc).await);

    // "test" already exists
    let result = client
        .create(Request::new(proto::CreateDatabaseRequest {
            name: "test".to_string(),
        }))
        .await;

    assert!(result.is_err());
    let status = result.unwrap_err();
    assert_eq!(status.code(), tonic::Code::AlreadyExists);
}

#[tokio::test]
async fn test_database_drop() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = DatabaseServiceClient::new(connect(grpc).await);

    // Create then drop
    client
        .create(Request::new(proto::CreateDatabaseRequest {
            name: "to_drop".to_string(),
        }))
        .await
        .unwrap();

    client
        .drop(Request::new(proto::DropDatabaseRequest {
            name: "to_drop".to_string(),
        }))
        .await
        .unwrap();

    // Verify it's gone
    let list = client.list(Request::new(())).await.unwrap().into_inner();
    assert!(!list.databases.contains(&"to_drop".to_string()));
}

#[tokio::test]
async fn test_database_stats() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = DatabaseServiceClient::new(connect(grpc).await);

    let resp = client
        .stats(Request::new(proto::StatsRequest {
            name: "test".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.name, "test");
    assert_eq!(resp.collections, 0);
    assert!(!resp.disk_size_human.is_empty());
}

// =============================================================================
// MetricsService tests
// =============================================================================

#[tokio::test]
async fn test_metrics_get_stats() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = MetricsServiceClient::new(connect(grpc).await);

    let resp = client
        .get_stats(Request::new(()))
        .await
        .unwrap()
        .into_inner();

    assert!(!resp.json.is_empty());
    // Should be valid JSON
    let parsed: serde_json::Value = serde_json::from_str(&resp.json).unwrap();
    assert!(parsed.is_object());
}

#[tokio::test]
async fn test_metrics_get_prometheus() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = MetricsServiceClient::new(connect(grpc).await);

    let resp = client
        .get_prometheus(Request::new(()))
        .await
        .unwrap()
        .into_inner();

    // Prometheus format is plain text
    assert!(resp.text.is_ascii() || resp.text.is_empty());
}

// =============================================================================
// Error handling tests
// =============================================================================

#[tokio::test]
async fn test_missing_database_metadata() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    // Send request WITHOUT x-database metadata
    let result = client
        .execute(Request::new(proto::ExecuteRequest {
            query: "SELECT 1".to_string(),
            session_id: None,
            query_timeout: None,
        }))
        .await;

    assert!(result.is_err());
    let status = result.unwrap_err();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    assert!(status.message().contains("x-database"));
}

#[tokio::test]
async fn test_nonexistent_database() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    let mut req = Request::new(proto::ExecuteRequest {
        query: "SELECT 1".to_string(),
        session_id: None,
        query_timeout: None,
    });
    req.metadata_mut()
        .insert("x-database", MetadataValue::from_static("does_not_exist"));

    let result = client.execute(req).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_document_empty_collection_name() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = DocumentServiceClient::new(connect(grpc).await);

    let result = client
        .get(with_db(proto::GetDocumentRequest {
            collection: "".to_string(),
            key: "key".to_string(),
        }))
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
}

// =============================================================================
// Value type round-trip tests
// =============================================================================

#[tokio::test]
async fn test_value_roundtrip_types() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut doc_client = DocumentServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(&mut query_client, "DEFINE COLLECTION typed").await;

    // Create document with various value types
    let mut fields = HashMap::new();
    fields.insert(
        "str_val".to_string(),
        proto_value(Kind::StringValue("hello".to_string())),
    );
    fields.insert("int_val".to_string(), proto_value(Kind::IntValue(42)));
    fields.insert("float_val".to_string(), proto_value(Kind::FloatValue(3.5)));
    fields.insert("bool_val".to_string(), proto_value(Kind::BoolValue(true)));
    fields.insert("null_val".to_string(), proto_value(Kind::NullValue(0)));
    fields.insert(
        "bytes_val".to_string(),
        proto_value(Kind::BytesValue(vec![1, 2, 3, 4])),
    );
    fields.insert(
        "datetime_val".to_string(),
        proto_value(Kind::DatetimeValue(1700000000000)),
    );
    fields.insert(
        "array_val".to_string(),
        proto_value(Kind::ArrayValue(proto::ArrayValue {
            values: vec![
                proto_value(Kind::IntValue(1)),
                proto_value(Kind::IntValue(2)),
                proto_value(Kind::IntValue(3)),
            ],
        })),
    );

    let mut inner_fields = HashMap::new();
    inner_fields.insert(
        "nested".to_string(),
        proto_value(Kind::StringValue("deep".to_string())),
    );
    fields.insert(
        "object_val".to_string(),
        proto_value(Kind::ObjectValue(proto::ObjectValue {
            fields: inner_fields,
        })),
    );

    let created = doc_client
        .create(with_db(proto::CreateDocumentRequest {
            collection: "typed".to_string(),
            fields,
        }))
        .await
        .unwrap()
        .into_inner();

    // Read it back
    let key = created.id.split_once(':').unwrap().1.to_string();
    let fetched = doc_client
        .get(with_db(proto::GetDocumentRequest {
            collection: "typed".to_string(),
            key,
        }))
        .await
        .unwrap()
        .into_inner();

    // Verify each field type survived the round-trip
    assert_eq!(
        fetched.fields.get("str_val").unwrap().kind,
        Some(Kind::StringValue("hello".to_string()))
    );
    assert_eq!(
        fetched.fields.get("int_val").unwrap().kind,
        Some(Kind::IntValue(42))
    );
    assert_eq!(
        fetched.fields.get("float_val").unwrap().kind,
        Some(Kind::FloatValue(3.5))
    );
    assert_eq!(
        fetched.fields.get("bool_val").unwrap().kind,
        Some(Kind::BoolValue(true))
    );

    // Null: when stored and retrieved, the field might be absent or null
    let null_field = fetched.fields.get("null_val");
    if let Some(v) = null_field {
        match &v.kind {
            Some(Kind::NullValue(_)) | None => {}
            other => panic!("expected null, got {:?}", other),
        }
    }

    // Bytes: stored via serde, comes back through the conversion layer
    let bytes_field = fetched.fields.get("bytes_val");
    assert!(bytes_field.is_some());

    // Datetime: stored as i64 millis
    let dt_field = fetched.fields.get("datetime_val");
    assert!(dt_field.is_some());

    // Array
    let arr = fetched.fields.get("array_val").unwrap();
    match &arr.kind {
        Some(Kind::ArrayValue(av)) => assert_eq!(av.values.len(), 3),
        other => panic!("expected array, got {:?}", other),
    }

    // Object
    let obj = fetched.fields.get("object_val").unwrap();
    match &obj.kind {
        Some(Kind::ObjectValue(ov)) => {
            assert_eq!(
                ov.fields.get("nested").unwrap().kind,
                Some(Kind::StringValue("deep".to_string()))
            );
        }
        other => panic!("expected object, got {:?}", other),
    }
}

// =============================================================================
// QueryService — advanced SQL tests
// =============================================================================

#[tokio::test]
async fn test_query_where_filter() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(
        &mut client,
        "DEFINE COLLECTION products; \
         INSERT INTO products { name: 'Apple', price: 1.5 }; \
         INSERT INTO products { name: 'Banana', price: 0.5 }; \
         INSERT INTO products { name: 'Cherry', price: 3.0 }",
    )
    .await;

    let resp = sql(
        &mut client,
        "SELECT * FROM products WHERE price > 1.0 ORDER price ASC",
    )
    .await;
    assert_eq!(resp.results[0].rows.len(), 2);

    let names: Vec<_> = resp.results[0]
        .rows
        .iter()
        .map(|r| match &r.fields.get("name").unwrap().kind {
            Some(Kind::StringValue(s)) => s.clone(),
            other => panic!("expected string, got {:?}", other),
        })
        .collect();
    assert_eq!(names, vec!["Apple", "Cherry"]);
}

#[tokio::test]
async fn test_query_order_by_desc() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(
        &mut client,
        "DEFINE COLLECTION nums; \
         INSERT INTO nums { val: 3 }; \
         INSERT INTO nums { val: 1 }; \
         INSERT INTO nums { val: 2 }",
    )
    .await;

    let resp = sql(&mut client, "SELECT val FROM nums ORDER val DESC").await;
    let vals: Vec<i64> = resp.results[0]
        .rows
        .iter()
        .map(|r| match r.fields.get("val").unwrap().kind {
            Some(Kind::IntValue(v)) => v,
            _ => panic!("expected int"),
        })
        .collect();
    assert_eq!(vals, vec![3, 2, 1]);
}

#[tokio::test]
async fn test_query_limit_offset() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(
        &mut client,
        "DEFINE COLLECTION items; \
         INSERT INTO items { idx: 0 }; \
         INSERT INTO items { idx: 1 }; \
         INSERT INTO items { idx: 2 }; \
         INSERT INTO items { idx: 3 }; \
         INSERT INTO items { idx: 4 }",
    )
    .await;

    let resp = sql(
        &mut client,
        "SELECT idx FROM items ORDER idx ASC LIMIT 2 OFFSET 1",
    )
    .await;
    assert_eq!(resp.results[0].rows.len(), 2);

    let vals: Vec<i64> = resp.results[0]
        .rows
        .iter()
        .map(|r| match r.fields.get("idx").unwrap().kind {
            Some(Kind::IntValue(v)) => v,
            _ => panic!("expected int"),
        })
        .collect();
    assert_eq!(vals, vec![1, 2]);
}

#[tokio::test]
async fn test_query_group_by_with_aggregates() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(
        &mut client,
        "DEFINE COLLECTION sales; \
         INSERT INTO sales { category: 'A', amount: 10 }; \
         INSERT INTO sales { category: 'B', amount: 20 }; \
         INSERT INTO sales { category: 'A', amount: 30 }; \
         INSERT INTO sales { category: 'B', amount: 40 }",
    )
    .await;

    let resp = sql(
        &mut client,
        "SELECT category, SUM(amount) AS total, COUNT(*) AS cnt \
         FROM sales GROUP category ORDER category ASC",
    )
    .await;

    assert_eq!(resp.results[0].rows.len(), 2);

    let row_a = &resp.results[0].rows[0];
    assert_eq!(
        row_a.fields.get("category").unwrap().kind,
        Some(Kind::StringValue("A".to_string()))
    );
    // SUM returns Decimal for precision
    assert_eq!(
        row_a.fields.get("total").unwrap().kind,
        Some(Kind::DecimalValue("40".to_string()))
    );
    assert_eq!(
        row_a.fields.get("cnt").unwrap().kind,
        Some(Kind::IntValue(2))
    );
}

#[tokio::test]
async fn test_query_distinct() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(
        &mut client,
        "DEFINE COLLECTION tags; \
         INSERT INTO tags { tag: 'rust' }; \
         INSERT INTO tags { tag: 'go' }; \
         INSERT INTO tags { tag: 'rust' }; \
         INSERT INTO tags { tag: 'go' }; \
         INSERT INTO tags { tag: 'python' }",
    )
    .await;

    let resp = sql(&mut client, "SELECT DISTINCT tag FROM tags ORDER tag ASC").await;
    assert_eq!(resp.results[0].rows.len(), 3);

    let tags: Vec<_> = resp.results[0]
        .rows
        .iter()
        .map(|r| match &r.fields.get("tag").unwrap().kind {
            Some(Kind::StringValue(s)) => s.clone(),
            _ => panic!("expected string"),
        })
        .collect();
    assert_eq!(tags, vec!["go", "python", "rust"]);
}

#[tokio::test]
async fn test_query_expressions_and_math_functions() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    let resp = sql(
        &mut client,
        "SELECT math::abs(-5) AS a, math::round(3.7) AS b, math::floor(3.9) AS c, math::ceil(3.1) AS d, math::sqrt(16.0) AS e",
    )
    .await;

    let row = &resp.results[0].rows[0];
    assert_eq!(row.fields.get("a").unwrap().kind, Some(Kind::IntValue(5)));
    // math functions may return int when result is whole number
    assert!(matches!(
        row.fields.get("b").unwrap().kind,
        Some(Kind::IntValue(4)) | Some(Kind::FloatValue(4.0))
    ));
    assert!(matches!(
        row.fields.get("c").unwrap().kind,
        Some(Kind::IntValue(3)) | Some(Kind::FloatValue(3.0))
    ));
    assert!(matches!(
        row.fields.get("d").unwrap().kind,
        Some(Kind::IntValue(4)) | Some(Kind::FloatValue(4.0))
    ));
    assert_eq!(
        row.fields.get("e").unwrap().kind,
        Some(Kind::FloatValue(4.0))
    );
}

#[tokio::test]
async fn test_query_string_functions() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    let resp = sql(
        &mut client,
        "SELECT string::upper('hello') AS u, string::lower('WORLD') AS l, string::length('test') AS len, \
         string::concat('a', 'b', 'c') AS c, string::trim('  hi  ') AS t",
    )
    .await;

    let row = &resp.results[0].rows[0];
    assert_eq!(
        row.fields.get("u").unwrap().kind,
        Some(Kind::StringValue("HELLO".to_string()))
    );
    assert_eq!(
        row.fields.get("l").unwrap().kind,
        Some(Kind::StringValue("world".to_string()))
    );
    assert_eq!(row.fields.get("len").unwrap().kind, Some(Kind::IntValue(4)));
    assert_eq!(
        row.fields.get("c").unwrap().kind,
        Some(Kind::StringValue("abc".to_string()))
    );
    assert_eq!(
        row.fields.get("t").unwrap().kind,
        Some(Kind::StringValue("hi".to_string()))
    );
}

#[tokio::test]
async fn test_query_type_functions() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    let resp = sql(
        &mut client,
        "SELECT type::is_int(42) AS a, type::is_string('hi') AS b, type::is_null(null) AS c, \
         type::is_float(3.14) AS d, type::is_bool(true) AS e, type::of(42) AS f",
    )
    .await;

    let row = &resp.results[0].rows[0];
    assert_eq!(
        row.fields.get("a").unwrap().kind,
        Some(Kind::BoolValue(true))
    );
    assert_eq!(
        row.fields.get("b").unwrap().kind,
        Some(Kind::BoolValue(true))
    );
    assert_eq!(
        row.fields.get("c").unwrap().kind,
        Some(Kind::BoolValue(true))
    );
    assert_eq!(
        row.fields.get("d").unwrap().kind,
        Some(Kind::BoolValue(true))
    );
    assert_eq!(
        row.fields.get("e").unwrap().kind,
        Some(Kind::BoolValue(true))
    );
    assert_eq!(
        row.fields.get("f").unwrap().kind,
        Some(Kind::StringValue("int".to_string()))
    );
}

#[tokio::test]
async fn test_query_coalesce() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    let resp = sql(&mut client, "SELECT null ?? null ?? 'fallback' AS val").await;

    assert_eq!(
        resp.results[0].rows[0].fields.get("val").unwrap().kind,
        Some(Kind::StringValue("fallback".to_string()))
    );
}

#[tokio::test]
async fn test_query_in_operator() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(
        &mut client,
        "DEFINE COLLECTION colors; \
         INSERT INTO colors { name: 'red' }; \
         INSERT INTO colors { name: 'blue' }; \
         INSERT INTO colors { name: 'green' }",
    )
    .await;

    let resp = sql(
        &mut client,
        "SELECT name FROM colors WHERE name IN ['red', 'green'] ORDER name ASC",
    )
    .await;

    assert_eq!(resp.results[0].rows.len(), 2);
    let names: Vec<_> = resp.results[0]
        .rows
        .iter()
        .map(|r| match &r.fields.get("name").unwrap().kind {
            Some(Kind::StringValue(s)) => s.clone(),
            _ => panic!("expected string"),
        })
        .collect();
    assert_eq!(names, vec!["green", "red"]);
}

#[tokio::test]
async fn test_query_let_variables() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    let resp = sql(&mut client, "LET x = 42; SELECT $x AS val").await;

    assert_eq!(resp.completed, 2);
    assert_eq!(
        resp.results[1].rows[0].fields.get("val").unwrap().kind,
        Some(Kind::IntValue(42))
    );
}

#[tokio::test]
async fn test_query_create_with_explicit_id() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(&mut client, "DEFINE COLLECTION users").await;

    let resp = sql(
        &mut client,
        r#"CREATE users:john SET name = "John", age = 25"#,
    )
    .await;

    assert_eq!(resp.results[0].rows.len(), 1);
    let row = &resp.results[0].rows[0];
    assert_eq!(
        row.fields.get("id").unwrap().kind,
        Some(Kind::StringValue("users:john".to_string()))
    );

    // Verify we can query it back
    let resp = sql(&mut client, "SELECT * FROM users:john").await;
    assert_eq!(resp.results[0].rows.len(), 1);
    assert_eq!(
        resp.results[0].rows[0].fields.get("name").unwrap().kind,
        Some(Kind::StringValue("John".to_string()))
    );
}

#[tokio::test]
async fn test_query_update_set() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(
        &mut client,
        r#"DEFINE COLLECTION users; CREATE users:alice SET name = "Alice", age = 30"#,
    )
    .await;

    let resp = sql(&mut client, "UPDATE users:alice SET age = 31").await;
    assert_eq!(resp.results[0].statement_type, "UPDATE");

    let resp = sql(&mut client, "SELECT age FROM users:alice").await;
    assert_eq!(
        resp.results[0].rows[0].fields.get("age").unwrap().kind,
        Some(Kind::IntValue(31))
    );
}

#[tokio::test]
async fn test_query_delete_document() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(
        &mut client,
        r#"DEFINE COLLECTION items; CREATE items:a SET v = 1; CREATE items:b SET v = 2"#,
    )
    .await;

    let resp = sql(&mut client, "DELETE items:a").await;
    assert_eq!(resp.results[0].statement_type, "DELETE");

    let resp = sql(&mut client, "SELECT * FROM items").await;
    assert_eq!(resp.results[0].rows.len(), 1);
}

#[tokio::test]
async fn test_query_upsert() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(&mut client, "DEFINE COLLECTION kv").await;

    // First upsert — creates
    sql(&mut client, r#"UPSERT kv:key1 SET val = "first""#).await;
    let resp = sql(&mut client, "SELECT val FROM kv:key1").await;
    assert_eq!(
        resp.results[0].rows[0].fields.get("val").unwrap().kind,
        Some(Kind::StringValue("first".to_string()))
    );

    // Second upsert — updates
    sql(&mut client, r#"UPSERT kv:key1 SET val = "second""#).await;
    let resp = sql(&mut client, "SELECT val FROM kv:key1").await;
    assert_eq!(
        resp.results[0].rows[0].fields.get("val").unwrap().kind,
        Some(Kind::StringValue("second".to_string()))
    );
}

#[tokio::test]
async fn test_query_ddl_define_and_drop_collection() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    let resp = sql(&mut client, "DEFINE COLLECTION temp_coll").await;
    assert!(resp.results[0].statement_type.starts_with("DEFINE"));

    // Insert something to verify it works
    sql(&mut client, "INSERT INTO temp_coll { val: 1 }").await;

    let resp = sql(&mut client, "DROP COLLECTION temp_coll CASCADE").await;
    assert!(resp.results[0].statement_type.starts_with("DROP"));

    // Verify it's gone — selecting from dropped collection returns empty
    let resp = sql(&mut client, "SELECT * FROM temp_coll").await;
    assert_eq!(resp.results[0].rows.len(), 0);
}

#[tokio::test]
async fn test_query_btree_index_usage() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(
        &mut client,
        "DEFINE COLLECTION indexed; \
         CREATE INDEX ON indexed(score); \
         INSERT INTO indexed { score: 10, name: 'a' }; \
         INSERT INTO indexed { score: 20, name: 'b' }; \
         INSERT INTO indexed { score: 30, name: 'c' }",
    )
    .await;

    // Range query should use the index
    let resp = sql(
        &mut client,
        "SELECT name FROM indexed WHERE score >= 20 ORDER score ASC",
    )
    .await;

    assert_eq!(resp.results[0].rows.len(), 2);
    let names: Vec<_> = resp.results[0]
        .rows
        .iter()
        .map(|r| match &r.fields.get("name").unwrap().kind {
            Some(Kind::StringValue(s)) => s.clone(),
            _ => panic!("expected string"),
        })
        .collect();
    assert_eq!(names, vec!["b", "c"]);
}

#[tokio::test]
async fn test_query_explain() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(&mut client, "DEFINE COLLECTION data").await;

    let resp = sql(&mut client, "EXPLAIN SELECT * FROM data").await;
    assert_eq!(resp.completed, 1);
    // EXPLAIN returns rows describing the plan
    assert!(!resp.results[0].rows.is_empty());
}

#[tokio::test]
async fn test_query_batch_partial_error() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    // First statement succeeds, second fails at bind (unknown function)
    let resp = sql(
        &mut client,
        "SELECT 1 AS ok; SELECT bad::unknown_function() AS fail",
    )
    .await;

    // First statement should complete
    assert_eq!(resp.completed, 1);
    let error = resp.error.expect("second statement must fail");
    assert_eq!(error.statement_index, 1);
    assert_eq!(error.code, "SDB-QE024");
    assert!(error.message.contains("Unknown function"));
}

#[tokio::test]
async fn test_query_empty_result_set() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(&mut client, "DEFINE COLLECTION empty").await;

    let resp = sql(&mut client, "SELECT * FROM empty").await;
    assert_eq!(resp.results[0].rows.len(), 0);
    assert_eq!(resp.results[0].count, Some(0));
}

#[tokio::test]
async fn test_query_nested_array_and_object_in_insert() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(&mut client, "DEFINE COLLECTION complex").await;

    let resp = sql(
        &mut client,
        r#"INSERT INTO complex { tags: ['rust', 'grpc'], meta: { version: 1, active: true } }"#,
    )
    .await;

    let row = &resp.results[0].rows[0];

    // Verify array
    match &row.fields.get("tags").unwrap().kind {
        Some(Kind::ArrayValue(arr)) => {
            assert_eq!(arr.values.len(), 2);
            assert_eq!(
                arr.values[0].kind,
                Some(Kind::StringValue("rust".to_string()))
            );
        }
        other => panic!("expected array, got {:?}", other),
    }

    // Verify object
    match &row.fields.get("meta").unwrap().kind {
        Some(Kind::ObjectValue(obj)) => {
            assert_eq!(
                obj.fields.get("version").unwrap().kind,
                Some(Kind::IntValue(1))
            );
            assert_eq!(
                obj.fields.get("active").unwrap().kind,
                Some(Kind::BoolValue(true))
            );
        }
        other => panic!("expected object, got {:?}", other),
    }
}

#[tokio::test]
async fn test_query_arithmetic_expressions() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    let resp = sql(
        &mut client,
        "SELECT 10 + 5 AS add, 10 - 3 AS sub, 4 * 5 AS mul, 15 / 3 AS div, math::modulo(17, 5) AS modulo",
    )
    .await;

    let row = &resp.results[0].rows[0];
    assert_eq!(
        row.fields.get("add").unwrap().kind,
        Some(Kind::IntValue(15))
    );
    assert_eq!(row.fields.get("sub").unwrap().kind, Some(Kind::IntValue(7)));
    assert_eq!(
        row.fields.get("mul").unwrap().kind,
        Some(Kind::IntValue(20))
    );
    assert_eq!(row.fields.get("div").unwrap().kind, Some(Kind::IntValue(5)));
    assert_eq!(
        row.fields.get("modulo").unwrap().kind,
        Some(Kind::IntValue(2))
    );
}

#[tokio::test]
async fn test_query_boolean_logic() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    let resp = sql(
        &mut client,
        "SELECT (true AND false) AS a, (true OR false) AS b, (NOT true) AS c",
    )
    .await;

    let row = &resp.results[0].rows[0];
    assert_eq!(
        row.fields.get("a").unwrap().kind,
        Some(Kind::BoolValue(false))
    );
    assert_eq!(
        row.fields.get("b").unwrap().kind,
        Some(Kind::BoolValue(true))
    );
    assert_eq!(
        row.fields.get("c").unwrap().kind,
        Some(Kind::BoolValue(false))
    );
}

#[tokio::test]
async fn test_query_is_null_is_not_null() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(
        &mut client,
        "DEFINE COLLECTION nullable; \
         INSERT INTO nullable { name: 'a', val: 1 }; \
         INSERT INTO nullable { name: 'b', val: null }",
    )
    .await;

    let resp = sql(&mut client, "SELECT name FROM nullable WHERE val IS NULL").await;
    assert_eq!(resp.results[0].rows.len(), 1);
    assert_eq!(
        resp.results[0].rows[0].fields.get("name").unwrap().kind,
        Some(Kind::StringValue("b".to_string()))
    );

    let resp = sql(
        &mut client,
        "SELECT name FROM nullable WHERE val IS NOT NULL",
    )
    .await;
    assert_eq!(resp.results[0].rows.len(), 1);
    assert_eq!(
        resp.results[0].rows[0].fields.get("name").unwrap().kind,
        Some(Kind::StringValue("a".to_string()))
    );
}

#[tokio::test]
async fn test_query_count_sum_avg_min_max() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(
        &mut client,
        "DEFINE COLLECTION nums; \
         INSERT INTO nums { v: 10 }; \
         INSERT INTO nums { v: 20 }; \
         INSERT INTO nums { v: 30 }",
    )
    .await;

    let resp = sql(
        &mut client,
        "SELECT COUNT(*) AS c, SUM(v) AS s, AVG(v) AS a, MIN(v) AS mn, MAX(v) AS mx FROM nums",
    )
    .await;

    let row = &resp.results[0].rows[0];
    assert_eq!(row.fields.get("c").unwrap().kind, Some(Kind::IntValue(3)));
    // SUM returns Decimal for precision
    assert_eq!(
        row.fields.get("s").unwrap().kind,
        Some(Kind::DecimalValue("60".to_string()))
    );
    assert_eq!(row.fields.get("mn").unwrap().kind, Some(Kind::IntValue(10)));
    assert_eq!(row.fields.get("mx").unwrap().kind, Some(Kind::IntValue(30)));
    // AVG may return Decimal
    assert!(row.fields.contains_key("a"));
}

#[tokio::test]
async fn test_query_subquery_in_from() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(&mut client, "DEFINE COLLECTION sub_src").await;
    sql(
        &mut client,
        "INSERT INTO sub_src { val: 1 }; INSERT INTO sub_src { val: 2 }; INSERT INTO sub_src { val: 3 }",
    )
    .await;

    let resp = sql(
        &mut client,
        "SELECT val FROM (SELECT val FROM sub_src) ORDER val ASC",
    )
    .await;

    assert_eq!(resp.results[0].rows.len(), 3);
}

// =============================================================================
// QueryService — graph traversal via SQL
// =============================================================================

#[tokio::test]
async fn test_query_relate_and_traverse() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(
        &mut client,
        r#"DEFINE COLLECTION person;
           CREATE person:alice SET name = "Alice";
           CREATE person:bob SET name = "Bob";
           CREATE person:charlie SET name = "Charlie";
           RELATE person:alice->knows->person:bob SET since = 2020;
           RELATE person:alice->knows->person:charlie SET since = 2021;
           RELATE person:bob->knows->person:charlie SET since = 2022"#,
    )
    .await;

    // Traverse outgoing edges from alice
    let resp = sql(
        &mut client,
        "SELECT ->knows->person.* AS friend FROM person:alice",
    )
    .await;

    assert_eq!(resp.completed, 1);
    let row = &resp.results[0].rows[0];
    // Should return friends as array
    match &row.fields.get("friend").unwrap().kind {
        Some(Kind::ArrayValue(arr)) => {
            assert_eq!(arr.values.len(), 2);
        }
        other => panic!("expected array of friends, got {:?}", other),
    }
}

#[tokio::test]
async fn test_query_relate_with_fields() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(
        &mut client,
        r#"DEFINE COLLECTION user;
           CREATE user:a SET name = "A";
           CREATE user:b SET name = "B";
           RELATE user:a->follows->user:b SET weight = 0.9"#,
    )
    .await;

    // Query edge fields
    let resp = sql(&mut client, "SELECT ->follows.weight AS w FROM user:a").await;

    assert_eq!(resp.completed, 1);
    assert!(!resp.results[0].rows.is_empty());
}

#[tokio::test]
async fn test_query_incoming_traversal() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(
        &mut client,
        r#"DEFINE COLLECTION node;
           CREATE node:a SET name = "A";
           CREATE node:b SET name = "B";
           CREATE node:c SET name = "C";
           RELATE node:a->link->node:c;
           RELATE node:b->link->node:c"#,
    )
    .await;

    // Incoming traversal — who links to C?
    let resp = sql(
        &mut client,
        "SELECT <-link<-node.name AS sources FROM node:c",
    )
    .await;

    assert_eq!(resp.completed, 1);
    match &resp.results[0].rows[0].fields.get("sources").unwrap().kind {
        Some(Kind::ArrayValue(arr)) => {
            assert_eq!(arr.values.len(), 2);
        }
        other => panic!("expected array, got {:?}", other),
    }
}

// =============================================================================
// QueryService — streaming advanced tests
// =============================================================================

#[tokio::test]
async fn test_query_stream_empty_result() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(&mut client, "DEFINE COLLECTION empty_stream").await;

    let response = client
        .execute_stream(with_db(proto::ExecuteRequest {
            query: "SELECT * FROM empty_stream".to_string(),
            session_id: None,
            query_timeout: None,
        }))
        .await
        .unwrap();

    let mut stream = response.into_inner();
    let mut headers = 0;
    let mut rows = 0;
    let mut footers = 0;

    while let Some(msg) = stream.message().await.unwrap() {
        match msg.payload {
            Some(proto::stream_row::Payload::Header(_)) => headers += 1,
            Some(proto::stream_row::Payload::Row(_)) => rows += 1,
            Some(proto::stream_row::Payload::Footer(f)) => {
                footers += 1;
                assert_eq!(f.count, Some(0));
            }
            _ => {}
        }
    }

    assert_eq!(headers, 1);
    assert_eq!(rows, 0);
    assert_eq!(footers, 1);
}

#[tokio::test]
async fn test_query_stream_large_result() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(&mut client, "DEFINE COLLECTION bulk").await;

    // Insert 100 documents
    let mut stmts = String::new();
    for i in 0..100 {
        stmts.push_str(&format!("INSERT INTO bulk {{ idx: {} }};", i));
    }
    sql(&mut client, &stmts).await;

    let response = client
        .execute_stream(with_db(proto::ExecuteRequest {
            query: "SELECT * FROM bulk".to_string(),
            session_id: None,
            query_timeout: None,
        }))
        .await
        .unwrap();

    let mut stream = response.into_inner();
    let mut row_count = 0;

    while let Some(msg) = stream.message().await.unwrap() {
        if matches!(msg.payload, Some(proto::stream_row::Payload::Row(_))) {
            row_count += 1;
        }
    }

    assert_eq!(row_count, 100);
}

#[tokio::test]
async fn test_query_stream_ddl_no_rows() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    let response = client
        .execute_stream(with_db(proto::ExecuteRequest {
            query: "DEFINE COLLECTION stream_ddl".to_string(),
            session_id: None,
            query_timeout: None,
        }))
        .await
        .unwrap();

    let mut stream = response.into_inner();
    let mut headers = 0;
    let mut footers = 0;

    while let Some(msg) = stream.message().await.unwrap() {
        match msg.payload {
            Some(proto::stream_row::Payload::Header(h)) => {
                headers += 1;
                assert!(h.statement_type.starts_with("DEFINE"));
            }
            Some(proto::stream_row::Payload::Footer(_)) => footers += 1,
            _ => {}
        }
    }

    assert_eq!(headers, 1);
    assert_eq!(footers, 1);
}

#[tokio::test]
async fn test_query_stream_with_batch_error() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    let response = client
        .execute_stream(with_db(proto::ExecuteRequest {
            query: "SELECT 1 AS ok; SELECT bad::unknown_function() AS fail".to_string(),
            session_id: None,
            query_timeout: None,
        }))
        .await
        .unwrap();

    let mut stream = response.into_inner();
    let mut had_header = false;
    let mut had_error = false;

    while let Some(msg) = stream.message().await.unwrap() {
        match msg.payload {
            Some(proto::stream_row::Payload::Header(_)) => had_header = true,
            Some(proto::stream_row::Payload::Error(e)) => {
                had_error = true;
                assert_eq!(e.statement_index, 1);
                assert_eq!(e.code, "SDB-QE024");
                assert!(e.message.contains("Unknown function"));
            }
            _ => {}
        }
    }

    assert!(had_header); // first statement completed
    assert!(had_error); // second statement errored
}

// =============================================================================
// QueryService — transaction tests
// =============================================================================

#[tokio::test]
async fn test_query_transaction_rollback() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    sql(&mut client, "DEFINE COLLECTION tx_rb").await;
    sql(&mut client, r#"CREATE tx_rb:item SET val = "original""#).await;

    // BEGIN
    let resp = sql(&mut client, "BEGIN").await;
    let session_id = resp.results[0].session_id.clone().unwrap();

    // UPDATE within transaction
    client
        .execute(with_db(proto::ExecuteRequest {
            query: r#"UPDATE tx_rb:item SET val = "modified""#.to_string(),
            session_id: Some(session_id.clone()),
            query_timeout: None,
        }))
        .await
        .unwrap();

    // ROLLBACK
    client
        .execute(with_db(proto::ExecuteRequest {
            query: "ROLLBACK".to_string(),
            session_id: Some(session_id),
            query_timeout: None,
        }))
        .await
        .unwrap();

    // Verify original value preserved
    let resp = sql(&mut client, "SELECT val FROM tx_rb:item").await;
    assert_eq!(
        resp.results[0].rows[0].fields.get("val").unwrap().kind,
        Some(Kind::StringValue("original".to_string()))
    );
}

#[tokio::test]
async fn test_query_transaction_isolation() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut client1 = QueryServiceClient::new(channel.clone());
    let mut client2 = QueryServiceClient::new(channel);

    sql(&mut client1, "DEFINE COLLECTION tx_iso").await;
    sql(&mut client1, r#"CREATE tx_iso:item SET val = "before""#).await;

    // BEGIN transaction on client1
    let resp = sql(&mut client1, "BEGIN").await;
    let session_id = resp.results[0].session_id.clone().unwrap();

    // Update inside transaction
    client1
        .execute(with_db(proto::ExecuteRequest {
            query: r#"UPDATE tx_iso:item SET val = "during""#.to_string(),
            session_id: Some(session_id.clone()),
            query_timeout: None,
        }))
        .await
        .unwrap();

    // client2 should still see old value (isolation)
    let resp = sql(&mut client2, "SELECT val FROM tx_iso:item").await;
    assert_eq!(
        resp.results[0].rows[0].fields.get("val").unwrap().kind,
        Some(Kind::StringValue("before".to_string()))
    );

    // COMMIT
    client1
        .execute(with_db(proto::ExecuteRequest {
            query: "COMMIT".to_string(),
            session_id: Some(session_id),
            query_timeout: None,
        }))
        .await
        .unwrap();

    // Now client2 should see the new value
    let resp = sql(&mut client2, "SELECT val FROM tx_iso:item").await;
    assert_eq!(
        resp.results[0].rows[0].fields.get("val").unwrap().kind,
        Some(Kind::StringValue("during".to_string()))
    );
}

// =============================================================================
// DocumentService — additional tests
// =============================================================================

#[tokio::test]
async fn test_document_create_with_explicit_id() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut doc_client = DocumentServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(&mut query_client, "DEFINE COLLECTION keyed").await;

    let mut fields = HashMap::new();
    fields.insert(
        "id".to_string(),
        proto_value(Kind::StringValue("keyed:mykey".to_string())),
    );
    fields.insert(
        "data".to_string(),
        proto_value(Kind::StringValue("hello".to_string())),
    );

    let created = doc_client
        .create(with_db(proto::CreateDocumentRequest {
            collection: "keyed".to_string(),
            fields,
        }))
        .await
        .unwrap()
        .into_inner();

    // Document should be created (id may or may not use the provided one, depends on impl)
    assert!(!created.id.is_empty());
}

#[tokio::test]
async fn test_document_update_nonexistent() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut doc_client = DocumentServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(&mut query_client, "DEFINE COLLECTION upd_test").await;

    let mut fields = HashMap::new();
    fields.insert("val".to_string(), proto_value(Kind::IntValue(1)));

    let result = doc_client
        .update(with_db(proto::UpdateDocumentRequest {
            collection: "upd_test".to_string(),
            key: "nonexistent_key".to_string(),
            fields,
        }))
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn test_document_delete_nonexistent() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut doc_client = DocumentServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(&mut query_client, "DEFINE COLLECTION del_test").await;

    let result = doc_client
        .delete(with_db(proto::DeleteDocumentRequest {
            collection: "del_test".to_string(),
            key: "nonexistent_key".to_string(),
        }))
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn test_document_list_empty_collection() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut doc_client = DocumentServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(&mut query_client, "DEFINE COLLECTION empty_list").await;

    let response = doc_client
        .list(with_db(proto::ListDocumentsRequest {
            collection: "empty_list".to_string(),
            pagination: Some(proto::PaginationRequest {
                limit: 50,
                after: None,
            }),
        }))
        .await
        .unwrap();

    let mut stream = response.into_inner();
    let mut docs = vec![];
    let mut has_more = None;

    while let Some(item) = stream.message().await.unwrap() {
        match item.payload {
            Some(proto::document_list_item::Payload::Document(d)) => docs.push(d),
            Some(proto::document_list_item::Payload::HasMore(hm)) => has_more = Some(hm),
            None => {}
        }
    }

    assert_eq!(docs.len(), 0);
    assert_eq!(has_more, Some(false));
}

#[tokio::test]
async fn test_document_list_multi_page_iteration() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut doc_client = DocumentServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(&mut query_client, "DEFINE COLLECTION paged").await;

    // Create 7 documents
    for i in 0..7 {
        let mut fields = HashMap::new();
        fields.insert("idx".to_string(), proto_value(Kind::IntValue(i)));
        doc_client
            .create(with_db(proto::CreateDocumentRequest {
                collection: "paged".to_string(),
                fields,
            }))
            .await
            .unwrap();
    }

    // Iterate with page size 3
    let mut all_docs = vec![];
    let mut cursor: Option<String> = None;

    loop {
        let response = doc_client
            .list(with_db(proto::ListDocumentsRequest {
                collection: "paged".to_string(),
                pagination: Some(proto::PaginationRequest {
                    limit: 3,
                    after: cursor.clone(),
                }),
            }))
            .await
            .unwrap();

        let mut stream = response.into_inner();
        let mut page_docs = vec![];
        let mut has_more = false;

        while let Some(item) = stream.message().await.unwrap() {
            match item.payload {
                Some(proto::document_list_item::Payload::Document(d)) => page_docs.push(d),
                Some(proto::document_list_item::Payload::HasMore(hm)) => has_more = hm,
                None => {}
            }
        }

        if let Some(last) = page_docs.last() {
            cursor = Some(last.id.split_once(':').unwrap().1.to_string());
        }
        all_docs.extend(page_docs);

        if !has_more {
            break;
        }
    }

    assert_eq!(all_docs.len(), 7);
}

#[tokio::test]
async fn test_document_with_nested_data() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut doc_client = DocumentServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(&mut query_client, "DEFINE COLLECTION nested").await;

    // Deep nesting: array of objects containing arrays
    let inner_obj = proto_value(Kind::ObjectValue(proto::ObjectValue {
        fields: HashMap::from([
            ("x".to_string(), proto_value(Kind::IntValue(1))),
            (
                "tags".to_string(),
                proto_value(Kind::ArrayValue(proto::ArrayValue {
                    values: vec![
                        proto_value(Kind::StringValue("a".to_string())),
                        proto_value(Kind::StringValue("b".to_string())),
                    ],
                })),
            ),
        ]),
    }));

    let mut fields = HashMap::new();
    fields.insert(
        "items".to_string(),
        proto_value(Kind::ArrayValue(proto::ArrayValue {
            values: vec![inner_obj],
        })),
    );

    let created = doc_client
        .create(with_db(proto::CreateDocumentRequest {
            collection: "nested".to_string(),
            fields,
        }))
        .await
        .unwrap()
        .into_inner();

    let key = created.id.split_once(':').unwrap().1.to_string();

    let fetched = doc_client
        .get(with_db(proto::GetDocumentRequest {
            collection: "nested".to_string(),
            key,
        }))
        .await
        .unwrap()
        .into_inner();

    // Verify deep nesting survived
    match &fetched.fields.get("items").unwrap().kind {
        Some(Kind::ArrayValue(arr)) => {
            assert_eq!(arr.values.len(), 1);
            match &arr.values[0].kind {
                Some(Kind::ObjectValue(obj)) => {
                    assert_eq!(obj.fields.get("x").unwrap().kind, Some(Kind::IntValue(1)));
                    match &obj.fields.get("tags").unwrap().kind {
                        Some(Kind::ArrayValue(tags)) => assert_eq!(tags.values.len(), 2),
                        other => panic!("expected array, got {:?}", other),
                    }
                }
                other => panic!("expected object, got {:?}", other),
            }
        }
        other => panic!("expected array, got {:?}", other),
    }
}

// =============================================================================
// EdgeService — additional tests
// =============================================================================

#[tokio::test]
async fn test_edge_create_without_fields() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut edge_client = EdgeServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(
        &mut query_client,
        r#"DEFINE COLLECTION nodes; CREATE nodes:a SET v = 1; CREATE nodes:b SET v = 2"#,
    )
    .await;

    let created = edge_client
        .create(with_db(proto::CreateEdgeRequest {
            from: "nodes:a".to_string(),
            label: "link".to_string(),
            to: "nodes:b".to_string(),
            fields: HashMap::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(created.id.starts_with("link:"));
    assert!(created.fields.is_empty());
}

#[tokio::test]
async fn test_edge_with_various_field_types() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut edge_client = EdgeServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(
        &mut query_client,
        r#"DEFINE COLLECTION n; CREATE n:a SET v = 1; CREATE n:b SET v = 2"#,
    )
    .await;

    let mut fields = HashMap::new();
    fields.insert("weight".to_string(), proto_value(Kind::FloatValue(0.95)));
    fields.insert(
        "label".to_string(),
        proto_value(Kind::StringValue("strong".to_string())),
    );
    fields.insert("active".to_string(), proto_value(Kind::BoolValue(true)));

    let created = edge_client
        .create(with_db(proto::CreateEdgeRequest {
            from: "n:a".to_string(),
            label: "rel".to_string(),
            to: "n:b".to_string(),
            fields,
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        created.fields.get("weight").unwrap().kind,
        Some(Kind::FloatValue(0.95))
    );
    assert_eq!(
        created.fields.get("label").unwrap().kind,
        Some(Kind::StringValue("strong".to_string()))
    );
    assert_eq!(
        created.fields.get("active").unwrap().kind,
        Some(Kind::BoolValue(true))
    );
}

#[tokio::test]
async fn test_edge_list_empty() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut edge_client = EdgeServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(
        &mut query_client,
        r#"DEFINE COLLECTION iso; CREATE iso:alone SET v = 1"#,
    )
    .await;

    let response = edge_client
        .list_out(with_db(proto::ListEdgesRequest {
            node_id: "iso:alone".to_string(),
            label: "nonexistent".to_string(),
            pagination: Some(proto::PaginationRequest {
                limit: 50,
                after: None,
            }),
        }))
        .await
        .unwrap();

    let mut stream = response.into_inner();
    let mut edges = vec![];
    let mut has_more = None;

    while let Some(item) = stream.message().await.unwrap() {
        match item.payload {
            Some(proto::edge_list_item::Payload::Edge(e)) => edges.push(e),
            Some(proto::edge_list_item::Payload::HasMore(hm)) => has_more = Some(hm),
            _ => {}
        }
    }

    assert_eq!(edges.len(), 0);
    assert_eq!(has_more, Some(false));
}

#[tokio::test]
async fn test_edge_delete_nonexistent() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = EdgeServiceClient::new(connect(grpc).await);

    let result = client
        .delete(with_db(proto::DeleteEdgeRequest {
            from: "x:1".to_string(),
            label: "nope".to_string(),
            to: "y:1".to_string(),
        }))
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn test_edge_bidirectional_consistency() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut edge_client = EdgeServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(
        &mut query_client,
        r#"DEFINE COLLECTION bidi;
           CREATE bidi:a SET v = 1;
           CREATE bidi:b SET v = 2;
           CREATE bidi:c SET v = 3"#,
    )
    .await;

    // Create edge a->b and a->c
    for to in &["bidi:b", "bidi:c"] {
        edge_client
            .create(with_db(proto::CreateEdgeRequest {
                from: "bidi:a".to_string(),
                label: "conn".to_string(),
                to: to.to_string(),
                fields: HashMap::new(),
            }))
            .await
            .unwrap();
    }

    // ListOut from a should show 2
    let out_resp = edge_client
        .list_out(with_db(proto::ListEdgesRequest {
            node_id: "bidi:a".to_string(),
            label: "conn".to_string(),
            pagination: Some(proto::PaginationRequest {
                limit: 50,
                after: None,
            }),
        }))
        .await
        .unwrap();

    let mut stream = out_resp.into_inner();
    let mut out_edges = vec![];
    while let Some(item) = stream.message().await.unwrap() {
        if let Some(proto::edge_list_item::Payload::Edge(e)) = item.payload {
            out_edges.push(e);
        }
    }
    assert_eq!(out_edges.len(), 2);

    // ListIn to b should show 1 (from a)
    let in_resp = edge_client
        .list_in(with_db(proto::ListEdgesRequest {
            node_id: "bidi:b".to_string(),
            label: "conn".to_string(),
            pagination: Some(proto::PaginationRequest {
                limit: 50,
                after: None,
            }),
        }))
        .await
        .unwrap();

    let mut stream = in_resp.into_inner();
    let mut in_edges = vec![];
    while let Some(item) = stream.message().await.unwrap() {
        if let Some(proto::edge_list_item::Payload::Edge(e)) = item.payload {
            in_edges.push(e);
        }
    }
    assert_eq!(in_edges.len(), 1);
    assert_eq!(in_edges[0].from, "bidi:a");
}

// =============================================================================
// DatabaseService — additional tests
// =============================================================================

#[tokio::test]
async fn test_database_create_empty_name() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = DatabaseServiceClient::new(connect(grpc).await);

    let result = client
        .create(Request::new(proto::CreateDatabaseRequest {
            name: "".to_string(),
        }))
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
}

#[tokio::test]
async fn test_database_drop_nonexistent() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = DatabaseServiceClient::new(connect(grpc).await);

    let result = client
        .drop(Request::new(proto::DropDatabaseRequest {
            name: "does_not_exist_db".to_string(),
        }))
        .await;

    // Should error — can't drop what doesn't exist
    assert!(result.is_err());
}

#[tokio::test]
async fn test_database_stats_after_collections() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut db_client = DatabaseServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    // Create some collections
    sql(
        &mut query_client,
        "DEFINE COLLECTION stats_a; DEFINE COLLECTION stats_b",
    )
    .await;

    let stats = db_client
        .stats(Request::new(proto::StatsRequest {
            name: "test".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(stats.name, "test");
    assert!(stats.collections >= 2);
}

#[tokio::test]
async fn test_database_stats_empty_name() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = DatabaseServiceClient::new(connect(grpc).await);

    let result = client
        .stats(Request::new(proto::StatsRequest {
            name: "".to_string(),
        }))
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
}

#[tokio::test]
async fn test_database_drop_empty_name() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = DatabaseServiceClient::new(connect(grpc).await);

    let result = client
        .drop(Request::new(proto::DropDatabaseRequest {
            name: "".to_string(),
        }))
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
}

// =============================================================================
// MetricsService — additional tests
// =============================================================================

#[tokio::test]
async fn grpc_metrics_never_expose_query_literals_or_malformed_secrets() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut query_client = QueryServiceClient::new(channel.clone());
    let mut metrics_client = MetricsServiceClient::new(channel);
    let queries = [
        "SELECT 'Елена Смирнова' AS customer, 731991 AS account_number",
        "CREATE USER 'grpc-user@example.com' PASSWORD 'grpc-password-secret'",
        "SELECT * FROM customers WHERE token = 'malformed-grpc-пии api_key=sk-grpc-secret",
    ];

    for query in &queries[..2] {
        let _ = query_client
            .execute(with_db(proto::ExecuteRequest {
                query: (*query).to_string(),
                session_id: None,
                query_timeout: None,
            }))
            .await;
    }
    let _ = query_client
        .execute_stream(with_db(proto::ExecuteRequest {
            query: queries[2].to_string(),
            session_id: None,
            query_timeout: None,
        }))
        .await;

    let stats = metrics_client
        .get_stats(Request::new(()))
        .await
        .unwrap()
        .into_inner();
    for sensitive in [
        "Елена Смирнова",
        "731991",
        "grpc-user@example.com",
        "grpc-password-secret",
        "malformed-grpc-пии",
        "sk-grpc-secret",
    ] {
        assert!(
            !stats.json.contains(sensitive),
            "gRPC stats leaked {sensitive}: {}",
            stats.json
        );
    }

    let parsed: serde_json::Value = serde_json::from_str(&stats.json).unwrap();
    let top_queries = parsed["top_queries"].as_array().unwrap();
    assert!(top_queries.iter().any(|query| {
        query["query_normalized"] == "CREATE USER '?' PASSWORD '?'"
            && query["query_example"] == "CREATE USER '?' PASSWORD '?'"
            && query["query_hash"].is_string()
    }));
    assert!(top_queries.iter().any(|query| {
        query["query_normalized"] == ""
            && query["query_example"] == ""
            && query["errors"].as_u64().unwrap_or(0) >= 1
    }));
}

#[tokio::test]
async fn test_metrics_after_queries() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut query_client = QueryServiceClient::new(channel.clone());
    let mut metrics_client = MetricsServiceClient::new(channel);

    // Run some queries
    sql(&mut query_client, "SELECT 1").await;
    sql(&mut query_client, "SELECT 2").await;
    sql(&mut query_client, "SELECT 3").await;

    let stats = metrics_client
        .get_stats(Request::new(()))
        .await
        .unwrap()
        .into_inner();

    let parsed: serde_json::Value = serde_json::from_str(&stats.json).unwrap();
    // queries_total is nested under "server"
    let total = parsed["server"]
        .get("queries_total")
        .and_then(|v| v.as_u64());
    assert!(total.unwrap_or(0) >= 3);
}

// =============================================================================
// Cross-service integration tests
// =============================================================================

#[tokio::test]
async fn test_cross_service_doc_create_then_sql_query() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut doc_client = DocumentServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(&mut query_client, "DEFINE COLLECTION cross").await;

    // Create via DocumentService
    let mut fields = HashMap::new();
    fields.insert(
        "name".to_string(),
        proto_value(Kind::StringValue("CrossDoc".to_string())),
    );
    fields.insert("score".to_string(), proto_value(Kind::IntValue(99)));

    doc_client
        .create(with_db(proto::CreateDocumentRequest {
            collection: "cross".to_string(),
            fields,
        }))
        .await
        .unwrap();

    // Query via QueryService
    let resp = sql(
        &mut query_client,
        "SELECT name, score FROM cross WHERE score > 50",
    )
    .await;
    assert_eq!(resp.results[0].rows.len(), 1);
    assert_eq!(
        resp.results[0].rows[0].fields.get("name").unwrap().kind,
        Some(Kind::StringValue("CrossDoc".to_string()))
    );
}

#[tokio::test]
async fn test_cross_service_edge_create_then_sql_traverse() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut edge_client = EdgeServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(
        &mut query_client,
        r#"DEFINE COLLECTION person;
           CREATE person:alice SET name = "Alice";
           CREATE person:bob SET name = "Bob""#,
    )
    .await;

    // Create edge via EdgeService
    let mut fields = HashMap::new();
    fields.insert("since".to_string(), proto_value(Kind::IntValue(2025)));

    edge_client
        .create(with_db(proto::CreateEdgeRequest {
            from: "person:alice".to_string(),
            label: "knows".to_string(),
            to: "person:bob".to_string(),
            fields,
        }))
        .await
        .unwrap();

    // Traverse via SQL — get target node names
    let resp = sql(
        &mut query_client,
        "SELECT ->knows->person.* AS friend FROM person:alice",
    )
    .await;

    assert_eq!(resp.completed, 1);
    let row = &resp.results[0].rows[0];
    // Traversal returns array of nodes
    match &row.fields.get("friend").unwrap().kind {
        Some(Kind::ArrayValue(arr)) => {
            assert_eq!(arr.values.len(), 1);
        }
        other => panic!("expected array, got {:?}", other),
    }
}

#[tokio::test]
async fn test_cross_service_sql_insert_then_doc_get() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let channel = connect(grpc).await;
    let mut doc_client = DocumentServiceClient::new(channel.clone());
    let mut query_client = QueryServiceClient::new(channel);

    sql(
        &mut query_client,
        r#"DEFINE COLLECTION items; CREATE items:mykey SET val = "from_sql""#,
    )
    .await;

    // Retrieve via DocumentService
    let doc = doc_client
        .get(with_db(proto::GetDocumentRequest {
            collection: "items".to_string(),
            key: "mykey".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(doc.id, "items:mykey");
    assert_eq!(
        doc.fields.get("val").unwrap().kind,
        Some(Kind::StringValue("from_sql".to_string()))
    );
}

// =============================================================================
// Error handling — additional tests
// =============================================================================

#[tokio::test]
async fn test_missing_database_metadata_document_service() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = DocumentServiceClient::new(connect(grpc).await);

    let result = client
        .get(Request::new(proto::GetDocumentRequest {
            collection: "test".to_string(),
            key: "key".to_string(),
        }))
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
}

#[tokio::test]
async fn test_missing_database_metadata_edge_service() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = EdgeServiceClient::new(connect(grpc).await);

    let result = client
        .get(Request::new(proto::GetEdgeRequest {
            from: "a:1".to_string(),
            label: "rel".to_string(),
            to: "b:1".to_string(),
        }))
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
}

#[tokio::test]
async fn test_wrong_database_metadata() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    let mut req = Request::new(proto::ExecuteRequest {
        query: "SELECT 1".to_string(),
        session_id: None,
        query_timeout: None,
    });
    req.metadata_mut()
        .insert("x-database", MetadataValue::from_static("wrong_db_name"));

    let result = client.execute(req).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_document_collection_key_empty() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = DocumentServiceClient::new(connect(grpc).await);

    // Empty key
    let result = client
        .get(with_db(proto::GetDocumentRequest {
            collection: "coll".to_string(),
            key: "".to_string(),
        }))
        .await;

    // Should get not_found or invalid_argument, not a crash
    assert!(result.is_err());
}

// =============================================================================
// Value type round-trip tests
// =============================================================================

#[tokio::test]
async fn test_value_roundtrip_via_sql() {
    let (_http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut client = QueryServiceClient::new(connect(grpc).await);

    // Test various value types via SQL
    let resp = sql(
        &mut client,
        "SELECT 'hello' AS s, 42 AS i, 3.5 AS f, true AS b, null AS n",
    )
    .await;

    let row = &resp.results[0].rows[0];

    assert_eq!(
        row.fields.get("s").unwrap().kind,
        Some(Kind::StringValue("hello".to_string()))
    );
    assert_eq!(row.fields.get("i").unwrap().kind, Some(Kind::IntValue(42)));
    assert_eq!(
        row.fields.get("f").unwrap().kind,
        Some(Kind::FloatValue(3.5))
    );
    assert_eq!(
        row.fields.get("b").unwrap().kind,
        Some(Kind::BoolValue(true))
    );

    let null_kind = &row.fields.get("n").unwrap().kind;
    match null_kind {
        Some(Kind::NullValue(_)) | None => {}
        other => panic!("expected null, got {:?}", other),
    }
}

#[tokio::test]
async fn authz_is_consistent_for_http_grpc_execute_stream_and_document_crud() {
    let (http, grpc, root_pw, _tmp) = common::spawn_server_with_auth_and_grpc().await;
    let root_token = common::login(http, "root", &root_pw).await;
    common::sql_with_auth(http, &root_token, "DEFINE COLLECTION items").await;
    common::sql_with_auth(http, &root_token, "CREATE items:seed SET name = 'seed'").await;
    common::sql_with_auth(http, &root_token, "CREATE USER 'reader' PASSWORD 'pass'").await;
    let reader_token = common::login(http, "reader", "pass").await;

    let http_denied = common::sql_with_auth(http, &reader_token, "SELECT * FROM items").await;
    assert!(http_denied["error"].is_object());

    let channel = connect(grpc).await;
    let mut query = QueryServiceClient::new(channel.clone());
    let response = query
        .execute(with_db_and_auth(
            proto::ExecuteRequest {
                query: "SELECT * FROM items".to_string(),
                session_id: None,
                query_timeout: None,
            },
            &reader_token,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(
        response.error.is_some(),
        "gRPC Execute must enforce default deny"
    );

    let mut stream = query
        .execute_stream(with_db_and_auth(
            proto::ExecuteRequest {
                query: "SELECT * FROM items".to_string(),
                session_id: None,
                query_timeout: None,
            },
            &reader_token,
        ))
        .await
        .unwrap()
        .into_inner();
    let mut saw_denial = false;
    while let Some(item) = stream.message().await.unwrap() {
        if matches!(item.payload, Some(proto::stream_row::Payload::Error(_))) {
            saw_denial = true;
        }
    }
    assert!(saw_denial, "gRPC ExecuteStream must enforce default deny");

    let mut documents = DocumentServiceClient::new(channel);
    let denied = documents
        .get(with_db_and_auth(
            proto::GetDocumentRequest {
                collection: "items".to_string(),
                key: "seed".to_string(),
            },
            &reader_token,
        ))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), tonic::Code::PermissionDenied);

    common::sql_with_auth(
        http,
        &root_token,
        "CREATE POLICY transport_read WHEN action = 'SELECT' ALLOW",
    )
    .await;

    let response = query
        .execute(with_db_and_auth(
            proto::ExecuteRequest {
                query: "SELECT * FROM items".to_string(),
                session_id: None,
                query_timeout: None,
            },
            &reader_token,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(response.error.is_none());

    documents
        .get(with_db_and_auth(
            proto::GetDocumentRequest {
                collection: "items".to_string(),
                key: "seed".to_string(),
            },
            &reader_token,
        ))
        .await
        .unwrap();
    let denied = documents
        .create(with_db_and_auth(
            proto::CreateDocumentRequest {
                collection: "items".to_string(),
                fields: HashMap::new(),
            },
            &reader_token,
        ))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), tonic::Code::PermissionDenied);
}

#[tokio::test]
async fn grpc_database_management_and_metrics_require_manage_policy() {
    let (http, grpc, root_pw, _tmp) = common::spawn_server_with_auth_and_grpc().await;
    let root_token = common::login(http, "root", &root_pw).await;
    common::sql_with_auth(http, &root_token, "CREATE USER 'operator' PASSWORD 'pass'").await;
    let operator_token = common::login(http, "operator", "pass").await;

    let channel = connect(grpc).await;
    let mut databases = DatabaseServiceClient::new(channel.clone());
    let denied = databases
        .create(with_auth(
            proto::CreateDatabaseRequest {
                name: "managed_grpc".to_string(),
            },
            &operator_token,
        ))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), tonic::Code::PermissionDenied);

    let mut metrics = MetricsServiceClient::new(channel);
    let denied = metrics
        .get_stats(with_auth((), &operator_token))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), tonic::Code::PermissionDenied);

    common::sql_with_auth(
        http,
        &root_token,
        "CREATE POLICY transport_manage WHEN action = 'MANAGE' ALLOW",
    )
    .await;

    databases
        .create(with_auth(
            proto::CreateDatabaseRequest {
                name: "managed_grpc".to_string(),
            },
            &operator_token,
        ))
        .await
        .unwrap();
    metrics
        .get_stats(with_auth((), &operator_token))
        .await
        .unwrap();
    databases
        .drop(with_auth(
            proto::DropDatabaseRequest {
                name: "managed_grpc".to_string(),
            },
            &operator_token,
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn grpc_query_authorization_uses_resource_database() {
    let (http, grpc, root_pw, _tmp) = common::spawn_server_with_auth_and_grpc().await;
    let root_token = common::login(http, "root", &root_pw).await;
    let root = common::auth_client(&root_token);
    assert_eq!(
        root.post(format!("http://{http}/databases"))
            .json(&serde_json::json!({"name": "other"}))
            .send()
            .await
            .unwrap()
            .status(),
        201
    );

    common::sql_with_auth(http, &root_token, "CREATE USER 'db_reader' PASSWORD 'pass'").await;
    common::sql_with_auth(
        http,
        &root_token,
        "CREATE POLICY test_database_read WHEN action = 'SELECT' AND resource.database = 'test' ALLOW",
    )
    .await;
    let token = common::login(http, "db_reader", "pass").await;
    let mut query = QueryServiceClient::new(connect(grpc).await);

    let allowed = query
        .execute(with_database_and_auth(
            proto::ExecuteRequest {
                query: "SELECT 1".to_string(),
                session_id: None,
                query_timeout: None,
            },
            "test",
            &token,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(allowed.error.is_none());

    let denied = query
        .execute(with_database_and_auth(
            proto::ExecuteRequest {
                query: "SELECT 1".to_string(),
                session_id: None,
                query_timeout: None,
            },
            "other",
            &token,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(denied.error.is_some());

    let mut stream = query
        .execute_stream(with_database_and_auth(
            proto::ExecuteRequest {
                query: "SELECT 1".to_string(),
                session_id: None,
                query_timeout: None,
            },
            "other",
            &token,
        ))
        .await
        .unwrap()
        .into_inner();
    let mut saw_denial = false;
    while let Some(item) = stream.message().await.unwrap() {
        if matches!(item.payload, Some(proto::stream_row::Payload::Error(_))) {
            saw_denial = true;
        }
    }
    assert!(saw_denial, "ExecuteStream must enforce resource.database");
}

#[tokio::test]
async fn unknown_session_fails_consistently_without_autocommit() {
    let (http, grpc, _tmp) = common::spawn_server_with_grpc().await;
    let mut query = QueryServiceClient::new(connect(grpc).await);
    sql(&mut query, "DEFINE COLLECTION session_guard").await;

    let grpc_error = query
        .execute(with_db(proto::ExecuteRequest {
            query: "INSERT INTO session_guard {id: 'grpc'}".to_string(),
            session_id: Some("missing-session".to_string()),
            query_timeout: None,
        }))
        .await
        .unwrap_err();
    assert_eq!(grpc_error.code(), tonic::Code::FailedPrecondition);
    assert!(grpc_error.message().contains("SDB-QE034"));

    let set_error = query
        .execute(with_db(proto::ExecuteRequest {
            query: "SET QUERY_TIMEOUT = 5s".to_string(),
            session_id: Some("missing-session".to_string()),
            query_timeout: None,
        }))
        .await
        .unwrap_err();
    assert_eq!(set_error.code(), tonic::Code::FailedPrecondition);

    let http_response = reqwest::Client::new()
        .post(format!("http://{http}/sql"))
        .header("X-Database", common::TEST_DB)
        .header("X-Session-Id", "missing-session")
        .json(&serde_json::json!({
            "query": "INSERT INTO session_guard {id: 'http'}"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(http_response.status(), reqwest::StatusCode::CONFLICT);
    let body: serde_json::Value = http_response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "SDB-QE034");

    let rows = sql(&mut query, "SELECT * FROM session_guard").await;
    assert!(rows.results[0].rows.is_empty());
}

#[tokio::test]
async fn transaction_session_ownership_is_shared_across_http_and_grpc() {
    let (http, grpc, root_pw, _tmp) = common::spawn_server_with_auth_and_grpc().await;
    let root_jwt = common::login(http, "root", &root_pw).await;
    common::sql_with_auth(http, &root_jwt, "CREATE USER 'alice' PASSWORD 'pass'").await;
    let alice_jwt = common::login(http, "alice", "pass").await;
    let key_body = common::sql_with_auth(
        http,
        &root_jwt,
        "CREATE API KEY 'session-key' FOR USER 'root'",
    )
    .await;
    let root_api_key = key_body["results"][0]["data"][0]["key"]
        .as_str()
        .unwrap()
        .to_string();

    let root_http = common::auth_client(&root_jwt);
    assert!(
        root_http
            .post(format!("http://{http}/databases"))
            .json(&serde_json::json!({"name": "other"}))
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );

    let begin = common::sql_with_auth(http, &root_jwt, "BEGIN").await;
    let http_session = begin["results"][0]["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut query = QueryServiceClient::new(connect(grpc).await);
    query
        .execute(with_db_and_auth(
            proto::ExecuteRequest {
                query: "ROLLBACK".to_string(),
                session_id: Some(http_session),
                query_timeout: None,
            },
            &root_api_key,
        ))
        .await
        .expect("the same user may continue an HTTP session over gRPC with an API key");

    let grpc_begin = query
        .execute(with_db_and_auth(
            proto::ExecuteRequest {
                query: "BEGIN".to_string(),
                session_id: None,
                query_timeout: None,
            },
            &root_jwt,
        ))
        .await
        .unwrap()
        .into_inner();
    let grpc_session = grpc_begin.results[0].session_id.clone().unwrap();

    let alice_http = common::auth_client(&alice_jwt);
    let denied = alice_http
        .post(format!("http://{http}/sql"))
        .header("X-Session-Id", &grpc_session)
        .json(&serde_json::json!({"query": "COMMIT"}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), reqwest::StatusCode::FORBIDDEN);
    let denied_body: serde_json::Value = denied.json().await.unwrap();
    assert_eq!(denied_body["error"]["code"], "SDB-QE031");

    let wrong_principal = query
        .execute(with_db_and_auth(
            proto::ExecuteRequest {
                query: "COMMIT".to_string(),
                session_id: Some(grpc_session.clone()),
                query_timeout: None,
            },
            &alice_jwt,
        ))
        .await
        .unwrap_err();
    assert_eq!(wrong_principal.code(), tonic::Code::PermissionDenied);
    assert!(wrong_principal.message().contains("SDB-QE031"));

    let wrong_database = query
        .execute(with_database_and_auth(
            proto::ExecuteRequest {
                query: "COMMIT".to_string(),
                session_id: Some(grpc_session.clone()),
                query_timeout: None,
            },
            "other",
            &root_jwt,
        ))
        .await
        .unwrap_err();
    assert_eq!(wrong_database.code(), tonic::Code::FailedPrecondition);
    assert!(wrong_database.message().contains("SDB-QE032"));

    let wrong_database_http = reqwest::Client::new()
        .post(format!("http://{http}/sql"))
        .header("Authorization", format!("Bearer {root_jwt}"))
        .header("X-Database", "other")
        .header("X-Session-Id", &grpc_session)
        .json(&serde_json::json!({"query": "COMMIT"}))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_database_http.status(), reqwest::StatusCode::CONFLICT);
    let wrong_database_body: serde_json::Value = wrong_database_http.json().await.unwrap();
    assert_eq!(wrong_database_body["error"]["code"], "SDB-QE032");

    let root_api_http = common::auth_client(&root_api_key);
    let rollback = root_api_http
        .post(format!("http://{http}/sql"))
        .header("X-Session-Id", &grpc_session)
        .json(&serde_json::json!({"query": "ROLLBACK"}))
        .send()
        .await
        .unwrap();
    assert!(rollback.status().is_success());

    let alice_begin = query
        .execute(with_db_and_auth(
            proto::ExecuteRequest {
                query: "BEGIN".to_string(),
                session_id: None,
                query_timeout: None,
            },
            &alice_jwt,
        ))
        .await
        .unwrap()
        .into_inner();
    let alice_session = alice_begin.results[0].session_id.clone().unwrap();
    let root_takeover = query
        .execute(with_db_and_auth(
            proto::ExecuteRequest {
                query: "ROLLBACK".to_string(),
                session_id: Some(alice_session.clone()),
                query_timeout: None,
            },
            &root_jwt,
        ))
        .await
        .unwrap_err();
    assert_eq!(root_takeover.code(), tonic::Code::PermissionDenied);

    query
        .execute(with_db_and_auth(
            proto::ExecuteRequest {
                query: "ROLLBACK".to_string(),
                session_id: Some(alice_session),
                query_timeout: None,
            },
            &alice_jwt,
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn grpc_db_admission_overload_returns_resource_exhausted() {
    let config = stellardb::server::admission::AdmissionConfig {
        max_db_concurrency: 0,
        ..Default::default()
    };
    let (http, grpc, root_pw, _tmp) =
        common::spawn_server_with_auth_and_grpc_with_admission(config).await;
    let token = common::login(http, "root", &root_pw).await;
    let mut query = QueryServiceClient::new(connect(grpc).await);
    let error = query
        .execute(with_db_and_auth(
            proto::ExecuteRequest {
                query: "SELECT 1".to_string(),
                session_id: None,
                query_timeout: None,
            },
            &token,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::ResourceExhausted);
}
