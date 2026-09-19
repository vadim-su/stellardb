use std::collections::HashMap;

use stellardb::schema::IndexDef;
use stellardb::storage::IndexBuildEvent;
use stellardb::{Database, Document, IndexBuildConfig, Value};

fn document(id: usize, value: i64) -> Document {
    Document {
        id: format!("items:{id}"),
        fields: HashMap::from([
            ("value".to_string(), Value::Int(value)),
            (
                "group".to_string(),
                Value::String(format!("g{}", value % 3)),
            ),
        ]),
    }
}

fn database(count: usize) -> (tempfile::TempDir, Database) {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open(tmp.path()).unwrap();
    db.get_or_create_collection("items").unwrap();
    for id in 0..count {
        db.set_document(&document(id, id as i64)).unwrap();
    }
    (tmp, db)
}

#[test]
fn btree_backfill_crosses_small_bulk_chunks_and_preserves_results() {
    let (_tmp, db) = database(19);
    db.set_index_build_config(IndexBuildConfig {
        chunk_size: 3,
        memory_budget_bytes: 1_024,
        worker_count: 3,
    })
    .unwrap();
    let index = IndexDef::new("items", vec!["value".to_string()]);
    let mut progress = Vec::new();

    db.initialize_index_build_artifact("items", &index, "bulk-result")
        .unwrap();
    assert!(
        db.build_index_artifact_with_progress("items", &index, "bulk-result", |event| {
            if let IndexBuildEvent::Building { processed_items } = event {
                progress.push(processed_items);
            }
            true
        })
        .unwrap()
    );
    db.promote_index_build_artifact("items", &index, "bulk-result")
        .unwrap();
    db.publish_ready_index("items", index.clone()).unwrap();

    let _guard = db.indexes().acquire_query_guard();
    let result = db
        .indexes()
        .scan_eq("items", &index.name, "value", &Value::Int(13), 10)
        .unwrap();
    assert_eq!(result.doc_ids, vec!["13"]);
    assert_eq!(progress.last(), Some(&19));
    assert!(progress.windows(2).all(|pair| pair[0] <= pair[1]));
}

#[test]
fn unique_duplicate_across_chunks_is_clear_and_remains_unpublished() {
    let (_tmp, db) = database(5);
    db.set_document(&document(5, 0)).unwrap();
    db.set_index_build_config(IndexBuildConfig {
        chunk_size: 1,
        memory_budget_bytes: 1_024,
        worker_count: 2,
    })
    .unwrap();
    let index = IndexDef::unique("items", vec!["value".to_string()]);

    db.initialize_index_build_artifact("items", &index, "duplicate")
        .unwrap();
    let error = db
        .build_index_artifact_with_progress("items", &index, "duplicate", |_| true)
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("Duplicate value for unique index")
    );
    assert!(!db.has_index("items", &index.fields));
    db.cleanup_index_build_artifact("items", &index, "duplicate")
        .unwrap();
}

#[test]
fn cancellation_is_checked_between_committed_chunks() {
    let (_tmp, db) = database(10);
    db.set_index_build_config(IndexBuildConfig {
        chunk_size: 2,
        memory_budget_bytes: 1_024,
        worker_count: 2,
    })
    .unwrap();
    let index = IndexDef::new("items", vec!["value".to_string()]);

    db.initialize_index_build_artifact("items", &index, "cancel")
        .unwrap();
    let completed = db
        .build_index_artifact_with_progress("items", &index, "cancel", |event| {
            !matches!(
                event,
                IndexBuildEvent::Building {
                    processed_items: 2..
                }
            )
        })
        .unwrap();

    assert!(!completed);
    assert!(!db.has_index("items", &index.fields));
    db.cleanup_index_build_artifact("items", &index, "cancel")
        .unwrap();
}
