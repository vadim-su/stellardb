//! Concurrency tests for document CRUD operations.
//!
//! Tests concurrent inserts, updates, deletes, and mixed operations.

use crate::common::{run, setup, try_run};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

/// Test concurrent inserts with different IDs.
#[test]
fn test_concurrent_inserts_different_ids() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION items");

    let num_threads = 10;
    let inserts_per_thread = 100;
    let barrier = Arc::new(Barrier::new(num_threads));
    let success_count = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..num_threads)
        .map(|thread_id| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            let success_count = Arc::clone(&success_count);

            thread::spawn(move || {
                barrier.wait();

                for i in 0..inserts_per_thread {
                    let doc_id = format!("t{}_i{}", thread_id, i);
                    let result = try_run(
                        &storage,
                        &format!(
                            "INSERT INTO items {{id: '{}', thread: {}, index: {}}}",
                            doc_id, thread_id, i
                        ),
                    );

                    if result.is_ok() {
                        success_count.fetch_add(1, Ordering::SeqCst);
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    let total = success_count.load(Ordering::SeqCst);
    let expected = num_threads * inserts_per_thread;
    assert_eq!(total, expected, "Expected {} inserts", expected);

    let result = run(&storage, "SELECT * FROM items");
    assert_eq!(result.as_array().unwrap().len(), expected);
}

/// Test concurrent inserts with the same ID.
/// Uses lock striping to ensure atomic primary key uniqueness check.
#[test]
fn test_concurrent_inserts_same_id() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION items");

    let num_threads = 10;
    let barrier = Arc::new(Barrier::new(num_threads));
    let success_count = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..num_threads)
        .map(|thread_id| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            let success_count = Arc::clone(&success_count);

            thread::spawn(move || {
                barrier.wait();

                let result = try_run(
                    &storage,
                    &format!(
                        "INSERT INTO items {{id: 'contested', winner: {}}}",
                        thread_id
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

    let total = success_count.load(Ordering::SeqCst);
    assert_eq!(total, 1, "Exactly one insert should succeed, got {}", total);

    let result = run(&storage, "SELECT * FROM items");
    assert_eq!(result.as_array().unwrap().len(), 1);
}

/// Test concurrent updates to the same document.
#[test]
fn test_concurrent_updates_same_document() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION counters");
    run(&storage, "INSERT INTO counters {id: 'counter', value: 0}");

    let num_threads = 10;
    let updates_per_thread = 50;
    let barrier = Arc::new(Barrier::new(num_threads));
    let success_count = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..num_threads)
        .map(|thread_id| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            let success_count = Arc::clone(&success_count);

            thread::spawn(move || {
                barrier.wait();

                for i in 0..updates_per_thread {
                    let result = try_run(
                        &storage,
                        &format!(
                            "UPDATE counters:counter SET value = {}, last_thread = {}",
                            thread_id * 1000 + i,
                            thread_id
                        ),
                    );

                    if result.is_ok() {
                        success_count.fetch_add(1, Ordering::SeqCst);
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    let total = success_count.load(Ordering::SeqCst);
    let expected = num_threads * updates_per_thread;
    assert_eq!(total, expected, "All updates should succeed");

    let result = run(&storage, "SELECT * FROM counters:counter");
    assert_eq!(result.as_array().unwrap().len(), 1);
}

/// Stress test with mixed operations.
#[test]
fn test_mixed_operations_stress() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION stress");

    for i in 0..50 {
        run(
            &storage,
            &format!("INSERT INTO stress {{id: 'doc{}', counter: 0}}", i),
        );
    }

    let num_threads = 8;
    let ops_per_thread = 100;
    let barrier = Arc::new(Barrier::new(num_threads));

    let handles: Vec<_> = (0..num_threads)
        .map(|thread_id| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);

            thread::spawn(move || {
                barrier.wait();

                for op in 0..ops_per_thread {
                    let doc_id = op % 50;

                    match (thread_id + op) % 4 {
                        0 => {
                            let _ =
                                try_run(&storage, &format!("SELECT * FROM stress:doc{}", doc_id));
                        }
                        1 => {
                            let _ = try_run(
                                &storage,
                                &format!("UPDATE stress:doc{} SET counter = counter + 1", doc_id),
                            );
                        }
                        2 => {
                            let _ = try_run(
                                &storage,
                                &format!(
                                    "INSERT INTO stress {{id: 'new_t{}_o{}', value: {}}}",
                                    thread_id, op, op
                                ),
                            );
                        }
                        3 => {
                            let _ = try_run(&storage, &format!("DELETE stress:doc{}", doc_id));
                            let _ = try_run(
                                &storage,
                                &format!("INSERT INTO stress {{id: 'doc{}', counter: 0}}", doc_id),
                            );
                        }
                        _ => unreachable!(),
                    }

                    if op % 10 == 0 {
                        thread::sleep(Duration::from_micros(10));
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    let result = run(&storage, "SELECT * FROM stress");
    let docs = result.as_array().unwrap();

    assert!(docs.len() >= 50, "Should have at least original documents");

    for doc in docs {
        assert!(doc.get("id").is_some(), "Document missing id");
    }
}

/// Stress test: rapidly modify different fields with different data types on a single document.
///
/// Each thread updates a different field using various data types:
/// null, bool, int, float, string, array, object.
#[test]
fn test_concurrent_different_types_same_document() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION typed");
    run(
        &storage,
        "INSERT INTO typed {
            id: 'target',
            int_field: 0,
            float_field: 0.0,
            bool_field: false,
            string_field: '',
            array_field: [],
            object_field: {},
            nullable_field: null
        }",
    );

    let num_threads = 12;
    let updates_per_thread = 200;
    let barrier = Arc::new(Barrier::new(num_threads));
    let success_count = Arc::new(AtomicUsize::new(0));
    let error_count = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..num_threads)
        .map(|thread_id| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            let success_count = Arc::clone(&success_count);
            let error_count = Arc::clone(&error_count);

            thread::spawn(move || {
                barrier.wait();

                for i in 0..updates_per_thread {
                    // Each thread cycles through different field types
                    let sql = match (thread_id + i) % 7 {
                        0 => format!(
                            "UPDATE typed:target SET int_field = {}",
                            thread_id as i64 * 10000 + i as i64
                        ),
                        1 => format!(
                            "UPDATE typed:target SET float_field = {}.{}",
                            thread_id, i
                        ),
                        2 => format!(
                            "UPDATE typed:target SET bool_field = {}",
                            i % 2 == 0
                        ),
                        3 => format!(
                            "UPDATE typed:target SET string_field = 'thread_{}_iter_{}'",
                            thread_id, i
                        ),
                        4 => format!(
                            "UPDATE typed:target SET array_field = [{}, {}, '{}']",
                            thread_id, i, thread_id
                        ),
                        5 => format!(
                            "UPDATE typed:target SET object_field = {{thread: {}, iter: {}, nested: {{a: {}}}}}",
                            thread_id, i, i * 2
                        ),
                        6 => {
                            if i % 2 == 0 {
                                "UPDATE typed:target SET nullable_field = null".to_string()
                            } else {
                                format!("UPDATE typed:target SET nullable_field = {}", i)
                            }
                        }
                        _ => unreachable!(),
                    };

                    match try_run(&storage, &sql) {
                        Ok(_) => {
                            success_count.fetch_add(1, Ordering::SeqCst);
                        }
                        Err(_) => {
                            error_count.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    let total_success = success_count.load(Ordering::SeqCst);
    let total_errors = error_count.load(Ordering::SeqCst);
    let expected = num_threads * updates_per_thread;

    assert_eq!(
        total_success, expected,
        "All updates should succeed. Got {} successes, {} errors out of {}",
        total_success, total_errors, expected
    );

    // Verify document is still valid and readable
    let result = run(&storage, "SELECT * FROM typed:target");
    let docs = result.as_array().unwrap();
    assert_eq!(docs.len(), 1, "Document should exist");

    let doc = &docs[0];
    assert!(doc.get("int_field").is_some(), "int_field should exist");
    assert!(doc.get("float_field").is_some(), "float_field should exist");
    assert!(doc.get("bool_field").is_some(), "bool_field should exist");
    assert!(
        doc.get("string_field").is_some(),
        "string_field should exist"
    );
    assert!(doc.get("array_field").is_some(), "array_field should exist");
    assert!(
        doc.get("object_field").is_some(),
        "object_field should exist"
    );
    // nullable_field may be null or int, both valid
}

/// Stress test: multiple fields updated simultaneously by different threads.
///
/// All threads update ALL fields of the same document concurrently,
/// maximizing contention.
#[test]
fn test_concurrent_all_fields_all_threads() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION chaos");
    run(
        &storage,
        "INSERT INTO chaos {
            id: 'target',
            a: 0, b: 0, c: 0, d: 0, e: 0, f: 0, g: 0, h: 0
        }",
    );

    let num_threads = 16;
    let updates_per_thread = 100;
    let barrier = Arc::new(Barrier::new(num_threads));
    let success_count = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..num_threads)
        .map(|thread_id| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            let success_count = Arc::clone(&success_count);

            thread::spawn(move || {
                barrier.wait();

                for i in 0..updates_per_thread {
                    // Update all fields at once with different types
                    let g_value = if i % 3 == 0 {
                        "null".to_string()
                    } else {
                        i.to_string()
                    };
                    let sql = format!(
                        "UPDATE chaos:target SET \
                            a = {}, \
                            b = {}.{}, \
                            c = {}, \
                            d = 'str_{}', \
                            e = [{}, {}], \
                            f = {{x: {}}}, \
                            g = {}, \
                            h = {}",
                        thread_id * 1000 + i, // int
                        thread_id,
                        i,          // float
                        i % 2 == 0, // bool
                        thread_id,  // string
                        thread_id,
                        i,                                    // array
                        i,                                    // object
                        g_value,                              // nullable int
                        -(thread_id as i64 * 100 + i as i64), // negative int
                    );

                    if try_run(&storage, &sql).is_ok() {
                        success_count.fetch_add(1, Ordering::SeqCst);
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    let total = success_count.load(Ordering::SeqCst);
    let expected = num_threads * updates_per_thread;
    assert_eq!(total, expected, "All updates should succeed");

    // Verify final state is consistent
    let result = run(&storage, "SELECT * FROM chaos:target");
    let docs = result.as_array().unwrap();
    assert_eq!(docs.len(), 1);

    let doc = &docs[0];
    // All fields should exist with some value
    for field in ["a", "b", "c", "d", "e", "f", "g", "h"] {
        assert!(doc.get(field).is_some(), "Field {} should exist", field);
    }
}

/// Rapid-fire updates: extremely high frequency updates to stress locking.
#[test]
fn test_rapid_fire_single_field() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION rapid");
    run(&storage, "INSERT INTO rapid {id: 'counter', value: 0}");

    let num_threads = 20;
    let updates_per_thread = 500;
    let barrier = Arc::new(Barrier::new(num_threads));
    let success_count = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..num_threads)
        .map(|thread_id| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            let success_count = Arc::clone(&success_count);

            thread::spawn(move || {
                barrier.wait();

                // No sleeps - maximum contention
                for i in 0..updates_per_thread {
                    let value = thread_id * updates_per_thread + i;
                    let sql = format!("UPDATE rapid:counter SET value = {}", value);

                    if try_run(&storage, &sql).is_ok() {
                        success_count.fetch_add(1, Ordering::SeqCst);
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    let total = success_count.load(Ordering::SeqCst);
    let expected = num_threads * updates_per_thread;
    assert_eq!(total, expected, "All {} updates should succeed", expected);

    // Final value should be from one of the threads
    let result = run(&storage, "SELECT value FROM rapid:counter");
    let docs = result.as_array().unwrap();
    assert_eq!(docs.len(), 1);

    let value = docs[0].get("value").unwrap().as_i64().unwrap();
    assert!(value >= 0, "Value should be non-negative");
}
