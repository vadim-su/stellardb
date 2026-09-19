// HTTP security integration tests
// Tests for path traversal prevention in database names

use crate::common;
use serde_json::json;

// =============================================================================
// Path Traversal in Database Names
// =============================================================================

#[tokio::test]
async fn test_create_database_rejects_path_traversal_dotdot_slash() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = reqwest::Client::new();

    let res = client
        .post(format!("http://{}/databases", addr))
        .json(&json!({"name": "../../etc"}))
        .send()
        .await
        .unwrap();

    assert!(
        res.status().is_client_error(),
        "Creating database with '../../etc' should fail, got {}",
        res.status()
    );
}

#[tokio::test]
async fn test_create_database_rejects_forward_slash() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = reqwest::Client::new();

    let res = client
        .post(format!("http://{}/databases", addr))
        .json(&json!({"name": "foo/bar"}))
        .send()
        .await
        .unwrap();

    assert!(
        res.status().is_client_error(),
        "Creating database with 'foo/bar' should fail, got {}",
        res.status()
    );
}

#[tokio::test]
async fn test_create_database_rejects_backslash() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = reqwest::Client::new();

    let res = client
        .post(format!("http://{}/databases", addr))
        .json(&json!({"name": "foo\\bar"}))
        .send()
        .await
        .unwrap();

    assert!(
        res.status().is_client_error(),
        "Creating database with 'foo\\bar' should fail, got {}",
        res.status()
    );
}

#[tokio::test]
async fn test_create_database_rejects_dotdot() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = reqwest::Client::new();

    let res = client
        .post(format!("http://{}/databases", addr))
        .json(&json!({"name": ".."}))
        .send()
        .await
        .unwrap();

    assert!(
        res.status().is_client_error(),
        "Creating database with '..' should fail, got {}",
        res.status()
    );
}

#[tokio::test]
async fn test_create_database_rejects_dot() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = reqwest::Client::new();

    let res = client
        .post(format!("http://{}/databases", addr))
        .json(&json!({"name": "."}))
        .send()
        .await
        .unwrap();

    assert!(
        res.status().is_client_error(),
        "Creating database with '.' should fail, got {}",
        res.status()
    );
}

#[tokio::test]
async fn test_drop_database_rejects_path_traversal() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = reqwest::Client::new();

    let res = client
        .delete(format!("http://{}/databases/..%2F..%2Fetc", addr))
        .send()
        .await
        .unwrap();

    assert!(
        res.status().is_client_error(),
        "Dropping database with path traversal should fail, got {}",
        res.status()
    );
}

#[tokio::test]
async fn test_x_database_header_rejects_path_traversal() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = reqwest::Client::new();

    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Database", "../../etc")
        .json(&json!({"query": "SELECT * FROM test"}))
        .send()
        .await
        .unwrap();

    assert!(
        res.status().is_client_error(),
        "X-Database header with path traversal should fail, got {}",
        res.status()
    );
}

#[tokio::test]
async fn test_x_database_header_rejects_forward_slash() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = reqwest::Client::new();

    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Database", "foo/bar")
        .json(&json!({"query": "SELECT * FROM test"}))
        .send()
        .await
        .unwrap();

    assert!(
        res.status().is_client_error(),
        "X-Database header with 'foo/bar' should fail, got {}",
        res.status()
    );
}
