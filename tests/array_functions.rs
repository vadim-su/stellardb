// Integration tests for array functions
// Tests the full query pipeline: parsing -> binding -> execution

mod common;

use serde_json::json;

async fn setup_test_data(addr: &std::net::SocketAddr, client: &reqwest::Client) {
    // Define flexible collection (schema-less) to allow any fields including arrays
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION arrtest"}))
        .send()
        .await
        .unwrap();

    // Insert test data
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO arrtest {"id": "u1", "name": "Alice", "items": [1, 2, 3], "tags": ["rust", "db", "sql"], "nested": [[1, 2], [3, 4]]}"#
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO arrtest {"id": "u2", "name": "Bob", "items": [4, 5], "tags": ["python", "ml"], "nested": [[5], [6, 7, 8]]}"#
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO arrtest {"id": "u3", "name": "Charlie", "items": [], "tags": ["rust", "wasm"], "nested": [[], [9]]}"#
        }))
        .send()
        .await
        .unwrap();
}

// =============================================================================
// array::length in SELECT
// =============================================================================

#[tokio::test]
async fn test_array_length_in_select() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT name, array::length(items) AS item_count FROM arrtest"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 3);

    let data = &body["results"][0]["data"];
    let arr = data.as_array().expect("data should be an array");

    // Collect name -> length pairs
    let mut lengths: Vec<(&str, i64)> = arr
        .iter()
        .map(|row| {
            (
                row["name"].as_str().unwrap(),
                row["item_count"].as_i64().unwrap(),
            )
        })
        .collect();
    lengths.sort_by_key(|(name, _)| *name);

    assert_eq!(lengths, vec![("Alice", 3), ("Bob", 2), ("Charlie", 0),]);
}

#[tokio::test]
async fn test_array_length_with_null() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION nullarrtest"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO nullarrtest {"id": "n1", "arr": null}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT array::length(arr) AS len FROM nullarrtest"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert!(body["results"][0]["data"][0]["len"].is_null());
}

// =============================================================================
// array::contains in WHERE clause
// =============================================================================

#[tokio::test]
async fn test_array_contains_filter() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name FROM arrtest WHERE array::contains(tags, "rust")"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 2);

    let mut names: Vec<&str> = body["results"][0]["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, vec!["Alice", "Charlie"]);
}

#[tokio::test]
async fn test_array_contains_no_match() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name FROM arrtest WHERE array::contains(tags, "java")"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 0);
}

#[tokio::test]
async fn test_array_contains_with_integer() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT name FROM arrtest WHERE array::contains(items, 2)"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["name"], "Alice");
}

#[tokio::test]
async fn test_array_contains_in_projection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, array::contains(tags, "rust") AS is_rust FROM arrtest"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 3);

    let data = &body["results"][0]["data"];
    let arr = data.as_array().expect("data should be an array");

    let mut results: Vec<(&str, bool)> = arr
        .iter()
        .map(|row| {
            (
                row["name"].as_str().unwrap(),
                row["is_rust"].as_bool().unwrap(),
            )
        })
        .collect();
    results.sort_by_key(|(name, _)| *name);

    assert_eq!(
        results,
        vec![("Alice", true), ("Bob", false), ("Charlie", true),]
    );
}

// =============================================================================
// array::append in projection
// =============================================================================

#[tokio::test]
async fn test_array_append_in_select() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT name, array::append(items, 99) AS extended FROM arrtest WHERE name = \"Alice\""
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);

    let extended = &body["results"][0]["data"][0]["extended"];
    let arr = extended.as_array().expect("extended should be an array");
    assert_eq!(arr.len(), 4);
    assert_eq!(arr, &[1, 2, 3, 99]);
}

#[tokio::test]
async fn test_array_append_to_empty() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, array::append(items, 1) AS extended FROM arrtest WHERE name = "Charlie""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);

    let extended = &body["results"][0]["data"][0]["extended"];
    let arr = extended.as_array().expect("extended should be an array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr, &[1]);
}

