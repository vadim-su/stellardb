mod common;

use serde_json::json;

/// Test basic FETCH - replaces reference with document
#[tokio::test]
async fn test_fetch_basic() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collections
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION post"}))
        .send()
        .await
        .unwrap();

    // Setup: create user and post with reference
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'alice', name: 'Alice', age: 30}"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO post {id: '1', title: 'Hello', author: user:alice}"}))
        .send()
        .await
        .unwrap();

    // Test FETCH
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM post FETCH author"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let data = &body["results"][0]["data"][0];

    // author should be expanded object, not reference string
    assert!(
        data["author"].is_object(),
        "author should be object after FETCH: {:?}",
        body
    );
    assert_eq!(data["author"]["name"], "Alice");
    assert_eq!(data["author"]["age"], 30);
}

/// Test FETCH with AS alias - keeps original, adds expanded copy
#[tokio::test]
async fn test_fetch_with_alias() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collections
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION post"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'alice', name: 'Alice'}"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO post {id: '1', author: user:alice}"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM post FETCH author AS author_doc"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let data = &body["results"][0]["data"][0];

    // Original reference preserved
    assert!(data["author"].as_str().unwrap().contains("alice"));
    // Expanded document in alias
    assert!(data["author_doc"].is_object());
    assert_eq!(data["author_doc"]["name"], "Alice");
}

/// Test FETCH with nested path - author.company
#[tokio::test]
async fn test_fetch_nested_path() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collections
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION company"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION post"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO company {id: 'acme', name: 'ACME Corp'}"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'alice', name: 'Alice', company: company:acme}"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO post {id: '1', author: user:alice}"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM post FETCH author.company"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let data = &body["results"][0]["data"][0];

    // author should be expanded
    assert!(data["author"].is_object());
    // author.company should also be expanded
    assert!(data["author"]["company"].is_object());
    assert_eq!(data["author"]["company"]["name"], "ACME Corp");
}

/// Test FETCH with array of references
#[tokio::test]
async fn test_fetch_array_references() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collections
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION tag"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION post"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO tag {id: 'rust', name: 'Rust'}"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO tag {id: 'db', name: 'Database'}"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO post {id: '1', tags: [tag:rust, tag:db]}"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM post FETCH tags"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let tags = &body["results"][0]["data"][0]["tags"];

    assert!(tags.is_array());
    let tags_arr = tags.as_array().unwrap();
    assert_eq!(tags_arr.len(), 2);
    assert!(tags_arr[0].is_object());
    assert!(tags_arr[1].is_object());

    // Check both tags were expanded
    let names: Vec<&str> = tags_arr
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"Rust"));
    assert!(names.contains(&"Database"));
}

/// Test FETCH with missing document - returns null
#[tokio::test]
async fn test_fetch_missing_document() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION post"}))
        .send()
        .await
        .unwrap();

    // Post with reference to non-existent user
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO post {id: '1', author: user:nobody}"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM post FETCH author"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let data = &body["results"][0]["data"][0];

    // Missing document should become null
    assert!(data["author"].is_null());
}

/// Test multiple FETCH items
#[tokio::test]
async fn test_fetch_multiple_items() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collections
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION category"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION post"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'alice', name: 'Alice'}"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO category {id: 'tech', name: 'Technology'}"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO post {id: '1', author: user:alice, category: category:tech}"}))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM post FETCH author, category"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let data = &body["results"][0]["data"][0];

    assert!(data["author"].is_object());
    assert_eq!(data["author"]["name"], "Alice");
    assert!(data["category"].is_object());
    assert_eq!(data["category"]["name"], "Technology");
}

// ============================================================================
// Automatic Reference Dereferencing (without FETCH)
// ============================================================================

/// Test automatic reference dereferencing with dot notation
/// SELECT author.name FROM post - automatically loads referenced document
#[tokio::test]
async fn test_auto_deref_simple() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collections
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION post"}))
        .send()
        .await
        .unwrap();

    // Create user
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'alice', name: 'Alice', tier: 'gold', email: 'alice@example.com'}"}))
        .send()
        .await
        .unwrap();

    // Create post with reference to user
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO post {id: '1', title: 'Hello World', author: user:alice}"}))
        .send()
        .await
        .unwrap();

    // Test: SELECT author.name should automatically dereference
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT title, author.name AS author_name, author.tier AS author_tier FROM post:1"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let data = &body["results"][0]["data"][0];

    // Verify auto-dereferencing worked
    assert_eq!(data["title"], "Hello World");
    assert_eq!(
        data["author_name"], "Alice",
        "author.name should auto-dereference: {:?}",
        body
    );
    assert_eq!(
        data["author_tier"], "gold",
        "author.tier should auto-dereference: {:?}",
        body
    );
}

/// Test chained reference dereferencing: author.company.name
#[tokio::test]
async fn test_auto_deref_chained() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collections
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION company"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION post"}))
        .send()
        .await
        .unwrap();

    // Create company
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO company {id: 'acme', name: 'ACME Corp', industry: 'Tech'}"}))
        .send()
        .await
        .unwrap();

    // Create user with reference to company
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'alice', name: 'Alice', company: company:acme}"}))
        .send()
        .await
        .unwrap();

    // Create post with reference to user
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO post {id: '1', title: 'Hello', author: user:alice}"}))
        .send()
        .await
        .unwrap();

    // Test: chained dereferencing - author.company.name
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT title, author.name AS author_name, author.company.name AS company_name FROM post:1"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let data = &body["results"][0]["data"][0];

    assert_eq!(data["title"], "Hello");
    assert_eq!(
        data["author_name"], "Alice",
        "author.name should work: {:?}",
        body
    );
    assert_eq!(
        data["company_name"], "ACME Corp",
        "author.company.name should chain dereference: {:?}",
        body
    );
}

/// Test auto-deref with missing reference - returns null
#[tokio::test]
async fn test_auto_deref_missing_reference() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION post"}))
        .send()
        .await
        .unwrap();

    // Create post with reference to non-existent user
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO post {id: '1', title: 'Hello', author: user:nobody}"}))
        .send()
        .await
        .unwrap();

    // Test: author.name on missing reference should return null
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT title, author.name AS author_name FROM post:1"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let data = &body["results"][0]["data"][0];

    assert_eq!(data["title"], "Hello");
    assert!(
        data["author_name"].is_null(),
        "Missing reference should return null: {:?}",
        body
    );
}

/// Test auto-deref in WHERE clause
#[tokio::test]
async fn test_auto_deref_in_where() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collections
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION post"}))
        .send()
        .await
        .unwrap();

    // Create users
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'alice', name: 'Alice', tier: 'gold'}"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO user {id: 'bob', name: 'Bob', tier: 'silver'}"}))
        .send()
        .await
        .unwrap();

    // Create posts
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "INSERT INTO post {id: '1', title: 'Post by Alice', author: user:alice}"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(
            &json!({"query": "INSERT INTO post {id: '2', title: 'Post by Bob', author: user:bob}"}),
        )
        .send()
        .await
        .unwrap();

    // Test: filter by author.tier
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT title, author.name AS author_name FROM post WHERE author.tier = 'gold'"}))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let results = &body["results"][0]["data"];

    // Should only return Alice's post
    assert_eq!(
        results.as_array().unwrap().len(),
        1,
        "Should find 1 post: {:?}",
        body
    );
    assert_eq!(results[0]["title"], "Post by Alice");
    assert_eq!(results[0]["author_name"], "Alice");
}
