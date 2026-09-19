//! Integration tests for metrics endpoints

mod common;

use std::sync::Arc;
use stellardb::namespace::Namespace;
use stellardb::server::create_router;

#[tokio::test]
async fn test_metrics_endpoint_prometheus_format() {
    let tmp = tempfile::tempdir().unwrap();
    let namespace = Arc::new(Namespace::open(tmp.path()).unwrap());
    namespace.create_database(common::TEST_DB).unwrap();

    let (app, _state) = create_router(namespace, None, None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = common::test_client();

    // Test /metrics endpoint (Prometheus format)
    let resp = client
        .get(format!("http://{}/metrics", addr))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    // Check content type
    let content_type = resp.headers().get("content-type").unwrap();
    assert_eq!(content_type, "text/plain; charset=utf-8");

    let body = resp.text().await.unwrap();

    // Verify Prometheus format
    assert!(body.contains("# HELP stellardb_uptime_seconds"));
    assert!(body.contains("# TYPE stellardb_uptime_seconds gauge"));
    assert!(body.contains("stellardb_uptime_seconds"));

    assert!(body.contains("# HELP stellardb_queries_total"));
    assert!(body.contains("# TYPE stellardb_queries_total counter"));
    assert!(body.contains("stellardb_queries_total 0"));

    assert!(body.contains("# HELP stellardb_queries_active"));
    assert!(body.contains("stellardb_queries_active 0"));

    assert!(body.contains("# HELP stellardb_storage_disk_bytes"));
    assert!(body.contains("# HELP stellardb_collections_total"));
}

#[tokio::test]
async fn test_stats_endpoint_json_format() {
    let tmp = tempfile::tempdir().unwrap();
    let namespace = Arc::new(Namespace::open(tmp.path()).unwrap());
    namespace.create_database(common::TEST_DB).unwrap();

    let (app, _state) = create_router(namespace, None, None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = common::test_client();

    // Test /stats endpoint (JSON format)
    let resp = client
        .get(format!("http://{}/stats", addr))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    let json: serde_json::Value = resp.json().await.unwrap();

    // Verify JSON structure
    assert!(json["server"].is_object());
    assert!(json["server"]["uptime_secs"].is_number());
    assert_eq!(json["server"]["queries_total"], 0);
    assert_eq!(json["server"]["queries_errors"], 0);
    assert_eq!(json["server"]["queries_active"], 0);

    assert!(json["storage"].is_object());
    assert!(json["storage"]["disk_size_bytes"].is_number());
    assert!(json["storage"]["collections_count"].is_number());
    assert!(json["storage"]["btree_indexes"].is_number());
    assert!(json["storage"]["vector_indexes"].is_number());
    assert!(json["storage"]["fts_indexes"].is_number());

    assert!(json["top_queries"].is_array());
    assert!(json["indexes"].is_object());
    assert!(json["collections"].is_object());
}

#[tokio::test]
async fn test_full_metrics_flow() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // 1. Create a collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&serde_json::json!({
            "query": "DEFINE COLLECTION product"
        }))
        .send()
        .await
        .unwrap();

    // 2. Insert documents
    for i in 0..5 {
        client
            .post(format!("http://{}/sql", addr))
            .json(&serde_json::json!({
                "query": format!("INSERT INTO product {{ id: 'product:{}', name: 'Product {}', price: {} }}", i, i, i * 10)
            }))
            .send()
            .await
            .unwrap();
    }

    // 3. Query documents
    for _ in 0..3 {
        client
            .post(format!("http://{}/sql", addr))
            .json(&serde_json::json!({
                "query": "SELECT * FROM product WHERE price > 20"
            }))
            .send()
            .await
            .unwrap();
    }

    // 4. Check Prometheus metrics
    let metrics_resp = client
        .get(format!("http://{}/metrics", addr))
        .send()
        .await
        .unwrap();
    let metrics_body = metrics_resp.text().await.unwrap();

    assert!(metrics_body.contains("stellardb_queries_total"));
    assert!(metrics_body.contains("stellardb_collections_total"));

    // 5. Check JSON stats
    let stats_resp = client
        .get(format!("http://{}/stats", addr))
        .send()
        .await
        .unwrap();
    let stats: serde_json::Value = stats_resp.json().await.unwrap();

    // Should have recorded queries (1 DEFINE + 5 INSERTs + 3 SELECTs = 9)
    assert!(stats["server"]["queries_total"].as_u64().unwrap() >= 9);

    // Should have top queries
    let top_queries = stats["top_queries"].as_array().unwrap();
    assert!(!top_queries.is_empty());

    // Find the SELECT query
    let select_query = top_queries.iter().find(|q| {
        q["query_normalized"]
            .as_str()
            .unwrap_or("")
            .contains("SELECT")
    });
    assert!(select_query.is_some());
    assert!(select_query.unwrap()["call_count"].as_u64().unwrap() >= 3);
}