#[tokio::test]
async fn test_array_append_string() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, array::append(tags, "new_tag") AS new_tags FROM arrtest WHERE name = "Alice""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);

    let new_tags = &body["results"][0]["data"][0]["new_tags"];
    let arr = new_tags.as_array().expect("new_tags should be an array");
    assert_eq!(arr.len(), 4);
    assert_eq!(arr, &["rust", "db", "sql", "new_tag"]);
}

#[tokio::test]
async fn test_array_append_null_array() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION nullappend"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO nullappend {"id": "n1", "arr": null}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT array::append(arr, 1) AS result FROM nullappend"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert!(body["results"][0]["data"][0]["result"].is_null());
}

// =============================================================================
// array::reverse
// =============================================================================

#[tokio::test]
async fn test_array_reverse() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, array::reverse(items) AS reversed FROM arrtest WHERE name = "Alice""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);

    let reversed = &body["results"][0]["data"][0]["reversed"];
    let arr = reversed.as_array().expect("reversed should be an array");
    assert_eq!(arr, &[3, 2, 1]);
}

#[tokio::test]
async fn test_array_reverse_empty() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, array::reverse(items) AS reversed FROM arrtest WHERE name = "Charlie""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);

    let reversed = &body["results"][0]["data"][0]["reversed"];
    let arr = reversed.as_array().expect("reversed should be an array");
    assert!(arr.is_empty());
}

#[tokio::test]
async fn test_array_reverse_strings() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, array::reverse(tags) AS reversed FROM arrtest WHERE name = "Alice""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);

    let reversed = &body["results"][0]["data"][0]["reversed"];
    let arr = reversed.as_array().expect("reversed should be an array");
    assert_eq!(arr, &["sql", "db", "rust"]);
}

#[tokio::test]
async fn test_array_reverse_null() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION nullrev"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO nullrev {"id": "n1", "arr": null}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT array::reverse(arr) AS result FROM nullrev"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert!(body["results"][0]["data"][0]["result"].is_null());
}

// =============================================================================
// array::flatten
// =============================================================================

#[tokio::test]
async fn test_array_flatten() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, array::flatten(nested) AS flat FROM arrtest WHERE name = "Alice""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);

    let flat = &body["results"][0]["data"][0]["flat"];
    let arr = flat.as_array().expect("flat should be an array");
    // [[1, 2], [3, 4]] -> [1, 2, 3, 4]
    assert_eq!(arr, &[1, 2, 3, 4]);
}

#[tokio::test]
async fn test_array_flatten_mixed_sizes() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, array::flatten(nested) AS flat FROM arrtest WHERE name = "Bob""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);

    let flat = &body["results"][0]["data"][0]["flat"];
    let arr = flat.as_array().expect("flat should be an array");
    // [[5], [6, 7, 8]] -> [5, 6, 7, 8]
    assert_eq!(arr, &[5, 6, 7, 8]);
}

#[tokio::test]
async fn test_array_flatten_with_empty() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, array::flatten(nested) AS flat FROM arrtest WHERE name = "Charlie""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);

    let flat = &body["results"][0]["data"][0]["flat"];
    let arr = flat.as_array().expect("flat should be an array");
    // [[], [9]] -> [9]
    assert_eq!(arr, &[9]);
}

#[tokio::test]
async fn test_array_flatten_null() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION nullflat"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO nullflat {"id": "n1", "arr": null}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT array::flatten(arr) AS result FROM nullflat"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert!(body["results"][0]["data"][0]["result"].is_null());
}

// =============================================================================
// Chained array functions
// =============================================================================

#[tokio::test]
async fn test_chained_flatten_reverse() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, array::reverse(array::flatten(nested)) AS result FROM arrtest WHERE name = "Alice""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);

    let result = &body["results"][0]["data"][0]["result"];
    let arr = result.as_array().expect("result should be an array");
    // [[1, 2], [3, 4]] -> [1, 2, 3, 4] -> [4, 3, 2, 1]
    assert_eq!(arr, &[4, 3, 2, 1]);
}

