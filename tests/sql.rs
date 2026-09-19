mod common;

use serde_json::json;

#[tokio::test]
async fn test_sql_select_all() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create some documents
    client
        .post(format!("http://{}/collections/user/documents", addr))
        .json(&json!({"id": "user:alice", "name": "Alice", "age": 30}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/collections/user/documents", addr))
        .json(&json!({"id": "user:bob", "name": "Bob", "age": 25}))
        .send()
        .await
        .unwrap();

    // Execute SQL
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["completed"], 1);
    assert_eq!(body["results"][0]["count"], 2);
    assert!(body["results"][0]["data"].as_array().unwrap().len() == 2);
}

#[tokio::test]
async fn test_sql_select_by_id() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/collections/user/documents", addr))
        .json(&json!({"id": "user:alice", "name": "Alice"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user:alice"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["name"], "Alice");
}

#[tokio::test]
async fn test_sql_select_with_where() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/collections/user/documents", addr))
        .json(&json!({"id": "user:alice", "name": "Alice", "age": 30}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/collections/user/documents", addr))
        .json(&json!({"id": "user:bob", "name": "Bob", "age": 20}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user WHERE age > 25"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["name"], "Alice");
}

#[tokio::test]
async fn test_sql_create() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // First create the collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "CREATE user:carol SET name = 'Carol', age = 28"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    // CREATE returns the created document in an array
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["id"], "user:carol");

    // Verify document exists
    let res = client
        .get(format!("http://{}/collections/user/documents/carol", addr))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let doc: serde_json::Value = res.json().await.unwrap();
    assert_eq!(doc["id"], "user:carol");
    assert_eq!(doc["name"], "Carol");
    assert_eq!(doc["age"], 28);
}

#[tokio::test]
async fn test_sql_update() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/collections/user/documents", addr))
        .json(&json!({"id": "user:dave", "name": "Dave", "age": 25}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "UPDATE user:dave SET age = 26"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    // UPDATE returns the updated document in an array
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["age"], 26);

    // Verify the update via GET
    let res = client
        .get(format!("http://{}/collections/user/documents/dave", addr))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let doc: serde_json::Value = res.json().await.unwrap();
    assert_eq!(doc["age"], 26);
}

#[tokio::test]
async fn test_sql_delete() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/collections/user/documents", addr))
        .json(&json!({"id": "user:eve", "name": "Eve"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DELETE user:eve"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);

    // Verify deleted
    let res = client
        .get(format!("http://{}/collections/user/documents/eve", addr))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn test_sql_parse_error() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INVALID QUERY"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 400);
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_object(),
        "Expected structured error object"
    );
    assert!(
        body["error"]["code"]
            .as_str()
            .unwrap()
            .starts_with("SDB-QP"),
        "Parse errors should have SDB-QP code, got: {}",
        body["error"]["code"]
    );
    assert_eq!(body["error"]["severity"], "ERROR");
}

#[tokio::test]
async fn test_sql_insert_single() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // First create the collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {name: 'Alice', age: 30}"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    // INSERT returns the created documents
    assert_eq!(body["results"][0]["count"], 1);
    assert!(
        body["results"][0]["data"][0]["id"]
            .as_str()
            .unwrap()
            .starts_with("user:")
    );
    assert_eq!(body["results"][0]["data"][0]["name"], "Alice");
    assert_eq!(body["results"][0]["data"][0]["age"], 30);

    // Verify the insert via SELECT
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user WHERE name = 'Alice'"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 1);
    assert!(
        body["results"][0]["data"][0]["id"]
            .as_str()
            .unwrap()
            .starts_with("user:")
    );
    assert_eq!(body["results"][0]["data"][0]["name"], "Alice");
    assert_eq!(body["results"][0]["data"][0]["age"], 30);
}

#[tokio::test]
async fn test_sql_insert_bulk() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // First create the collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {name: 'Alice'}, {name: 'Bob'}, {name: 'Carol'}"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    // INSERT returns the created documents
    assert_eq!(body["results"][0]["count"], 3);
    assert_eq!(body["results"][0]["data"].as_array().unwrap().len(), 3);

    // Verify all users exist
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM user"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 3);
}

#[tokio::test]
async fn test_sql_insert_with_id() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // First create the collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'alice', name: 'Alice'}"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    // INSERT returns the created document
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["id"], "user:alice");

    // Verify via GET
    let res = client
        .get(format!("http://{}/collections/user/documents/alice", addr))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let doc: serde_json::Value = res.json().await.unwrap();
    assert_eq!(doc["id"], "user:alice");
}

#[tokio::test]
async fn test_sql_insert_duplicate_error() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // First create the collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    // First insert
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'alice', name: 'Alice'}"}))
        .send()
        .await
        .unwrap();

    // Duplicate insert should return 200 with error in response
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'alice', name: 'Alice2'}"}))
        .send()
        .await
        .unwrap();

    // With batch execution, errors are returned in the response body
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_object(), "Expected batch error object");
    assert_eq!(body["completed"], 0);
    // Batch errors include statement_index and message
    assert_eq!(body["error"]["statement_index"], 0);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("already exists"),
        "Expected 'already exists' error, got: {}",
        body["error"]["message"]
    );
}

#[tokio::test]
async fn test_sql_error_response_structure() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Verify the structured error response format for top-level errors
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT ??? broken"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 400);
    let body: serde_json::Value = res.json().await.unwrap();

    // Verify the error is a structured object with required fields
    let error = &body["error"];
    assert!(error.is_object(), "Error should be a structured object");
    assert!(
        error["code"].as_str().unwrap().starts_with("SDB-"),
        "Error code should start with SDB-, got: {}",
        error["code"]
    );
    assert!(
        error["message"].as_str().is_some(),
        "Error should have a message field"
    );
    assert_eq!(
        error["severity"].as_str().unwrap(),
        "ERROR",
        "Error severity should be ERROR"
    );
}

#[tokio::test]
async fn test_sql_invalid_datetime_error_code() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": r#"SELECT d"not-a-date" as dt"#}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 400);
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_object(),
        "Expected structured error object"
    );
    assert_eq!(
        body["error"]["code"].as_str().unwrap(),
        "SDB-QP007",
        "Invalid datetime should return SDB-QP007"
    );
    assert_eq!(body["error"]["severity"], "ERROR");
}

#[tokio::test]
async fn test_sql_empty_input_error_code() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": ""}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 400);
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_object(),
        "Expected structured error object"
    );
    assert_eq!(
        body["error"]["code"].as_str().unwrap(),
        "SDB-QP003",
        "Empty input should return SDB-QP003"
    );
    assert_eq!(body["error"]["severity"], "ERROR");
}
