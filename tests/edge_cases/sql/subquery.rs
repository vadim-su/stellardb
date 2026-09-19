//! Subquery and iteration integration tests
//!
//! Phase 1: FROM subquery — `SELECT * FROM (SELECT ...)`
//! Phase 2: FROM field path — `SELECT * FROM collection:key.field`
//! Phase 3: Correlated subquery — `SELECT (SELECT ... WHERE x = $parent.y) FROM ...`

use crate::common;
use serde_json::json;

// =============================================================================
// Helpers
// =============================================================================

async fn query(
    addr: &std::net::SocketAddr,
    client: &reqwest::Client,
    sql: &str,
) -> serde_json::Value {
    let resp = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": sql}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].is_null(), "SQL error: {:?}", body);
    body["results"][0]["data"].clone()
}

async fn query_error(
    addr: &std::net::SocketAddr,
    client: &reqwest::Client,
    sql: &str,
) -> serde_json::Value {
    let resp = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": sql}))
        .send()
        .await
        .unwrap();
    resp.json().await.unwrap()
}

async fn exec(addr: &std::net::SocketAddr, client: &reqwest::Client, sql: &str) {
    client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": sql}))
        .send()
        .await
        .unwrap();
}

async fn setup_users_and_orders(addr: &std::net::SocketAddr, client: &reqwest::Client) {
    exec(addr, client, "DEFINE COLLECTION users").await;
    exec(addr, client, "DEFINE COLLECTION orders").await;

    let queries = [
        r#"INSERT INTO users {id: "alice", name: "alice", age: 30}"#,
        r#"INSERT INTO users {id: "bob", name: "bob", age: 25}"#,
        r#"INSERT INTO users {id: "carol", name: "carol", age: 35}"#,
        r#"INSERT INTO orders {id: "o1", user_name: "alice", amount: 100, status: "active"}"#,
        r#"INSERT INTO orders {id: "o2", user_name: "alice", amount: 200, status: "inactive"}"#,
        r#"INSERT INTO orders {id: "o3", user_name: "bob", amount: 50, status: "active"}"#,
        r#"INSERT INTO orders {id: "o4", user_name: "carol", amount: 300, status: "active"}"#,
    ];

    for q in queries {
        exec(addr, client, q).await;
    }
}

// =============================================================================
// Phase 1: FROM subquery
// =============================================================================

#[tokio::test]
async fn test_from_subquery_basic() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users_and_orders(&addr, &client).await;

    let data = query(&addr, &client, "SELECT * FROM (SELECT * FROM users)").await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 3, "Expected 3 users, got: {:?}", arr);
}

#[tokio::test]
async fn test_from_subquery_with_inner_where() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users_and_orders(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT * FROM (SELECT * FROM users WHERE age > 25)",
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(
        arr.len(),
        2,
        "Expected alice(30) and carol(35), got: {:?}",
        arr
    );

    let names: Vec<&str> = arr.iter().filter_map(|v| v["name"].as_str()).collect();
    assert!(names.contains(&"alice"), "Expected alice in results");
    assert!(names.contains(&"carol"), "Expected carol in results");
}

#[tokio::test]
async fn test_from_subquery_with_outer_where() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users_and_orders(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT * FROM (SELECT * FROM users) WHERE age > 30",
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 1, "Expected only carol(35), got: {:?}", arr);
    assert_eq!(arr[0]["name"], "carol");
}

#[tokio::test]
async fn test_from_subquery_with_inner_limit() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users_and_orders(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT * FROM (SELECT * FROM users LIMIT 2)",
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(
        arr.len(),
        2,
        "Expected 2 users from inner LIMIT, got: {:?}",
        arr
    );
}

#[tokio::test]
async fn test_from_subquery_with_outer_order() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users_and_orders(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT * FROM (SELECT * FROM users) ORDER name",
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0]["name"], "alice");
    assert_eq!(arr[1]["name"], "bob");
    assert_eq!(arr[2]["name"], "carol");
}

#[tokio::test]
async fn test_from_subquery_nested() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users_and_orders(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT * FROM (SELECT * FROM (SELECT * FROM users WHERE age > 25))",
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(
        arr.len(),
        2,
        "Expected alice and carol from nested subquery, got: {:?}",
        arr
    );

    let names: Vec<&str> = arr.iter().filter_map(|v| v["name"].as_str()).collect();
    assert!(names.contains(&"alice"));
    assert!(names.contains(&"carol"));
}

