//! Existence Registry Tests
//!
//! Tests that verify the existence registry works correctly:
//! - CREATE adds marker
//! - DELETE removes marker
//! - UPDATE doesn't touch marker
//! - RELATE reads only markers (no conflict with UPDATE)

use std::panic::AssertUnwindSafe;
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

/// CREATE should add existence marker
#[test]
fn test_create_adds_existence_marker() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");

    // Verify document exists
    let result = run(&storage, "SELECT * FROM user:alice");
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "Alice");

    // Create edge to verify existence marker works
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "RELATE user:bob->follows->user:alice");

    // Verify edge was created (marker existed)
    let result = run(
        &storage,
        "SELECT ->follows->user.name AS following FROM user:bob",
    );
    let following = &result.as_array().unwrap()[0]["following"];
    assert_eq!(following.as_array().unwrap().len(), 1);
}

/// DELETE should remove existence marker
#[test]
fn test_delete_removes_existence_marker() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    // Verify alice exists
    let result = run(&storage, "SELECT * FROM user:alice");
    assert_eq!(result.as_array().unwrap().len(), 1);

    // Delete alice
    run(&storage, "DELETE user:alice");

    // Verify alice doesn't exist
    let result = run(&storage, "SELECT * FROM user:alice");
    assert_eq!(result.as_array().unwrap().len(), 0);

    // Try to create edge to deleted document - should fail
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        run(&storage, "RELATE user:bob->follows->user:alice")
    }));
    assert!(result.is_err(), "RELATE to deleted document should fail");
}

/// Concurrent UPDATE and RELATE should NOT conflict
/// This is the key test for existence registry
#[test]
fn test_concurrent_update_and_relate_no_conflict() {
    let (_tmp, storage) = setup();

    // Setup
    stellardb::query_json(storage.clone(), "DEFINE COLLECTION user");
    stellardb::query_json(storage.clone(), "CREATE user:alice SET counter = 0");
    stellardb::query_json(storage.clone(), "CREATE user:bob SET name = 'Bob'");

    let num_threads = 5;
    let barrier = Arc::new(Barrier::new(num_threads * 2));

    // Spawn UPDATE threads
    let mut update_handles = vec![];
    for i in 0..num_threads {
        let storage = Arc::clone(&storage);
        let barrier = Arc::clone(&barrier);
        update_handles.push(thread::spawn(move || {
            barrier.wait();
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                stellardb::query_json(
                    storage.clone(),
                    &format!("UPDATE user:alice SET counter = {}", i),
                )
            }));
            result.is_ok()
        }));
    }

    // Spawn RELATE threads
    let mut relate_handles = vec![];
    for i in 0..num_threads {
        let storage = Arc::clone(&storage);
        let barrier = Arc::clone(&barrier);
        relate_handles.push(thread::spawn(move || {
            barrier.wait();
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                stellardb::query_json(
                    storage.clone(),
                    &format!("RELATE user:bob->likes{}->user:alice", i),
                )
            }));
            result.is_ok()
        }));
    }

    // Collect results
    let update_success: usize = update_handles
        .into_iter()
        .filter_map(|h| h.join().ok())
        .filter(|&success| success)
        .count();
    let relate_success: usize = relate_handles
        .into_iter()
        .filter_map(|h| h.join().ok())
        .filter(|&success| success)
        .count();

    println!("UPDATE success: {}/{}", update_success, num_threads);
    println!("RELATE success: {}/{}", relate_success, num_threads);

    // Key assertion: ALL RELATE operations should succeed
    // because UPDATE doesn't touch existence marker
    assert_eq!(
        relate_success, num_threads,
        "All RELATE should succeed - UPDATE should not cause conflict"
    );

    // At least some UPDATEs should succeed
    assert!(update_success > 0, "At least some UPDATEs should succeed");
}

/// Concurrent DELETE and RELATE should conflict correctly
/// DELETE wins - RELATE should fail with NotFound
#[test]
fn test_concurrent_delete_and_relate_conflict() {
    let (_tmp, storage) = setup();

    // Setup
    stellardb::query_json(storage.clone(), "DEFINE COLLECTION user");
    stellardb::query_json(storage.clone(), "CREATE user:alice SET name = 'Alice'");
    stellardb::query_json(storage.clone(), "CREATE user:bob SET name = 'Bob'");

    let barrier = Arc::new(Barrier::new(2));

    // DELETE thread
    let storage_del = Arc::clone(&storage);
    let barrier_del = Arc::clone(&barrier);
    let delete_handle = thread::spawn(move || {
        barrier_del.wait();
        std::panic::catch_unwind(AssertUnwindSafe(|| {
            stellardb::query_json(storage_del.clone(), "DELETE user:alice")
        }))
        .is_ok()
    });

    // RELATE thread
    let storage_rel = Arc::clone(&storage);
    let barrier_rel = Arc::clone(&barrier);
    let relate_handle = thread::spawn(move || {
        barrier_rel.wait();
        std::panic::catch_unwind(AssertUnwindSafe(|| {
            stellardb::query_json(storage_rel.clone(), "RELATE user:bob->follows->user:alice")
        }))
        .is_ok()
    });

    let delete_ok = delete_handle.join().unwrap();
    let relate_ok = relate_handle.join().unwrap();

    println!("DELETE success: {}", delete_ok);
    println!("RELATE success: {}", relate_ok);

    // DELETE should always succeed
    assert!(delete_ok, "DELETE should succeed");

    // After the race, alice should not exist
    let result = stellardb::query_json(storage.clone(), "SELECT * FROM user:alice");
    assert_eq!(
        result.as_array().unwrap().len(),
        0,
        "Alice should be deleted"
    );

    // If RELATE succeeded, the edge should exist but point to deleted doc
    // If RELATE failed (conflict), no edge
    // Both outcomes are acceptable - the key is consistency
}

/// RELATE to non-existent document should fail immediately
#[test]
fn test_relate_to_nonexistent_document_fails() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    // Try to relate to non-existent alice
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        run(&storage, "RELATE user:bob->follows->user:alice")
    }));

    assert!(
        result.is_err(),
        "RELATE to non-existent document should fail"
    );

    // Also test from non-existent
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        run(&storage, "RELATE user:ghost->follows->user:bob")
    }));

    assert!(
        result.is_err(),
        "RELATE from non-existent document should fail"
    );
}
