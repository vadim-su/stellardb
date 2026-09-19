//! Concurrency tests for index lifecycle operations.
//!
//! Tests CREATE INDEX and DROP INDEX behavior during concurrent operations.

use crate::common::{run, setup, try_run};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::Duration;

/// Test CREATE INDEX while concurrent inserts are happening.
#[test]
fn test_create_index_during_inserts() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION users");

    for i in 0..50 {
        run(
            &storage,
            &format!(
                "INSERT INTO users {{id: 'pre{}', email: 'pre{}@test.com'}}",
                i, i
            ),
        );
    }

    let num_inserters = 4;
    let inserts_per_thread = 25;
    let barrier = Arc::new(Barrier::new(num_inserters + 1));
    let index_created = Arc::new(AtomicBool::new(false));

    let insert_handles: Vec<_> = (0..num_inserters)
        .map(|thread_id| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            let index_created = Arc::clone(&index_created);

            thread::spawn(move || {
                barrier.wait();

                for i in 0..inserts_per_thread {
                    let doc_id = thread_id * inserts_per_thread + i;
                    let result = try_run(
                        &storage,
                        &format!(
                            "INSERT INTO users {{id: 'post{}', email: 'post{}@test.com'}}",
                            doc_id, doc_id
                        ),
                    );

                    if result.is_err() && index_created.load(Ordering::SeqCst) {
                        eprintln!("Insert failed after index created: {:?}", result.err());
                    }

                    if i % 5 == 0 {
                        thread::sleep(Duration::from_micros(100));
                    }
                }
            })
        })
        .collect();

    let storage_clone = Arc::clone(&storage);
    let barrier_clone = Arc::clone(&barrier);
    let index_created_clone = Arc::clone(&index_created);

    let index_handle = thread::spawn(move || {
        barrier_clone.wait();
        thread::sleep(Duration::from_millis(1));

        let result = try_run(&storage_clone, "CREATE INDEX ON users(email)");
        index_created_clone.store(true, Ordering::SeqCst);
        result
    });

    for handle in insert_handles {
        handle.join().unwrap();
    }
    let index_result = index_handle.join().unwrap();

    assert!(
        index_result.is_ok(),
        "Index creation failed: {:?}",
        index_result
    );

    let result = run(&storage, "SELECT * FROM users");
    let total_docs = result.as_array().unwrap().len();
    let expected = 50 + (num_inserters * inserts_per_thread);

    assert_eq!(total_docs, expected, "Expected {} documents", expected);
}

/// Test DROP INDEX while concurrent queries are using it.
/// Verifies that queries see consistent results even when an index is dropped mid-flight.
#[test]
fn test_drop_index_during_queries() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION users");
    run(&storage, "CREATE INDEX ON users(status)");

    for i in 0..100 {
        let status = if i % 2 == 0 { "active" } else { "inactive" };
        run(
            &storage,
            &format!(
                "INSERT INTO users {{id: 'user{}', status: '{}'}}",
                i, status
            ),
        );
    }

    // Verify all inserts succeeded BEFORE starting concurrent reads
    let verify = run(&storage, "SELECT * FROM users WHERE status = 'active'");
    let verify_count = verify.as_array().unwrap().len();
    assert_eq!(
        verify_count, 50,
        "Pre-test verification: expected 50 active users, got {}",
        verify_count
    );

    let num_readers = 4;
    let reads_per_thread = 50;
    let barrier = Arc::new(Barrier::new(num_readers + 1));
    let successful_reads = Arc::new(AtomicUsize::new(0));

    let reader_handles: Vec<_> = (0..num_readers)
        .map(|_| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            let successful_reads = Arc::clone(&successful_reads);

            thread::spawn(move || {
                barrier.wait();

                for _ in 0..reads_per_thread {
                    let result = try_run(&storage, "SELECT * FROM users WHERE status = 'active'");

                    if let Ok(data) = result {
                        let count = data.as_array().unwrap().len();
                        assert_eq!(count, 50, "Query returned wrong count: {}", count);
                        successful_reads.fetch_add(1, Ordering::SeqCst);
                    }

                    thread::sleep(Duration::from_micros(50));
                }
            })
        })
        .collect();

    let storage_clone = Arc::clone(&storage);
    let barrier_clone = Arc::clone(&barrier);

    let drop_handle = thread::spawn(move || {
        barrier_clone.wait();
        thread::sleep(Duration::from_millis(5));
        try_run(&storage_clone, "DROP INDEX idx_users_btree_status ON users")
    });

    for handle in reader_handles {
        handle.join().unwrap();
    }
    let drop_result = drop_handle.join().unwrap();

    assert!(drop_result.is_ok(), "DROP INDEX failed: {:?}", drop_result);

    let total_reads = successful_reads.load(Ordering::SeqCst);
    assert_eq!(
        total_reads,
        num_readers * reads_per_thread,
        "Some reads failed"
    );
}

/// Test creating multiple indexes concurrently.
/// Verifies that all indexes are created correctly without race conditions.
#[test]
fn test_concurrent_index_creation() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION users");

    for i in 0..50 {
        run(
            &storage,
            &format!(
                "INSERT INTO users {{id: 'user{}', email: 'u{}@test.com', name: 'User{}', age: {}}}",
                i,
                i,
                i,
                20 + i
            ),
        );
    }

    let barrier = Arc::new(Barrier::new(3));
    let results = Arc::new(Mutex::new(Vec::new()));

    let fields = vec!["email", "name", "age"];

    let handles: Vec<_> = fields
        .into_iter()
        .map(|field| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            let results = Arc::clone(&results);

            thread::spawn(move || {
                barrier.wait();

                let result = try_run(&storage, &format!("CREATE INDEX ON users({})", field));

                results.lock().unwrap().push((field, result));
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    let results = results.lock().unwrap();
    for (field, result) in results.iter() {
        assert!(result.is_ok(), "Index on {} failed: {:?}", field, result);
    }

    let result = run(&storage, "DESCRIBE COLLECTION users");
    let schema = &result.as_array().unwrap()[0];
    let indexes = schema["indexes"].as_array().unwrap();
    assert_eq!(indexes.len(), 3, "Should have 3 indexes");
}
