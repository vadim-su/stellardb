//! Reproducible benchmark suite for release notes and benchmark pages.
//!
//! Keep this harness small enough to compile quickly while covering startup,
//! memory, writes, FTS, vector, traversal, and hybrid search execution.

use std::collections::HashMap;
use std::hint::black_box;
use std::sync::Arc;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use stellardb::query::result::BatchResult;
use stellardb::query::{ExecuteContext, Query};
use stellardb::schema::IndexDef;
use stellardb::session::{SessionConfig, SessionManager};
use stellardb::{Database, Document, IndexBuildConfig, Value};

struct BenchDb {
    storage: Arc<Database>,
    sessions: SessionManager,
    _tmp: tempfile::TempDir,
}

impl BenchDb {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let storage = Arc::new(Database::open(tmp.path()).expect("open database"));
        let sessions = SessionManager::new(SessionConfig::default());

        Self {
            storage,
            sessions,
            _tmp: tmp,
        }
    }

    fn run(&self, sql: &str) {
        let result = self.query(sql);

        assert!(result.error.is_none(), "query failed: {:?}", result.error);
    }

    fn query(&self, sql: &str) -> BatchResult {
        let ctx = ExecuteContext::with_session(&self.sessions, None);
        Query::parse(sql)
            .expect("parse")
            .bind(self.storage.clone())
            .expect("bind")
            .execute(&ctx)
            .expect("execute")
    }

    fn with_documents(count: usize) -> Self {
        let db = Self::new();
        db.storage
            .get_or_create_collection("items")
            .expect("create collection");

        for i in 0..count {
            db.storage
                .set_document(&document(
                    "items",
                    &format!("item{i}"),
                    [
                        ("name", Value::String(format!("Item {i}"))),
                        ("category", Value::String(format!("cat{}", i % 8))),
                        ("value", Value::Int(i as i64)),
                    ],
                ))
                .expect("insert document");
        }

        db
    }

    fn with_search_indexes(count: usize) -> Self {
        let db = Self::new();
        db.run("DEFINE COLLECTION products");
        db.run("CREATE INDEX ON products(description) FULLTEXT");
        db.run("CREATE INDEX ON products(embedding) HNSW DIMENSION 3 DIST COSINE");

        for i in 0..count {
            let x = if i % 3 == 0 { 1.0 } else { 0.1 };
            let y = if i % 3 == 1 { 1.0 } else { 0.1 };
            let z = if i % 3 == 2 { 1.0 } else { 0.1 };
            db.run(&format!(
                "INSERT INTO products {{id: 'p{i}', name: 'product {i}', \
                 description: 'rust database vector search item {i}', \
                 embedding: [{x}, {y}, {z}]}}",
            ));
        }

        db
    }

    fn with_vector_recall_dataset() -> Self {
        let db = Self::new();
        db.run("DEFINE COLLECTION vectors");
        db.run("CREATE INDEX ON vectors(embedding) HNSW DIMENSION 3 DIST COSINE");

        for i in 0..100 {
            let (x, y, z) = if i < 10 {
                (1.0, (i as f64) / 1000.0, 0.0)
            } else if i < 55 {
                (0.0, 1.0, (i as f64) / 1000.0)
            } else {
                (0.0, (i as f64) / 1000.0, 1.0)
            };
            db.run(&format!(
                "INSERT INTO vectors {{id: 'v{i}', embedding: [{x}, {y}, {z}]}}",
            ));
        }

        db
    }

    fn with_traversal_graph() -> Self {
        let db = Self::new();
        db.run("DEFINE COLLECTION user");

        for i in 0..80 {
            db.run(&format!("CREATE user:u{i} SET name = 'User {i}'"));
        }

        for i in 0..80 {
            for j in 1..=5 {
                let target = (i + j) % 80;
                db.run(&format!("RELATE user:u{i}->follows->user:u{target}"));
            }
        }

        db
    }
}

fn document<const N: usize>(collection: &str, key: &str, fields: [(&str, Value); N]) -> Document {
    Document {
        id: format!("{collection}:{key}"),
        fields: fields
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect::<HashMap<_, _>>(),
    }
}

