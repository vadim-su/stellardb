use std::sync::Arc;

use stellardb::namespace::Namespace;
use stellardb::server::create_router;

#[tokio::test]
async fn capabilities_exposes_the_versioned_server_contract() {
    let tmp = tempfile::tempdir().unwrap();
    let namespace = Arc::new(Namespace::open(tmp.path()).unwrap());
    let (app, _state) = create_router(namespace, None, None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let response = reqwest::get(format!("http://{addr}/v1/capabilities"))
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = response.json().await.unwrap();

    assert_eq!(body["apiVersion"], "v1");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(body["features"]["explain"], true);
    assert_eq!(body["features"]["explainAnalyze"], true);
    assert_eq!(body["features"]["queryCancellation"], false);
    assert_eq!(body["features"]["auditLog"], false);
    assert_eq!(body["features"]["schemaMutations"], true);
    assert_eq!(body["features"]["importJobs"], false);
}
