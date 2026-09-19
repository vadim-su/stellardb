//! Integration tests for IF/FOR/BREAK/CONTINUE control flow expressions

mod common;

use serde_json::json;

async fn query(
    client: &reqwest::Client,
    addr: &std::net::SocketAddr,
    sql: &str,
) -> serde_json::Value {
    let resp = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({ "query": sql }))
        .send()
        .await
        .unwrap();
    resp.json().await.unwrap()
}

// =============================================================================
// IF expression tests
// =============================================================================

#[tokio::test]
async fn test_if_returns_value() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "IF true THEN 42 END").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], 42);
}

#[tokio::test]
async fn test_if_else_true_branch() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "IF true THEN 1 ELSE 2 END").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], 1);
}

#[tokio::test]
async fn test_if_else_false_branch() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "IF false THEN 1 ELSE 2 END").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], 2);
}

#[tokio::test]
async fn test_if_no_else_returns_null() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "IF false THEN 42 END").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert!(result["results"][0]["data"][0].is_null());
}

#[tokio::test]
async fn test_if_else_if() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(
        &client,
        &addr,
        r#"LET x = 7; IF $x > 10 THEN "high" ELSE IF $x > 5 THEN "mid" ELSE "low" END"#,
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][1]["data"][0], "mid");
}

#[tokio::test]
async fn test_if_with_comparison() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "IF 5 > 3 THEN \"yes\" ELSE \"no\" END").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], "yes");
}

#[tokio::test]
async fn test_if_in_let() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "LET val = IF true THEN 42 ELSE 0 END; $val").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][1]["data"][0], 42);
}

// =============================================================================
// FOR expression tests
// =============================================================================

#[tokio::test]
async fn test_for_collects_array() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "FOR $x IN [1, 2, 3] DO $x * 2 END").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], json!([2, 4, 6]));
}

#[tokio::test]
async fn test_for_empty_array() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "FOR $x IN [] DO $x END").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], json!([]));
}

#[tokio::test]
async fn test_for_with_break() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(
        &client,
        &addr,
        "FOR $x IN [1, 2, 3, 4] DO IF $x = 3 THEN BREAK END; $x END",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], json!([1, 2]));
}

#[tokio::test]
async fn test_for_with_continue() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(
        &client,
        &addr,
        "FOR $x IN [1, 2, 3, 4] DO IF $x = 2 THEN CONTINUE END; $x END",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], json!([1, 3, 4]));
}

#[tokio::test]
async fn test_for_with_string_array() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(
        &client,
        &addr,
        r#"FOR $name IN ["alice", "bob"] DO $name END"#,
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], json!(["alice", "bob"]));
}

// =============================================================================
// Block scoping tests
// =============================================================================

#[tokio::test]
async fn test_block_scoping_if() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Variable inside IF block should not leak out
    let result = query(&client, &addr, "LET x = 1; IF true THEN LET x = 2 END; $x").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    // $x should still be 1 (block scoping)
    assert_eq!(result["results"][2]["data"][0], 1);
}

#[tokio::test]
async fn test_for_variable_scoping() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // FOR loop variable should not leak out
    let result = query(&client, &addr, "LET x = 99; FOR $x IN [1, 2] DO $x END; $x").await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    // $x should still be 99 after FOR
    assert_eq!(result["results"][2]["data"][0], 99);
}

// =============================================================================
// Error cases
// =============================================================================

#[tokio::test]
async fn test_break_outside_for_errors() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "BREAK").await;
    // Should have a top-level batch error
    assert!(
        !result["error"].is_null(),
        "Expected error for BREAK outside FOR, got: {:?}",
        result
    );
}

#[tokio::test]
async fn test_continue_outside_for_errors() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(&client, &addr, "CONTINUE").await;
    // Should have a top-level batch error
    assert!(
        !result["error"].is_null(),
        "Expected error for CONTINUE outside FOR, got: {:?}",
        result
    );
}

// =============================================================================
// Nested control flow tests
// =============================================================================

#[tokio::test]
async fn test_nested_if_in_for() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(
        &client,
        &addr,
        "FOR $x IN [1, 2, 3] DO IF $x > 1 THEN $x * 10 ELSE 0 END END",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], json!([0, 20, 30]));
}

#[tokio::test]
async fn test_for_in_let() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(
        &client,
        &addr,
        "LET doubled = FOR $x IN [1, 2, 3] DO $x * 2 END; $doubled",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][1]["data"][0], json!([2, 4, 6]));
}

#[tokio::test]
async fn test_if_with_multiple_statements() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let result = query(
        &client,
        &addr,
        "IF true THEN LET a = 10; LET b = 20; $a + $b END",
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    assert_eq!(result["results"][0]["data"][0], 30);
}

// =============================================================================
// Variable in aggregate projection (regression: variables dropped in aggregate)
// =============================================================================

#[tokio::test]
async fn test_variable_in_aggregate_projection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Setup: create collection with data
    let setup = r#"
        DEFINE COLLECTION item (category string, price int);
        CREATE item:1 SET category = "a", price = 10;
        CREATE item:2 SET category = "a", price = 20;
        CREATE item:3 SET category = "b", price = 30
    "#;
    let r = query(&client, &addr, setup).await;
    assert!(r["error"].is_null(), "Setup error: {:?}", r);

    // Variable alongside COUNT(*) in a non-GROUP query
    let result = query(
        &client,
        &addr,
        r#"LET label = "all_items"; (SELECT $label AS label, COUNT(*) AS cnt FROM item)[0]"#,
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let row = &result["results"][1]["data"][0];
    assert_eq!(
        row["label"], "all_items",
        "Variable should appear in projection"
    );
    assert_eq!(row["cnt"], 3);
}

#[tokio::test]
async fn test_for_variable_in_aggregate_subquery() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Setup: create collection with data
    let setup = r#"
        DEFINE COLLECTION product2 (category string, price int);
        CREATE product2:1 SET category = "X", price = 10;
        CREATE product2:2 SET category = "X", price = 20;
        CREATE product2:3 SET category = "Y", price = 30;
        CREATE product2:4 SET category = "Y", price = 40;
        CREATE product2:5 SET category = "Y", price = 50
    "#;
    let r = query(&client, &addr, setup).await;
    assert!(r["error"].is_null(), "Setup error: {:?}", r);

    // FOR loop with variable in aggregate subquery projection
    let result = query(
        &client,
        &addr,
        r#"FOR $cat IN ["X", "Y"] DO (SELECT $cat AS category, COUNT(*) AS cnt FROM product2 WHERE category = $cat)[0] END"#,
    )
    .await;
    assert!(result["error"].is_null(), "Error: {:?}", result);
    let arr = &result["results"][0]["data"][0];
    let items = arr.as_array().unwrap();
    assert_eq!(items.len(), 2);

    assert_eq!(items[0]["category"], "X");
    assert_eq!(items[0]["cnt"], 2);

    assert_eq!(items[1]["category"], "Y");
    assert_eq!(items[1]["cnt"], 3);
}
