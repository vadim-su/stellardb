#![cfg(all(feature = "server", feature = "ui"))]

use std::sync::Arc;

use stellardb::namespace::Namespace;
use stellardb::server::create_router;

#[tokio::test]
async fn station_index_accepts_the_canonical_trailing_slash_url() {
    let tmp = tempfile::tempdir().unwrap();
    let namespace = Arc::new(Namespace::open(tmp.path()).unwrap());
    let (app, _state) = create_router(namespace, None, None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let response = reqwest::get(format!("http://{addr}/_station/"))
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/html")
    );
    assert!(response.text().await.unwrap().contains("StellarDB Station"));

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let root = client.get(format!("http://{addr}/")).send().await.unwrap();
    assert_eq!(root.status(), reqwest::StatusCode::PERMANENT_REDIRECT);
    assert_eq!(
        root.headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/_station/")
    );
}