#[tokio::test]
async fn test_from_subquery_with_projection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users_and_orders(&addr, &client).await;

    // Inner query selects specific fields; outer query sees those fields
    let data = query(
        &addr,
        &client,
        "SELECT name FROM (SELECT name, age FROM users WHERE age > 25)",
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 2);

    // Each row should have "name" but not "age" (outer projection strips it)
    for row in arr {
        assert!(row["name"].is_string(), "Expected name field in: {:?}", row);
    }
}

#[tokio::test]
async fn test_from_subquery_with_aggregate() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users_and_orders(&addr, &client).await;

    // Subquery with aggregate — should produce single row, outer select reads it
    let data = query(
        &addr,
        &client,
        "SELECT * FROM (SELECT COUNT(*) AS total FROM users)",
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["total"], 3);
}

#[tokio::test]
async fn test_from_subquery_empty_inner() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users_and_orders(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        "SELECT * FROM (SELECT * FROM users WHERE age > 100)",
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(
        arr.len(),
        0,
        "Expected empty result from subquery with no matches"
    );
}

// =============================================================================
// Phase 2: Correlated subquery in SELECT projection
// =============================================================================

#[tokio::test]
async fn test_correlated_subquery_basic() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users_and_orders(&addr, &client).await;

    // For a single user, fetch their orders as a nested array
    let data = query(
        &addr,
        &client,
        r#"SELECT name, (SELECT * FROM orders WHERE user_name = $parent.name) AS user_orders FROM users:alice"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 1, "Expected 1 row for alice, got: {:?}", arr);

    let alice = &arr[0];
    assert_eq!(alice["name"], "alice");

    let orders = alice["user_orders"].as_array().unwrap();
    assert_eq!(
        orders.len(),
        2,
        "Alice should have 2 orders, got: {:?}",
        orders
    );
}