#[tokio::test]
async fn test_chained_append_length() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, array::length(array::append(items, 99)) AS new_len FROM arrtest WHERE name = "Alice""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    // [1, 2, 3] + 99 -> [1, 2, 3, 99] -> length 4
    assert_eq!(body["results"][0]["data"][0]["new_len"], 4);
}

// =============================================================================
// Combining array functions with other SQL features
// =============================================================================

#[tokio::test]
async fn test_array_functions_with_order() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT name, array::length(items) AS item_count FROM arrtest ORDER array::length(items) DESC"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 3);

    let data = &body["results"][0]["data"];
    let arr = data.as_array().expect("data should be an array");

    // Should be ordered by length descending: Alice (3), Bob (2), Charlie (0)
    assert_eq!(arr[0]["name"], "Alice");
    assert_eq!(arr[0]["item_count"], 3);
    assert_eq!(arr[1]["name"], "Bob");
    assert_eq!(arr[1]["item_count"], 2);
    assert_eq!(arr[2]["name"], "Charlie");
    assert_eq!(arr[2]["item_count"], 0);
}

#[tokio::test]
async fn test_array_functions_with_limit() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT name, array::length(items) AS item_count FROM arrtest LIMIT 2"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 2);
}

#[tokio::test]
async fn test_array_contains_with_and() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name FROM arrtest WHERE array::contains(tags, "rust") AND array::length(items) > 0"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    // Alice has rust and 3 items, Charlie has rust but 0 items
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["name"], "Alice");
}

#[tokio::test]
async fn test_array_contains_with_or() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name FROM arrtest WHERE array::contains(tags, "python") OR array::contains(tags, "wasm")"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 2);

    let mut names: Vec<&str> = body["results"][0]["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, vec!["Bob", "Charlie"]);
}

// =============================================================================
// array::distinct
// =============================================================================

#[tokio::test]
async fn test_array_distinct_integers() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT array::distinct([1, 2, 1, 3, 2, 1]) AS result FROM arrtest LIMIT 1"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);

    let result = &body["results"][0]["data"][0]["result"];
    let arr = result.as_array().expect("result should be an array");
    // [1, 2, 1, 3, 2, 1] -> [1, 2, 3]
    assert_eq!(arr, &[1, 2, 3]);
}

#[tokio::test]
async fn test_array_distinct_strings() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT array::distinct(["a", "b", "a", "c", "b"]) AS result FROM arrtest LIMIT 1"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);

    let result = &body["results"][0]["data"][0]["result"];
    let arr = result.as_array().expect("result should be an array");
    assert_eq!(arr, &["a", "b", "c"]);
}

#[tokio::test]
async fn test_array_distinct_preserves_order() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT array::distinct([3, 1, 2, 1, 3, 2]) AS result FROM arrtest LIMIT 1"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let result = &body["results"][0]["data"][0]["result"];
    let arr = result.as_array().expect("result should be an array");
    // First occurrences: 3, 1, 2 (in that order)
    assert_eq!(arr, &[3, 1, 2]);
}

#[tokio::test]
async fn test_array_distinct_empty() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT array::distinct([]) AS result FROM arrtest LIMIT 1"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let result = &body["results"][0]["data"][0]["result"];
    let arr = result.as_array().expect("result should be an array");
    assert!(arr.is_empty());
}

#[tokio::test]
async fn test_array_distinct_null_input() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION nulldistinct"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO nulldistinct {"id": "n1", "arr": null}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT array::distinct(arr) AS result FROM nulldistinct"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert!(body["results"][0]["data"][0]["result"].is_null());
}

#[tokio::test]
async fn test_array_distinct_on_field() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION distinctfield"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO distinctfield {"id": "d1", "nums": [1, 2, 2, 3, 1]}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT array::distinct(nums) AS unique_nums FROM distinctfield"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let result = &body["results"][0]["data"][0]["unique_nums"];
    let arr = result.as_array().expect("result should be an array");
    assert_eq!(arr, &[1, 2, 3]);
}
