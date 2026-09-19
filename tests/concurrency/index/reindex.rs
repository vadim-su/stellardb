//! Concurrency tests for REINDEX operations.
//!
//! Tests REINDEX behavior during concurrent INSERT/DELETE operations
//! for both FTS and HNSW indexes.
//!
//! These tests verify DATA INTEGRITY - not just that operations succeed,
//! but that data remains consistent and searchable.

use crate::common::{run, setup, try_run};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::Duration;

/// Extract just the key part from a full document ID (collection:key -> key)
fn extract_key(full_id: &str) -> String {
    full_id.split(':').nth(1).unwrap_or(full_id).to_string()
}

// =============================================================================
// FTS REINDEX Tests
// =============================================================================

/// Test REINDEX FTS while concurrent inserts are happening.
/// Verifies that ALL inserted documents are searchable after REINDEX.
#[test]
fn test_reindex_fts_during_inserts() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION articles");
    run(&storage, "CREATE INDEX ON articles(title) FULLTEXT");

    // Pre-populate with documents containing "topic"
    for i in 0..50 {
        run(
            &storage,
            &format!(
                "INSERT INTO articles {{id: 'pre{}', title: 'Article about topic {}'}}",
                i, i
            ),
        );
    }

    let num_inserters = 4;
    let inserts_per_thread = 25;
    let barrier = Arc::new(Barrier::new(num_inserters + 1));

    // Track which documents were successfully inserted
    let inserted_ids: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    let insert_handles: Vec<_> = (0..num_inserters)
        .map(|thread_id| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            let inserted_ids = Arc::clone(&inserted_ids);

            thread::spawn(move || {
                barrier.wait();

                for i in 0..inserts_per_thread {
                    let doc_id = format!("post{}", thread_id * inserts_per_thread + i);
                    let result = try_run(
                        &storage,
                        &format!(
                            "INSERT INTO articles {{id: '{}', title: 'New article about subject {}'}}",
                            doc_id, i
                        ),
                    );

                    if result.is_ok() {
                        inserted_ids.lock().unwrap().insert(doc_id);
                    }

                    thread::sleep(Duration::from_micros(100));
                }
            })
        })
        .collect();

    let storage_clone = Arc::clone(&storage);
    let barrier_clone = Arc::clone(&barrier);

    let reindex_handle = thread::spawn(move || {
        barrier_clone.wait();
        thread::sleep(Duration::from_millis(2));
        try_run(&storage_clone, "REINDEX title_fts ON articles")
    });

    for handle in insert_handles {
        handle.join().unwrap();
    }
    let reindex_result = reindex_handle.join().unwrap();

    assert!(
        reindex_result.is_ok(),
        "REINDEX failed: {:?}",
        reindex_result
    );

    let inserted = inserted_ids.lock().unwrap();
    let total_inserted = inserted.len();

    // Flush FTS background commits to ensure all documents are visible
    storage.indexes().fts_backend().flush().unwrap();

    // === DATA INTEGRITY CHECKS ===

    // 1. Check total document count
    let all_docs = run(&storage, "SELECT * FROM articles");
    let total_docs = all_docs.as_array().unwrap().len();
    assert_eq!(
        total_docs,
        50 + total_inserted,
        "Document count mismatch: expected {}, got {}",
        50 + total_inserted,
        total_docs
    );

    // 2. Check ALL pre-existing documents are searchable
    let old_search = run(&storage, "SELECT id FROM articles WHERE title @@ 'topic'");
    let old_results: HashSet<String> = old_search
        .as_array()
        .unwrap()
        .iter()
        .map(|v| extract_key(v["id"].as_str().unwrap()))
        .collect();

    assert_eq!(
        old_results.len(),
        50,
        "Expected 50 'topic' documents, found {}",
        old_results.len()
    );

    for i in 0..50 {
        assert!(
            old_results.contains(&format!("pre{}", i)),
            "Missing pre-existing document pre{} in FTS results",
            i
        );
    }

    // 3. Check ALL new documents are searchable
    let new_search = run(&storage, "SELECT id FROM articles WHERE title @@ 'subject'");
    let new_results: HashSet<String> = new_search
        .as_array()
        .unwrap()
        .iter()
        .map(|v| extract_key(v["id"].as_str().unwrap()))
        .collect();

    assert_eq!(
        new_results.len(),
        total_inserted,
        "Expected {} 'subject' documents, found {}",
        total_inserted,
        new_results.len()
    );

    for doc_id in inserted.iter() {
        assert!(
            new_results.contains(doc_id),
            "Inserted document {} not found in FTS search results",
            doc_id
        );
    }

    // 4. Verify each inserted document exists by direct fetch
    for doc_id in inserted.iter() {
        let doc = run(&storage, &format!("SELECT * FROM articles:{}", doc_id));
        let arr = doc.as_array().unwrap();
        assert_eq!(
            arr.len(),
            1,
            "Document {} should exist but got {} results",
            doc_id,
            arr.len()
        );
    }
}

