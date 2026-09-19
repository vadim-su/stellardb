// IN expression integration tests
// Tests for IN and NOT IN operators in WHERE clauses

use crate::common;
use serde_json::json;

async fn setup_tasks(addr: &std::net::SocketAddr, client: &reqwest::Client) {
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION tasks (title string, status string)"}))
        .send()
        .await
        .unwrap();

    for (id, title, status) in [
        ("t1", "Task A", "active"),
        ("t2", "Task B", "pending"),
        ("t3", "Task C", "active"),
        ("t4", "Task D", "deleted"),
    ] {
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": format!(
                    r#"INSERT INTO tasks {{"id": "{}", "title": "{}", "status": "{}"}}"#,
                    id, title, status
                )
            }))
            .send()
            .await
            .unwrap();
    }
}

// =============================================================================
// IN Literal List
// =============================================================================

#[tokio::test]
async fn test_in_literal_list() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_tasks(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT * FROM tasks WHERE status IN ["active", "pending"]"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(
        body["results"][0]["count"], 3,
        "Should find 3 tasks with status 'active' or 'pending'"
    );
}

// =============================================================================
// NOT IN Literal List
// =============================================================================

#[tokio::test]
async fn test_not_in_literal_list() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_tasks(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT * FROM tasks WHERE status NOT IN ["deleted"]"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(
        body["results"][0]["count"], 3,
        "Should find 3 tasks that are not 'deleted'"
    );
}

// =============================================================================
// IN with Variable
// =============================================================================

#[tokio::test]
async fn test_in_variable() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_tasks(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"LET statuses = ["active", "pending"]; SELECT * FROM tasks WHERE status IN $statuses"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(
        body["results"][1]["count"], 3,
        "Should find 3 tasks with status in $statuses variable"
    );
}

#[tokio::test]
async fn test_not_in_variable() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_tasks(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"LET excluded = ["deleted", "archived"]; SELECT * FROM tasks WHERE status NOT IN $excluded"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(
        body["results"][1]["count"], 3,
        "Should find 3 tasks not in $excluded variable"
    );
}

#[tokio::test]
async fn test_in_empty_variable() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_tasks(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"LET empty = []; SELECT * FROM tasks WHERE status IN $empty"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(
        body["results"][1]["count"], 0,
        "Should find 0 tasks when variable is empty array"
    );
}

#[tokio::test]
async fn test_in_variable_with_null() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_tasks(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"LET mixed = ["active", null]; SELECT * FROM tasks WHERE status IN $mixed"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    // Should find 2 active tasks, null in array doesn't match string values
    assert_eq!(
        body["results"][1]["count"], 2,
        "Should find 2 active tasks (null in array doesn't affect matching)"
    );
}
