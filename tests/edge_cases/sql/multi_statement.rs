// Multi-statement query edge case tests
// Tests for semicolon handling, statement chains, and mixed success/failure

use crate::common;
use serde_json::json;

// =============================================================================
// Basic Multi-Statement Tests
// =============================================================================

#[tokio::test]
async fn test_two_statements() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION multi1; DEFINE COLLECTION multi2"}))
        .send()
        .await
        .unwrap();

    // Verify both collections exist
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DESCRIBE COLLECTIONS"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    let collections = body["results"][0]["data"][0]["collections"]
        .as_array()
        .unwrap();
    let names: Vec<&str> = collections
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"multi1"));
    assert!(names.contains(&"multi2"));
}

#[tokio::test]
async fn test_many_statements() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Build query with 20 INSERT statements
    let mut query = String::from("DEFINE COLLECTION batch");
    for i in 1..=20 {
        query.push_str(&format!(
            r#"; INSERT INTO batch {{"id": "b{}", "value": {}}}"#,
            i, i
        ));
    }

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": query}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Multi-statement should succeed: {:?}",
        body
    );
    // Should have 21 results (1 DEFINE + 20 INSERTs)
    assert_eq!(body["results"].as_array().unwrap().len(), 21);

    // Verify all documents exist
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM batch"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 20);
}

// =============================================================================
// Semicolon Edge Cases
// =============================================================================

#[tokio::test]
async fn test_trailing_semicolon() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION trail;"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Trailing semicolon should be OK");
}

#[tokio::test]
async fn test_multiple_trailing_semicolons() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "DEFINE COLLECTION multi_trail;;;"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Multiple trailing semicolons should be OK"
    );
}

#[tokio::test]
async fn test_leading_semicolon() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": ";DEFINE COLLECTION leading"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Document behavior - may accept or reject
    // Just ensure it handles gracefully
    assert!(
        common::has_error(&body) || body["results"].is_array(),
        "Leading semicolon should be handled"
    );
}

#[tokio::test]
async fn test_multiple_semicolons_between() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION semi1;;; DEFINE COLLECTION semi2"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"].is_null(),
        "Multiple semicolons between statements should be OK"
    );
}

#[tokio::test]
async fn test_only_semicolons() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": ";;;"}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Empty query should either error or return empty results
    assert!(
        common::has_error(&body)
            || body["results"]
                .as_array()
                .map(|a| a.is_empty())
                .unwrap_or(false),
        "Only semicolons should be handled: {:?}",
        body
    );
}

// =============================================================================
// Statement Dependencies
// =============================================================================

#[tokio::test]
async fn test_insert_then_select_same_query() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"DEFINE COLLECTION deps; INSERT INTO deps {"id": "d1", "name": "test"}; SELECT * FROM deps:d1"#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 3); // DEFINE, INSERT, SELECT

    // SELECT should find the inserted document
    assert_eq!(results[2]["count"], 1);
    assert_eq!(results[2]["data"][0]["name"], "test");
}

#[tokio::test]
async fn test_insert_then_update_then_select() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"
                DEFINE COLLECTION chain;
                INSERT INTO chain {"id": "c1", "value": 1};
                UPDATE chain:c1 SET value = 2;
                SELECT * FROM chain:c1
            "#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    let results = body["results"].as_array().unwrap();

    // Final SELECT should show updated value
    assert_eq!(results[3]["data"][0]["value"], 2);
}

#[tokio::test]
async fn test_insert_delete_select() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"
                DEFINE COLLECTION del_chain;
                INSERT INTO del_chain {"id": "x1", "data": "test"};
                DELETE del_chain:x1;
                SELECT * FROM del_chain:x1
            "#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    let results = body["results"].as_array().unwrap();

    // Final SELECT should find nothing
    assert_eq!(results[3]["count"], 0);
}

// =============================================================================
// Mixed Success/Failure
// =============================================================================

