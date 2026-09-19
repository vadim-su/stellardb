//! Benchmarks for graph traversal across different graph shapes and scales.
//!
//! These benchmarks measure traversal, fan-out, deduplication, bidirectional
//! lookup, edge-only lookup, and field-selection performance.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::Arc;
use stellardb::Database;
use tempfile::TempDir;

/// Setup a graph with specified node count and edges per node.
/// Creates a social network-like topology where each node follows `edges_per_node` other nodes.
fn setup_graph(node_count: usize, edges_per_node: usize) -> (TempDir, Arc<Database>) {
    let tmp = TempDir::new().unwrap();
    let storage = Arc::new(Database::open(tmp.path()).unwrap());

    // Define collection
    stellardb::query_json(storage.clone(), "DEFINE COLLECTION user");

    // Create nodes
    for i in 0..node_count {
        stellardb::query_json(
            storage.clone(),
            &format!("CREATE user:u{} SET name = 'User {}'", i, i),
        );
    }

    // Create edges: each node follows `edges_per_node` others
    for i in 0..node_count {
        for j in 0..edges_per_node {
            let target = (i + j + 1) % node_count;
            stellardb::query_json(
                storage.clone(),
                &format!("RELATE user:u{}->follows->user:u{}", i, target),
            );
        }
    }

    (tmp, storage)
}

/// Setup a star topology graph: one center node connected to `leaf_count` leaf nodes.
fn setup_star_graph(leaf_count: usize) -> (TempDir, Arc<Database>) {
    let tmp = TempDir::new().unwrap();
    let storage = Arc::new(Database::open(tmp.path()).unwrap());

    stellardb::query_json(storage.clone(), "DEFINE COLLECTION user");
    stellardb::query_json(storage.clone(), "CREATE user:center SET name = 'Center'");

    for i in 0..leaf_count {
        stellardb::query_json(
            storage.clone(),
            &format!("CREATE user:leaf{} SET name = 'Leaf {}'", i, i),
        );
        stellardb::query_json(
            storage.clone(),
            &format!("RELATE user:center->follows->user:leaf{}", i),
        );
    }

    (tmp, storage)
}

/// Setup a diamond graph for deduplication testing:
/// source -> middle_count nodes -> single target
fn setup_diamond_graph(middle_count: usize) -> (TempDir, Arc<Database>) {
    let tmp = TempDir::new().unwrap();
    let storage = Arc::new(Database::open(tmp.path()).unwrap());

    stellardb::query_json(storage.clone(), "DEFINE COLLECTION user");
    stellardb::query_json(storage.clone(), "CREATE user:source SET name = 'Source'");
    stellardb::query_json(storage.clone(), "CREATE user:target SET name = 'Target'");

    for i in 0..middle_count {
        stellardb::query_json(
            storage.clone(),
            &format!("CREATE user:middle{} SET name = 'Middle {}'", i, i),
        );
        stellardb::query_json(
            storage.clone(),
            &format!("RELATE user:source->follows->user:middle{}", i),
        );
        stellardb::query_json(
            storage.clone(),
            &format!("RELATE user:middle{}->follows->user:target", i),
        );
    }

    (tmp, storage)
}

/// Benchmark two-hop traversal (friends of friends) at different graph scales.
/// This tests batch loading performance when the number of intermediate nodes
/// exceeds the batch threshold (64).
fn bench_traversal_two_hop(c: &mut Criterion) {
    let mut group = c.benchmark_group("traversal_two_hop");
    group.sample_size(50);

    for size in [50, 100, 200, 500] {
        let (_tmp, storage) = setup_graph(size, 10);

        group.bench_with_input(BenchmarkId::new("nodes", size), &storage, |b, storage| {
            b.iter(|| {
                let result = stellardb::query_json(
                    storage.clone(),
                    "SELECT ->follows->follows->user.* AS fof FROM user:u0",
                );
                black_box(result)
            })
        });
    }

    group.finish();
}

/// Benchmark single-hop traversal with varying fan-out (number of edges).
/// Tests batch loading when a single node has many outgoing edges.
fn bench_traversal_fan_out(c: &mut Criterion) {
    let mut group = c.benchmark_group("traversal_fan_out");
    group.sample_size(50);

    for leaf_count in [50, 100, 200, 500] {
        let (_tmp, storage) = setup_star_graph(leaf_count);

        group.bench_with_input(
            BenchmarkId::new("edges", leaf_count),
            &storage,
            |b, storage| {
                b.iter(|| {
                    let result = stellardb::query_json(
                        storage.clone(),
                        "SELECT ->follows->user.* AS following FROM user:center",
                    );
                    black_box(result)
                })
            },
        );
    }

    group.finish();
}