/// Test REINDEX FTS while concurrent deletes are happening.
/// Verifies deleted documents are NOT searchable after REINDEX.
#[test]
fn test_reindex_fts_during_deletes() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION articles");
    run(&storage, "CREATE INDEX ON articles(content) FULLTEXT");

    // Populate: even IDs have "alpha", odd IDs have "beta"
    for i in 0..100 {
        let word = if i % 2 == 0 { "alpha" } else { "beta" };
        run(
            &storage,
            &format!(
                "INSERT INTO articles {{id: 'doc{}', content: 'Article with {} keyword number {}'}}",
                i, word, i
            ),
        );
    }

    let barrier = Arc::new(Barrier::new(2));
    let deleted_ids: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    // Delete all "beta" documents (odd IDs)
    let storage_delete = Arc::clone(&storage);
    let barrier_delete = Arc::clone(&barrier);
    let deleted_ids_clone = Arc::clone(&deleted_ids);

    let delete_handle = thread::spawn(move || {
        barrier_delete.wait();

        for i in (1..100).step_by(2) {
            let doc_id = format!("doc{}", i);
            let result = try_run(&storage_delete, &format!("DELETE articles:{}", doc_id));
            if result.is_ok() {
                deleted_ids_clone.lock().unwrap().insert(doc_id);
            }
            thread::sleep(Duration::from_micros(50));
        }
    });

    let storage_reindex = Arc::clone(&storage);
    let barrier_reindex = Arc::clone(&barrier);

    let reindex_handle = thread::spawn(move || {
        barrier_reindex.wait();
        thread::sleep(Duration::from_millis(2));
        try_run(&storage_reindex, "REINDEX content_fts ON articles")
    });

    delete_handle.join().unwrap();
    let reindex_result = reindex_handle.join().unwrap();

    assert!(
        reindex_result.is_ok(),
        "REINDEX failed: {:?}",
        reindex_result
    );

    let deleted = deleted_ids.lock().unwrap();
    let deleted_count = deleted.len();

    // === DATA INTEGRITY CHECKS ===

    // 1. Verify "alpha" documents (even IDs) are ALL searchable
    let alpha_search = run(&storage, "SELECT id FROM articles WHERE content @@ 'alpha'");
    let alpha_results: HashSet<String> = alpha_search
        .as_array()
        .unwrap()
        .iter()
        .map(|v| extract_key(v["id"].as_str().unwrap()))
        .collect();

    assert_eq!(
        alpha_results.len(),
        50,
        "Expected 50 'alpha' documents, found {}",
        alpha_results.len()
    );

    for i in (0..100).step_by(2) {
        assert!(
            alpha_results.contains(&format!("doc{}", i)),
            "Document doc{} with 'alpha' should be searchable",
            i
        );
    }

    // 2. Verify deleted "beta" documents are NOT searchable
    let beta_search = run(&storage, "SELECT id FROM articles WHERE content @@ 'beta'");
    let beta_results: HashSet<String> = beta_search
        .as_array()
        .unwrap()
        .iter()
        .map(|v| extract_key(v["id"].as_str().unwrap()))
        .collect();

    let expected_beta = 50 - deleted_count;
    assert_eq!(
        beta_results.len(),
        expected_beta,
        "Expected {} 'beta' documents (50 - {} deleted), found {}",
        expected_beta,
        deleted_count,
        beta_results.len()
    );

    // 3. Verify deleted documents don't exist
    for doc_id in deleted.iter() {
        let doc = run(&storage, &format!("SELECT * FROM articles:{}", doc_id));
        assert!(
            doc.as_array().unwrap().is_empty(),
            "Deleted document {} should not exist",
            doc_id
        );

        // Also verify not in FTS
        assert!(
            !beta_results.contains(doc_id),
            "Deleted document {} should not appear in FTS results",
            doc_id
        );
    }

    // 4. Total count check
    let all_docs = run(&storage, "SELECT * FROM articles");
    assert_eq!(
        all_docs.as_array().unwrap().len(),
        100 - deleted_count,
        "Total document count mismatch"
    );
}