#[tokio::test]
async fn test_correlated_subquery_all_users() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users_and_orders(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        r#"SELECT name, (SELECT * FROM orders WHERE user_name = $parent.name) AS user_orders FROM users ORDER name"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 3);

    // alice: 2 orders
    let alice = &arr[0];
    assert_eq!(alice["name"], "alice");
    assert_eq!(alice["user_orders"].as_array().unwrap().len(), 2);

    // bob: 1 order
    let bob = &arr[1];
    assert_eq!(bob["name"], "bob");
    assert_eq!(bob["user_orders"].as_array().unwrap().len(), 1);

    // carol: 1 order
    let carol = &arr[2];
    assert_eq!(carol["name"], "carol");
    assert_eq!(carol["user_orders"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_correlated_subquery_count() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users_and_orders(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        r#"SELECT name, (SELECT COUNT(*) FROM orders WHERE user_name = $parent.name) AS order_count FROM users ORDER name"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 3);

    // The subquery returns an array with one aggregate row, e.g. [{"count(*)": 2}]
    let alice = &arr[0];
    assert_eq!(alice["name"], "alice");
    let alice_count = alice["order_count"].as_array().unwrap();
    assert_eq!(alice_count.len(), 1);
    assert_eq!(alice_count[0]["count(*)"], 2);

    let bob = &arr[1];
    assert_eq!(bob["name"], "bob");
    let bob_count = bob["order_count"].as_array().unwrap();
    assert_eq!(bob_count[0]["count(*)"], 1);

    let carol = &arr[2];
    assert_eq!(carol["name"], "carol");
    let carol_count = carol["order_count"].as_array().unwrap();
    assert_eq!(carol_count[0]["count(*)"], 1);
}

#[tokio::test]
async fn test_correlated_subquery_sum() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users_and_orders(&addr, &client).await;

    let data = query(
        &addr,
        &client,
        r#"SELECT name, (SELECT SUM(amount) FROM orders WHERE user_name = $parent.name) AS total_spent FROM users ORDER name"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 3);

    // alice: 100 + 200 = 300
    let alice_total = &arr[0]["total_spent"].as_array().unwrap()[0]["sum(amount)"];
    assert_eq!(alice_total, 300.0);

    // bob: 50
    let bob_total = &arr[1]["total_spent"].as_array().unwrap()[0]["sum(amount)"];
    assert_eq!(bob_total, 50.0);

    // carol: 300
    let carol_total = &arr[2]["total_spent"].as_array().unwrap()[0]["sum(amount)"];
    assert_eq!(carol_total, 300.0);
}

#[tokio::test]
async fn test_correlated_subquery_no_matches() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users_and_orders(&addr, &client).await;

    // Insert a user with no orders
    exec(
        &addr,
        &client,
        r#"INSERT INTO users {id: "dave", name: "dave", age: 40}"#,
    )
    .await;

    let data = query(
        &addr,
        &client,
        r#"SELECT name, (SELECT * FROM orders WHERE user_name = $parent.name) AS user_orders FROM users:dave"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "dave");

    let orders = arr[0]["user_orders"].as_array().unwrap();
    assert_eq!(orders.len(), 0, "Dave should have 0 orders");
}

#[tokio::test]
async fn test_correlated_subquery_with_filter_on_inner() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users_and_orders(&addr, &client).await;

    // Only active orders for alice
    let data = query(
        &addr,
        &client,
        r#"SELECT name, (SELECT * FROM orders WHERE user_name = $parent.name AND status = "active") AS active_orders FROM users:alice"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 1);

    let orders = arr[0]["active_orders"].as_array().unwrap();
    assert_eq!(
        orders.len(),
        1,
        "Alice has 1 active order, got: {:?}",
        orders
    );
    assert_eq!(orders[0]["status"], "active");
}

#[tokio::test]
async fn test_correlated_subquery_with_outer_where() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users_and_orders(&addr, &client).await;

    // Only users older than 25 get their orders
    let data = query(
        &addr,
        &client,
        r#"SELECT name, (SELECT * FROM orders WHERE user_name = $parent.name) AS user_orders FROM users WHERE age > 25 ORDER name"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(
        arr.len(),
        2,
        "Expected alice(30) and carol(35), got: {:?}",
        arr
    );
    assert_eq!(arr[0]["name"], "alice");
    assert_eq!(arr[1]["name"], "carol");
}

#[tokio::test]
async fn test_correlated_subquery_without_alias() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    setup_users_and_orders(&addr, &client).await;

    // Without AS alias, the key should be "(SELECT ...)"
    let data = query(
        &addr,
        &client,
        r#"SELECT name, (SELECT * FROM orders WHERE user_name = $parent.name) FROM users:alice"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 1);

    // The default alias name is "(SELECT ...)"
    let alice = &arr[0];
    assert_eq!(alice["name"], "alice");
    assert!(
        alice["(SELECT ...)"].is_array(),
        "Expected default alias '(SELECT ...)' with array value, got: {:?}",
        alice
    );
}

// =============================================================================
// Regression: CREATE with unquoted object keys in arrays
// =============================================================================

#[tokio::test]
async fn test_create_with_unquoted_keys_in_array_of_objects() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    exec(&addr, &client, "DEFINE COLLECTION orders").await;

    // Use unquoted keys (like Python script does): product_id, quantity, unit_price
    exec(
        &addr,
        &client,
        r#"CREATE orders:1 SET customer_id = 1, items = [{product_id: 1, quantity: 2, unit_price: 50.0}]"#,
    )
    .await;

    let data = query(&addr, &client, "SELECT * FROM orders:1").await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 1, "Expected 1 order, got: {:?}", arr);

    let order = &arr[0];
    assert_eq!(order["customer_id"], 1);

    let items = order["items"].as_array();
    assert!(
        items.is_some(),
        "Expected 'items' field to be an array, got: {:?}",
        order
    );

    let items = items.unwrap();
    assert_eq!(items.len(), 1, "Expected 1 item, got: {:?}", items);

    let item = &items[0];
    assert_eq!(item["product_id"], 1);
    assert_eq!(item["quantity"], 2);
    assert_eq!(item["unit_price"], 50.0);
}

// =============================================================================
// Regression: $parent reference to field NOT in projection
// =============================================================================