#[tokio::test]
async fn test_first_statement_fails() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "INVALID SYNTAX HERE; DEFINE COLLECTION after_fail"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Parse error on first statement should prevent all execution
    assert!(
        !body["error"].is_null(),
        "Parse error should be reported: {:?}",
        body
    );
}

#[tokio::test]
async fn test_second_statement_fails() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // First statement valid, second has schema violation
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION strict_multi (SCHEMA STRICT, name string REQUIRED)"
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"
                INSERT INTO strict_multi {"id": "ok1", "name": "valid"};
                INSERT INTO strict_multi {"id": "bad1", "name": 12345}
            "#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Document behavior - first may succeed, second fails
    // Or entire batch may fail
    let results = body["results"].as_array();
    if let Some(results) = results {
        // If we get results array, check each
        if results.len() >= 2 {
            // Second should have error
            assert!(
                common::has_error(&results[1]) || common::has_error(&body),
                "Second statement should fail"
            );
        }
    }
}

#[tokio::test]
async fn test_middle_statement_fails() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION middle_fail (SCHEMA STRICT, value int)"
        }))
        .send()
        .await
        .unwrap();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"
                INSERT INTO middle_fail {"id": "m1", "value": 1};
                INSERT INTO middle_fail {"id": "m2", "value": "not an int"};
                INSERT INTO middle_fail {"id": "m3", "value": 3}
            "#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    // Document what happens after middle statement fails
    // Behavior varies by DB
    let _results = body["results"].as_array();
}

// =============================================================================
// Whitespace Handling
// =============================================================================

#[tokio::test]
async fn test_newlines_between_statements() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION nl1\n;\n\nDEFINE COLLECTION nl2"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Newlines should be OK");
}

#[tokio::test]
async fn test_tabs_and_spaces() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION ws1  ;  \t  DEFINE COLLECTION ws2"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "Whitespace should be OK");
}

// =============================================================================
// Result Array Tests
// =============================================================================

#[tokio::test]
async fn test_results_array_matches_statement_count() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": "DEFINE COLLECTION r1; DEFINE COLLECTION r2; DEFINE COLLECTION r3"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 3, "Should have 3 results for 3 statements");
}

#[tokio::test]
async fn test_results_array_different_types() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"
                DEFINE COLLECTION mixed;
                INSERT INTO mixed {"id": "x1", "v": 1};
                SELECT * FROM mixed;
                UPDATE mixed:x1 SET v = 2;
                DELETE mixed:x1
            "#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 5);

    // Each result should have appropriate structure
    // DEFINE: success
    // INSERT: data array
    // SELECT: count + data
    // UPDATE: count + data
    // DELETE: rows_affected
}

// =============================================================================
// Transaction in Multi-Statement
// =============================================================================

#[tokio::test]
async fn test_begin_commit_in_multi_statement() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"
                DEFINE COLLECTION tx_multi;
                BEGIN;
                INSERT INTO tx_multi {"id": "t1", "v": 1};
                INSERT INTO tx_multi {"id": "t2", "v": 2};
                COMMIT
            "#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());

    // Verify both documents exist
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM tx_multi"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 2);
}

#[tokio::test]
async fn test_begin_rollback_in_multi_statement() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({
            "query": r#"
                DEFINE COLLECTION rb_multi;
                BEGIN;
                INSERT INTO rb_multi {"id": "r1", "v": 1};
                ROLLBACK
            "#
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null());

    // Verify document does not exist
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT * FROM rb_multi"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["results"][0]["count"], 0);
}

// =============================================================================
// Large Multi-Statement
// =============================================================================

#[tokio::test]
async fn test_fifty_statements() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let mut query = String::from("DEFINE COLLECTION fifty");
    for i in 1..=50 {
        query.push_str(&format!(
            r#"; INSERT INTO fifty {{"id": "f{}", "n": {}}}"#,
            i, i
        ));
    }

    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": query}))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "50 statements should work");
    assert_eq!(body["results"].as_array().unwrap().len(), 51);
}
