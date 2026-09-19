//! Graph Node Delete Tests
//!
//! Tests that deleting a node automatically removes all edges to and from it.

use std::panic;
use std::sync::{Arc, Barrier};
use std::thread;
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

/// When a node is deleted, all outgoing edges from it should be removed.
#[test]
fn test_delete_node_removes_outgoing_edges() {
    let (_tmp, storage) = setup();

    // Setup graph
    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "CREATE user:carol SET name = 'Carol'");

    // Create outgoing edges from alice
    run(&storage, "RELATE user:alice->follows->user:bob");
    run(&storage, "RELATE user:alice->follows->user:carol");

    // Verify edges exist
    let result = run(
        &storage,
        "SELECT ->follows->user.name AS following FROM user:alice",
    );
    let following = &result.as_array().unwrap()[0]["following"];
    assert_eq!(following.as_array().unwrap().len(), 2);

    // Delete alice
    run(&storage, "DELETE user:alice");

    // Verify: alice's outgoing edges should be gone
    // Since alice doesn't exist, we check by querying bob's incoming edges
    let result = run(
        &storage,
        "SELECT <-follows<-user.name AS followers FROM user:bob",
    );
    let followers = &result.as_array().unwrap()[0]["followers"];
    assert_eq!(
        followers.as_array().unwrap().len(),
        0,
        "Bob should have no followers after alice was deleted"
    );

    // Same for carol
    let result = run(
        &storage,
        "SELECT <-follows<-user.name AS followers FROM user:carol",
    );
    let followers = &result.as_array().unwrap()[0]["followers"];
    assert_eq!(
        followers.as_array().unwrap().len(),
        0,
        "Carol should have no followers after alice was deleted"
    );
}

/// When a node is deleted, all incoming edges pointing to it should be removed.
#[test]
fn test_delete_node_removes_incoming_edges() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "CREATE user:carol SET name = 'Carol'");

    // Bob and Carol follow Alice
    run(&storage, "RELATE user:bob->follows->user:alice");
    run(&storage, "RELATE user:carol->follows->user:alice");

    // Verify edges exist
    let result = run(
        &storage,
        "SELECT ->follows->user.name AS following FROM user:bob",
    );
    let following = &result.as_array().unwrap()[0]["following"];
    assert_eq!(following.as_array().unwrap().len(), 1);

    // Delete alice (the target)
    run(&storage, "DELETE user:alice");

    // Verify: bob's outgoing edge should be gone
    let result = run(
        &storage,
        "SELECT ->follows->user.name AS following FROM user:bob",
    );
    let following = &result.as_array().unwrap()[0]["following"];
    assert_eq!(
        following.as_array().unwrap().len(),
        0,
        "Bob's edge to alice should be removed when alice is deleted"
    );

    // Same for carol
    let result = run(
        &storage,
        "SELECT ->follows->user.name AS following FROM user:carol",
    );
    let following = &result.as_array().unwrap()[0]["following"];
    assert_eq!(
        following.as_array().unwrap().len(),
        0,
        "Carol's edge to alice should be removed when alice is deleted"
    );
}

/// Deleting a node should clean up edges across collections.
#[test]
fn test_delete_node_cross_collection_edges() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "DEFINE COLLECTION post");

    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE post:p1 SET title = 'Hello'");
    run(&storage, "CREATE post:p2 SET title = 'World'");

    // Alice likes both posts
    run(&storage, "RELATE user:alice->likes->post:p1");
    run(&storage, "RELATE user:alice->likes->post:p2");

    // Verify edges exist
    let result = run(&storage, "SELECT <-likes<-user.name AS likers FROM post:p1");
    let likers = &result.as_array().unwrap()[0]["likers"];
    assert_eq!(likers.as_array().unwrap().len(), 1);

    // Delete alice
    run(&storage, "DELETE user:alice");

    // Verify: posts should have no likers
    let result = run(&storage, "SELECT <-likes<-user.name AS likers FROM post:p1");
    let likers = &result.as_array().unwrap()[0]["likers"];
    assert_eq!(
        likers.as_array().unwrap().len(),
        0,
        "Post should have no likers after alice was deleted"
    );

    let result = run(&storage, "SELECT <-likes<-user.name AS likers FROM post:p2");
    let likers = &result.as_array().unwrap()[0]["likers"];
    assert_eq!(likers.as_array().unwrap().len(), 0);
}

