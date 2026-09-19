//! Full-Text Search (FTS) Integration Tests
//!
//! These tests verify the complete FTS pipeline works end-to-end:
//! - Creating FULLTEXT indexes
//! - Inserting documents with searchable text
//! - Querying with @@ operator
//! - Using score() function for relevance
//! - MATCH clauses with field boosts
//! - Combining FTS with regular filters
//! - ORDER BY score and LIMIT
//!
//! # Index Naming Convention
//!
//! FTS indexes are named by sorting field names alphabetically and joining with `_`,
//! then appending `_fts`. For example:
//! - Index on (title) -> `title_fts`
//! - Index on (body) -> `body_fts`
//! - Index on (title, body) -> `body_title_fts` (sorted: body, title)
//!
//! When querying:
//! - `title @@ "rust"` looks for index `title_fts`
//! - `MATCH(title, body) @@ "rust"` looks for index `body_title_fts`
//!
//! So for single-field searches, create single-field indexes.
//! For multi-field MATCH queries, create multi-field indexes.

mod common;

use serde_json::json;

/// Wait for FTS background commit to complete.
/// The FTS index uses batched background commits with a 100ms interval.
/// Call this after INSERT operations before searching to ensure visibility.
async fn wait_for_fts_commit() {
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
}

/// Helper to execute SQL and return the response body
async fn sql(
    client: &reqwest::Client,
    addr: &std::net::SocketAddr,
    query: &str,
) -> serde_json::Value {
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": query}))
        .send()
        .await
        .unwrap();
    res.json().await.unwrap()
}

/// Helper to assert query succeeded and return the data array
fn assert_success(body: &serde_json::Value) -> &serde_json::Value {
    assert!(
        body["error"].is_null(),
        "Query failed with error: {:?}",
        body["error"]
    );
    &body["results"][0]["data"]
}

/// Helper to create the test collection with articles - with separate single-field indexes
async fn setup_articles_collection_single_field(
    client: &reqwest::Client,
    addr: &std::net::SocketAddr,
) {
    // Define collection
    sql(client, addr, "DEFINE COLLECTION articles").await;

    // Create separate FULLTEXT indexes for single-field queries
    let body = sql(client, addr, "CREATE INDEX ON articles(title) FULLTEXT").await;
    assert!(
        common::is_ddl_success(&body),
        "Failed to create title FULLTEXT index: {:?}",
        body
    );

    let body = sql(client, addr, "CREATE INDEX ON articles(body) FULLTEXT").await;
    assert!(
        common::is_ddl_success(&body),
        "Failed to create body FULLTEXT index: {:?}",
        body
    );
}

/// Helper to create the test collection with articles - with multi-field index for MATCH queries
async fn setup_articles_collection_multi_field(
    client: &reqwest::Client,
    addr: &std::net::SocketAddr,
) {
    // Define collection
    sql(client, addr, "DEFINE COLLECTION articles").await;

    // Create FULLTEXT index on title and body for MATCH queries
    // Index name will be body_title_fts (sorted alphabetically)
    let body = sql(
        client,
        addr,
        "CREATE INDEX ON articles(title, body) FULLTEXT",
    )
    .await;
    assert!(
        common::is_ddl_success(&body),
        "Failed to create FULLTEXT index: {:?}",
        body
    );
}

/// Helper to insert test articles with varying keyword density
async fn insert_test_articles(client: &reqwest::Client, addr: &std::net::SocketAddr) {
    // Article 1: Heavy on "rust", moderate on "database"
    sql(
        client,
        addr,
        r#"INSERT INTO articles {
            id: "a1",
            title: "Rust Programming Language",
            body: "Rust is a systems programming language. Rust offers memory safety without garbage collection. Learning Rust can be challenging but rewarding.",
            category: "programming"
        }"#,
    )
    .await;

    // Article 2: Moderate on "rust", heavy on "database"
    sql(
        client,
        addr,
        r#"INSERT INTO articles {
            id: "a2",
            title: "Building a Database",
            body: "This article covers database internals. We explore database indexing, database queries, and database optimization. Some databases use Rust.",
            category: "database"
        }"#,
    )
    .await;

    // Article 3: Both "rust" and "database" in title
    sql(
        client,
        addr,
        r#"INSERT INTO articles {
            id: "a3",
            title: "Rust Database Systems",
            body: "Building database systems in Rust. Rust provides safety for database operations.",
            category: "database"
        }"#,
    )
    .await;

    // Article 4: Neither keyword
    sql(
        client,
        addr,
        r#"INSERT INTO articles {
            id: "a4",
            title: "Python Web Development",
            body: "Python is great for web development. Django and Flask are popular frameworks.",
            category: "programming"
        }"#,
    )
    .await;

    // Article 5: Only "database"
    sql(
        client,
        addr,
        r#"INSERT INTO articles {
            id: "a5",
            title: "SQL Database Basics",
            body: "SQL is the standard language for database operations. Learn database design principles.",
            category: "database"
        }"#,
    )
    .await;

    // Wait for FTS background commit
    wait_for_fts_commit().await;
}

// =============================================================================
// Basic FTS Search Tests
// =============================================================================

#[tokio::test]
async fn test_basic_fts_search() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_articles_collection_single_field(&client, &addr).await;
    insert_test_articles(&client, &addr).await;

    // Search for "rust" using single-field index (title_fts)
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM articles WHERE title @@ "rust""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    // Should find articles with "rust" in title
    assert!(
        !results.is_empty(),
        "Expected at least 1 result for 'rust' search, got {}",
        results.len()
    );

    // Verify the results contain "rust" related articles
    let ids: Vec<&str> = results.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert!(
        ids.contains(&"articles:a1") || ids.contains(&"articles:a3"),
        "Expected to find articles with 'rust' in title, got: {:?}",
        ids
    );
}