#[tokio::test]
async fn stats_never_exposes_http_query_literals_or_malformed_secrets() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();
    let queries = [
        "SELECT 'Мария Иванова' AS customer, 424242 AS account_number",
        "CREATE USER 'unicode-user@example.com' PASSWORD 'password-secret-987'",
        "SELECT * FROM customers WHERE email = 'malformed-пии@example.com api_key=sk-malformed-secret",
    ];

    for query in queries {
        let _ = client
            .post(format!("http://{}/sql", addr))
            .json(&serde_json::json!({"query": query}))
            .send()
            .await
            .unwrap();
    }

    let response = client
        .get(format!("http://{}/stats", addr))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let stats: serde_json::Value = response.json().await.unwrap();
    let serialized = stats.to_string();

    for sensitive in [
        "Мария Иванова",
        "424242",
        "unicode-user@example.com",
        "password-secret-987",
        "malformed-пии@example.com",
        "sk-malformed-secret",
    ] {
        assert!(
            !serialized.contains(sensitive),
            "/stats leaked {sensitive}: {serialized}"
        );
    }

    let top_queries = stats["top_queries"].as_array().unwrap();
    assert!(top_queries.iter().any(|query| {
        query["query_normalized"] == "SELECT '?' AS customer, 0 AS account_number"
            && query["query_example"] == "SELECT '?' AS customer, 0 AS account_number"
            && query["query_hash"].is_string()
            && query["call_count"].is_number()
            && query["total_time_us"].is_number()
            && query["rows_returned"].is_number()
            && query["errors"].is_number()
    }));
    assert!(top_queries.iter().any(|query| {
        query["query_normalized"] == ""
            && query["query_example"] == ""
            && query["errors"].as_u64().unwrap_or(0) >= 1
    }));
}