fn bench_startup(c: &mut Criterion) {
    let mut group = c.benchmark_group("reproducible_startup");

    group.bench_function("open_empty_database", |b| {
        b.iter_batched(
            || tempfile::tempdir().expect("tempdir"),
            |tmp| Database::open(tmp.path()).expect("open database"),
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

fn bench_memory(c: &mut Criterion) {
    let mut group = c.benchmark_group("reproducible_memory");
    let db = BenchDb::with_documents(1_000);

    group.bench_function("rss_after_1k_documents", |b| {
        b.iter(|| black_box(current_rss_bytes(&db)))
    });

    group.finish();
}

fn current_rss_bytes(_db: &BenchDb) -> u64 {
    #[cfg(target_os = "linux")]
    {
        let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
            return 0;
        };
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                let kb = rest
                    .split_whitespace()
                    .next()
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(0);
                return kb * 1024;
            }
        }
        0
    }

    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

fn bench_writes(c: &mut Criterion) {
    let mut group = c.benchmark_group("reproducible_writes");

    group.bench_function("set_100_documents", |b| {
        b.iter_batched(
            BenchDb::new,
            |db| {
                db.storage
                    .get_or_create_collection("items")
                    .expect("create collection");
                for i in 0..100 {
                    db.storage
                        .set_document(&document(
                            "items",
                            &format!("item{i}"),
                            [("value", Value::Int(i as i64))],
                        ))
                        .expect("set document");
                }
            },
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

fn bench_btree_bulk_backfill(c: &mut Criterion) {
    let mut group = c.benchmark_group("reproducible_btree_backfill");
    group.sample_size(10);
    group.bench_function("bulk_10k_documents", |b| {
        b.iter_batched(
            || BenchDb::with_documents(10_000),
            |db| {
                db.storage
                    .set_index_build_config(IndexBuildConfig {
                        chunk_size: 1_024,
                        memory_budget_bytes: 16 * 1024 * 1024,
                        worker_count: 4,
                    })
                    .expect("configure backfill");
                db.storage
                    .build_index("items", &IndexDef::new("items", vec!["value".to_string()]))
                    .expect("bulk B-tree backfill");
            },
            BatchSize::LargeInput,
        )
    });
    group.finish();
}

fn bench_fts(c: &mut Criterion) {
    let mut group = c.benchmark_group("reproducible_fts");
    let db = BenchDb::with_search_indexes(200);

    group.bench_function("description_search", |b| {
        b.iter(|| {
            db.run(black_box(
                "SELECT id FROM products WHERE description @@ 'database'",
            ))
        })
    });

    group.finish();
}

fn bench_vector(c: &mut Criterion) {
    let mut group = c.benchmark_group("reproducible_vector");
    let db = BenchDb::with_search_indexes(200);

    group.bench_function("top10_cosine_search", |b| {
        b.iter(|| {
            db.run(black_box(
                "SELECT id FROM products WHERE embedding <|10|> [1.0, 0.0, 0.0]",
            ))
        })
    });

    group.finish();
}

fn bench_vector_recall(c: &mut Criterion) {
    let mut group = c.benchmark_group("reproducible_vector_recall");
    let db = BenchDb::with_vector_recall_dataset();

    group.bench_function("top10_cluster_recall", |b| {
        b.iter(|| {
            let result = db.query(black_box(
                "SELECT id FROM vectors WHERE embedding <|10, 200|> [1.0, 0.0, 0.0]",
            ));
            assert!(result.error.is_none(), "query failed: {:?}", result.error);
            let rows = &result.results[0].rows;
            let hits = rows
                .iter()
                .filter(|row| {
                    row.doc
                        .id
                        .strip_prefix("vectors:v")
                        .and_then(|suffix| suffix.parse::<usize>().ok())
                        .is_some_and(|idx| idx < 10)
                })
                .count();
            let recall = hits as f64 / 10.0;
            assert!(
                recall >= 0.9,
                "expected recall@10 >= 0.9, got {recall} from {rows:?}"
            );
            black_box(recall)
        })
    });

    group.finish();
}

fn bench_traversal(c: &mut Criterion) {
    let mut group = c.benchmark_group("reproducible_traversal");
    let db = BenchDb::with_traversal_graph();

    group.bench_function("two_hop_fanout", |b| {
        b.iter(|| {
            let result = db.query(black_box(
                "SELECT ->follows->follows->user.* AS fof FROM user:u0",
            ));
            assert!(result.error.is_none(), "query failed: {:?}", result.error);
            assert!(
                !result.results[0].rows.is_empty(),
                "two-hop traversal should return at least one row"
            );
            black_box(result.results[0].rows.len())
        })
    });

    group.finish();
}

fn bench_hybrid(c: &mut Criterion) {
    let mut group = c.benchmark_group("reproducible_hybrid");
    let db = BenchDb::with_search_indexes(200);

    group.bench_function("rrf_fts_vector", |b| {
        b.iter(|| {
            db.run(black_box(
                "LET fts = (SELECT id, name FROM products WHERE description @@ 'database'); \
                 LET vec = (SELECT id, name FROM products WHERE embedding <|10|> [1.0, 0.0, 0.0]); \
                 SELECT * FROM (search::rrf([$fts, $vec], 10))",
            ))
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_startup,
    bench_memory,
    bench_writes,
    bench_btree_bulk_backfill,
    bench_fts,
    bench_vector,
    bench_vector_recall,
    bench_traversal,
    bench_hybrid,
);
criterion_main!(benches);