#[tokio::test]
async fn test_correlated_subquery_parent_field_not_in_projection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Setup: reviews with customer_name (not in projection), customers matched by name
    exec(&addr, &client, "DEFINE COLLECTION reviews2").await;
    exec(&addr, &client, "DEFINE COLLECTION customers2").await;

    exec(
        &addr,
        &client,
        r#"CREATE customers2:alice SET name = "alice", tier = "gold", balance = 100"#,
    )
    .await;
    exec(
        &addr,
        &client,
        r#"CREATE customers2:bob SET name = "bob", tier = "silver", balance = 50"#,
    )
    .await;
    exec(
        &addr,
        &client,
        r#"CREATE reviews2:r1 SET customer_name = "alice", rating = 5, title = "Excellent""#,
    )
    .await;
    exec(
        &addr,
        &client,
        r#"CREATE reviews2:r2 SET customer_name = "bob", rating = 4, title = "Good""#,
    )
    .await;

    // Bug fix: customer_name is NOT in the projection, but used in $parent.customer_name
    // The engine should automatically fetch it for $parent resolution, then strip it from output
    let data = query(
        &addr,
        &client,
        r#"SELECT
            rating,
            title,
            (SELECT tier, balance FROM customers2 WHERE name = $parent.customer_name) AS reviewer
        FROM reviews2
        ORDER rating DESC"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 2, "Expected 2 reviews, got: {:?}", arr);

    // Review r1 (rating 5) should have Alice as reviewer
    let r1 = &arr[0];
    assert_eq!(r1["rating"], 5);
    assert_eq!(r1["title"], "Excellent");
    let reviewer1 = r1["reviewer"].as_array().unwrap();
    assert_eq!(
        reviewer1.len(),
        1,
        "Expected 1 customer match, got: {:?}",
        reviewer1
    );
    assert_eq!(reviewer1[0]["tier"], "gold");
    assert_eq!(reviewer1[0]["balance"], 100);

    // Review r2 (rating 4) should have Bob as reviewer
    let r2 = &arr[1];
    assert_eq!(r2["rating"], 4);
    assert_eq!(r2["title"], "Good");
    let reviewer2 = r2["reviewer"].as_array().unwrap();
    assert_eq!(reviewer2.len(), 1);
    assert_eq!(reviewer2[0]["tier"], "silver");
    assert_eq!(reviewer2[0]["balance"], 50);
}

#[tokio::test]
async fn test_correlated_subquery_parent_ref_in_select_projection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    exec(&addr, &client, "DEFINE COLLECTION items").await;
    exec(
        &addr,
        &client,
        r#"CREATE items:1 SET name = "Widget", secret_code = 42"#,
    )
    .await;

    // $parent.secret_code used in the subquery's SELECT, not WHERE
    // secret_code is not in outer projection
    let data = query(
        &addr,
        &client,
        r#"SELECT
            name,
            (SELECT $parent.secret_code AS code) AS extracted
        FROM items:1"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 1);

    let item = &arr[0];
    assert_eq!(item["name"], "Widget");

    let extracted = item["extracted"].as_array().unwrap();
    assert_eq!(extracted.len(), 1);
    // The bug: this will be null instead of 42
    assert_eq!(
        extracted[0]["code"], 42,
        "Expected $parent.secret_code to resolve to 42, got: {:?}",
        extracted[0]["code"]
    );
}

// =============================================================================
// Regression: CREATE with unquoted object keys in arrays
// =============================================================================

#[tokio::test]
async fn test_create_with_unquoted_keys_in_array_of_objects_with_schema() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define collection with schema including array of objects
    exec(
        &addr,
        &client,
        r#"DEFINE COLLECTION orders (
            SCHEMA STRICT,
            customer_id int REQUIRED,
            items [{product_id int, quantity int, unit_price float}] REQUIRED
        )"#,
    )
    .await;

    // Use unquoted keys (like Python script does): product_id, quantity, unit_price
    // This should succeed - unquoted keys in object literals should work
    let result = query_error(
        &addr,
        &client,
        r#"CREATE orders:1 SET customer_id = 1, items = [{product_id: 1, quantity: 2, unit_price: 50.0}]"#,
    )
    .await;
    assert!(
        result["error"].is_null(),
        "CREATE should succeed, but got error: {:?}",
        result
    );

    let data = query(&addr, &client, "SELECT * FROM orders:1").await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 1, "Expected 1 order, got: {:?}", arr);

    let order = &arr[0];
    assert_eq!(order["customer_id"], 1);

    let items = order["items"].as_array();
    assert!(
        items.is_some(),
        "Expected 'items' field to be an array, got: {:?}",
        order
    );

    let items = items.unwrap();
    assert_eq!(items.len(), 1, "Expected 1 item, got: {:?}", items);

    let item = &items[0];
    assert_eq!(item["product_id"], 1);
    assert_eq!(item["quantity"], 2);
    assert_eq!(item["unit_price"], 50.0);
}

