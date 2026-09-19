//! Integration tests for RELATE statement.

use std::sync::Arc;
use stellardb::Database;
use tempfile::TempDir;

fn setup() -> (TempDir, Arc<Database>) {
    let tmp = TempDir::new().unwrap();
    let storage = Arc::new(Database::open(tmp.path()).unwrap());
    (tmp, storage)
}

fn run(storage: &Arc<Database>, sql: &str) -> serde_json::Value {
    stellardb::query_json(storage.clone(), sql)
}

fn run_err(storage: &Arc<Database>, sql: &str) -> String {
    stellardb::try_run_sql!(storage.clone(), sql)
        .unwrap_err()
        .to_string()
}

// =============================================================================
// Basic RELATE tests
// =============================================================================

#[test]
fn test_relate_basic() {
    let (_tmp, storage) = setup();

    // Create documents first
    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    // Create edge
    let result = run(&storage, "RELATE user:alice->follows->user:bob");
    let edges = result.as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["from"], "user:alice");
    assert_eq!(edges[0]["to"], "user:bob");
}

#[test]
fn test_relate_with_set() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    let result = run(
        &storage,
        "RELATE user:alice->follows->user:bob SET since = 2024, mutual = true",
    );
    let edges = result.as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["since"], 2024);
    assert_eq!(edges[0]["mutual"], true);
}

#[test]
fn test_relate_with_content() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    let result = run(
        &storage,
        "RELATE user:alice->follows->user:bob CONTENT {since: 2024, source: 'web'}",
    );
    let edges = result.as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["since"], 2024);
    assert_eq!(edges[0]["source"], "web");
}

#[test]
fn test_relate_array_target() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "CREATE user:carol SET name = 'Carol'");

    let result = run(
        &storage,
        "RELATE user:alice->follows->[user:bob, user:carol]",
    );
    let edges = result.as_array().unwrap();
    assert_eq!(edges.len(), 2);

    // Verify all targets
    let outs: Vec<&str> = edges.iter().map(|e| e["to"].as_str().unwrap()).collect();
    assert!(outs.contains(&"user:bob"));
    assert!(outs.contains(&"user:carol"));
}

#[test]
fn test_relate_array_source() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "CREATE user:carol SET name = 'Carol'");

    let result = run(
        &storage,
        "RELATE [user:alice, user:bob]->follows->user:carol",
    );
    let edges = result.as_array().unwrap();
    assert_eq!(edges.len(), 2);

    // Verify all sources
    let ins: Vec<&str> = edges.iter().map(|e| e["from"].as_str().unwrap()).collect();
    assert!(ins.contains(&"user:alice"));
    assert!(ins.contains(&"user:bob"));

    // All should point to carol
    for edge in edges {
        assert_eq!(edge["to"], "user:carol");
    }
}

#[test]
fn test_relate_return_none() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    let result = run(&storage, "RELATE user:alice->follows->user:bob RETURN NONE");
    // Should return count as [{"count": N}]
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["count"], 1);
}

#[test]
fn test_relate_upsert_updates_existing() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    // Create edge
    run(
        &storage,
        "RELATE user:alice->follows->user:bob SET since = 2023",
    );

    // Create same edge again (upsert) with different data
    let result = run(
        &storage,
        "RELATE user:alice->follows->user:bob SET since = 2024",
    );
    let edges = result.as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["since"], 2024);

    // Verify there's still only one edge (not two) via traversal
    let check = run(&storage, "SELECT ->follows AS following FROM user:alice");
    let data = check.as_array().unwrap();
    assert_eq!(data.len(), 1);
    let following = data[0]["following"].as_array().unwrap();
    assert_eq!(following.len(), 1);
}