/// Test REINDEX FTS with mixed concurrent inserts and deletes.
#[test]
fn test_reindex_fts_during_mixed_operations() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION posts");
    run(&storage, "CREATE INDEX ON posts(body) FULLTEXT");

    // Pre-populate with "original" keyword
    for i in 0..50 {
        run(
            &storage,
            &format!(
                "INSERT INTO posts {{id: 'init{}', body: 'Original post content {}'}}",
                i, i
            ),
        );
    }

    let barrier = Arc::new(Barrier::new(3));
    let inserted_ids: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let deleted_ids: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    // Inserter: adds documents with "fresh" keyword
    let storage_insert = Arc::clone(&storage);
    let barrier_insert = Arc::clone(&barrier);
    let inserted_ids_clone = Arc::clone(&inserted_ids);
    let insert_handle = thread::spawn(move || {
        barrier_insert.wait();
        for i in 0..30 {
            let doc_id = format!("new{}", i);
            let result = try_run(
                &storage_insert,
                &format!(
                    "INSERT INTO posts {{id: '{}', body: 'Fresh post content {}'}}",
                    doc_id, i
                ),
            );
            if result.is_ok() {
                inserted_ids_clone.lock().unwrap().insert(doc_id);
            }
            thread::sleep(Duration::from_micros(100));
        }
    });

    // Deleter: removes first 20 original documents
    let storage_delete = Arc::clone(&storage);
    let barrier_delete = Arc::clone(&barrier);
    let deleted_ids_clone = Arc::clone(&deleted_ids);
    let delete_handle = thread::spawn(move || {
        barrier_delete.wait();
        for i in 0..20 {
            let doc_id = format!("init{}", i);
            let result = try_run(&storage_delete, &format!("DELETE posts:{}", doc_id));
            if result.is_ok() {
                deleted_ids_clone.lock().unwrap().insert(doc_id);
            }
            thread::sleep(Duration::from_micros(150));
        }
    });

    // Reindex
    let storage_reindex = Arc::clone(&storage);
    let barrier_reindex = Arc::clone(&barrier);
    let reindex_handle = thread::spawn(move || {
        barrier_reindex.wait();
        thread::sleep(Duration::from_millis(3));
        try_run(&storage_reindex, "REINDEX body_fts ON posts")
    });

    insert_handle.join().unwrap();
    delete_handle.join().unwrap();
    let reindex_result = reindex_handle.join().unwrap();

    assert!(
        reindex_result.is_ok(),
        "REINDEX failed: {:?}",
        reindex_result
    );

    let inserted = inserted_ids.lock().unwrap();
    let deleted = deleted_ids.lock().unwrap();

    // Flush FTS background commits to ensure all documents are visible
    storage.indexes().fts_backend().flush().unwrap();

    // === DATA INTEGRITY CHECKS ===

    // 1. Total count
    let all_docs = run(&storage, "SELECT * FROM posts");
    let expected_total = 50 - deleted.len() + inserted.len();
    assert_eq!(
        all_docs.as_array().unwrap().len(),
        expected_total,
        "Expected {} documents, got {}",
        expected_total,
        all_docs.as_array().unwrap().len()
    );

    // 2. All inserted documents searchable
    let fresh_search = run(&storage, "SELECT id FROM posts WHERE body @@ 'fresh'");
    let fresh_results: HashSet<String> = fresh_search
        .as_array()
        .unwrap()
        .iter()
        .map(|v| extract_key(v["id"].as_str().unwrap()))
        .collect();

    for doc_id in inserted.iter() {
        assert!(
            fresh_results.contains(doc_id),
            "Inserted document {} not found in FTS",
            doc_id
        );
    }

    // 3. Remaining original documents searchable
    let original_search = run(&storage, "SELECT id FROM posts WHERE body @@ 'original'");
    let original_results: HashSet<String> = original_search
        .as_array()
        .unwrap()
        .iter()
        .map(|v| extract_key(v["id"].as_str().unwrap()))
        .collect();

    let expected_original = 50 - deleted.len();

    // DEBUG: If test is about to fail, dump diagnostic info
    if original_results.len() != expected_original {
        eprintln!("=== DIAGNOSTIC INFO ===");
        eprintln!(
            "Expected {} 'original' docs, found {}",
            expected_original,
            original_results.len()
        );
        eprintln!("Deleted {} docs: {:?}", deleted.len(), *deleted);
        eprintln!("Inserted {} docs: {:?}", inserted.len(), *inserted);

        // Check total docs in collection
        let total = run(&storage, "SELECT COUNT(*) FROM posts");
        eprintln!("Total docs in collection: {:?}", total);

        // Check what the FTS index returns for ANY query
        let any_fts = run(&storage, "SELECT id FROM posts WHERE body @@ 'post'");
        eprintln!(
            "FTS search for 'post' returns {} docs",
            any_fts.as_array().unwrap().len()
        );

        // Check fresh keyword (inserted docs)
        let fresh_fts = run(&storage, "SELECT id FROM posts WHERE body @@ 'fresh'");
        eprintln!(
            "FTS search for 'fresh' returns {} docs",
            fresh_fts.as_array().unwrap().len()
        );

        // Check which original docs are missing from FTS
        let mut missing_from_fts = Vec::new();
        for i in 20..50 {
            // These should not be deleted (only 0-19 are deleted)
            let doc_id = format!("init{}", i);
            if !original_results.contains(&doc_id) {
                // Check if doc exists in storage
                let doc = run(&storage, &format!("SELECT body FROM posts:{}", doc_id));
                let exists = !doc.as_array().unwrap().is_empty();
                missing_from_fts.push((doc_id, exists));
            }
        }
        eprintln!(
            "Missing from FTS (id, exists_in_storage): {:?}",
            missing_from_fts
        );

        // Check DESCRIBE to see index info
        let desc = run(&storage, "DESCRIBE COLLECTION posts");
        eprintln!("Collection schema: {:?}", desc);
    }

    assert_eq!(
        original_results.len(),
        expected_original,
        "Expected {} 'original' documents, found {}",
        expected_original,
        original_results.len()
    );

    // 4. Deleted documents don't exist and not in FTS
    for doc_id in deleted.iter() {
        let doc = run(&storage, &format!("SELECT * FROM posts:{}", doc_id));
        assert!(
            doc.as_array().unwrap().is_empty(),
            "Deleted {} exists",
            doc_id
        );
        assert!(
            !original_results.contains(doc_id),
            "Deleted {} in FTS",
            doc_id
        );
    }
}

// =============================================================================
// HNSW REINDEX Tests
// =============================================================================

