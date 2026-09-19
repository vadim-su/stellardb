// Integration tests for string functions
// Tests the full query pipeline: parsing -> binding -> execution

mod common;

use serde_json::json;

async fn setup_test_data(addr: &std::net::SocketAddr, client: &reqwest::Client) {
    // Define collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION strtest (name string, email string, path string, text string)"}))
        .send()
        .await
        .unwrap();

    // Insert test data
    for (id, name, email, path, text) in [
        (
            "u1",
            "Alice",
            "alice@example.com",
            "/home/alice/docs",
            "  Hello World  ",
        ),
        ("u2", "Bob", "bob@test.org", "/var/log/app", "   Trimmed   "),
        (
            "u3",
            "Charlie",
            "charlie@example.com",
            "/usr/local/bin",
            "Mixed Case",
        ),
    ] {
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": format!(
                    r#"INSERT INTO strtest {{"id": "{}", "name": "{}", "email": "{}", "path": "{}", "text": "{}"}}"#,
                    id, name, email, path, text
                )
            }))
            .send()
            .await
            .unwrap();
    }
}

// =============================================================================
// string::length in SELECT
// =============================================================================

#[tokio::test]
async fn test_string_length_in_select() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT name, string::length(name) AS name_len FROM strtest"
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
                row["name_len"].as_i64().unwrap(),
            )
        })
        .collect();
    lengths.sort_by_key(|(name, _)| *name);

    assert_eq!(lengths, vec![("Alice", 5), ("Bob", 3), ("Charlie", 7),]);
}

#[tokio::test]
async fn test_string_length_with_null() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION nulltest"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO nulltest {"id": "n1", "val": null}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT string::length(val) AS len FROM nulltest"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert!(body["results"][0]["data"][0]["len"].is_null());
}

// =============================================================================
// string::split returning array
// =============================================================================

#[tokio::test]
async fn test_string_split_path() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, string::split(path, "/") AS parts FROM strtest WHERE name = "Alice""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);

    let parts = &body["results"][0]["data"][0]["parts"];
    let arr = parts.as_array().expect("parts should be an array");
    // "/home/alice/docs" splits into ["", "home", "alice", "docs"]
    assert_eq!(arr.len(), 4);
    assert_eq!(arr[0], "");
    assert_eq!(arr[1], "home");
    assert_eq!(arr[2], "alice");
    assert_eq!(arr[3], "docs");
}

#[tokio::test]
async fn test_string_split_delimiter() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION splitdata"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO splitdata {"id": "s1", "csv": "apple,banana,cherry"}"#
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT string::split(csv, ",") AS items FROM splitdata"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let items = &body["results"][0]["data"][0]["items"];
    let arr = items.as_array().expect("items should be an array");
    assert_eq!(arr, &["apple", "banana", "cherry"]);
}

// =============================================================================
// string::starts_with in WHERE clause
// =============================================================================

#[tokio::test]
async fn test_string_starts_with_filter() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT * FROM strtest WHERE string::starts_with(email, "alice")"#
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
async fn test_string_starts_with_multiple_matches() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    // Both alice@example.com and charlie@example.com have @example.com
    // But only alice and charlie start with specific prefixes
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name FROM strtest WHERE string::starts_with(path, "/home") OR string::starts_with(path, "/usr")"#
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

// =============================================================================
// string::ends_with in WHERE clause
// =============================================================================

#[tokio::test]
async fn test_string_ends_with_filter() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name FROM strtest WHERE string::ends_with(email, "example.com")"#
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

// =============================================================================
// Chained functions: string::upper(string::trim(text))
// =============================================================================

#[tokio::test]
async fn test_chained_upper_trim() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, string::upper(string::trim(text)) AS clean FROM strtest WHERE name = "Alice""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    // "  Hello World  " -> trim -> "Hello World" -> upper -> "HELLO WORLD"
    assert_eq!(body["results"][0]["data"][0]["clean"], "HELLO WORLD");
}

#[tokio::test]
async fn test_chained_lower_trim() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, string::lower(string::trim(text)) AS clean FROM strtest WHERE name = "Charlie""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    // "Mixed Case" -> trim (no-op) -> "Mixed Case" -> lower -> "mixed case"
    assert_eq!(body["results"][0]["data"][0]["clean"], "mixed case");
}

// =============================================================================
// string::substring
// =============================================================================

#[tokio::test]
async fn test_string_substring() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, string::substring(name, 0, 3) AS short FROM strtest"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 3);

    let data = &body["results"][0]["data"];
    let arr = data.as_array().expect("data should be an array");

    let mut shorts: Vec<(&str, &str)> = arr
        .iter()
        .map(|row| {
            (
                row["name"].as_str().unwrap(),
                row["short"].as_str().unwrap(),
            )
        })
        .collect();
    shorts.sort_by_key(|(name, _)| *name);

    assert_eq!(
        shorts,
        vec![("Alice", "Ali"), ("Bob", "Bob"), ("Charlie", "Cha"),]
    );
}