#[test]
fn test_relate_cartesian_product() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "CREATE user:carol SET name = 'Carol'");

    // 2 from x 2 to = 4 edges
    let result = run(
        &storage,
        "RELATE [user:alice, user:bob]->follows->[user:bob, user:carol]",
    );
    let edges = result.as_array().unwrap();
    assert_eq!(edges.len(), 4);

    // Verify expected combinations
    let pairs: Vec<(&str, &str)> = edges
        .iter()
        .map(|e| (e["from"].as_str().unwrap(), e["to"].as_str().unwrap()))
        .collect();

    assert!(pairs.contains(&("user:alice", "user:bob")));
    assert!(pairs.contains(&("user:alice", "user:carol")));
    assert!(pairs.contains(&("user:bob", "user:bob")));
    assert!(pairs.contains(&("user:bob", "user:carol")));
}

// =============================================================================
// Edge ID and metadata tests
// =============================================================================

#[test]
fn test_relate_edge_id_format() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    let result = run(&storage, "RELATE user:alice->follows->user:bob");
    let edges = result.as_array().unwrap();
    let id = edges[0]["id"].as_str().unwrap();

    // Edge ID format is label:nanoid (e.g., "follows:TAdhA4qzHUPCSMMfFter6")
    assert!(
        id.starts_with("follows:"),
        "Edge ID should start with label prefix, got: {id}"
    );
    let nanoid_part = &id["follows:".len()..];
    assert_eq!(nanoid_part.len(), 21, "Nanoid should be 21 chars");
}

#[test]
fn test_relate_protected_fields_cannot_be_modified() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    // Attempting to set protected fields should fail
    let err = run_err(
        &storage,
        "RELATE user:alice->follows->user:bob SET id = 'custom'",
    );
    assert!(err.contains("protected edge field"), "Error: {}", err);

    let err = run_err(
        &storage,
        "RELATE user:alice->follows->user:bob SET from = 'hacked'",
    );
    assert!(err.contains("protected edge field"), "Error: {}", err);

    let err = run_err(
        &storage,
        "RELATE user:alice->follows->user:bob SET to = 'hacked'",
    );
    assert!(err.contains("protected edge field"), "Error: {}", err);

    // But setting custom fields like 'label' (user-defined, shadows system label) should work
    let result = run(
        &storage,
        "RELATE user:alice->follows->user:bob SET label = 'custom_label'",
    );
    let edges = result.as_array().unwrap();
    assert_eq!(edges[0]["label"], "custom_label");
}

// =============================================================================
// Different edge labels
// =============================================================================