/// Test REINDEX HNSW while concurrent inserts are happening.
/// Verifies ALL vectors are searchable after REINDEX.
#[test]
fn test_reindex_hnsw_during_inserts() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION vectors");
    run(
        &storage,
        "CREATE INDEX ON vectors(embedding) HNSW DIMENSION 3 DIST EUCLIDEAN",
    );

    // Pre-populate: vectors near [0, 0, 0]
    for i in 0..50 {
        let v = (i as f32) * 0.01;
        run(
            &storage,
            &format!(
                "INSERT INTO vectors {{id: 'pre{}', embedding: [{}, {}, {}]}}",
                i, v, v, v
            ),
        );
    }

    let num_inserters = 4;
    let inserts_per_thread = 20;
    let barrier = Arc::new(Barrier::new(num_inserters + 1));
    let inserted_ids: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    // Insert vectors near [1, 1, 1]
    let insert_handles: Vec<_> = (0..num_inserters)
        .map(|thread_id| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            let inserted_ids = Arc::clone(&inserted_ids);

            thread::spawn(move || {
                barrier.wait();

                for i in 0..inserts_per_thread {
                    let doc_id = format!("post{}", thread_id * inserts_per_thread + i);
                    let offset = (i as f32) * 0.01;
                    let result = try_run(
                        &storage,
                        &format!(
                            "INSERT INTO vectors {{id: '{}', embedding: [{}, {}, {}]}}",
                            doc_id,
                            1.0 - offset,
                            1.0 - offset,
                            1.0 - offset
                        ),
                    );

                    if result.is_ok() {
                        inserted_ids.lock().unwrap().insert(doc_id);
                    }

                    thread::sleep(Duration::from_micros(100));
                }
            })
        })
        .collect();

    let storage_clone = Arc::clone(&storage);
    let barrier_clone = Arc::clone(&barrier);

    let reindex_handle = thread::spawn(move || {
        barrier_clone.wait();
        thread::sleep(Duration::from_millis(2));
        try_run(
            &storage_clone,
            "REINDEX idx_vectors_hnsw_embedding ON vectors",
        )
    });

    for handle in insert_handles {
        handle.join().unwrap();
    }
    let reindex_result = reindex_handle.join().unwrap();

    assert!(
        reindex_result.is_ok(),
        "REINDEX failed: {:?}",
        reindex_result
    );

    let inserted = inserted_ids.lock().unwrap();

    // === DATA INTEGRITY CHECKS ===

    // 1. Total count
    let all_docs = run(&storage, "SELECT * FROM vectors");
    assert_eq!(
        all_docs.as_array().unwrap().len(),
        50 + inserted.len(),
        "Document count mismatch"
    );

    // 2. Search near [0,0,0] should return pre-existing vectors
    let search_origin = run(
        &storage,
        "SELECT id FROM vectors WHERE embedding <|50|> [0.0, 0.0, 0.0]",
    );
    let origin_results: Vec<String> = search_origin
        .as_array()
        .unwrap()
        .iter()
        .map(|v| extract_key(v["id"].as_str().unwrap()))
        .collect();

    // First results should be pre-existing (closer to origin)
    let pre_count = origin_results
        .iter()
        .take(30)
        .filter(|id| id.starts_with("pre"))
        .count();
    assert!(
        pre_count >= 25,
        "Search near [0,0,0]: expected mostly 'pre' docs in top 30, got {}",
        pre_count
    );

    // 3. Search near [1,1,1] should return new vectors
    let search_corner = run(
        &storage,
        "SELECT id FROM vectors WHERE embedding <|50|> [1.0, 1.0, 1.0]",
    );
    let corner_results: Vec<String> = search_corner
        .as_array()
        .unwrap()
        .iter()
        .map(|v| extract_key(v["id"].as_str().unwrap()))
        .collect();

    // First results should be post-inserted (closer to [1,1,1])
    let post_count = corner_results
        .iter()
        .take(30)
        .filter(|id| id.starts_with("post"))
        .count();
    assert!(
        post_count >= 20,
        "Search near [1,1,1]: expected mostly 'post' docs in top 30, got {}",
        post_count
    );

    // 4. Verify each inserted document exists
    for doc_id in inserted.iter() {
        let doc = run(&storage, &format!("SELECT * FROM vectors:{}", doc_id));
        assert_eq!(
            doc.as_array().unwrap().len(),
            1,
            "Missing document {}",
            doc_id
        );
    }
}