// =============================================================================
// string::replace
// =============================================================================

#[tokio::test]
async fn test_string_replace() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, string::replace(email, ".com", ".net") AS new_email FROM strtest WHERE name = "Alice""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(
        body["results"][0]["data"][0]["new_email"],
        "alice@example.net"
    );
}

// =============================================================================
// Complex chained operations
// =============================================================================

#[tokio::test]
async fn test_complex_string_chain() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    // Extract domain from email: replace @ with space, then get length
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name, string::length(string::replace(email, "@", "_")) AS mod_len FROM strtest WHERE name = "Alice""#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    // "alice@example.com" -> "alice_example.com" -> length = 17
    assert_eq!(body["results"][0]["data"][0]["mod_len"], 17);
}

// =============================================================================
// Combining string functions with other SQL features
// =============================================================================

#[tokio::test]
async fn test_string_functions_with_order() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    // Namespaced function sorting must differ from the underlying scan order.
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT name, string::upper(name) AS upper_name FROM strtest ORDER string::upper(name) DESC"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);

    let data = &body["results"][0]["data"];
    let arr = data.as_array().expect("data should be an array");

    // Descending uppercase keys: CHARLIE, BOB, ALICE.
    assert_eq!(arr[0]["name"], "Charlie");
    assert_eq!(arr[1]["name"], "Bob");
    assert_eq!(arr[2]["name"], "Alice");
}

#[tokio::test]
async fn test_string_functions_with_limit() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_test_data(&addr, &client).await;

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "SELECT name, string::upper(name) AS upper_name FROM strtest LIMIT 2"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 2);
}

// =============================================================================
// string::matches - regex pattern matching
// =============================================================================

#[tokio::test]
async fn test_string_matches_basic() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION matchtest"}))
        .send()
        .await
        .unwrap();

    for (id, color) in [("t1", "grey"), ("t2", "gray"), ("t3", "green")] {
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": format!(r#"INSERT INTO matchtest {{"id": "{}", "color": "{}"}}"#, id, color)
            }))
            .send()
            .await
            .unwrap();
    }

    // Match both grey and gray
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT color FROM matchtest WHERE string::matches(color, "gr(e|a)y")"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 2);

    let mut colors: Vec<&str> = body["results"][0]["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["color"].as_str().unwrap())
        .collect();
    colors.sort();
    assert_eq!(colors, vec!["gray", "grey"]);
}

#[tokio::test]
async fn test_string_matches_case_insensitive() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION matchtest"}))
        .send()
        .await
        .unwrap();

    for (id, name) in [("t1", "Alice"), ("t2", "ALICE"), ("t3", "Bob")] {
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": format!(r#"INSERT INTO matchtest {{"id": "{}", "name": "{}"}}"#, id, name)
            }))
            .send()
            .await
            .unwrap();
    }

    // Case-insensitive with (?i)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT name FROM matchtest WHERE string::matches(name, "(?i)alice")"#
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
    assert_eq!(names, vec!["ALICE", "Alice"]);
}

#[tokio::test]
async fn test_string_matches_substring() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION matchtest"}))
        .send()
        .await
        .unwrap();

    for (id, email) in [("t1", "alice@example.com"), ("t2", "bob@test.org")] {
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": format!(r#"INSERT INTO matchtest {{"id": "{}", "email": "{}"}}"#, id, email)
            }))
            .send()
            .await
            .unwrap();
    }

    // Find emails containing "example"
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT email FROM matchtest WHERE string::matches(email, "example")"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["email"], "alice@example.com");
}

#[tokio::test]
async fn test_string_matches_anchors() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION matchtest"}))
        .send()
        .await
        .unwrap();

    for (id, code) in [("t1", "ABC123"), ("t2", "123ABC")] {
        client
            .post(format!("http://{}/sql", addr))
            .json(&json!({
                "query": format!(r#"INSERT INTO matchtest {{"id": "{}", "code": "{}"}}"#, id, code)
            }))
            .send()
            .await
            .unwrap();
    }

    // Starts with letters
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT code FROM matchtest WHERE string::matches(code, "^[A-Z]+")"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["code"], "ABC123");
}

#[tokio::test]
async fn test_string_matches_in_select() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION matchtest"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO matchtest {"id": "t1", "text": "hello world"}"#
        }))
        .send()
        .await
        .unwrap();

    // Use matches in SELECT projection
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT string::matches(text, "world") AS has_world FROM matchtest"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["has_world"], true);
}

#[tokio::test]
async fn test_string_matches_null() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION matchtest"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO matchtest {"id": "t1", "x": null}"#
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"INSERT INTO matchtest {"id": "t2", "x": "test"}"#
        }))
        .send()
        .await
        .unwrap();

    // NULL input returns NULL (falsy in WHERE)
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"SELECT x FROM matchtest WHERE string::matches(x, "test")"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Error: {:?}", body);
    assert_eq!(body["results"][0]["count"], 1);
    assert_eq!(body["results"][0]["data"][0]["x"], "test");
}
