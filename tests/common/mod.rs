use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use reqwest::header::{HeaderMap, HeaderValue};
use stellardb::namespace::Namespace;
use stellardb::server::admission::AdmissionConfig;
use stellardb::server::{
    CorsConfig, HttpServerConfig, create_router, create_router_with_admission_and_cors,
    create_router_with_config,
};

/// Default database name used in integration tests
pub const TEST_DB: &str = "test";

/// Create an HTTP client pre-configured with the X-Database header for tests.
#[allow(dead_code)]
pub fn test_client() -> reqwest::Client {
    let mut headers = HeaderMap::new();
    headers.insert("X-Database", HeaderValue::from_static(TEST_DB));
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap()
}

/// Check if the response body contains an error.
/// Errors are structured objects: `{"error": {"code": "SDB-XXNNN", "message": "...", "severity": "ERROR"}}`
/// For batch errors from /sql: `{"error": {"statement_index": N, "message": "..."}}`
#[allow(dead_code)]
pub fn has_error(body: &serde_json::Value) -> bool {
    body["error"].is_object()
}

/// Get error message from response body.
/// Extracts the "message" field from the structured error object.
#[allow(dead_code)]
pub fn get_error_message(body: &serde_json::Value) -> String {
    let err = &body["error"];
    if err.is_object() {
        err["message"].as_str().unwrap_or("").to_string()
    } else {
        String::new()
    }
}

/// Check if the first result of a DDL operation was successful.
#[allow(dead_code)]
pub fn is_ddl_success(body: &serde_json::Value) -> bool {
    body["results"][0]["data"][0]["status"].is_string()
}

#[allow(dead_code)]
pub async fn spawn_server() -> (SocketAddr, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let namespace = Arc::new(Namespace::open(tmp.path()).unwrap());

    // Create default test database
    namespace.create_database(TEST_DB).unwrap();

    let (app, _state) = create_router(namespace, None, None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, tmp)
}

#[allow(dead_code)]
pub async fn spawn_server_with_admission(
    admission: AdmissionConfig,
) -> (SocketAddr, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let namespace = Arc::new(Namespace::open(tmp.path()).unwrap());
    namespace.create_database(TEST_DB).unwrap();
    let (app, _state) = create_router_with_admission_and_cors(
        namespace,
        None,
        None,
        admission,
        CorsConfig::Disabled,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (addr, tmp)
}

#[allow(dead_code)]
pub async fn spawn_server_with_cors(value: Option<&str>) -> (SocketAddr, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let namespace = Arc::new(Namespace::open(tmp.path()).unwrap());
    namespace.create_database(TEST_DB).unwrap();
    let cors = CorsConfig::from_value(value).unwrap_or(CorsConfig::Disabled);
    let (app, _state) =
        create_router_with_admission_and_cors(namespace, None, None, Default::default(), cors);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, tmp)
}