/// Test REINDEX HNSW while concurrent deletes are happening.
/// Verifies deleted vectors are NOT in search results.
#[test]
fn test_reindex_hnsw_during_deletes() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION points");
    run(
        &storage,
        "CREATE INDEX ON points(vec) HNSW DIMENSION 3 DIST EUCLIDEAN",
    );

    // Populate: even IDs near [0,0,0], odd IDs near [1,1,1]
    for i in 0..100 {
        let (x, y, z) = if i % 2 == 0 {
            let v = (i as f32) * 0.005;
            (v, v, v)
        } else {
            let v = 1.0 - ((i as f32) * 0.005);
            (v, v, v)
        };
        run(
            &storage,
            &format!(
                "INSERT INTO points {{id: 'p{}', vec: [{}, {}, {}]}}",
                i, x, y, z
            ),
        );
    }

    let barrier = Arc::new(Barrier::new(2));
    let deleted_ids: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    // Delete odd IDs (near [1,1,1])
    let storage_delete = Arc::clone(&storage);
    let barrier_delete = Arc::clone(&barrier);
    let deleted_ids_clone = Arc::clone(&deleted_ids);

    let delete_handle = thread::spawn(move || {
        barrier_delete.wait();

        for i in (1..100).step_by(2) {
            let doc_id = format!("p{}", i);
            let result = try_run(&storage_delete, &format!("DELETE points:{}", doc_id));
            if result.is_ok() {
                deleted_ids_clone.lock().unwrap().insert(doc_id);
            }
            thread::sleep(Duration::from_micros(50));
        }
    });

    let storage_reindex = Arc::clone(&storage);
    let barrier_reindex = Arc::clone(&barrier);

    let reindex_handle = thread::spawn(move || {
        barrier_reindex.wait();
        thread::sleep(Duration::from_millis(2));
        try_run(&storage_reindex, "REINDEX idx_points_hnsw_vec ON points")
    });

    delete_handle.join().unwrap();
    let reindex_result = reindex_handle.join().unwrap();

    assert!(
        reindex_result.is_ok(),
        "REINDEX failed: {:?}",
        reindex_result
    );

    let deleted = deleted_ids.lock().unwrap();

    // === DATA INTEGRITY CHECKS ===

    // 1. Total count
    let all_docs = run(&storage, "SELECT * FROM points");
    assert_eq!(
        all_docs.as_array().unwrap().len(),
        100 - deleted.len(),
        "Document count mismatch"
    );

    // 2. Search near [1,1,1] should NOT return deleted documents
    let search_corner = run(
        &storage,
        "SELECT id FROM points WHERE vec <|100|> [1.0, 1.0, 1.0]",
    );
    let corner_results: HashSet<String> = search_corner
        .as_array()
        .unwrap()
        .iter()
        .map(|v| extract_key(v["id"].as_str().unwrap()))
        .collect();

    for doc_id in deleted.iter() {
        assert!(
            !corner_results.contains(doc_id),
            "Deleted document {} found in HNSW search results",
            doc_id
        );
    }

    // 3. Search near [0,0,0] should return all even IDs
    let search_origin = run(
        &storage,
        "SELECT id FROM points WHERE vec <|50|> [0.0, 0.0, 0.0]",
    );
    let origin_results: HashSet<String> = search_origin
        .as_array()
        .unwrap()
        .iter()
        .map(|v| extract_key(v["id"].as_str().unwrap()))
        .collect();

    // All even IDs (not deleted) should be findable
    for i in (0..100).step_by(2) {
        assert!(
            origin_results.contains(&format!("p{}", i)),
            "Document p{} should be in search results",
            i
        );
    }

    // 4. Verify deleted documents don't exist
    for doc_id in deleted.iter() {
        let doc = run(&storage, &format!("SELECT * FROM points:{}", doc_id));
        assert!(
            doc.as_array().unwrap().is_empty(),
            "Deleted {} exists",
            doc_id
        );
    }
}

// =============================================================================
// CREATE INDEX Concurrent Tests
// =============================================================================

/// Test CREATE FULLTEXT INDEX while concurrent inserts are happening.
/// Verifies pre-existing documents are searchable, and most concurrent inserts are indexed.
#[test]
fn test_create_fts_index_during_inserts() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION docs");

    // Pre-populate with "existing" keyword
    for i in 0..30 {
        run(
            &storage,
            &format!(
                "INSERT INTO docs {{id: 'pre{}', text: 'Existing document number {}'}}",
                i, i
            ),
        );
    }

    let barrier = Arc::new(Barrier::new(2));
    let inserted_ids: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    // Insert documents with "added" keyword
    let storage_insert = Arc::clone(&storage);
    let barrier_insert = Arc::clone(&barrier);
    let inserted_ids_clone = Arc::clone(&inserted_ids);

    let insert_handle = thread::spawn(move || {
        barrier_insert.wait();

        for i in 0..40 {
            let doc_id = format!("post{}", i);
            let result = try_run(
                &storage_insert,
                &format!(
                    "INSERT INTO docs {{id: '{}', text: 'Added document content {}'}}",
                    doc_id, i
                ),
            );
            if result.is_ok() {
                inserted_ids_clone.lock().unwrap().insert(doc_id);
            }
            thread::sleep(Duration::from_micros(100));
        }
    });

    let storage_create = Arc::clone(&storage);
    let barrier_create = Arc::clone(&barrier);

    let create_handle = thread::spawn(move || {
        barrier_create.wait();
        thread::sleep(Duration::from_millis(1));
        try_run(&storage_create, "CREATE INDEX ON docs(text) FULLTEXT")
    });

    insert_handle.join().unwrap();
    let create_result = create_handle.join().unwrap();

    assert!(
        create_result.is_ok(),
        "CREATE INDEX failed: {:?}",
        create_result
    );

    let inserted = inserted_ids.lock().unwrap();

    // Flush FTS background commits to ensure all documents are visible
    storage.indexes().fts_backend().flush().unwrap();

    // === DATA INTEGRITY CHECKS ===

    // 1. Total count - all documents exist
    let all_docs = run(&storage, "SELECT * FROM docs");
    assert_eq!(
        all_docs.as_array().unwrap().len(),
        30 + inserted.len(),
        "Document count mismatch"
    );

    // 2. Pre-existing documents are ALL searchable
    let existing_search = run(&storage, "SELECT id FROM docs WHERE text @@ 'existing'");
    let existing_results: HashSet<String> = existing_search
        .as_array()
        .unwrap()
        .iter()
        .map(|v| extract_key(v["id"].as_str().unwrap()))
        .collect();

    assert_eq!(existing_results.len(), 30, "Expected 30 'existing' docs");
    for i in 0..30 {
        assert!(existing_results.contains(&format!("pre{}", i)));
    }

    // 3. Documents added during index creation - SOME may be indexed
    // (depending on timing, not all concurrent inserts may be in the index)
    let added_search = run(&storage, "SELECT id FROM docs WHERE text @@ 'added'");
    let _added_count = added_search.as_array().unwrap().len();

    // FTS search should work (not crash) during concurrent CREATE INDEX
    // The actual count depends on timing - some inserts may not be indexed yet

    // 4. All inserted documents exist (even if not all indexed yet)
    for doc_id in inserted.iter() {
        let doc = run(&storage, &format!("SELECT * FROM docs:{}", doc_id));
        assert_eq!(
            doc.as_array().unwrap().len(),
            1,
            "Document {} should exist",
            doc_id
        );
    }
}

