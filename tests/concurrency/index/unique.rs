//! Concurrency tests for unique index constraints.
//!
//! Tests race conditions in unique constraint validation during concurrent operations.

use crate::common::{run, setup, try_run};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

/// Test concurrent inserts with same unique value.
/// Only ONE insert should succeed, all others should fail.
/// Uses lock striping to ensure atomic check-and-insert in storage layer.
#[test]
fn test_concurrent_unique_inserts_same_value() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION users");
    run(&storage, "CREATE INDEX ON users(email) UNIQUE");

    let num_threads = 10;
    let barrier = Arc::new(Barrier::new(num_threads));
    let success_count = Arc::new(AtomicUsize::new(0));
    let failure_count = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..num_threads)
        .map(|i| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            let success_count = Arc::clone(&success_count);
            let failure_count = Arc::clone(&failure_count);

            thread::spawn(move || {
                barrier.wait();

                let result = try_run(
                    &storage,
                    &format!(
                        "INSERT INTO users {{id: 'user{}', email: 'same@test.com'}}",
                        i
                    ),
                );

                match result {
                    Ok(_) => {
                        success_count.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(e) => {
                        assert!(
                            e.contains("Conflict")
                                || e.contains("Unique")
                                || e.contains("constraint")
                                || e.contains("Already"),
                            "Unexpected error: {}",
                            e
                        );
                        failure_count.fetch_add(1, Ordering::SeqCst);
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    let successes = success_count.load(Ordering::SeqCst);
    let failures = failure_count.load(Ordering::SeqCst);

    assert_eq!(
        successes, 1,
        "Expected exactly 1 success, got {} successes and {} failures",
        successes, failures
    );

    let result = run(
        &storage,
        "SELECT * FROM users WHERE email = 'same@test.com'",
    );
    assert_eq!(result.as_array().unwrap().len(), 1);
}

/// Test concurrent inserts with different unique values.
#[test]
fn test_concurrent_unique_inserts_different_values() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION users");
    run(&storage, "CREATE INDEX ON users(email) UNIQUE");

    let num_threads = 20;
    let barrier = Arc::new(Barrier::new(num_threads));
    let success_count = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..num_threads)
        .map(|i| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            let success_count = Arc::clone(&success_count);

            thread::spawn(move || {
                barrier.wait();

                let result = try_run(
                    &storage,
                    &format!(
                        "INSERT INTO users {{id: 'user{}', email: 'user{}@test.com'}}",
                        i, i
                    ),
                );

                if result.is_ok() {
                    success_count.fetch_add(1, Ordering::SeqCst);
                } else {
                    panic!("Insert failed unexpectedly: {:?}", result);
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    assert_eq!(success_count.load(Ordering::SeqCst), num_threads);

    let result = run(&storage, "SELECT * FROM users");
    assert_eq!(result.as_array().unwrap().len(), num_threads);
}

/// Stress test: many threads doing random operations on unique field.
/// Uses lock striping to ensure atomic operations under concurrent load.
#[test]
fn test_unique_index_stress() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION items");
    run(&storage, "CREATE INDEX ON items(code) UNIQUE");

    for i in 0..10 {
        run(
            &storage,
            &format!("INSERT INTO items {{id: 'item{}', code: 'CODE{}'}}", i, i),
        );
    }

    let num_threads = 8;
    let ops_per_thread = 50;
    let barrier = Arc::new(Barrier::new(num_threads));

    let handles: Vec<_> = (0..num_threads)
        .map(|thread_id| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);

            thread::spawn(move || {
                barrier.wait();

                for op in 0..ops_per_thread {
                    let item_id = (thread_id * ops_per_thread + op) % 20;
                    let code = format!("CODE{}", item_id);

                    match op % 3 {
                        0 => {
                            let _ = try_run(
                                &storage,
                                &format!(
                                    "INSERT INTO items {{id: 'new_{}_{}', code: '{}'}}",
                                    thread_id, op, code
                                ),
                            );
                        }
                        1 => {
                            let _ = try_run(
                                &storage,
                                &format!(
                                    "UPDATE items:item{} SET code = 'UPDATED{}'",
                                    item_id % 10,
                                    op
                                ),
                            );
                        }
                        2 => {
                            let _ =
                                try_run(&storage, &format!("DELETE items:item{}", item_id % 10));
                        }
                        _ => unreachable!(),
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    // Verify no duplicate codes
    let result = run(&storage, "SELECT code FROM items");
    let items = result.as_array().unwrap();
    let mut codes: Vec<&str> = items
        .iter()
        .filter_map(|item| item["code"].as_str())
        .collect();
    let original_len = codes.len();
    codes.sort();
    codes.dedup();

    assert_eq!(
        codes.len(),
        original_len,
        "Found duplicate codes after stress test!"
    );
}

// ============================================================================
// Edge Case Tests for Unique Constraint Race Conditions
// ============================================================================

/// Edge case: DELETE + INSERT race on the same unique value.
/// Thread 1 deletes a document with value X, Thread 2 inserts a new document with value X.
/// Expected: Exactly one document with value X should exist after both operations complete.
#[test]
fn test_delete_insert_same_value_race() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION items");
    run(&storage, "CREATE INDEX ON items(code) UNIQUE");

    // Insert initial document
    run(
        &storage,
        "INSERT INTO items {id: 'original', code: 'UNIQUE_CODE'}",
    );

    let num_iterations: usize = 50;

    for iteration in 0..num_iterations {
        // Reset: ensure we have exactly one document with the code
        let _ = try_run(&storage, "DELETE items:original");
        if iteration > 0 {
            let _ = try_run(&storage, &format!("DELETE items:new{}", iteration - 1));
        }
        run(
            &storage,
            "INSERT INTO items {id: 'original', code: 'UNIQUE_CODE'}",
        );

        let barrier = Arc::new(Barrier::new(2));
        let storage1 = Arc::clone(&storage);
        let storage2 = Arc::clone(&storage);
        let barrier1 = Arc::clone(&barrier);
        let barrier2 = Arc::clone(&barrier);

        let delete_handle = thread::spawn(move || {
            barrier1.wait();
            try_run(&storage1, "DELETE items:original")
        });

        let insert_handle = thread::spawn(move || {
            barrier2.wait();
            try_run(
                &storage2,
                &format!(
                    "INSERT INTO items {{id: 'new{}', code: 'UNIQUE_CODE'}}",
                    iteration
                ),
            )
        });

        let _delete_result = delete_handle.join().unwrap();
        let _insert_result = insert_handle.join().unwrap();

        // Verify: exactly one document with UNIQUE_CODE
        let result = run(&storage, "SELECT id FROM items WHERE code = 'UNIQUE_CODE'");
        let count = result.as_array().unwrap().len();
        assert!(
            count <= 1,
            "Iteration {}: Found {} documents with UNIQUE_CODE (expected 0 or 1)",
            iteration,
            count
        );
    }
}

/// Edge case: UPDATE frees a value that INSERT can then use.
/// Thread 1: UPDATE doc1 SET code = 'B' (was 'A')
/// Thread 2: INSERT doc2 with code = 'A'
/// Expected: Either both succeed (if serialized correctly) or INSERT fails (conflict).
/// Key invariant: no duplicate unique values.
#[test]
fn test_update_frees_value_for_insert() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION items");
    run(&storage, "CREATE INDEX ON items(code) UNIQUE");

    let num_iterations = 30;

    for iteration in 0..num_iterations {
        // Clean slate for each iteration - delete ALL items
        let _ = try_run(&storage, "DELETE items:doc1");
        let _ = try_run(&storage, "DELETE items:doc2");

        // Start with doc1 having VALUE_A
        run(&storage, "INSERT INTO items {id: 'doc1', code: 'VALUE_A'}");

        let barrier = Arc::new(Barrier::new(2));
        let storage1 = Arc::clone(&storage);
        let storage2 = Arc::clone(&storage);
        let barrier1 = Arc::clone(&barrier);
        let barrier2 = Arc::clone(&barrier);

        let update_handle = thread::spawn(move || {
            barrier1.wait();
            try_run(
                &storage1,
                &format!("UPDATE items:doc1 SET code = 'VALUE_B_{}'", iteration),
            )
        });

        let insert_handle = thread::spawn(move || {
            barrier2.wait();
            // Use consistent doc2 id
            try_run(&storage2, "INSERT INTO items {id: 'doc2', code: 'VALUE_A'}")
        });

        let _update_result = update_handle.join().unwrap();
        let _insert_result = insert_handle.join().unwrap();

        // Verify no duplicates - this is the key invariant
        let result = run(&storage, "SELECT code FROM items");
        let items = result.as_array().unwrap();
        let mut codes: Vec<&str> = items
            .iter()
            .filter_map(|item| item["code"].as_str())
            .collect();
        let original_len = codes.len();
        codes.sort();
        codes.dedup();

        assert_eq!(
            codes.len(),
            original_len,
            "Iteration {}: Found duplicate codes",
            iteration
        );
    }
}

/// Edge case: Two documents swap their unique values.
/// Thread 1: UPDATE doc1 SET code = 'B' (was 'A')
/// Thread 2: UPDATE doc2 SET code = 'A' (was 'B')
/// This is a classic deadlock/race scenario.
#[test]
fn test_swap_unique_values() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION items");
    run(&storage, "CREATE INDEX ON items(code) UNIQUE");

    let num_iterations = 30;

    for iteration in 0..num_iterations {
        // Reset to known state
        let _ = try_run(&storage, "DELETE items:doc1");
        let _ = try_run(&storage, "DELETE items:doc2");
        run(&storage, "INSERT INTO items {id: 'doc1', code: 'ALPHA'}");
        run(&storage, "INSERT INTO items {id: 'doc2', code: 'BETA'}");

        let barrier = Arc::new(Barrier::new(2));
        let storage1 = Arc::clone(&storage);
        let storage2 = Arc::clone(&storage);
        let barrier1 = Arc::clone(&barrier);
        let barrier2 = Arc::clone(&barrier);

        // Thread 1: ALPHA -> BETA
        let handle1 = thread::spawn(move || {
            barrier1.wait();
            try_run(&storage1, "UPDATE items:doc1 SET code = 'BETA'")
        });

        // Thread 2: BETA -> ALPHA
        let handle2 = thread::spawn(move || {
            barrier2.wait();
            try_run(&storage2, "UPDATE items:doc2 SET code = 'ALPHA'")
        });

        let result1 = handle1.join().unwrap();
        let result2 = handle2.join().unwrap();

        // At least one should fail (conflict) OR both succeed if serialized properly
        // The key invariant: no duplicate codes
        let result = run(&storage, "SELECT code FROM items");
        let items = result.as_array().unwrap();
        let mut codes: Vec<&str> = items
            .iter()
            .filter_map(|item| item["code"].as_str())
            .collect();
        let original_len = codes.len();
        codes.sort();
        codes.dedup();

        assert_eq!(
            codes.len(),
            original_len,
            "Iteration {}: Swap caused duplicates! Results: {:?}, {:?}",
            iteration,
            result1,
            result2
        );
    }
}

/// Edge case: Compound unique index with partial field matches.
/// Unique on (field1, field2) - documents with same field1 but different field2 should not conflict.
#[test]
fn test_compound_unique_partial_match() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION items");
    run(&storage, "CREATE INDEX ON items(category, sku) UNIQUE");

    let num_threads = 10;
    let barrier = Arc::new(Barrier::new(num_threads));
    let success_count = Arc::new(AtomicUsize::new(0));

    // All threads insert same category but different sku - should all succeed
    let handles: Vec<_> = (0..num_threads)
        .map(|i| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            let success_count = Arc::clone(&success_count);

            thread::spawn(move || {
                barrier.wait();
                let result = try_run(
                    &storage,
                    &format!(
                        "INSERT INTO items {{id: 'item{}', category: 'electronics', sku: 'SKU{}'}}",
                        i, i
                    ),
                );
                if result.is_ok() {
                    success_count.fetch_add(1, Ordering::SeqCst);
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    // All should succeed since (category, sku) combinations are unique
    assert_eq!(
        success_count.load(Ordering::SeqCst),
        num_threads,
        "All inserts should succeed with different compound key values"
    );

    // Now test conflict: same (category, sku)
    let barrier2 = Arc::new(Barrier::new(5));
    let conflict_success = Arc::new(AtomicUsize::new(0));

    let handles2: Vec<_> = (0..5)
        .map(|i| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier2);
            let conflict_success = Arc::clone(&conflict_success);

            thread::spawn(move || {
                barrier.wait();
                let result = try_run(
                    &storage,
                    &format!(
                        "INSERT INTO items {{id: 'conflict{}', category: 'books', sku: 'SAME_SKU'}}",
                        i
                    ),
                );
                if result.is_ok() {
                    conflict_success.fetch_add(1, Ordering::SeqCst);
                }
            })
        })
        .collect();

    for handle in handles2 {
        handle.join().unwrap();
    }

    assert_eq!(
        conflict_success.load(Ordering::SeqCst),
        1,
        "Only one insert should succeed for same compound key"
    );
}

/// Edge case: Documents without the unique field should not conflict.
/// NULL/missing values in unique fields should be allowed multiple times.
#[test]
fn test_null_unique_values_no_conflict() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION items");
    run(&storage, "CREATE INDEX ON items(code) UNIQUE");

    let num_threads = 10;
    let barrier = Arc::new(Barrier::new(num_threads));
    let success_count = Arc::new(AtomicUsize::new(0));

    // All threads insert documents WITHOUT the unique field
    let handles: Vec<_> = (0..num_threads)
        .map(|i| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            let success_count = Arc::clone(&success_count);

            thread::spawn(move || {
                barrier.wait();
                let result = try_run(
                    &storage,
                    &format!("INSERT INTO items {{id: 'item{}', name: 'Item {}'}}", i, i),
                );
                if result.is_ok() {
                    success_count.fetch_add(1, Ordering::SeqCst);
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    // All should succeed - missing field means no constraint check
    assert_eq!(
        success_count.load(Ordering::SeqCst),
        num_threads,
        "All inserts should succeed when unique field is missing"
    );

    let result = run(&storage, "SELECT * FROM items");
    assert_eq!(result.as_array().unwrap().len(), num_threads);
}

/// Edge case: Extreme contention on a single unique value.
/// Many threads fight for the same value simultaneously.
#[test]
fn test_extreme_contention_single_value() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION items");
    run(&storage, "CREATE INDEX ON items(code) UNIQUE");

    let num_threads = 50;
    let barrier = Arc::new(Barrier::new(num_threads));
    let success_count = Arc::new(AtomicUsize::new(0));
    let failure_count = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..num_threads)
        .map(|i| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            let success_count = Arc::clone(&success_count);
            let failure_count = Arc::clone(&failure_count);

            thread::spawn(move || {
                barrier.wait();
                let result = try_run(
                    &storage,
                    &format!(
                        "INSERT INTO items {{id: 'item{}', code: 'THE_ONLY_VALUE'}}",
                        i
                    ),
                );
                match result {
                    Ok(_) => success_count.fetch_add(1, Ordering::SeqCst),
                    Err(_) => failure_count.fetch_add(1, Ordering::SeqCst),
                };
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    let successes = success_count.load(Ordering::SeqCst);
    let failures = failure_count.load(Ordering::SeqCst);

    assert_eq!(
        successes, 1,
        "Exactly 1 should succeed under extreme contention, got {} successes, {} failures",
        successes, failures
    );
    assert_eq!(failures, num_threads - 1);

    // Verify only one document exists
    let result = run(
        &storage,
        "SELECT * FROM items WHERE code = 'THE_ONLY_VALUE'",
    );
    assert_eq!(result.as_array().unwrap().len(), 1);
}

/// Edge case: Rapid delete-reinsert cycle on same document.
/// Tests that lock release and reacquire work correctly.
#[test]
fn test_rapid_delete_reinsert_cycle() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION items");
    run(&storage, "CREATE INDEX ON items(code) UNIQUE");
    run(
        &storage,
        "INSERT INTO items {id: 'target', code: 'CYCLING'}",
    );

    let num_threads = 4;
    let cycles_per_thread = 20;
    let barrier = Arc::new(Barrier::new(num_threads));

    let handles: Vec<_> = (0..num_threads)
        .map(|thread_id| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);

            thread::spawn(move || {
                barrier.wait();
                for cycle in 0..cycles_per_thread {
                    // Try to delete
                    let _ = try_run(&storage, "DELETE items:target");
                    // Try to reinsert with same code
                    let _ = try_run(
                        &storage,
                        "INSERT INTO items {id: 'target', code: 'CYCLING'}",
                    );
                    // Try to reinsert with different id but same code (should fail if target exists)
                    let _ = try_run(
                        &storage,
                        &format!(
                            "INSERT INTO items {{id: 'alt_{}_{}', code: 'CYCLING'}}",
                            thread_id, cycle
                        ),
                    );
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    // Verify: at most one document with code CYCLING
    let result = run(&storage, "SELECT * FROM items WHERE code = 'CYCLING'");
    let count = result.as_array().unwrap().len();
    assert!(
        count <= 1,
        "Found {} documents with code CYCLING (expected 0 or 1)",
        count
    );
}
