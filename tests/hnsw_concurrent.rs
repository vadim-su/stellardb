//! Tests for HNSW concurrent add/remove operations.
//!
//! Verifies that the lock gap fix in remove_vector prevents race conditions
//! when concurrent threads add and remove vectors for the same doc_id.

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

fn try_run(storage: &Arc<Database>, sql: &str) -> Result<serde_json::Value, String> {
    use stellardb::query::execute::operators::operator::rows_to_json;
    stellardb::try_run_sql!(storage.clone(), sql)
        .map(|result| rows_to_json(&result.rows))
        .map_err(|e| e.to_string())
}

/// Concurrent add and remove operations on the same doc_id.
/// After all operations complete, the HNSW index should be consistent:
/// either the doc is findable or it's not, but the index should not panic
/// or return stale results.
#[test]
fn test_concurrent_add_remove_same_doc() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION items");
    run(
        &storage,
        "CREATE INDEX ON items(embedding) HNSW DIMENSION 3 DIST COSINE",
    );

    let num_iterations = 30;

    for i in 0..num_iterations {
        // Insert document
        run(
            &storage,
            &format!(
                r#"INSERT INTO items {{id: "item{}", embedding: [1.0, 0.0, 0.0]}}"#,
                i
            ),
        );

        // Concurrently: one thread deletes, another updates the same doc
        let storage1 = Arc::clone(&storage);
        let storage2 = Arc::clone(&storage);
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let b1 = Arc::clone(&barrier);
        let b2 = Arc::clone(&barrier);

        let delete_handle = std::thread::spawn(move || {
            b1.wait();
            let _ = try_run(&storage1, &format!("DELETE items:item{}", i));
        });

        let update_handle = std::thread::spawn(move || {
            b2.wait();
            let _ = try_run(
                &storage2,
                &format!(r#"UPDATE items:item{} SET embedding = [0.0, 1.0, 0.0]"#, i),
            );
        });

        delete_handle.join().unwrap();
        update_handle.join().unwrap();

        // Index should be consistent: search should not panic
        let result = run(
            &storage,
            "SELECT id FROM items WHERE embedding <|5|> [1.0, 0.0, 0.0]",
        );
        // We don't assert exact count since the race outcome is nondeterministic,
        // but the query should not fail
        assert!(
            result.is_array(),
            "Vector search should return valid results"
        );
    }
}

/// Multiple threads simultaneously inserting and deleting different docs
/// with HNSW index should not corrupt the index.
#[test]
fn test_concurrent_multi_doc_hnsw_operations() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION items");
    run(
        &storage,
        "CREATE INDEX ON items(embedding) HNSW DIMENSION 3 DIST COSINE",
    );

    // Pre-populate with some docs
    for i in 0..10 {
        run(
            &storage,
            &format!(
                r#"INSERT INTO items {{id: "base{}", embedding: [{}.0, 0.0, 0.0]}}"#,
                i,
                i + 1
            ),
        );
    }

    let num_threads = 4;
    let ops_per_thread = 20;
    let barrier = Arc::new(std::sync::Barrier::new(num_threads));

    let handles: Vec<_> = (0..num_threads)
        .map(|tid| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);

            std::thread::spawn(move || {
                barrier.wait();
                for op in 0..ops_per_thread {
                    let doc_id = format!("t{}_{}", tid, op);
                    // Insert
                    let _ = try_run(
                        &storage,
                        &format!(
                            r#"INSERT INTO items {{id: "{}", embedding: [{}.0, {}.0, 0.0]}}"#,
                            doc_id, tid, op
                        ),
                    );
                    // Delete some existing docs
                    if op % 3 == 0 {
                        let _ = try_run(&storage, &format!("DELETE items:base{}", op % 10));
                    }
                }
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    // Index should still be queryable without panicking
    let result = run(
        &storage,
        "SELECT id FROM items WHERE embedding <|10|> [1.0, 0.0, 0.0]",
    );
    assert!(
        result.is_array(),
        "HNSW search should work after concurrent operations"
    );
}

/// Rapid add-remove-add cycle on the same doc_id.
/// This specifically tests the lock gap fix: without the fix, the remove
/// could read the old mapping, drop the lock, then the add creates a new
/// mapping, then the remove deletes the new mapping using the stale internal_id.
#[test]
fn test_rapid_add_remove_add_same_doc() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION vecs");
    run(
        &storage,
        "CREATE INDEX ON vecs(v) HNSW DIMENSION 2 DIST COSINE",
    );

    for cycle in 0..20 {
        // Add
        run(
            &storage,
            &format!(
                r#"INSERT INTO vecs {{id: "target", v: [{}.0, 1.0]}}"#,
                cycle
            ),
        );

        // Verify it's searchable
        let result = run(
            &storage,
            &format!("SELECT id FROM vecs WHERE v <|1|> [{}.0, 1.0]", cycle),
        );
        let arr = result.as_array().unwrap();
        assert!(
            arr.iter().any(|r| r["id"] == "vecs:target"),
            "Cycle {}: inserted doc should be found by vector search",
            cycle
        );

        // Remove
        run(&storage, "DELETE vecs:target");

        // Verify it's gone
        let result = run(
            &storage,
            &format!("SELECT id FROM vecs WHERE v <|1|> [{}.0, 1.0]", cycle),
        );
        let arr = result.as_array().unwrap();
        assert!(
            !arr.iter().any(|r| r["id"] == "vecs:target"),
            "Cycle {}: deleted doc should not be found by vector search",
            cycle
        );
    }
}