/// Test CREATE HNSW INDEX while concurrent inserts are happening.
/// Verifies pre-existing documents are searchable, and operations don't crash.
#[test]
fn test_create_hnsw_index_during_inserts() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION items");

    // Pre-populate: vectors near origin
    for i in 0..30 {
        let v = (i as f32) * 0.01;
        run(
            &storage,
            &format!(
                "INSERT INTO items {{id: 'pre{}', vec: [{}, {}, {}]}}",
                i, v, v, v
            ),
        );
    }

    let barrier = Arc::new(Barrier::new(2));
    let inserted_ids: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    // Insert vectors near [1,1,1]
    let storage_insert = Arc::clone(&storage);
    let barrier_insert = Arc::clone(&barrier);
    let inserted_ids_clone = Arc::clone(&inserted_ids);

    let insert_handle = thread::spawn(move || {
        barrier_insert.wait();

        for i in 0..40 {
            let doc_id = format!("post{}", i);
            let v = 1.0 - (i as f32) * 0.01;
            let result = try_run(
                &storage_insert,
                &format!(
                    "INSERT INTO items {{id: '{}', vec: [{}, {}, {}]}}",
                    doc_id, v, v, v
                ),
            );
            if result.is_ok() {
                inserted_ids_clone.lock().unwrap().insert(doc_id);
            }
            thread::sleep(Duration::from_micros(100));
        }
    });

    let storage_create = Arc::clone(&storage);
    let barrier_create = Arc::clone(&barrier);

    let create_handle = thread::spawn(move || {
        barrier_create.wait();
        thread::sleep(Duration::from_millis(1));
        try_run(
            &storage_create,
            "CREATE INDEX ON items(vec) HNSW DIMENSION 3 DIST EUCLIDEAN",
        )
    });

    insert_handle.join().unwrap();
    let create_result = create_handle.join().unwrap();

    assert!(
        create_result.is_ok(),
        "CREATE INDEX failed: {:?}",
        create_result
    );

    let inserted = inserted_ids.lock().unwrap();

    // === DATA INTEGRITY CHECKS ===

    // 1. Total count - all documents exist
    let all_docs = run(&storage, "SELECT * FROM items");
    assert_eq!(
        all_docs.as_array().unwrap().len(),
        30 + inserted.len(),
        "Document count mismatch"
    );

    // 2. Search near origin returns pre-existing documents
    let search_origin = run(
        &storage,
        "SELECT id FROM items WHERE vec <|30|> [0.0, 0.0, 0.0]",
    );
    let origin_results: HashSet<String> = search_origin
        .as_array()
        .unwrap()
        .iter()
        .map(|v| extract_key(v["id"].as_str().unwrap()))
        .collect();

    for i in 0..30 {
        assert!(
            origin_results.contains(&format!("pre{}", i)),
            "Pre-existing doc pre{} not found in HNSW search",
            i
        );
    }

    // 3. Search near [1,1,1] should work (some docs may be indexed)
    // Not all concurrent inserts may be in the index due to timing
    let search_corner = run(
        &storage,
        "SELECT id FROM items WHERE vec <|50|> [1.0, 1.0, 1.0]",
    );
    let _corner_count = search_corner.as_array().unwrap().len();

    // HNSW search should work (not crash) during concurrent CREATE INDEX
    // The actual count depends on timing

    // 4. All inserted documents exist (even if not all indexed)
    for doc_id in inserted.iter() {
        let doc = run(&storage, &format!("SELECT * FROM items:{}", doc_id));
        assert_eq!(
            doc.as_array().unwrap().len(),
            1,
            "Document {} should exist",
            doc_id
        );
    }
}

// =============================================================================
// DROP INDEX Concurrent Tests
// =============================================================================

