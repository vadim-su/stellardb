//! Integration tests for batch edge loading in graph traversal.

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

/// Test that traversal still works correctly with small graph (sequential path)
#[test]
fn test_traversal_small_graph_sequential() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    for i in 0..10 {
        run(
            &storage,
            &format!("CREATE user:u{} SET name = 'User {}'", i, i),
        );
    }

    // Create edges: u0 follows u1..u9
    for i in 1..10 {
        run(&storage, &format!("RELATE user:u0->follows->user:u{}", i));
    }

    let result = run(
        &storage,
        "SELECT ->follows->user.* AS following FROM user:u0",
    );
    let following = result.as_array().unwrap()[0]["following"]
        .as_array()
        .unwrap();
    assert_eq!(following.len(), 9);
}

/// Test traversal with graph large enough to trigger batching (>64 nodes)
#[test]
fn test_traversal_large_graph_batch() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");

    // Create 100 users
    for i in 0..100 {
        run(
            &storage,
            &format!("CREATE user:u{} SET name = 'User {}'", i, i),
        );
    }

    // u0 follows everyone else
    for i in 1..100 {
        run(&storage, &format!("RELATE user:u0->follows->user:u{}", i));
    }

    // Each of u1..u99 follows u100..u199 (need more users)
    for i in 100..200 {
        run(
            &storage,
            &format!("CREATE user:u{} SET name = 'User {}'", i, i),
        );
    }

    for i in 1..100 {
        for j in 100..105 {
            run(
                &storage,
                &format!("RELATE user:u{}->follows->user:u{}", i, j),
            );
        }
    }

    // Two-hop traversal: u0 -> u1..u99 -> u100..u104
    // This should trigger batch loading on second hop (99 nodes > 64 threshold)
    let result = run(
        &storage,
        "SELECT ->follows->follows->user.* AS friends_of_friends FROM user:u0",
    );
    let fof = result.as_array().unwrap()[0]["friends_of_friends"]
        .as_array()
        .unwrap();

    // Should have 5 unique users (u100..u104), deduplicated
    assert_eq!(
        fof.len(),
        5,
        "Should find 5 friends of friends (deduplicated)"
    );
}

/// Test that BFS order is preserved with batching
#[test]
fn test_traversal_bfs_order_with_batch() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");

    // Create star topology: center -> 100 nodes
    run(&storage, "CREATE user:center SET name = 'Center'");
    for i in 0..100 {
        run(
            &storage,
            &format!("CREATE user:leaf{} SET name = 'Leaf {}'", i, i),
        );
        run(
            &storage,
            &format!("RELATE user:center->follows->user:leaf{}", i),
        );
    }

    let result = run(
        &storage,
        "SELECT ->follows->user.* AS leaves FROM user:center",
    );
    let leaves = result.as_array().unwrap()[0]["leaves"].as_array().unwrap();
    assert_eq!(leaves.len(), 100, "Should get all 100 leaves");
}

/// Test bidirectional traversal with batching
#[test]
fn test_traversal_bidirectional_batch() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:center SET name = 'Center'");

    // 50 outgoing + 50 incoming = 100 connections
    for i in 0..50 {
        run(
            &storage,
            &format!("CREATE user:out{} SET name = 'Out {}'", i, i),
        );
        run(
            &storage,
            &format!("RELATE user:center->follows->user:out{}", i),
        );
    }
    for i in 0..50 {
        run(
            &storage,
            &format!("CREATE user:in{} SET name = 'In {}'", i, i),
        );
        run(
            &storage,
            &format!("RELATE user:in{}->follows->user:center", i),
        );
    }

    let result = run(
        &storage,
        "SELECT <->follows<->user.* AS all_connections FROM user:center",
    );
    let connections = result.as_array().unwrap()[0]["all_connections"]
        .as_array()
        .unwrap();
    assert_eq!(connections.len(), 100, "Should get all 100 connections");
}

/// Test deduplication works correctly with batching
#[test]
fn test_traversal_dedup_with_batch() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");

    // Diamond: A -> B, C -> D (both B and C point to D)
    // Scale up: 100 middle nodes all pointing to same target
    run(&storage, "CREATE user:source SET name = 'Source'");
    run(&storage, "CREATE user:target SET name = 'Target'");

    for i in 0..100 {
        run(
            &storage,
            &format!("CREATE user:middle{} SET name = 'Middle {}'", i, i),
        );
        run(
            &storage,
            &format!("RELATE user:source->follows->user:middle{}", i),
        );
        run(
            &storage,
            &format!("RELATE user:middle{}->follows->user:target", i),
        );
    }

    // Two-hop: source -> middle0..99 -> target
    // Target should appear only once due to deduplication
    let result = run(
        &storage,
        "SELECT ->follows->follows->user.* AS targets FROM user:source",
    );
    let targets = result.as_array().unwrap()[0]["targets"].as_array().unwrap();
    assert_eq!(
        targets.len(),
        1,
        "Target should appear only once (deduplicated)"
    );
    assert_eq!(targets[0]["name"], "Target");
}