#[test]
fn test_relate_different_labels() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    // Create edges with different labels
    run(&storage, "RELATE user:alice->follows->user:bob");
    run(&storage, "RELATE user:alice->knows->user:bob");
    run(&storage, "RELATE user:alice->likes->user:bob");

    // Each should be a separate edge
    let follows = run(&storage, "SELECT ->follows AS f FROM user:alice");
    let knows = run(&storage, "SELECT ->knows AS k FROM user:alice");
    let likes = run(&storage, "SELECT ->likes AS l FROM user:alice");

    assert_eq!(
        follows.as_array().unwrap()[0]["f"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        knows.as_array().unwrap()[0]["k"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        likes.as_array().unwrap()[0]["l"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn test_relate_bidirectional() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    // Create edges in both directions
    run(&storage, "RELATE user:alice->follows->user:bob");
    run(&storage, "RELATE user:bob->follows->user:alice");

    // Check outgoing from alice - basic traversal returns edge IDs
    let out_alice = run(&storage, "SELECT ->follows AS following FROM user:alice");
    let following = out_alice.as_array().unwrap()[0]["following"]
        .as_array()
        .unwrap();
    assert_eq!(following.len(), 1);
    // Edge ID format is label:nanoid
    let following_id = following[0]["id"].as_str().unwrap();
    assert!(
        following_id.starts_with("follows:"),
        "Edge ID should start with 'follows:', got: {following_id}"
    );

    // Check incoming to alice
    let in_alice = run(&storage, "SELECT <-follows AS followers FROM user:alice");
    let followers = in_alice.as_array().unwrap()[0]["followers"]
        .as_array()
        .unwrap();
    assert_eq!(followers.len(), 1);
    // Edge ID format is label:nanoid
    let followers_id = followers[0]["id"].as_str().unwrap();
    assert!(
        followers_id.starts_with("follows:"),
        "Edge ID should start with 'follows:', got: {followers_id}"
    );
}

// =============================================================================
// Complex data types in SET/CONTENT
// =============================================================================

#[test]
fn test_relate_with_nested_content() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    let result = run(
        &storage,
        "RELATE user:alice->follows->user:bob CONTENT {metadata: {created_by: 'system', version: 1}}",
    );
    let edges = result.as_array().unwrap();
    assert_eq!(edges.len(), 1);

    let metadata = &edges[0]["metadata"];
    assert_eq!(metadata["created_by"], "system");
    assert_eq!(metadata["version"], 1);
}

#[test]
fn test_relate_with_array_content() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    let result = run(
        &storage,
        "RELATE user:alice->follows->user:bob CONTENT {tags: ['friend', 'colleague', 'mentor']}",
    );
    let edges = result.as_array().unwrap();
    assert_eq!(edges.len(), 1);

    let tags = edges[0]["tags"].as_array().unwrap();
    assert_eq!(tags.len(), 3);
    assert_eq!(tags[0], "friend");
    assert_eq!(tags[1], "colleague");
    assert_eq!(tags[2], "mentor");
}

// =============================================================================
// Edge with different document collections
// =============================================================================

#[test]
fn test_relate_cross_collection() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "DEFINE COLLECTION post");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE post:1 SET title = 'Hello World'");

    // User authors post
    let result = run(&storage, "RELATE user:alice->authored->post:1");
    let edges = result.as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["from"], "user:alice");
    assert_eq!(edges[0]["to"], "post:1");

    // User likes post
    let result = run(&storage, "RELATE user:alice->likes->post:1 SET rating = 5");
    let edges = result.as_array().unwrap();
    assert_eq!(edges[0]["rating"], 5);
}

// =============================================================================
// RETURN clause variations
// =============================================================================

#[test]
fn test_relate_return_after() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    // RETURN AFTER is the default but let's be explicit
    let result = run(
        &storage,
        "RELATE user:alice->follows->user:bob SET since = 2024 RETURN AFTER",
    );
    let edges = result.as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["since"], 2024);
}

#[test]
fn test_relate_return_none_multiple() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "CREATE user:carol SET name = 'Carol'");

    // Create multiple edges with RETURN NONE
    let result = run(
        &storage,
        "RELATE user:alice->follows->[user:bob, user:carol] RETURN NONE",
    );
    // Result is now [{"count": 2}]
    let arr = result.as_array().unwrap();
    assert_eq!(arr[0]["count"], 2);
}

// =============================================================================
// Self-referential edges
// =============================================================================

#[test]
fn test_relate_self_reference() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");

    // User follows themselves (some systems allow this)
    let result = run(&storage, "RELATE user:alice->follows->user:alice");
    let edges = result.as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["from"], "user:alice");
    assert_eq!(edges[0]["to"], "user:alice");
}

// =============================================================================
// RELATE with expressions in SET
// =============================================================================

#[test]
fn test_relate_set_with_expressions() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    let result = run(
        &storage,
        "RELATE user:alice->follows->user:bob SET weight = 5 * 2, active = true AND true",
    );
    let edges = result.as_array().unwrap();
    assert_eq!(edges[0]["weight"], 10);
    assert_eq!(edges[0]["active"], true);
}

#[test]
fn test_relate_set_with_string_concat() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    let result = run(
        &storage,
        "RELATE user:alice->follows->user:bob SET note = 'friend' + ' since 2024'",
    );
    let edges = result.as_array().unwrap();
    assert_eq!(edges[0]["note"], "friend since 2024");
}
