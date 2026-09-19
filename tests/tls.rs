#![cfg(feature = "tls")]

use std::net::SocketAddr;
#[cfg(feature = "cli")]
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ConnectInfo;
use axum::routing::get;
use stellardb::namespace::Namespace;
use stellardb::server::create_router;
#[cfg(feature = "grpc")]
use stellardb::server::create_router_with_config;
use stellardb::server::tls::TlsConfig;

fn ephemeral_identity() -> (Vec<u8>, Vec<u8>) {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(["localhost".to_string(), "127.0.0.1".to_string()])
            .unwrap();
    (
        cert.pem().into_bytes(),
        signing_key.serialize_pem().into_bytes(),
    )
}

#[cfg(feature = "grpc")]
fn reserve_address() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    address
}

async fn spawn_https_server(
    certificate_pem: &[u8],
    private_key_pem: &[u8],
) -> (SocketAddr, tempfile::TempDir, tokio::task::JoinHandle<()>) {
    let temporary_directory = tempfile::tempdir().unwrap();
    let namespace = Arc::new(Namespace::open(temporary_directory.path()).unwrap());
    namespace.create_database("test").unwrap();
    let (app, _) = create_router(namespace, None, None);
    let app = app.route(
        "/test-peer",
        get(|ConnectInfo(peer): ConnectInfo<SocketAddr>| async move { peer.ip().to_string() }),
    );

    let tls = TlsConfig::from_pem(certificate_pem.to_vec(), private_key_pem.to_vec()).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let server = axum_server::from_tcp_rustls(listener, tls.http_config()).unwrap();
    let handle = tokio::spawn(async move {
        server
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await
            .unwrap();
    });
    (address, temporary_directory, handle)
}

fn https_client(certificate_pem: &[u8], address: SocketAddr) -> reqwest::Client {
    let root = reqwest::Certificate::from_pem(certificate_pem).unwrap();
    reqwest::Client::builder()
        .add_root_certificate(root)
        .resolve("localhost", address)
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap()
}

#[tokio::test]
async fn http_tls_handshake_request_and_connect_info_work_and_plaintext_is_rejected() {
    let (certificate_pem, private_key_pem) = ephemeral_identity();
    let (address, _temporary_directory, server) =
        spawn_https_server(&certificate_pem, &private_key_pem).await;
    let client = https_client(&certificate_pem, address);

    let response = client
        .get(format!(
            "https://localhost:{}/v1/capabilities",
            address.port()
        ))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    let peer = client
        .get(format!("https://localhost:{}/test-peer", address.port()))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(peer, "127.0.0.1");

    let plaintext = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap()
        .get(format!("http://{address}/v1/capabilities"))
        .send()
        .await;
    assert!(
        plaintext.is_err(),
        "plaintext must fail on an HTTPS listener"
    );

    server.abort();
}

#[cfg(feature = "cli")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_sql_preserves_explicit_https_endpoint_through_tls_handshake() {
    let (certificate_pem, private_key_pem) = ephemeral_identity();
    let (address, _temporary_directory, server) =
        spawn_https_server(&certificate_pem, &private_key_pem).await;
    let endpoint = format!("https://127.0.0.1:{}/", address.port());

    let output = Command::new(env!("CARGO_BIN_EXE_stellar"))
        .args([
            "sql",
            "SELECT 1 AS value",
            "--host",
            &endpoint,
            "--database",
            "test",
            "--json",
        ])
        .output()
        .unwrap();

    server.abort();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!("https://127.0.0.1:{}/sql", address.port())),
        "CLI did not preserve the HTTPS endpoint: {stderr}"
    );
    assert!(!stderr.contains("http://https"));
}

#[cfg(all(feature = "cli", target_os = "linux"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_sql_uses_explicit_https_endpoint() {
    let (certificate_pem, private_key_pem) = ephemeral_identity();
    let (address, temporary_directory, server) =
        spawn_https_server(&certificate_pem, &private_key_pem).await;
    let certificate_path = temporary_directory.path().join("test-root.pem");
    std::fs::write(&certificate_path, certificate_pem).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_stellar"))
        .env("SSL_CERT_FILE", &certificate_path)
        .env_remove("SSL_CERT_DIR")
        .args([
            "sql",
            "SELECT 1 AS value",
            "--host",
            &format!("https://127.0.0.1:{}/", address.port()),
            "--database",
            "test",
            "--json",
        ])
        .output()
        .unwrap();

    server.abort();
    assert!(
        output.status.success(),
        "CLI HTTPS request failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("value"));
}