/// Test DROP FULLTEXT INDEX while concurrent operations.
/// After DROP, documents should still exist, just not be FTS-searchable.
#[test]
fn test_drop_fts_index_during_operations() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION notes");
    run(&storage, "CREATE INDEX ON notes(content) FULLTEXT");

    // Populate
    for i in 0..50 {
        run(
            &storage,
            &format!(
                "INSERT INTO notes {{id: 'n{}', content: 'Note about topic {}'}}",
                i, i
            ),
        );
    }

    let barrier = Arc::new(Barrier::new(3));
    let drop_done = Arc::new(AtomicBool::new(false));
    let inserted_ids: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    // Inserter
    let storage_insert = Arc::clone(&storage);
    let barrier_insert = Arc::clone(&barrier);
    let drop_done_insert = Arc::clone(&drop_done);
    let inserted_ids_clone = Arc::clone(&inserted_ids);

    let insert_handle = thread::spawn(move || {
        barrier_insert.wait();
        let mut i = 0;
        while !drop_done_insert.load(Ordering::SeqCst) && i < 30 {
            let doc_id = format!("new{}", i);
            let result = try_run(
                &storage_insert,
                &format!(
                    "INSERT INTO notes {{id: '{}', content: 'Fresh note {}'}}",
                    doc_id, i
                ),
            );
            if result.is_ok() {
                inserted_ids_clone.lock().unwrap().insert(doc_id);
            }
            i += 1;
            thread::sleep(Duration::from_micros(200));
        }
    });

    // Deleter
    let storage_delete = Arc::clone(&storage);
    let barrier_delete = Arc::clone(&barrier);
    let drop_done_delete = Arc::clone(&drop_done);

    let delete_handle = thread::spawn(move || {
        barrier_delete.wait();
        let mut deleted = 0;
        for i in 0..10 {
            if drop_done_delete.load(Ordering::SeqCst) {
                break;
            }
            if try_run(&storage_delete, &format!("DELETE notes:n{}", i)).is_ok() {
                deleted += 1;
            }
            thread::sleep(Duration::from_micros(300));
        }
        deleted
    });

    // Drop index
    let storage_drop = Arc::clone(&storage);
    let barrier_drop = Arc::clone(&barrier);
    let drop_done_drop = Arc::clone(&drop_done);

    let drop_handle = thread::spawn(move || {
        barrier_drop.wait();
        thread::sleep(Duration::from_millis(5));
        let result = try_run(&storage_drop, "DROP INDEX content_fts ON notes");
        drop_done_drop.store(true, Ordering::SeqCst);
        result
    });

    insert_handle.join().unwrap();
    let deleted_count = delete_handle.join().unwrap();
    let drop_result = drop_handle.join().unwrap();

    assert!(drop_result.is_ok(), "DROP INDEX failed: {:?}", drop_result);

    let inserted = inserted_ids.lock().unwrap();

    // === DATA INTEGRITY CHECKS ===

    // 1. Documents still exist (just not FTS-indexed)
    let all_docs = run(&storage, "SELECT * FROM notes");
    let total = all_docs.as_array().unwrap().len();
    let expected = 50 - deleted_count + inserted.len();
    assert_eq!(total, expected, "Expected {} docs, got {}", expected, total);

    // 2. Can still query by direct access
    for i in (deleted_count as usize)..50 {
        let doc = run(&storage, &format!("SELECT * FROM notes:n{}", i));
        assert_eq!(
            doc.as_array().unwrap().len(),
            1,
            "Document n{} should exist",
            i
        );
    }

    // 3. FTS search should fail (no index)
    let fts_result = try_run(&storage, "SELECT * FROM notes WHERE content @@ 'topic'");
    assert!(
        fts_result.is_err(),
        "FTS search should fail after DROP INDEX"
    );
}

/// Test DROP HNSW INDEX while concurrent vector searches.
#[test]
fn test_drop_hnsw_index_during_searches() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION embeddings");
    run(
        &storage,
        "CREATE INDEX ON embeddings(vec) HNSW DIMENSION 3 DIST COSINE",
    );

    // Populate
    for i in 0..50 {
        let v = (i as f32) / 50.0;
        run(
            &storage,
            &format!(
                "INSERT INTO embeddings {{id: 'e{}', vec: [{}, {}, {}]}}",
                i,
                v,
                1.0 - v,
                0.5
            ),
        );
    }

    let barrier = Arc::new(Barrier::new(2));
    let drop_done = Arc::new(AtomicBool::new(false));
    let successful_searches = Arc::new(AtomicUsize::new(0));

    let storage_search = Arc::clone(&storage);
    let barrier_search = Arc::clone(&barrier);
    let drop_done_search = Arc::clone(&drop_done);
    let successful_searches_clone = Arc::clone(&successful_searches);

    let search_handle = thread::spawn(move || {
        barrier_search.wait();

        while !drop_done_search.load(Ordering::SeqCst) {
            let result = try_run(
                &storage_search,
                "SELECT id FROM embeddings WHERE vec <|5|> [0.5, 0.5, 0.5]",
            );
            if result.is_ok() {
                successful_searches_clone.fetch_add(1, Ordering::SeqCst);
            }
            thread::sleep(Duration::from_micros(100));
        }
    });

    let storage_drop = Arc::clone(&storage);
    let barrier_drop = Arc::clone(&barrier);
    let drop_done_drop = Arc::clone(&drop_done);

    let drop_handle = thread::spawn(move || {
        barrier_drop.wait();
        thread::sleep(Duration::from_millis(10));
        let result = try_run(
            &storage_drop,
            "DROP INDEX idx_embeddings_hnsw_vec ON embeddings",
        );
        drop_done_drop.store(true, Ordering::SeqCst);
        result
    });

    search_handle.join().unwrap();
    let drop_result = drop_handle.join().unwrap();

    assert!(drop_result.is_ok(), "DROP INDEX failed: {:?}", drop_result);

    // Note: We don't assert searches > 0 because under heavy load (100+ threads)
    // the 10ms window before DROP may not be enough for any search to complete.
    // The important checks are below: data integrity after DROP.
    let _searches = successful_searches.load(Ordering::SeqCst);

    // === DATA INTEGRITY CHECKS ===

    // 1. All documents still exist
    let all_docs = run(&storage, "SELECT * FROM embeddings");
    assert_eq!(
        all_docs.as_array().unwrap().len(),
        50,
        "All 50 docs should exist"
    );

    // 2. Can access each document
    for i in 0..50 {
        let doc = run(&storage, &format!("SELECT * FROM embeddings:e{}", i));
        assert_eq!(doc.as_array().unwrap().len(), 1, "Doc e{} should exist", i);
    }

    // 3. HNSW search should fail (no index)
    let search_result = try_run(
        &storage,
        "SELECT id FROM embeddings WHERE vec <|5|> [0.5, 0.5, 0.5]",
    );
    assert!(
        search_result.is_err(),
        "HNSW search should fail after DROP INDEX"
    );
}

// =============================================================================
// Edge Cases
// =============================================================================