#[allow(dead_code)]
pub async fn spawn_server_with_http_body_limit(
    http_body_limit: usize,
) -> (SocketAddr, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let namespace = Arc::new(Namespace::open(tmp.path()).unwrap());
    namespace.create_database(TEST_DB).unwrap();
    let (app, _state) = create_router_with_config(
        namespace,
        None,
        None,
        HttpServerConfig {
            http_body_limit,
            login_body_limit: http_body_limit,
            ..Default::default()
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, tmp)
}

#[allow(dead_code)]
pub struct TestServer {
    pub addr: SocketAddr,
    handle: tokio::task::JoinHandle<()>,
}

#[allow(dead_code)]
impl TestServer {
    pub async fn abort(self) {
        self.handle.abort();
        let _ = self.handle.await;
    }
}

#[allow(dead_code)]
pub async fn spawn_server_on_data_dir(path: &Path) -> TestServer {
    let namespace = Arc::new(Namespace::open(path).unwrap());

    if !namespace.list_databases().contains(&TEST_DB.to_string()) {
        namespace.create_database(TEST_DB).unwrap();
    }

    let (app, _state) = create_router(namespace, None, None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    TestServer { addr, handle }
}

/// Spawn a server with both HTTP and gRPC listeners on random ports.
/// Returns (http_addr, grpc_addr, temp_dir).
#[cfg(feature = "grpc")]
#[allow(dead_code)]
pub async fn spawn_server_with_grpc() -> (SocketAddr, SocketAddr, tempfile::TempDir) {
    spawn_server_with_grpc_admission_and_config(Default::default(), Default::default()).await
}

#[cfg(feature = "grpc")]
#[allow(dead_code)]
pub async fn spawn_server_with_grpc_config(
    grpc_config: stellardb::server::grpc::GrpcConfig,
) -> (SocketAddr, SocketAddr, tempfile::TempDir) {
    spawn_server_with_grpc_admission_and_config(Default::default(), grpc_config).await
}

#[cfg(feature = "grpc")]
#[allow(dead_code)]
pub async fn spawn_server_with_grpc_admission(
    admission: stellardb::server::admission::AdmissionConfig,
) -> (SocketAddr, SocketAddr, tempfile::TempDir) {
    spawn_server_with_grpc_admission_and_config(admission, Default::default()).await
}

#[cfg(feature = "grpc")]
#[allow(dead_code)]
pub async fn spawn_server_with_grpc_admission_and_config(
    admission: stellardb::server::admission::AdmissionConfig,
    grpc_config: stellardb::server::grpc::GrpcConfig,
) -> (SocketAddr, SocketAddr, tempfile::TempDir) {
    use stellardb::server::grpc::GrpcServer;

    let tmp = tempfile::tempdir().unwrap();
    let namespace = Arc::new(Namespace::open(tmp.path()).unwrap());

    // Create default test database
    namespace.create_database(TEST_DB).unwrap();

    let (app, state) = create_router_with_admission_and_cors(
        namespace,
        None,
        None,
        admission,
        CorsConfig::Disabled,
    );

    // Bind HTTP listener
    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_addr = http_listener.local_addr().unwrap();

    // Bind gRPC listener to get a random port
    let grpc_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc_addr = grpc_listener.local_addr().unwrap();
    // Drop the TCP listener so tonic can bind to the same port
    drop(grpc_listener);

    // Start gRPC server
    let grpc_server = GrpcServer::with_config(state, grpc_config);
    tokio::spawn(async move {
        grpc_server.start(grpc_addr).await.unwrap();
    });

    // Start HTTP server
    tokio::spawn(async move {
        axum::serve(http_listener, app).await.unwrap();
    });

    // Give gRPC server a moment to bind
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    (http_addr, grpc_addr, tmp)
}

/// Spawn a server with authentication enabled.
/// Returns (http_addr, root_password, temp_dir).
#[allow(dead_code)]
pub async fn spawn_server_with_auth() -> (SocketAddr, String, tempfile::TempDir) {
    spawn_server_with_auth_and_admission(Default::default()).await
}

#[allow(dead_code)]
pub async fn spawn_server_with_auth_and_admission(
    admission_config: stellardb::server::admission::AdmissionConfig,
) -> (SocketAddr, String, tempfile::TempDir) {
    spawn_server_with_auth_config(admission_config, 0).await
}

#[allow(dead_code)]
pub async fn spawn_server_with_auth_config(
    admission: stellardb::server::admission::AdmissionConfig,
    trusted_proxy_hops: usize,
) -> (SocketAddr, String, tempfile::TempDir) {
    spawn_server_with_auth_http_limits(admission, trusted_proxy_hops, 16 * 1024, 10 * 1024 * 1024)
        .await
}

#[allow(dead_code)]
pub async fn spawn_server_with_auth_http_limits(
    admission: stellardb::server::admission::AdmissionConfig,
    trusted_proxy_hops: usize,
    login_body_limit: usize,
    http_body_limit: usize,
) -> (SocketAddr, String, tempfile::TempDir) {
    spawn_server_with_auth_transport(
        admission,
        trusted_proxy_hops,
        login_body_limit,
        http_body_limit,
        true,
    )
    .await
}

#[allow(dead_code)]
pub async fn spawn_server_with_auth_without_connect_info() -> (SocketAddr, String, tempfile::TempDir)
{
    spawn_server_with_auth_without_connect_info_and_admission(Default::default()).await
}

#[allow(dead_code)]
pub async fn spawn_server_with_auth_without_connect_info_and_admission(
    admission: stellardb::server::admission::AdmissionConfig,
) -> (SocketAddr, String, tempfile::TempDir) {
    spawn_server_with_auth_transport(admission, 0, 16 * 1024, 10 * 1024 * 1024, false).await
}

async fn spawn_server_with_auth_transport(
    admission: stellardb::server::admission::AdmissionConfig,
    trusted_proxy_hops: usize,
    login_body_limit: usize,
    http_body_limit: usize,
    include_connect_info: bool,
) -> (SocketAddr, String, tempfile::TempDir) {
    use stellardb::auth::AuthService;
    use stellardb::server::{HttpServerConfig, create_router_with_config};

    let tmp = tempfile::tempdir().unwrap();
    let namespace = Arc::new(Namespace::open(tmp.path()).unwrap());

    // Create default test database
    namespace.create_database(TEST_DB).unwrap();

    // Initialize auth with _system database
    let system_db = namespace.ensure_system_database().unwrap();
    let (auth_service, root_password) =
        AuthService::init(system_db, b"integration-test-jwt-secret!!", 3600).unwrap();
    let root_password = root_password.expect("first boot should generate root password");

    let auth = Some(Arc::new(auth_service));
    let (app, _state) = create_router_with_config(
        namespace,
        None,
        auth,
        HttpServerConfig {
            admission,
            trusted_proxy_hops,
            login_body_limit,
            http_body_limit,
            ..Default::default()
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    if include_connect_info {
        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
    } else {
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
    }

    (addr, root_password, tmp)
}

/// Spawn a server with authentication plus HTTP and gRPC listeners.
/// Returns (http_addr, grpc_addr, root_password, temp_dir).
#[cfg(feature = "grpc")]
#[allow(dead_code)]
pub async fn spawn_server_with_auth_and_grpc() -> (SocketAddr, SocketAddr, String, tempfile::TempDir)
{
    spawn_server_with_auth_and_grpc_with_admission(Default::default()).await
}

#[cfg(feature = "grpc")]
#[allow(dead_code)]
pub async fn spawn_server_with_auth_and_grpc_with_admission(
    admission_config: stellardb::server::admission::AdmissionConfig,
) -> (SocketAddr, SocketAddr, String, tempfile::TempDir) {
    use stellardb::auth::AuthService;
    use stellardb::server::create_router_with_admission;
    use stellardb::server::grpc::GrpcServer;

    let tmp = tempfile::tempdir().unwrap();
    let namespace = Arc::new(Namespace::open(tmp.path()).unwrap());
    namespace.create_database(TEST_DB).unwrap();

    let system_db = namespace.ensure_system_database().unwrap();
    let (auth_service, root_password) =
        AuthService::init(system_db, b"integration-test-jwt-secret!!", 3600).unwrap();
    let root_password = root_password.expect("first boot should generate root password");

    let (app, state) = create_router_with_admission(
        namespace,
        None,
        Some(Arc::new(auth_service)),
        admission_config,
    );
    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_addr = http_listener.local_addr().unwrap();

    let grpc_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc_addr = grpc_listener.local_addr().unwrap();
    drop(grpc_listener);

    tokio::spawn(async move {
        GrpcServer::new(state).start(grpc_addr).await.unwrap();
    });
    tokio::spawn(async move {
        axum::serve(
            http_listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    (http_addr, grpc_addr, root_password, tmp)
}

/// Create an HTTP client with Authorization header for authenticated requests.
#[allow(dead_code)]
pub fn auth_client(token: &str) -> reqwest::Client {
    let mut headers = HeaderMap::new();
    headers.insert("X-Database", HeaderValue::from_static(TEST_DB));
    headers.insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap()
}

/// Login with username/password and return the JWT token.
#[allow(dead_code)]
pub async fn login(addr: SocketAddr, username: &str, password: &str) -> String {
    let client = reqwest::Client::new();
    let res = client
        .post(format!("http://{}/auth/login", addr))
        .json(&serde_json::json!({
            "username": username,
            "password": password,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "login failed");
    let body: serde_json::Value = res.json().await.unwrap();
    body["token"].as_str().unwrap().to_string()
}

/// Execute a SQL query with authentication and return the response body.
#[allow(dead_code)]
pub async fn sql_with_auth(addr: SocketAddr, token: &str, query: &str) -> serde_json::Value {
    let client = auth_client(token);
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&serde_json::json!({"query": query}))
        .send()
        .await
        .unwrap();
    res.json().await.unwrap()
}