/// Benchmark traversal with deduplication at different scales.
/// Diamond topology: many middle nodes all pointing to same target.
/// Tests that deduplication doesn't degrade performance.
fn bench_traversal_dedup(c: &mut Criterion) {
    let mut group = c.benchmark_group("traversal_dedup");
    group.sample_size(50);

    for middle_count in [50, 100, 200, 500] {
        let (_tmp, storage) = setup_diamond_graph(middle_count);

        group.bench_with_input(
            BenchmarkId::new("middle_nodes", middle_count),
            &storage,
            |b, storage| {
                b.iter(|| {
                    let result = stellardb::query_json(
                        storage.clone(),
                        "SELECT ->follows->follows->user.* AS targets FROM user:source",
                    );
                    black_box(result)
                })
            },
        );
    }

    group.finish();
}

/// Benchmark bidirectional traversal at different scales.
fn bench_traversal_bidirectional(c: &mut Criterion) {
    let mut group = c.benchmark_group("traversal_bidirectional");
    group.sample_size(50);

    for size in [50, 100, 200] {
        let tmp = TempDir::new().unwrap();
        let storage = Arc::new(Database::open(tmp.path()).unwrap());

        stellardb::query_json(storage.clone(), "DEFINE COLLECTION user");
        stellardb::query_json(storage.clone(), "CREATE user:center SET name = 'Center'");

        // Half outgoing, half incoming edges
        let half = size / 2;
        for i in 0..half {
            stellardb::query_json(
                storage.clone(),
                &format!("CREATE user:out{} SET name = 'Out {}'", i, i),
            );
            stellardb::query_json(
                storage.clone(),
                &format!("RELATE user:center->follows->user:out{}", i),
            );
        }
        for i in 0..half {
            stellardb::query_json(
                storage.clone(),
                &format!("CREATE user:in{} SET name = 'In {}'", i, i),
            );
            stellardb::query_json(
                storage.clone(),
                &format!("RELATE user:in{}->follows->user:center", i),
            );
        }

        // Keep tmp alive
        let _keep = tmp;

        group.bench_with_input(
            BenchmarkId::new("connections", size),
            &storage,
            |b, storage| {
                b.iter(|| {
                    let result = stellardb::query_json(
                        storage.clone(),
                        "SELECT <->follows<->user.* AS connections FROM user:center",
                    );
                    black_box(result)
                })
            },
        );
    }

    group.finish();
}

/// Benchmark edge-only traversal (without resolving target documents).
fn bench_traversal_edges_only(c: &mut Criterion) {
    let mut group = c.benchmark_group("traversal_edges_only");
    group.sample_size(50);

    for leaf_count in [50, 100, 200, 500] {
        let (_tmp, storage) = setup_star_graph(leaf_count);

        group.bench_with_input(
            BenchmarkId::new("edges", leaf_count),
            &storage,
            |b, storage| {
                b.iter(|| {
                    let result = stellardb::query_json(
                        storage.clone(),
                        "SELECT ->follows AS edges FROM user:center",
                    );
                    black_box(result)
                })
            },
        );
    }

    group.finish();
}

/// Benchmark multi-field selection on traversal results.
fn bench_traversal_field_selection(c: &mut Criterion) {
    let mut group = c.benchmark_group("traversal_field_selection");
    group.sample_size(50);

    // Setup graph with edge data
    let tmp = TempDir::new().unwrap();
    let storage = Arc::new(Database::open(tmp.path()).unwrap());

    stellardb::query_json(storage.clone(), "DEFINE COLLECTION user");

    for i in 0..200 {
        stellardb::query_json(
            storage.clone(),
            &format!(
                "CREATE user:u{} SET name = 'User {}', age = {}, email = 'user{}@example.com'",
                i,
                i,
                20 + i % 50,
                i
            ),
        );
    }

    for i in 1..200 {
        stellardb::query_json(
            storage.clone(),
            &format!(
                "RELATE user:u0->follows->user:u{} SET since = {}, notes = 'Connection {}'",
                i,
                2020 + i % 5,
                i
            ),
        );
    }

    let _keep = tmp;

    // Benchmark: all fields
    group.bench_function("all_fields", |b| {
        b.iter(|| {
            let result = stellardb::query_json(
                storage.clone(),
                "SELECT ->follows->user.* AS following FROM user:u0",
            );
            black_box(result)
        })
    });

    // Benchmark: single field
    group.bench_function("single_field", |b| {
        b.iter(|| {
            let result = stellardb::query_json(
                storage.clone(),
                "SELECT ->follows->user.name AS names FROM user:u0",
            );
            black_box(result)
        })
    });

    // Benchmark: multi-field selection
    group.bench_function("multi_field", |b| {
        b.iter(|| {
            let result = stellardb::query_json(
                storage.clone(),
                "SELECT ->follows->user.{name, email} AS following FROM user:u0",
            );
            black_box(result)
        })
    });

    // Benchmark: edge fields with alias
    group.bench_function("edge_fields_alias", |b| {
        b.iter(|| {
            let result = stellardb::query_json(
                storage.clone(),
                "SELECT ->follows.{year: since, to} AS edges FROM user:u0",
            );
            black_box(result)
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_traversal_two_hop,
    bench_traversal_fan_out,
    bench_traversal_dedup,
    bench_traversal_bidirectional,
    bench_traversal_edges_only,
    bench_traversal_field_selection,
);

criterion_main!(benches);