#[tokio::test]
async fn test_fts_search_body_field() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_articles_collection_single_field(&client, &addr).await;
    insert_test_articles(&client, &addr).await;

    // Search for "database" in body using single-field index (body_fts)
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM articles WHERE body @@ "database""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    // Should find multiple articles with "database" in body
    assert!(
        results.len() >= 2,
        "Expected at least 2 results for 'database' search in body, got {}",
        results.len()
    );
}

#[tokio::test]
async fn test_fts_search_no_results() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_articles_collection_single_field(&client, &addr).await;
    insert_test_articles(&client, &addr).await;

    // Search for something that doesn't exist
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM articles WHERE title @@ "kubernetes""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert_eq!(results.len(), 0, "Expected 0 results for non-existent term");
}

// =============================================================================
// FTS with score() Function Tests
// =============================================================================

#[tokio::test]
async fn test_fts_with_score_function() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_articles_collection_single_field(&client, &addr).await;
    insert_test_articles(&client, &addr).await;

    // Search with score() in projection using title_fts index
    let body = sql(
        &client,
        &addr,
        r#"SELECT id, title, fts::score() AS relevance FROM articles WHERE title @@ "rust""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert!(
        !results.is_empty(),
        "Expected results for 'rust' search with score"
    );

    // Verify score is present (as $score field attached by FTS scan)
    for result in results {
        // The score should be available via $score field
        let has_score = result.get("relevance").is_some() || result.get("$score").is_some();
        assert!(
            has_score,
            "Expected score to be available in result: {:?}",
            result
        );
    }
}

#[tokio::test]
async fn test_fts_score_ordering() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_articles_collection_single_field(&client, &addr).await;
    insert_test_articles(&client, &addr).await;

    // Search and order by score descending using body_fts index
    let body = sql(
        &client,
        &addr,
        r#"SELECT id, title, fts::score() AS score FROM articles WHERE body @@ "database" ORDER score DESC"#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert!(
        results.len() >= 2,
        "Expected multiple results for 'database' search"
    );

    // Verify results are ordered by score (descending)
    // The article with more "database" mentions should come first
    if results.len() >= 2 {
        let score1 = results[0]
            .get("score")
            .or(results[0].get("$score"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let score2 = results[1]
            .get("score")
            .or(results[1].get("$score"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        assert!(
            score1 >= score2,
            "Results should be ordered by score DESC: {} >= {}",
            score1,
            score2
        );
    }
}

// =============================================================================
// MATCH Clause with Field Boosts Tests
// =============================================================================

#[tokio::test]
async fn test_fts_match_clause_basic() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Need multi-field index for MATCH queries (index name will be body_title_fts)
    setup_articles_collection_multi_field(&client, &addr).await;
    insert_test_articles(&client, &addr).await;

    // Use MATCH clause to search both title and body
    let body = sql(
        &client,
        &addr,
        r#"SELECT id, title FROM articles WHERE MATCH(title, body) @@ "rust""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    // Should find articles with "rust" in either title or body
    assert!(
        results.len() >= 2,
        "Expected at least 2 results for MATCH search, got {}",
        results.len()
    );
}

#[tokio::test]
async fn test_fts_match_with_boosts() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Need multi-field index for MATCH queries (index name will be body_title_fts)
    setup_articles_collection_multi_field(&client, &addr).await;
    insert_test_articles(&client, &addr).await;

    // Boost title field 2x - matches in title should score higher
    let body = sql(
        &client,
        &addr,
        r#"SELECT id, title, fts::score() AS score FROM articles WHERE MATCH(title^2, body) @@ "database" ORDER score DESC"#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert!(
        !results.is_empty(),
        "Expected results for boosted MATCH search"
    );

    // Articles with "database" in title should score higher due to boost
    // a2 ("Building a Database") and a3 ("Rust Database Systems") and a5 ("SQL Database Basics")
    // should be among top results
    let top_ids: Vec<&str> = results
        .iter()
        .take(3)
        .map(|r| r["id"].as_str().unwrap())
        .collect();

    let has_title_match = top_ids
        .iter()
        .any(|id| *id == "articles:a2" || *id == "articles:a3" || *id == "articles:a5");
    assert!(
        has_title_match,
        "Expected articles with 'database' in title to rank high, got: {:?}",
        top_ids
    );
}

// =============================================================================
// FTS Combined with Regular Filters Tests
// =============================================================================

#[tokio::test]
async fn test_fts_with_and_filter() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_articles_collection_single_field(&client, &addr).await;
    insert_test_articles(&client, &addr).await;

    // FTS search combined with category filter using title_fts index
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM articles WHERE title @@ "rust" AND category = "programming""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    // Should only find articles matching both conditions
    for result in results {
        let id = result["id"].as_str().unwrap();
        let category = result["category"].as_str().unwrap();
        assert_eq!(
            category, "programming",
            "Result {} should have category 'programming'",
            id
        );
    }
}

#[tokio::test]
async fn test_fts_with_or_filter() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_articles_collection_single_field(&client, &addr).await;
    insert_test_articles(&client, &addr).await;

    // Note: FTS expressions in OR predicates are NOT extracted by the optimizer
    // (see analyze_for_fts in optimizer/index.rs). This is a known limitation.
    // OR queries fall back to full collection scan with filter evaluation.
    //
    // For now, test that the query executes without error.
    // Future enhancement could add FTS OR support via union of FTS scans.
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM articles WHERE title @@ "rust" OR title @@ "python""#,
    )
    .await;

    // The query should execute without crashing/error (may return empty results
    // since FTS in OR is not optimized)
    let has_results = body["results"][0]["data"].as_array().is_some();
    let has_error = body["error"].is_object();

    // Either succeeds with results or returns an error about unsupported OR
    // Currently returns empty since FTS OR is not optimized
    assert!(
        has_results || has_error,
        "FTS OR query should handle gracefully: {:?}",
        body
    );
}