// =============================================================================
// $parent.id with reference fields (regression: ref index encoding mismatch)
// =============================================================================

#[tokio::test]
async fn test_correlated_subquery_parent_id_with_ref_field() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create parent collection and child collection with a ref field + index
    exec(&addr, &client, "DEFINE COLLECTION author (name string)").await;
    exec(
        &addr,
        &client,
        "DEFINE COLLECTION book (title string, author ref)",
    )
    .await;
    exec(&addr, &client, "CREATE INDEX ON book(author)").await;

    // Insert authors
    exec(&addr, &client, r#"CREATE author:1 SET name = "Tolkien""#).await;
    exec(&addr, &client, r#"CREATE author:2 SET name = "Asimov""#).await;

    // Insert books with reference to author
    exec(
        &addr,
        &client,
        r#"CREATE book:1 SET title = "The Hobbit", author = author:1"#,
    )
    .await;
    exec(
        &addr,
        &client,
        r#"CREATE book:2 SET title = "LOTR", author = author:1"#,
    )
    .await;
    exec(
        &addr,
        &client,
        r#"CREATE book:3 SET title = "Foundation", author = author:2"#,
    )
    .await;

    // Correlated subquery: count books per author using $parent.id
    // This tests that $parent.id (a reference) matches the indexed ref field
    let data = query(
        &addr,
        &client,
        r#"SELECT name, (SELECT COUNT(*) FROM book WHERE author = $parent.id) AS book_count FROM author ORDER name"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 2);

    // Asimov: 1 book
    assert_eq!(arr[0]["name"], "Asimov");
    let asimov_count = &arr[0]["book_count"].as_array().unwrap()[0]["count(*)"];
    assert_eq!(asimov_count, 1, "Asimov should have 1 book");

    // Tolkien: 2 books
    assert_eq!(arr[1]["name"], "Tolkien");
    let tolkien_count = &arr[1]["book_count"].as_array().unwrap()[0]["count(*)"];
    assert_eq!(tolkien_count, 2, "Tolkien should have 2 books");
}

/// Test FROM $parent.field — resolves parent ref field as data source.
/// Regression test: FROM $parent.customer was parsed as Variable("parent")
/// instead of ParentRef, causing "Undefined variable $parent" error.
#[tokio::test]
async fn test_from_parent_ref_field() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Setup: customer collection with ref field pointing to address
    let setup = r#"
        DEFINE COLLECTION customer3 (name string, address ref);
        DEFINE COLLECTION address3 (city string, zip string);
        CREATE address3:a1 SET city = "Moscow", zip = "101000";
        CREATE address3:a2 SET city = "London", zip = "SW1A";
        CREATE customer3:c1 SET name = "Alice", address = address3:a1;
        CREATE customer3:c2 SET name = "Bob", address = address3:a2
    "#;
    let r = query(&addr, &client, setup).await;
    // Last statement result should not be an error
    assert!(
        r.as_array().unwrap().last().unwrap()["error"].is_null(),
        "Setup error: {:?}",
        r
    );

    // Query: SELECT name, (SELECT city FROM $parent.address) AS addr FROM customer3
    let data = query(
        &addr,
        &client,
        r#"SELECT name, (SELECT city FROM $parent.address) AS addr FROM customer3 ORDER name"#,
    )
    .await;
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 2, "Should have 2 customers");

    // Alice -> address3:a1 -> Moscow
    assert_eq!(arr[0]["name"], "Alice");
    let alice_addr = arr[0]["addr"].as_array().unwrap();
    assert_eq!(alice_addr.len(), 1);
    assert_eq!(alice_addr[0]["city"], "Moscow");

    // Bob -> address3:a2 -> London
    assert_eq!(arr[1]["name"], "Bob");
    let bob_addr = arr[1]["addr"].as_array().unwrap();
    assert_eq!(bob_addr.len(), 1);
    assert_eq!(bob_addr[0]["city"], "London");
}