#[tokio::test]
async fn test_sql_response_includes_numeric_elapsed_us() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let resp = client
        .post(format!("http://{}/sql", addr))
        .json(&serde_json::json!({
            "query": "SELECT 1 AS ok"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();

    assert!(
        body["elapsed_us"].is_u64(),
        "elapsed_us should be a numeric microsecond duration, got: {:?}",
        body
    );
    assert!(
        body["elapsed"].is_string(),
        "legacy elapsed string should remain during transition"
    );
}

#[tokio::test]
async fn test_v1_collections_include_stats_and_schema() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    client
        .post(format!("http://{}/sql", addr))
        .json(&serde_json::json!({
            "query": "DEFINE COLLECTION products (SCHEMA STRICT, name string REQUIRED, category string)"
        }))
        .send()
        .await
        .unwrap();
    client
        .post(format!("http://{}/sql", addr))
        .json(&serde_json::json!({
            "query": "CREATE INDEX ON products(category)"
        }))
        .send()
        .await
        .unwrap();
    client
        .post(format!("http://{}/sql", addr))
        .json(&serde_json::json!({
            "query": "INSERT INTO products {id: 'p1', name: 'Hammer', category: 'tools'}"
        }))
        .send()
        .await
        .unwrap();

    let resp = client
        .get(format!(
            "http://{}/v1/collections?include=stats,schema",
            addr
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let collections = body["collections"].as_array().unwrap();
    let products = collections
        .iter()
        .find(|collection| collection["name"] == "products")
        .expect("products collection should be returned");

    assert_eq!(products["stats"]["document_count"], 1);
    let fields = products["schema"]["fields"].as_array().unwrap();
    let name_field = fields
        .iter()
        .find(|field| field["name"] == "name")
        .expect("name field should be returned");
    assert_eq!(name_field["type"], "string");
    assert_eq!(name_field["required"], true);
    assert!(
        name_field.get("field_type").is_none(),
        "v1 schema contract should not expose internal Rust field_type shape"
    );
    assert_eq!(products["schema"]["indexes"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_v1_query_returns_typed_db_values() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let resp = client
        .post(format!("http://{}/v1/query", addr))
        .json(&serde_json::json!({
            "query": r#"
                SELECT
                  9223372036854775807 AS big,
                  123.45dec AS price,
                  d"2024-01-15T10:30:00Z" AS created_at,
                  1h30m AS ttl,
                  b"SGVsbG8=" AS payload,
                  user:alice AS owner
            "#
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let row = &body["results"][0]["data"][0];

    assert_eq!(
        row["big"],
        serde_json::json!({"$type": "int64", "value": "9223372036854775807"})
    );
    assert_eq!(
        row["price"],
        serde_json::json!({"$type": "decimal", "value": "123.45"})
    );
    assert_eq!(
        row["created_at"],
        serde_json::json!({"$type": "datetime", "value": "2024-01-15T10:30:00+00:00"})
    );
    assert_eq!(
        row["ttl"],
        serde_json::json!({"$type": "duration", "value": "5400000000000"})
    );
    assert_eq!(
        row["payload"],
        serde_json::json!({"$type": "bytes", "value": "SGVsbG8="})
    );
    assert_eq!(
        row["owner"],
        serde_json::json!({"$type": "reference", "value": "user:alice"})
    );
    assert!(body["elapsed_us"].is_u64());
    assert!(
        body.get("error").is_none(),
        "successful v1 query responses must omit the optional error field"
    );
}

#[tokio::test]
async fn test_v1_query_matches_shared_typed_value_fixture() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/v1_query_typed_values.json")).unwrap();
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    let resp = client
        .post(format!("http://{}/v1/query", addr))
        .json(&serde_json::json!({
            "query": fixture["query"].as_str().unwrap()
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["results"][0]["data"][0], fixture["expected_row"]);
}

#[tokio::test]
async fn test_query_metrics_recorded() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // First create a collection
    client
        .post(format!("http://{}/sql", addr))
        .json(&serde_json::json!({"query": "DEFINE COLLECTION user"}))
        .send()
        .await
        .unwrap();

    // Execute queries with different values but same structure
    client
        .post(format!("http://{}/sql", addr))
        .json(&serde_json::json!({"query": "SELECT * FROM user WHERE id = 'alice'"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("http://{}/sql", addr))
        .json(&serde_json::json!({"query": "SELECT * FROM user WHERE id = 'bob'"}))
        .send()
        .await
        .unwrap();

    let resp = client
        .get(format!("http://{}/stats", addr))
        .send()
        .await
        .unwrap();

    let json: serde_json::Value = resp.json().await.unwrap();
    let top_queries = json["top_queries"].as_array().unwrap();

    // Same normalized query should be grouped
    let select_queries: Vec<_> = top_queries
        .iter()
        .filter(|q| q["query_normalized"] == "SELECT * FROM user WHERE id = '?'")
        .collect();

    // Should have one replayable, literal-redacted entry with call_count >= 2.
    assert!(
        !select_queries.is_empty(),
        "No SELECT queries found in top_queries: {:?}",
        top_queries
    );
    let call_count = select_queries[0]["call_count"].as_u64().unwrap();
    assert!(
        call_count >= 2,
        "Expected call_count >= 2, got {}",
        call_count
    );
}