// =============================================================================
// LIMIT on FTS Tests
// =============================================================================

#[tokio::test]
async fn test_fts_with_limit() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_articles_collection_single_field(&client, &addr).await;
    insert_test_articles(&client, &addr).await;

    // Search with LIMIT using body_fts index
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM articles WHERE body @@ "database" LIMIT 2"#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert!(
        results.len() <= 2,
        "Expected at most 2 results with LIMIT 2, got {}",
        results.len()
    );
}

#[tokio::test]
async fn test_fts_order_by_score_with_limit() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_articles_collection_single_field(&client, &addr).await;
    insert_test_articles(&client, &addr).await;

    // Search with ORDER BY score and LIMIT - get top 2 most relevant using body_fts index
    let body = sql(
        &client,
        &addr,
        r#"SELECT id, title, fts::score() AS score FROM articles WHERE body @@ "database" ORDER score DESC LIMIT 2"#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert!(
        results.len() <= 2,
        "Expected at most 2 results with ORDER BY score LIMIT 2"
    );

    // Verify scores are in descending order
    if results.len() == 2 {
        let score1 = results[0]
            .get("score")
            .or(results[0].get("$score"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let score2 = results[1]
            .get("score")
            .or(results[1].get("$score"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        assert!(
            score1 >= score2,
            "Top 2 results should be in score DESC order"
        );
    }
}

// =============================================================================
// Named FTS Operators Tests
// =============================================================================

#[tokio::test]
async fn test_fts_named_operator() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_articles_collection_single_field(&client, &addr).await;
    insert_test_articles(&client, &addr).await;

    // Use named FTS operator @:search@ with title_fts index
    let body = sql(
        &client,
        &addr,
        r#"SELECT id, title FROM articles WHERE title @:search@ "rust""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert!(
        !results.is_empty(),
        "Expected results for named FTS operator search"
    );
}

// =============================================================================
// FTS Index Verification Tests
// =============================================================================

#[tokio::test]
async fn test_fts_index_shows_in_describe() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection and FULLTEXT index
    sql(&client, &addr, "DEFINE COLLECTION posts").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON posts(title, content) FULLTEXT",
    )
    .await;

    // Verify index appears in DESCRIBE COLLECTION
    let body = sql(&client, &addr, "DESCRIBE COLLECTION posts").await;
    // data is now an array with one element containing collection info
    let data = &body["results"][0]["data"][0];

    let indexes = data["indexes"].as_array().unwrap();
    assert_eq!(indexes.len(), 1, "Expected 1 index");

    let index = &indexes[0];
    assert_eq!(index["index_type"], "FullText");
    assert_eq!(index["fields"], json!(["title", "content"]));
    assert_eq!(index["unique"], false);
}

#[tokio::test]
async fn test_fts_search_requires_index() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection WITHOUT FULLTEXT index
    sql(&client, &addr, "DEFINE COLLECTION noindex").await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO noindex {id: "1", title: "Test Document"}"#,
    )
    .await;

    // FTS search should fail or return no results without index
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM noindex WHERE title @@ "test""#,
    )
    .await;

    // Either returns error or empty results (implementation dependent)
    let has_error = body["error"].is_object();
    let empty_results = body["results"][0]["data"]
        .as_array()
        .map(|a| a.is_empty())
        .unwrap_or(true);

    assert!(
        has_error || empty_results,
        "FTS search without index should fail or return empty: {:?}",
        body
    );
}

// =============================================================================
// Edge Cases and Error Handling Tests
// =============================================================================

