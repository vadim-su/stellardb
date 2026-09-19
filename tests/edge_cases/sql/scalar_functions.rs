// Scalar function integration tests
// Tests for UPPER, LOWER, and scalar functions in projections and WHERE clauses

use crate::common;
use serde_json::json;

async fn setup_users(addr: &std::net::SocketAddr, client: &reqwest::Client) {
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION sf_users (name string)"}))
        .send()
        .await
        .unwrap();

    for (id, name) in [("u1", "Alice"), ("u2", "Bob"), ("u3", "Charlie")] {
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": format!(
                    r#"INSERT INTO sf_users {{"id": "{}", "name": "{}"}}"#,
                    id, name
                )
            }))
            .send()
            .await
            .unwrap();
    }
}

// =============================================================================
// UPPER Function
// =============================================================================

#[tokio::test]
async fn test_upper() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT string::upper(name) AS uname FROM sf_users"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"];
    let arr = data.as_array().expect("data should be an array");
    assert_eq!(arr.len(), 3, "Should return 3 rows");

    let mut names: Vec<&str> = arr
        .iter()
        .map(|row| row["uname"].as_str().expect("uname should be string"))
        .collect();
    names.sort();
    assert_eq!(names, vec!["ALICE", "BOB", "CHARLIE"]);
}

// =============================================================================
// LOWER Function
// =============================================================================

#[tokio::test]
async fn test_lower() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT string::lower(name) AS lname FROM sf_users"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"];
    let arr = data.as_array().expect("data should be an array");
    assert_eq!(arr.len(), 3, "Should return 3 rows");

    let mut names: Vec<&str> = arr
        .iter()
        .map(|row| row["lname"].as_str().expect("lname should be string"))
        .collect();
    names.sort();
    assert_eq!(names, vec!["alice", "bob", "charlie"]);
}

// =============================================================================
// UPPER in WHERE Clause
// =============================================================================

#[tokio::test]
async fn test_upper_in_where() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT * FROM sf_users WHERE string::upper(name) = "ALICE""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(
        body["results"][0]["count"], 1,
        "Should find exactly 1 user with UPPER(name) = ALICE"
    );
    assert_eq!(body["results"][0]["data"][0]["name"], "Alice");
}