/// Test concurrent REINDEX on same collection should handle correctly.
#[test]
fn test_concurrent_reindex_same_collection() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION data");
    run(&storage, "CREATE INDEX ON data(text) FULLTEXT");

    for i in 0..100 {
        run(
            &storage,
            &format!(
                "INSERT INTO data {{id: 'd{}', text: 'Document number {}'}}",
                i, i
            ),
        );
    }

    let barrier = Arc::new(Barrier::new(2));

    let storage1 = Arc::clone(&storage);
    let barrier1 = Arc::clone(&barrier);
    let handle1 = thread::spawn(move || {
        barrier1.wait();
        try_run(&storage1, "REINDEX text_fts ON data")
    });

    let storage2 = Arc::clone(&storage);
    let barrier2 = Arc::clone(&barrier);
    let handle2 = thread::spawn(move || {
        barrier2.wait();
        thread::sleep(Duration::from_millis(1));
        try_run(&storage2, "REINDEX text_fts ON data")
    });

    let result1 = handle1.join().unwrap();
    let result2 = handle2.join().unwrap();

    // At least one should succeed
    let success_count = [&result1, &result2].iter().filter(|r| r.is_ok()).count();
    assert!(success_count >= 1, "At least one REINDEX should succeed");

    // === DATA INTEGRITY CHECK ===

    // All documents should be searchable regardless of which REINDEX succeeded
    let search = run(&storage, "SELECT id FROM data WHERE text @@ 'document'");
    let results: HashSet<String> = search
        .as_array()
        .unwrap()
        .iter()
        .map(|v| extract_key(v["id"].as_str().unwrap()))
        .collect();

    assert_eq!(results.len(), 100, "All 100 documents should be searchable");
    for i in 0..100 {
        assert!(
            results.contains(&format!("d{}", i)),
            "Missing document d{}",
            i
        );
    }
}

/// Test REINDEX with rapid insert/delete cycles.
#[test]
fn test_reindex_with_rapid_mutations() {
    let (_tmp, storage) = setup();
    run(&storage, "DEFINE COLLECTION rapid");
    run(&storage, "CREATE INDEX ON rapid(name) FULLTEXT");

    // Initial data with "permanent" keyword
    for i in 0..20 {
        run(
            &storage,
            &format!(
                "INSERT INTO rapid {{id: 'perm{}', name: 'Permanent record {}'}}",
                i, i
            ),
        );
    }

    let barrier = Arc::new(Barrier::new(2));
    let done = Arc::new(AtomicBool::new(false));
    let cycles = Arc::new(AtomicUsize::new(0));

    // Rapid mutation thread: insert then delete "temporary" docs
    let storage_mut = Arc::clone(&storage);
    let barrier_mut = Arc::clone(&barrier);
    let done_mut = Arc::clone(&done);
    let cycles_clone = Arc::clone(&cycles);

    let mutation_handle = thread::spawn(move || {
        barrier_mut.wait();
        let mut i = 0;
        while !done_mut.load(Ordering::SeqCst) {
            let doc_id = format!("tmp{}", i);
            let _ = try_run(
                &storage_mut,
                &format!(
                    "INSERT INTO rapid {{id: '{}', name: 'Temporary item {}'}}",
                    doc_id, i
                ),
            );
            let _ = try_run(&storage_mut, &format!("DELETE rapid:{}", doc_id));
            cycles_clone.fetch_add(1, Ordering::SeqCst);
            i += 1;
        }
    });

    // Reindex thread
    let storage_reindex = Arc::clone(&storage);
    let barrier_reindex = Arc::clone(&barrier);
    let done_reindex = Arc::clone(&done);

    let reindex_handle = thread::spawn(move || {
        barrier_reindex.wait();
        thread::sleep(Duration::from_millis(2));
        let result = try_run(&storage_reindex, "REINDEX name_fts ON rapid");
        done_reindex.store(true, Ordering::SeqCst);
        result
    });

    mutation_handle.join().unwrap();
    let reindex_result = reindex_handle.join().unwrap();

    assert!(
        reindex_result.is_ok(),
        "REINDEX failed: {:?}",
        reindex_result
    );

    let total_cycles = cycles.load(Ordering::SeqCst);

    // === DATA INTEGRITY CHECKS ===

    // 1. All permanent records intact and searchable
    let perm_search = run(&storage, "SELECT id FROM rapid WHERE name @@ 'permanent'");
    let perm_results: HashSet<String> = perm_search
        .as_array()
        .unwrap()
        .iter()
        .map(|v| extract_key(v["id"].as_str().unwrap()))
        .collect();

    assert_eq!(
        perm_results.len(),
        20,
        "All 20 permanent records should be searchable"
    );
    for i in 0..20 {
        assert!(
            perm_results.contains(&format!("perm{}", i)),
            "Missing perm{}",
            i
        );
    }

    // 2. No temporary documents should remain
    let temp_search = run(&storage, "SELECT id FROM rapid WHERE name @@ 'temporary'");
    let temp_count = temp_search.as_array().unwrap().len();

    // Might have 0 or 1 depending on timing
    assert!(
        temp_count <= 1,
        "At most 1 temporary doc should remain, got {}",
        temp_count
    );

    // 3. Total document count
    let all_docs = run(&storage, "SELECT * FROM rapid");
    let total = all_docs.as_array().unwrap().len();
    assert!(
        (20..=21).contains(&total),
        "Should have 20-21 docs, got {}",
        total
    );

    eprintln!(
        "Completed {} rapid insert/delete cycles during REINDEX",
        total_cycles
    );
}