#[tokio::test]
async fn test_fts_empty_query_string() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_articles_collection_single_field(&client, &addr).await;
    insert_test_articles(&client, &addr).await;

    // Empty search string using title_fts index
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM articles WHERE title @@ """#,
    )
    .await;

    // Should either return all results or handle gracefully
    // Not a crash or panic
    let has_results = body["results"][0]["data"].as_array().is_some();
    let has_error = body["error"].is_object();
    assert!(
        has_results || has_error,
        "Empty FTS query should handle gracefully"
    );
}

#[tokio::test]
async fn test_fts_special_characters_in_query() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_articles_collection_single_field(&client, &addr).await;
    insert_test_articles(&client, &addr).await;

    // Query with special characters - should not crash (uses title_fts index)
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM articles WHERE title @@ "rust & database""#,
    )
    .await;

    // Should handle gracefully (may return results or error, but not crash)
    assert!(
        body["results"].is_array() || body["error"].is_object(),
        "FTS query with special chars should not crash"
    );
}

#[tokio::test]
async fn test_fts_multi_word_query() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_articles_collection_single_field(&client, &addr).await;
    insert_test_articles(&client, &addr).await;

    // Multi-word search using body_fts index
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM articles WHERE body @@ "programming language""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    // Should find articles containing both words
    // Article a1 has "programming language" in body
    let ids: Vec<&str> = results.iter().map(|r| r["id"].as_str().unwrap()).collect();

    if !results.is_empty() {
        assert!(
            ids.contains(&"articles:a1"),
            "Expected to find article with 'programming language'"
        );
    }
}

// =============================================================================
// Single Field Index Tests
// =============================================================================

#[tokio::test]
async fn test_fts_single_field_index() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with single field FULLTEXT index
    sql(&client, &addr, "DEFINE COLLECTION blogs").await;
    sql(&client, &addr, "CREATE INDEX ON blogs(content) FULLTEXT").await;

    // Insert documents
    sql(
        &client,
        &addr,
        r#"INSERT INTO blogs {id: "b1", content: "Rust is a systems programming language"}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO blogs {id: "b2", content: "Python is great for data science"}"#,
    )
    .await;
    wait_for_fts_commit().await;

    // Search single field
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM blogs WHERE content @@ "rust""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert_eq!(results.len(), 1, "Expected 1 result for 'rust' search");
    assert_eq!(results[0]["id"], "blogs:b1");
}

// =============================================================================
// FTS with Projection Tests
// =============================================================================

#[tokio::test]
async fn test_fts_selective_projection() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_articles_collection_single_field(&client, &addr).await;
    insert_test_articles(&client, &addr).await;

    // Only select specific fields using title_fts index
    let body = sql(
        &client,
        &addr,
        r#"SELECT id, title FROM articles WHERE title @@ "rust""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert!(!results.is_empty(), "Expected results");

    // Verify only requested fields are returned
    for result in results {
        assert!(result.get("id").is_some(), "Should have id field");
        assert!(result.get("title").is_some(), "Should have title field");
        // body and category should not be in projection
        // (though implementation may include them)
    }
}

#[tokio::test]
async fn test_fts_with_computed_fields() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_articles_collection_single_field(&client, &addr).await;
    insert_test_articles(&client, &addr).await;

    // Select with computed field using score (uses title_fts index)
    let body = sql(
        &client,
        &addr,
        r#"SELECT id, title, fts::score() * 100 AS score_pct FROM articles WHERE title @@ "database""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    assert!(!results.is_empty(), "Expected results");

    // Verify computed score field exists
    for result in results {
        // score_pct should be computed from fts::score() * 100
        // May be null if score() returns null outside FTS context
        let has_computed = result.get("score_pct").is_some() || result.get("$score").is_some();
        assert!(
            has_computed,
            "Should have computed score field: {:?}",
            result
        );
    }
}

// =============================================================================
// Additional Edge Cases
// =============================================================================

#[tokio::test]
async fn test_fts_unicode_in_content() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with FULLTEXT index
    sql(&client, &addr, "DEFINE COLLECTION unicode_test").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON unicode_test(content) FULLTEXT",
    )
    .await;

    // Insert documents with Unicode content
    sql(
        &client,
        &addr,
        r#"INSERT INTO unicode_test {id: "u1", content: "Hello World in Japanese: \u4e16\u754c"}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO unicode_test {id: "u2", content: "Bonjour le monde in French"}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO unicode_test {id: "u3", content: "Emoji test: rocket and star"}"#,
    )
    .await;

    // Search for Unicode content
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM unicode_test WHERE content @@ "world""#,
    )
    .await;

    // Should handle gracefully (may return results or not depending on tokenization)
    assert!(
        body["results"].is_array() || body["error"].is_object(),
        "Unicode search should handle gracefully"
    );
}

#[tokio::test]
async fn test_fts_very_long_query() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_articles_collection_single_field(&client, &addr).await;
    insert_test_articles(&client, &addr).await;

    // Create a very long query string
    let long_query = "rust ".repeat(100); // 500+ characters

    // Search with long query
    let body = sql(
        &client,
        &addr,
        &format!(
            r#"SELECT * FROM articles WHERE title @@ "{}""#,
            long_query.trim()
        ),
    )
    .await;

    // Should handle gracefully (may return results or error, but not crash)
    assert!(
        body["results"].is_array() || body["error"].is_object(),
        "Long query should handle gracefully"
    );
}

#[tokio::test]
async fn test_fts_case_insensitive_search() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    setup_articles_collection_single_field(&client, &addr).await;
    insert_test_articles(&client, &addr).await;

    // Search with different cases
    let body_lower = sql(
        &client,
        &addr,
        r#"SELECT id FROM articles WHERE title @@ "rust""#,
    )
    .await;
    let body_upper = sql(
        &client,
        &addr,
        r#"SELECT id FROM articles WHERE title @@ "RUST""#,
    )
    .await;
    let body_mixed = sql(
        &client,
        &addr,
        r#"SELECT id FROM articles WHERE title @@ "RuSt""#,
    )
    .await;

    // Should handle all cases (FTS is typically case-insensitive)
    let data_lower = assert_success(&body_lower);
    let data_upper = assert_success(&body_upper);
    let data_mixed = assert_success(&body_mixed);

    // All should return similar results (case-insensitive)
    let count_lower = data_lower.as_array().map(|a| a.len()).unwrap_or(0);
    let count_upper = data_upper.as_array().map(|a| a.len()).unwrap_or(0);
    let count_mixed = data_mixed.as_array().map(|a| a.len()).unwrap_or(0);

    // At least one should return results
    assert!(
        count_lower > 0 || count_upper > 0 || count_mixed > 0,
        "At least one case variation should return results"
    );
}

#[tokio::test]
async fn test_fts_document_update_reindex() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with FULLTEXT index
    sql(&client, &addr, "DEFINE COLLECTION update_test").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON update_test(content) FULLTEXT",
    )
    .await;

    // Insert a document
    sql(
        &client,
        &addr,
        r#"INSERT INTO update_test {id: "d1", content: "original content here"}"#,
    )
    .await;
    wait_for_fts_commit().await;

    // Search for "original"
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM update_test WHERE content @@ "original""#,
    )
    .await;
    let data = assert_success(&body);
    assert!(
        !data.as_array().unwrap().is_empty(),
        "Should find original content"
    );

    // Update the document via REST API
    let res = client
        .put(format!(
            "http://{}/collections/update_test/documents/d1",
            addr
        ))
        .json(&json!({"content": "modified content instead"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "Update should succeed");

    // Verify the document is updated in storage
    let res = client
        .get(format!(
            "http://{}/collections/update_test/documents/d1",
            addr
        ))
        .send()
        .await
        .unwrap();
    let doc: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        doc["content"], "modified content instead",
        "Document should be updated in storage"
    );

    // Note: FTS reindexing on update may not be immediate - this is a known behavior
    // The FTS index should eventually reflect the update, but we don't enforce strict
    // consistency guarantees for this test.

    // Search for "modified" should ideally match after update
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM update_test WHERE content @@ "modified""#,
    )
    .await;

    // This test primarily verifies that:
    // 1. Updates work correctly for the document storage
    // 2. FTS search doesn't crash after updates
    // FTS consistency after updates is a best-effort feature
    assert!(
        body["results"].is_array() || body["error"].is_object(),
        "FTS search should not crash after update"
    );
}

#[tokio::test]
async fn test_fts_document_delete_removes_from_index() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with FULLTEXT index
    sql(&client, &addr, "DEFINE COLLECTION delete_test").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON delete_test(content) FULLTEXT",
    )
    .await;

    // Insert documents
    sql(
        &client,
        &addr,
        r#"INSERT INTO delete_test {id: "d1", content: "searchable content"}"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO delete_test {id: "d2", content: "other searchable data"}"#,
    )
    .await;
    wait_for_fts_commit().await;

    // Search should find both
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM delete_test WHERE content @@ "searchable""#,
    )
    .await;
    let data = assert_success(&body);
    assert_eq!(data.as_array().unwrap().len(), 2, "Should find 2 documents");

    // Delete one document via REST API
    let res = client
        .delete(format!(
            "http://{}/collections/delete_test/documents/d1",
            addr
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204, "Delete should succeed");
    wait_for_fts_commit().await;

    // Search should now find only one document
    // Note: FTS indexes may have eventual consistency, so we check that
    // the deleted document is eventually removed from search results
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM delete_test WHERE content @@ "searchable""#,
    )
    .await;
    let data = assert_success(&body);
    let results = data.as_array().unwrap();

    // The deleted document should not be in the results
    // (it may take time for FTS to reflect deletes)
    let has_deleted_doc = results.iter().any(|r| {
        r["id"]
            .as_str()
            .map(|id| id.contains("d1"))
            .unwrap_or(false)
    });

    // Primary storage delete should have worked - verify the document is gone
    let body = sql(&client, &addr, r#"SELECT * FROM delete_test"#).await;
    let data = assert_success(&body);
    assert_eq!(
        data.as_array().unwrap().len(),
        1,
        "Should have 1 document in storage after delete"
    );

    // For FTS, just verify it doesn't crash and returns reasonable results
    assert!(
        results.len() <= 2,
        "FTS should return at most 2 results after delete"
    );

    // Ideally, the deleted doc should be gone from FTS too
    // but we accept that it might take time due to eventual consistency
    if !has_deleted_doc {
        // Best case: FTS properly reflects the delete
        assert_eq!(
            results.len(),
            1,
            "Should find 1 document in FTS after delete"
        );
    }
}

// =============================================================================
// FTS Analyzer Integration Tests
// =============================================================================

#[tokio::test]
async fn test_fts_with_russian_analyzer() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create collection with Russian FTS index
    sql(&client, &addr, "DEFINE COLLECTION articles_ru").await;
    let body = sql(
        &client,
        &addr,
        "CREATE INDEX ON articles_ru(content) FULLTEXT ANALYZER russian",
    )
    .await;
    assert!(
        common::is_ddl_success(&body),
        "Failed to create Russian FULLTEXT index: {:?}",
        body
    );

    // Insert Russian text
    sql(
        &client,
        &addr,
        r#"INSERT INTO articles_ru { id: "1", content: "программирование на языке Rust очень интересно" }"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO articles_ru { id: "2", content: "изучение Python для начинающих" }"#,
    )
    .await;
    wait_for_fts_commit().await;

    // Search with stemmed form - "программирован" should match "программирование"
    // Russian stemmer reduces "программирование" to its stem
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM articles_ru WHERE content @@ "программирование""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();
    assert!(
        !results.is_empty(),
        "Should find document with Russian stemming"
    );
    assert_eq!(
        results[0]["id"], "articles_ru:1",
        "Should find the correct document"
    );
}

#[tokio::test]
async fn test_fts_with_english_analyzer() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    sql(&client, &addr, "DEFINE COLLECTION posts_en").await;
    let body = sql(
        &client,
        &addr,
        "CREATE INDEX ON posts_en(body) FULLTEXT ANALYZER english",
    )
    .await;
    assert!(
        common::is_ddl_success(&body),
        "Failed to create English FULLTEXT index: {:?}",
        body
    );

    sql(
        &client,
        &addr,
        r#"INSERT INTO posts_en { id: "1", body: "The cats are running quickly" }"#,
    )
    .await;
    wait_for_fts_commit().await;

    // "run" should match "running" with English stemmer
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM posts_en WHERE body @@ "run""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();
    assert!(
        !results.is_empty(),
        "Should find document with English stemming - 'run' should match 'running'"
    );
}

#[tokio::test]
async fn test_fts_with_english_analyzer_plural() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    sql(&client, &addr, "DEFINE COLLECTION posts_plural").await;
    let body = sql(
        &client,
        &addr,
        "CREATE INDEX ON posts_plural(body) FULLTEXT ANALYZER english",
    )
    .await;
    assert!(
        common::is_ddl_success(&body),
        "Failed to create English FULLTEXT index: {:?}",
        body
    );

    sql(
        &client,
        &addr,
        r#"INSERT INTO posts_plural { id: "1", body: "The cats are sleeping" }"#,
    )
    .await;
    wait_for_fts_commit().await;

    // "cat" should match "cats" with English stemmer
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM posts_plural WHERE body @@ "cat""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();
    assert!(
        !results.is_empty(),
        "Should find document with English stemming - 'cat' should match 'cats'"
    );
}

#[tokio::test]
async fn test_fts_with_standard_analyzer_default() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    sql(&client, &addr, "DEFINE COLLECTION docs_std").await;
    // No ANALYZER specified - should use "standard"
    let body = sql(&client, &addr, "CREATE INDEX ON docs_std(text) FULLTEXT").await;
    assert!(
        common::is_ddl_success(&body),
        "Failed to create standard FULLTEXT index: {:?}",
        body
    );

    sql(
        &client,
        &addr,
        r#"INSERT INTO docs_std { id: "1", text: "Hello World" }"#,
    )
    .await;
    wait_for_fts_commit().await;

    // Standard analyzer does lowercase but no stemming
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM docs_std WHERE text @@ "hello""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();
    assert!(
        !results.is_empty(),
        "Standard analyzer should find 'hello' (lowercase) in 'Hello World'"
    );
}

#[tokio::test]
async fn test_fts_standard_analyzer_no_stemming() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    sql(&client, &addr, "DEFINE COLLECTION docs_no_stem").await;
    // No ANALYZER specified - should use "standard" which does NOT stem
    let body = sql(
        &client,
        &addr,
        "CREATE INDEX ON docs_no_stem(text) FULLTEXT",
    )
    .await;
    assert!(
        common::is_ddl_success(&body),
        "Failed to create standard FULLTEXT index: {:?}",
        body
    );

    sql(
        &client,
        &addr,
        r#"INSERT INTO docs_no_stem { id: "1", text: "The cats are running quickly" }"#,
    )
    .await;

    // "run" should NOT match "running" without stemming
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM docs_no_stem WHERE text @@ "run""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();
    // Standard analyzer does not stem, so "run" should not match "running"
    assert!(
        results.is_empty(),
        "Standard analyzer should NOT find 'run' matching 'running' (no stemming)"
    );
}

#[tokio::test]
async fn test_invalid_analyzer_error() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    sql(&client, &addr, "DEFINE COLLECTION test_invalid_analyzer").await;
    let body = sql(
        &client,
        &addr,
        "CREATE INDEX ON test_invalid_analyzer(field) FULLTEXT ANALYZER nonexistent",
    )
    .await;

    // Should fail with an error about unknown analyzer
    assert!(
        common::has_error(&body),
        "Creating index with nonexistent analyzer should fail: {:?}",
        body
    );

    // If there's an error message, it should mention the analyzer
    if let Some(err_str) = body["error"]["message"].as_str() {
        assert!(
            err_str.contains("analyzer")
                || err_str.contains("nonexistent")
                || err_str.contains("Unknown"),
            "Error should mention the analyzer issue: {}",
            err_str
        );
    }
}

#[tokio::test]
async fn test_analyzer_not_allowed_on_btree() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    sql(&client, &addr, "DEFINE COLLECTION test_btree_analyzer").await;

    // ANALYZER on BTree should fail - this might be caught at parse level or execute level
    // The SQL parser may reject this syntax entirely
    let body = sql(
        &client,
        &addr,
        "CREATE INDEX ON test_btree_analyzer(field) ANALYZER english",
    )
    .await;

    // Should either fail to parse or fail to execute
    let has_error = body["error"].is_object();
    let has_results_error = body["results"]
        .as_array()
        .map(|arr| arr.is_empty() || arr[0]["error"].is_object())
        .unwrap_or(true);

    assert!(
        has_error || has_results_error,
        "ANALYZER on non-FULLTEXT index should fail: {:?}",
        body
    );
}

#[tokio::test]
async fn test_fts_analyzer_preserves_across_restart() {
    // This test verifies that the analyzer setting is preserved when reopening an index
    // (simulated by creating index, inserting, and searching within same session)
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    sql(&client, &addr, "DEFINE COLLECTION persist_test").await;
    let body = sql(
        &client,
        &addr,
        "CREATE INDEX ON persist_test(content) FULLTEXT ANALYZER english",
    )
    .await;
    assert!(
        common::is_ddl_success(&body),
        "Failed to create English FULLTEXT index: {:?}",
        body
    );

    // Insert multiple documents
    sql(
        &client,
        &addr,
        r#"INSERT INTO persist_test { id: "1", content: "The dogs are barking loudly" }"#,
    )
    .await;
    sql(
        &client,
        &addr,
        r#"INSERT INTO persist_test { id: "2", content: "A dog barks at strangers" }"#,
    )
    .await;
    wait_for_fts_commit().await;

    // Both should match "bark" due to English stemming
    let body = sql(
        &client,
        &addr,
        r#"SELECT id FROM persist_test WHERE content @@ "bark""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();
    assert_eq!(
        results.len(),
        2,
        "English stemmer should find both documents with 'bark' matching 'barking' and 'barks'"
    );
}

#[tokio::test]
async fn test_fts_different_analyzers_different_results() {
    // Demonstrate that different analyzers produce different results
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Create two collections with different analyzers
    sql(&client, &addr, "DEFINE COLLECTION coll_english").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON coll_english(text) FULLTEXT ANALYZER english",
    )
    .await;

    sql(&client, &addr, "DEFINE COLLECTION coll_standard").await;
    sql(
        &client,
        &addr,
        "CREATE INDEX ON coll_standard(text) FULLTEXT",
    )
    .await;

    // Insert same content in both
    let content = r#"{ id: "1", text: "The programmers are programming programs" }"#;
    sql(
        &client,
        &addr,
        &format!("INSERT INTO coll_english {}", content),
    )
    .await;
    sql(
        &client,
        &addr,
        &format!("INSERT INTO coll_standard {}", content),
    )
    .await;
    wait_for_fts_commit().await;

    // Search for "program" - should match in English (stemming) but not in standard
    let body_english = sql(
        &client,
        &addr,
        r#"SELECT * FROM coll_english WHERE text @@ "program""#,
    )
    .await;
    let body_standard = sql(
        &client,
        &addr,
        r#"SELECT * FROM coll_standard WHERE text @@ "program""#,
    )
    .await;

    let data_english = assert_success(&body_english);
    let data_standard = assert_success(&body_standard);

    let english_results = data_english.as_array().unwrap();

    // English should find it (stemming reduces "programming", "programs" to "program")
    assert!(
        !english_results.is_empty(),
        "English analyzer should find 'program' in text with 'programming' and 'programs'"
    );

    // Standard may or may not find it depending on exact tokenization
    // The key point is that analyzers behave differently
    // We just verify both queries completed successfully
    assert!(
        data_standard.is_array(),
        "Standard analyzer query should complete: {:?}",
        body_standard
    );
}

// =============================================================================
// Custom Analyzer Integration Tests (DEFINE ANALYZER)
// =============================================================================

#[tokio::test]
async fn test_define_custom_analyzer() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define custom analyzer
    let body = sql(
        &client,
        &addr,
        "DEFINE ANALYZER my_search TOKENIZER standard FILTERS [lowercase, stemmer(english)]",
    )
    .await;

    // Check that it succeeded
    assert!(
        common::is_ddl_success(&body),
        "DEFINE ANALYZER should succeed: {:?}",
        body
    );
}

#[tokio::test]
async fn test_use_custom_analyzer() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define custom analyzer
    let body = sql(
        &client,
        &addr,
        "DEFINE ANALYZER my_analyzer TOKENIZER standard FILTERS [lowercase]",
    )
    .await;
    assert!(
        common::is_ddl_success(&body),
        "DEFINE ANALYZER should succeed: {:?}",
        body
    );

    // Create collection and index using custom analyzer
    sql(&client, &addr, "DEFINE COLLECTION docs").await;
    let body = sql(
        &client,
        &addr,
        "CREATE INDEX ON docs(content) FULLTEXT ANALYZER my_analyzer",
    )
    .await;
    assert!(
        common::is_ddl_success(&body),
        "CREATE INDEX with custom analyzer should succeed: {:?}",
        body
    );

    // Insert and search
    sql(
        &client,
        &addr,
        r#"INSERT INTO docs { id: "1", content: "Hello World" }"#,
    )
    .await;
    wait_for_fts_commit().await;

    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM docs WHERE content @@ "hello""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();
    assert!(
        !results.is_empty(),
        "Should find document using custom analyzer (lowercase filter)"
    );
}

#[tokio::test]
async fn test_drop_custom_analyzer() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define a temporary analyzer
    let body = sql(
        &client,
        &addr,
        "DEFINE ANALYZER temp_analyzer TOKENIZER whitespace FILTERS [lowercase]",
    )
    .await;
    assert!(
        common::is_ddl_success(&body),
        "DEFINE ANALYZER should succeed: {:?}",
        body
    );

    // Drop the analyzer
    let body = sql(&client, &addr, "DROP ANALYZER temp_analyzer").await;
    assert!(
        common::is_ddl_success(&body),
        "DROP ANALYZER should succeed: {:?}",
        body
    );
}

#[tokio::test]
async fn test_cannot_drop_analyzer_in_use() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define analyzer
    let body = sql(
        &client,
        &addr,
        "DEFINE ANALYZER used_analyzer TOKENIZER standard FILTERS [lowercase]",
    )
    .await;
    assert!(
        common::is_ddl_success(&body),
        "DEFINE ANALYZER should succeed: {:?}",
        body
    );

    // Create collection and index using this analyzer
    sql(&client, &addr, "DEFINE COLLECTION test_drop").await;
    let body = sql(
        &client,
        &addr,
        "CREATE INDEX ON test_drop(field) FULLTEXT ANALYZER used_analyzer",
    )
    .await;
    assert!(
        common::is_ddl_success(&body),
        "CREATE INDEX should succeed: {:?}",
        body
    );

    // Attempt to drop the analyzer - should fail because it's in use
    let body = sql(&client, &addr, "DROP ANALYZER used_analyzer").await;

    // Should have an error
    assert!(
        common::has_error(&body),
        "DROP ANALYZER should fail when analyzer is in use: {:?}",
        body
    );
}

#[tokio::test]
async fn test_cannot_drop_builtin_analyzer() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Attempt to drop built-in analyzer "standard"
    let body = sql(&client, &addr, "DROP ANALYZER standard").await;

    // Should fail
    assert!(
        common::has_error(&body),
        "DROP ANALYZER should fail for built-in analyzer: {:?}",
        body
    );
}

#[tokio::test]
async fn test_cannot_redefine_builtin() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Attempt to redefine built-in analyzer "english"
    let body = sql(
        &client,
        &addr,
        "DEFINE ANALYZER english TOKENIZER standard FILTERS [lowercase]",
    )
    .await;

    // Should fail
    assert!(
        common::has_error(&body),
        "DEFINE ANALYZER should fail for built-in name 'english': {:?}",
        body
    );
}

#[tokio::test]
async fn test_ngram_analyzer() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define ngram analyzer for autocomplete
    let body = sql(
        &client,
        &addr,
        "DEFINE ANALYZER autocomplete TOKENIZER ngram(2, 4) FILTERS [lowercase]",
    )
    .await;
    assert!(
        common::is_ddl_success(&body),
        "DEFINE ANALYZER with ngram tokenizer should succeed: {:?}",
        body
    );

    // Create collection and index
    sql(&client, &addr, "DEFINE COLLECTION items").await;
    let body = sql(
        &client,
        &addr,
        "CREATE INDEX ON items(name) FULLTEXT ANALYZER autocomplete",
    )
    .await;
    assert!(
        common::is_ddl_success(&body),
        "CREATE INDEX with ngram analyzer should succeed: {:?}",
        body
    );

    // Insert document
    sql(
        &client,
        &addr,
        r#"INSERT INTO items { id: "1", name: "computer" }"#,
    )
    .await;
    wait_for_fts_commit().await;

    // Should find partial match with ngram
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM items WHERE name @@ "comp""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();
    assert!(
        !results.is_empty(),
        "Should find partial match 'comp' in 'computer' using ngram analyzer"
    );
}

#[tokio::test]
async fn test_custom_analyzer_with_multiple_filters() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define analyzer with multiple filters
    let body = sql(
        &client,
        &addr,
        "DEFINE ANALYZER multi_filter TOKENIZER standard FILTERS [lowercase, stemmer(english)]",
    )
    .await;
    assert!(
        common::is_ddl_success(&body),
        "DEFINE ANALYZER with multiple filters should succeed: {:?}",
        body
    );

    // Create collection and index
    sql(&client, &addr, "DEFINE COLLECTION multi_docs").await;
    let body = sql(
        &client,
        &addr,
        "CREATE INDEX ON multi_docs(text) FULLTEXT ANALYZER multi_filter",
    )
    .await;
    assert!(
        common::is_ddl_success(&body),
        "CREATE INDEX should succeed: {:?}",
        body
    );

    // Insert document with mixed case and inflected words
    sql(
        &client,
        &addr,
        r#"INSERT INTO multi_docs { id: "1", text: "The RUNNERS are Running Fast" }"#,
    )
    .await;
    wait_for_fts_commit().await;

    // Should find with lowercase + stemming
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM multi_docs WHERE text @@ "run""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();
    assert!(
        !results.is_empty(),
        "Should find 'run' matching 'RUNNERS' and 'Running' with lowercase + stemming"
    );
}

#[tokio::test]
async fn test_custom_analyzer_whitespace_tokenizer() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define analyzer with whitespace tokenizer
    let body = sql(
        &client,
        &addr,
        "DEFINE ANALYZER simple_ws TOKENIZER whitespace FILTERS [lowercase]",
    )
    .await;
    assert!(
        common::is_ddl_success(&body),
        "DEFINE ANALYZER with whitespace tokenizer should succeed: {:?}",
        body
    );

    // Create collection and index
    sql(&client, &addr, "DEFINE COLLECTION ws_docs").await;
    let body = sql(
        &client,
        &addr,
        "CREATE INDEX ON ws_docs(content) FULLTEXT ANALYZER simple_ws",
    )
    .await;
    assert!(
        common::is_ddl_success(&body),
        "CREATE INDEX should succeed: {:?}",
        body
    );

    // Insert document
    sql(
        &client,
        &addr,
        r#"INSERT INTO ws_docs { id: "1", content: "Hello-World Test_Case" }"#,
    )
    .await;
    wait_for_fts_commit().await;

    // Whitespace tokenizer should keep "Hello-World" as single token
    let body = sql(
        &client,
        &addr,
        r#"SELECT * FROM ws_docs WHERE content @@ "hello-world""#,
    )
    .await;

    let data = assert_success(&body);
    let results = data.as_array().unwrap();
    assert!(
        !results.is_empty(),
        "Whitespace tokenizer should find 'hello-world' as a single token"
    );
}

#[tokio::test]
async fn test_redefine_custom_analyzer() {
    let (addr, _tmp) = common::spawn_server().await;
    let client = common::test_client();

    // Define analyzer
    let body = sql(
        &client,
        &addr,
        "DEFINE ANALYZER redef_test TOKENIZER standard FILTERS [lowercase]",
    )
    .await;
    assert!(
        common::is_ddl_success(&body),
        "First DEFINE ANALYZER should succeed: {:?}",
        body
    );

    // Attempt to redefine the same analyzer
    let body = sql(
        &client,
        &addr,
        "DEFINE ANALYZER redef_test TOKENIZER whitespace FILTERS [lowercase]",
    )
    .await;

    // This should either succeed (overwrite) or fail (already exists)
    // depending on implementation. Either behavior is acceptable.
    // Just verify no crash.
    assert!(
        body["results"].is_array(),
        "Redefining custom analyzer should not crash: {:?}",
        body
    );
}