#[cfg(feature = "cli")]
#[test]
fn incomplete_and_invalid_cli_tls_config_fail_before_data_open_without_leaks() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let data = temporary_directory.path().join("must-not-exist");
    let incomplete = Command::new(env!("CARGO_BIN_EXE_stellar"))
        .args([
            "serve",
            "--quiet",
            "--tls-cert-file",
            "certificate.pem",
            "--data",
            data.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!incomplete.status.success());
    assert!(!data.exists());
    assert!(String::from_utf8_lossy(&incomplete.stderr).contains("must be provided together"));

    let env_data = temporary_directory.path().join("env-must-not-exist");
    let incomplete_env = Command::new(env!("CARGO_BIN_EXE_stellar"))
        .env_remove("STELLARDB_TLS_CERT_FILE")
        .env_remove("STELLARDB_TLS_KEY_FILE")
        .env("STELLARDB_TLS_CERT_FILE", "certificate.pem")
        .args(["serve", "--quiet", "--data", env_data.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!incomplete_env.status.success());
    assert!(!env_data.exists());
    assert!(String::from_utf8_lossy(&incomplete_env.stderr).contains("must be provided together"));

    let marker = "TLS-PRIVATE-CONTENT-MUST-NOT-LEAK";
    let certificate_path = temporary_directory
        .path()
        .join("certificate-with-sensitive-name.pem");
    let private_key_path = temporary_directory
        .path()
        .join("private-key-with-sensitive-name.pem");
    std::fs::write(&certificate_path, marker).unwrap();
    std::fs::write(&private_key_path, marker).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&private_key_path, std::fs::Permissions::from_mode(0o600))
            .unwrap();
    }

    let invalid = Command::new(env!("CARGO_BIN_EXE_stellar"))
        .args([
            "serve",
            "--quiet",
            "--tls-cert-file",
            certificate_path.to_str().unwrap(),
            "--tls-key-file",
            private_key_path.to_str().unwrap(),
            "--data",
            data.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(!data.exists());
    let stderr = String::from_utf8_lossy(&invalid.stderr);
    assert!(stderr.contains("invalid TLS certificate or private key"));
    assert!(!stderr.contains(marker));
    assert!(!stderr.contains(certificate_path.to_str().unwrap()));
    assert!(!stderr.contains(private_key_path.to_str().unwrap()));
}

#[cfg(feature = "grpc")]
#[tokio::test]
async fn grpc_tls_query_works_and_plaintext_is_rejected() {
    use stellardb::server::grpc::proto;
    use stellardb::server::grpc::proto::query_service_client::QueryServiceClient;
    use stellardb::server::grpc::{GrpcConfig, GrpcServer};
    use tonic::Request;
    use tonic::metadata::MetadataValue;
    use tonic::transport::{Certificate, Channel, ClientTlsConfig};

    let (certificate_pem, private_key_pem) = ephemeral_identity();
    let tls = TlsConfig::from_pem(certificate_pem.clone(), private_key_pem).unwrap();
    let temporary_directory = tempfile::tempdir().unwrap();
    let namespace = Arc::new(Namespace::open(temporary_directory.path()).unwrap());
    namespace.create_database("test").unwrap();
    let (_app, state) = create_router_with_config(namespace, None, None, Default::default());
    let address = reserve_address();
    let server = GrpcServer::with_config(
        state,
        GrpcConfig {
            tls: Some(tls),
            ..Default::default()
        },
    );
    let server = tokio::spawn(async move {
        server.start(address).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(75)).await;

    let plaintext_channel = Channel::from_shared(format!("http://{address}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let plaintext_request = Request::new(proto::ExecuteRequest {
        query: "SELECT 1".to_string(),
        session_id: None,
        query_timeout: None,
    });
    let plaintext = tokio::time::timeout(
        Duration::from_secs(2),
        QueryServiceClient::new(plaintext_channel).execute(plaintext_request),
    )
    .await;
    assert!(
        !matches!(plaintext, Ok(Ok(_))),
        "plaintext RPC must fail on a TLS gRPC listener"
    );

    let channel = Channel::from_shared(format!("https://{address}"))
        .unwrap()
        .tls_config(
            ClientTlsConfig::new()
                .ca_certificate(Certificate::from_pem(&certificate_pem))
                .domain_name("localhost"),
        )
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut request = Request::new(proto::ExecuteRequest {
        query: "SELECT 1 AS value".to_string(),
        session_id: None,
        query_timeout: None,
    });
    request
        .metadata_mut()
        .insert("x-database", MetadataValue::from_static("test"));
    let response = QueryServiceClient::new(channel)
        .execute(request)
        .await
        .unwrap()
        .into_inner();
    assert_eq!(response.completed, 1);
    assert!(response.error.is_none());

    server.abort();
}

#[test]
fn pem_certificate_chain_is_accepted() {
    let (certificate_pem, private_key_pem) = ephemeral_identity();
    let mut chain = certificate_pem.clone();
    chain.extend_from_slice(&certificate_pem);
    assert!(TlsConfig::from_pem(chain, private_key_pem).is_ok());
}