/// Deleting the target of an edge should clean up the edge.
#[test]
fn test_delete_target_node_cleans_edges() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    run(&storage, "RELATE user:alice->follows->user:bob");

    // Verify edge exists
    let result = run(
        &storage,
        "SELECT ->follows->user.name AS following FROM user:alice",
    );
    assert_eq!(
        result.as_array().unwrap()[0]["following"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // Delete bob (the target)
    run(&storage, "DELETE user:bob");

    // Verify: alice's outgoing edge should be cleaned up
    let result = run(
        &storage,
        "SELECT ->follows->user.name AS following FROM user:alice",
    );
    let following = &result.as_array().unwrap()[0]["following"];
    assert_eq!(
        following.as_array().unwrap().len(),
        0,
        "Edge should be removed when target node is deleted"
    );
}

/// When a node with bidirectional edges is deleted, both directions should be cleaned.
#[test]
fn test_delete_node_bidirectional_edges() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    // Bidirectional: alice follows bob, bob follows alice
    run(&storage, "RELATE user:alice->follows->user:bob");
    run(&storage, "RELATE user:bob->follows->user:alice");

    // Verify both directions
    let result = run(
        &storage,
        "SELECT ->follows->user.name AS following FROM user:alice",
    );
    assert_eq!(
        result.as_array().unwrap()[0]["following"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let result = run(
        &storage,
        "SELECT ->follows->user.name AS following FROM user:bob",
    );
    assert_eq!(
        result.as_array().unwrap()[0]["following"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // Delete alice
    run(&storage, "DELETE user:alice");

    // Verify: bob has no outgoing edge (target deleted)
    let result = run(
        &storage,
        "SELECT ->follows->user.name AS following FROM user:bob",
    );
    let following = &result.as_array().unwrap()[0]["following"];
    assert_eq!(
        following.as_array().unwrap().len(),
        0,
        "Bob's edge to alice should be removed"
    );

    // Verify: bob has no incoming edge (source deleted)
    let result = run(
        &storage,
        "SELECT <-follows<-user.name AS followers FROM user:bob",
    );
    let followers = &result.as_array().unwrap()[0]["followers"];
    assert_eq!(
        followers.as_array().unwrap().len(),
        0,
        "Bob should have no followers (alice was deleted)"
    );
}

/// Deleting a node in the middle of a chain should break the chain.
#[test]
fn test_delete_middle_node_in_chain() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:a SET name = 'A'");
    run(&storage, "CREATE user:b SET name = 'B'");
    run(&storage, "CREATE user:c SET name = 'C'");
    run(&storage, "CREATE user:d SET name = 'D'");

    // Chain: A -> B -> C -> D
    run(&storage, "RELATE user:a->follows->user:b");
    run(&storage, "RELATE user:b->follows->user:c");
    run(&storage, "RELATE user:c->follows->user:d");

    // Delete B (middle node)
    run(&storage, "DELETE user:b");

    // A should have no outgoing edges (target B deleted)
    let result = run(
        &storage,
        "SELECT ->follows->user.name AS following FROM user:a",
    );
    let following = &result.as_array().unwrap()[0]["following"];
    assert_eq!(
        following.as_array().unwrap().len(),
        0,
        "A->B edge should be removed"
    );

    // C should have no incoming edges (source B deleted)
    let result = run(
        &storage,
        "SELECT <-follows<-user.name AS followers FROM user:c",
    );
    let followers = &result.as_array().unwrap()[0]["followers"];
    assert_eq!(
        followers.as_array().unwrap().len(),
        0,
        "B->C edge should be removed"
    );

    // C -> D should still work
    let result = run(
        &storage,
        "SELECT ->follows->user.name AS following FROM user:c",
    );
    let following = &result.as_array().unwrap()[0]["following"];
    assert_eq!(
        following.as_array().unwrap().len(),
        1,
        "C->D edge should remain"
    );
}

/// Deleting a node should clean up edges of ALL labels.
#[test]
fn test_delete_node_multiple_edge_labels() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "DEFINE COLLECTION post");

    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "CREATE post:p1 SET title = 'Hello'");

    // Multiple edge types from alice
    run(&storage, "RELATE user:alice->follows->user:bob");
    run(&storage, "RELATE user:alice->likes->post:p1");
    run(&storage, "RELATE user:alice->wrote->post:p1");

    // Delete alice
    run(&storage, "DELETE user:alice");

    // Verify all edge types are cleaned up
    let result = run(
        &storage,
        "SELECT <-follows<-user.name AS followers FROM user:bob",
    );
    assert_eq!(
        result.as_array().unwrap()[0]["followers"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    let result = run(&storage, "SELECT <-likes<-user.name AS likers FROM post:p1");
    assert_eq!(
        result.as_array().unwrap()[0]["likers"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    let result = run(
        &storage,
        "SELECT <-wrote<-user.name AS authors FROM post:p1",
    );
    assert_eq!(
        result.as_array().unwrap()[0]["authors"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

/// Test that concurrent DELETE and RELATE operations are handled safely.
/// Either the delete succeeds and all edges are cleaned up,
/// or the relate fails because the node doesn't exist.
#[test]
fn test_concurrent_delete_and_relate() {
    let (_tmp, storage) = setup();

    // Setup: create alice and bob
    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    // Create some initial edges
    run(&storage, "RELATE user:alice->follows->user:bob");

    let barrier = Arc::new(Barrier::new(11)); // 10 threads + main

    // Use separate vectors for delete and relate handles since they have different return types
    let mut delete_handles = vec![];
    let mut relate_handles = vec![];

    // 5 threads try to delete alice
    for _ in 0..5 {
        let storage = Arc::clone(&storage);
        let barrier = Arc::clone(&barrier);
        delete_handles.push(thread::spawn(move || {
            barrier.wait();
            stellardb::query_json(storage.clone(), "DELETE user:alice")
        }));
    }

    // 5 threads try to create edges to alice
    // Some may fail if alice is deleted before the RELATE completes
    for i in 0..5 {
        let storage = Arc::clone(&storage);
        let barrier = Arc::clone(&barrier);
        relate_handles.push(thread::spawn(move || {
            barrier.wait();
            // Catch panic since query_json panics on errors
            panic::catch_unwind(panic::AssertUnwindSafe(|| {
                stellardb::query_json(
                    storage.clone(),
                    &format!("RELATE user:bob->likes->user:alice SET attempt = {}", i),
                )
            }))
        }));
    }

    // Start all threads
    barrier.wait();

    // Collect results
    for handle in delete_handles {
        let _ = handle.join().unwrap();
    }

    let mut relate_success_count = 0;
    let mut relate_fail_count = 0;
    for handle in relate_handles {
        match handle.join().unwrap() {
            Ok(_) => relate_success_count += 1,
            Err(_) => relate_fail_count += 1,
        }
    }

    // Some RELATE calls may have failed because alice was deleted
    // This is expected and correct behavior
    eprintln!(
        "Results: {} RELATE succeeded, {} RELATE failed (expected if alice deleted first)",
        relate_success_count, relate_fail_count
    );

    // Verify final state is consistent:
    // If alice exists -> edges may or may not exist
    // If alice doesn't exist -> no edges should point to/from alice

    let alice_exists = run(&storage, "SELECT * FROM user:alice");
    let alice_arr = alice_exists.as_array().unwrap();

    if alice_arr.is_empty() {
        // Alice was deleted - verify no edges remain
        // The key invariant: if alice is deleted, bob shouldn't have dangling edges to her
        let bob_outgoing = run(&storage, "SELECT ->likes->user.* AS liked FROM user:bob");
        let liked = &bob_outgoing.as_array().unwrap()[0]["liked"];
        let liked_arr = liked.as_array().unwrap();

        // None of the liked users should be alice (since alice is deleted)
        for user in liked_arr {
            assert_ne!(
                user["id"].as_str().unwrap_or(""),
                "user:alice",
                "Bob should not have edge to deleted alice"
            );
        }
    }

    // Either way, no crash = success for this concurrent test
}

/// Test that concurrent UPDATE and RELATE operations do NOT conflict.
/// UPDATE modifies document content, RELATE creates edges - these should coexist.
#[test]
fn test_concurrent_update_and_relate_no_conflict() {
    let (_tmp, storage) = setup();

    // Setup: create alice and bob
    run(&storage, "DEFINE COLLECTION user");
    run(
        &storage,
        "CREATE user:alice SET name = 'Alice', counter = 0",
    );
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    let barrier = Arc::new(Barrier::new(11)); // 10 threads + main

    let mut update_handles = vec![];
    let mut relate_handles = vec![];

    // 5 threads update alice's counter
    for i in 0..5 {
        let storage = Arc::clone(&storage);
        let barrier = Arc::clone(&barrier);
        update_handles.push(thread::spawn(move || {
            barrier.wait();
            // Each thread sets counter to its own value
            panic::catch_unwind(panic::AssertUnwindSafe(|| {
                stellardb::query_json(
                    storage.clone(),
                    &format!("UPDATE user:alice SET counter = {}", i),
                )
            }))
        }));
    }

    // 5 threads create edges to alice
    for i in 0..5 {
        let storage = Arc::clone(&storage);
        let barrier = Arc::clone(&barrier);
        relate_handles.push(thread::spawn(move || {
            barrier.wait();
            panic::catch_unwind(panic::AssertUnwindSafe(|| {
                stellardb::query_json(
                    storage.clone(),
                    &format!("RELATE user:bob->likes->user:alice SET attempt = {}", i),
                )
            }))
        }));
    }

    // Start all threads
    barrier.wait();

    // Collect results
    let mut update_success = 0;
    let mut update_fail = 0;
    for handle in update_handles {
        match handle.join().unwrap() {
            Ok(_) => update_success += 1,
            Err(_) => update_fail += 1,
        }
    }

    let mut relate_success = 0;
    let mut relate_fail = 0;
    for handle in relate_handles {
        match handle.join().unwrap() {
            Ok(_) => relate_success += 1,
            Err(_) => relate_fail += 1,
        }
    }

    eprintln!(
        "Results: UPDATE {}/{} succeeded, RELATE {}/{} succeeded",
        update_success,
        update_success + update_fail,
        relate_success,
        relate_success + relate_fail
    );

    // Key assertion: RELATE should NOT fail due to concurrent UPDATEs
    // All RELATEs should succeed since alice is never deleted
    assert_eq!(
        relate_fail, 0,
        "RELATE should not fail due to concurrent UPDATE operations"
    );

    // Verify edges were created
    let edges = run(&storage, "SELECT ->likes->user.id AS liked FROM user:bob");
    let liked = &edges.as_array().unwrap()[0]["liked"];
    let liked_arr = liked.as_array().unwrap();

    // Should have at least one edge (edges might be upserted due to same from/to)
    assert!(
        !liked_arr.is_empty(),
        "At least one edge should have been created"
    );

    // Alice should still exist
    let alice = run(&storage, "SELECT * FROM user:alice");
    assert_eq!(
        alice.as_array().unwrap().len(),
        1,
        "Alice should still exist"
    );
}
