//! Stress tests for graph traversal with large datasets.

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

/// Test with 10K nodes, ~10 edges each
#[test]
fn stress_traversal_10k_nodes_uniform() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");

    // Create 10K users
    for i in 0..10_000 {
        run(
            &storage,
            &format!("CREATE user:u{} SET name = 'User {}'", i, i),
        );
    }

    // Each user follows next 10 users (circular)
    for i in 0..10_000 {
        for j in 1..=10 {
            let target = (i + j) % 10_000;
            run(
                &storage,
                &format!("RELATE user:u{}->follows->user:u{}", i, target),
            );
        }
    }

    // Two-hop from u0
    let start = std::time::Instant::now();
    let result = run(
        &storage,
        "SELECT ->follows->follows->user.* AS fof FROM user:u0",
    );
    let elapsed = start.elapsed();

    let fof = result.as_array().unwrap()[0]["fof"].as_array().unwrap();

    // u0 -> u1..u10 -> their followers (up to 100, deduplicated)
    assert!(!fof.is_empty(), "Should have friends of friends");
    println!("10K uniform: {} results in {:?}", fof.len(), elapsed);
}

/// Test with power-law distribution (hubs)
#[test]
fn stress_traversal_10k_nodes_power_law() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");

    // Create 10K users
    for i in 0..10_000 {
        run(
            &storage,
            &format!("CREATE user:u{} SET name = 'User {}'", i, i),
        );
    }

    // Power-law: the first 10 high-degree users get many followers.
    // u0 gets 1000 followers, u1 gets 500, etc.
    for center in 0..10 {
        let follower_count = 1000 / (center + 1);
        for i in 0..follower_count {
            let follower = 10 + center * 100 + i;
            if follower < 10_000 {
                run(
                    &storage,
                    &format!("RELATE user:u{}->follows->user:u{}", follower, center),
                );
            }
        }
    }

    // Query the highest-degree user's followers.
    let start = std::time::Instant::now();
    let result = run(
        &storage,
        "SELECT <-follows<-user.* AS followers FROM user:u0",
    );
    let elapsed = start.elapsed();

    let followers = result.as_array().unwrap()[0]["followers"]
        .as_array()
        .unwrap();
    println!(
        "Power-law center: {} followers in {:?}",
        followers.len(),
        elapsed
    );
}

/// Test deep traversal (100 levels, 10 nodes per level)
#[test]
fn stress_traversal_deep_narrow() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");

    // Create chain: level0 -> level1 -> ... -> level99
    for level in 0..100 {
        for node in 0..10 {
            let id = level * 10 + node;
            run(
                &storage,
                &format!("CREATE user:u{} SET level = {}", id, level),
            );
        }
    }

    // Connect each level to next
    for level in 0..99 {
        for node in 0..10 {
            let from_id = level * 10 + node;
            let to_id = (level + 1) * 10 + node;
            run(
                &storage,
                &format!("RELATE user:u{}->next->user:u{}", from_id, to_id),
            );
        }
    }

    // 5-hop traversal from level 0
    let start = std::time::Instant::now();
    let result = run(
        &storage,
        "SELECT ->next->next->next->next->next->user.* AS level5 FROM user:u0",
    );
    let elapsed = start.elapsed();

    let level5 = result.as_array().unwrap()[0]["level5"].as_array().unwrap();
    assert_eq!(level5.len(), 1, "Should reach level 5");
    println!("Deep narrow: {} results in {:?}", level5.len(), elapsed);
}

/// Test wide shallow (3 levels, 10K per level)
#[test]
fn stress_traversal_wide_shallow() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");

    // Level 0: 1 root
    run(&storage, "CREATE user:root SET level = 0");

    // Level 1: 100 nodes
    for i in 0..100 {
        run(&storage, &format!("CREATE user:l1_{} SET level = 1", i));
        run(
            &storage,
            &format!("RELATE user:root->follows->user:l1_{}", i),
        );
    }

    // Level 2: 100 per level1 node = 10K total
    for l1 in 0..100 {
        for l2 in 0..100 {
            let id = l1 * 100 + l2;
            run(&storage, &format!("CREATE user:l2_{} SET level = 2", id));
            run(
                &storage,
                &format!("RELATE user:l1_{}->follows->user:l2_{}", l1, id),
            );
        }
    }

    // Two-hop: root -> l1 (100) -> l2 (10K)
    let start = std::time::Instant::now();
    let result = run(
        &storage,
        "SELECT ->follows->follows->user.* AS l2_nodes FROM user:root",
    );
    let elapsed = start.elapsed();

    let l2 = result.as_array().unwrap()[0]["l2_nodes"]
        .as_array()
        .unwrap();
    assert_eq!(l2.len(), 10_000, "Should reach all 10K level 2 nodes");
    println!("Wide shallow: {} results in {:?}", l2.len(), elapsed);
}
