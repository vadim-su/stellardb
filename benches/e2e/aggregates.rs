//! Aggregate and GROUP BY benchmarks

use criterion::{BenchmarkId, Criterion};
use std::hint::black_box;

use crate::helpers::TestDb;

pub fn bench_aggregates(c: &mut Criterion) {
    let mut group = c.benchmark_group("e2e_aggregates");

    let db = TestDb::with_users(1000);

    let queries = [
        ("count", "SELECT COUNT(*) FROM users"),
        ("count_field", "SELECT COUNT(age) FROM users"),
        ("sum", "SELECT SUM(age) FROM users"),
        ("avg", "SELECT AVG(score) FROM users"),
        ("min_max", "SELECT MIN(age), MAX(age) FROM users"),
        (
            "multi_agg",
            "SELECT COUNT(*), SUM(age), AVG(score) FROM users",
        ),
    ];

    for (name, query) in queries {
        group.bench_with_input(BenchmarkId::new("aggregate", name), &query, |b, q| {
            b.iter(|| db.execute_query(black_box(q)))
        });
    }

    group.finish();
}

pub fn bench_group_by(c: &mut Criterion) {
    let mut group = c.benchmark_group("e2e_group_by");

    let db = TestDb::with_users(1000);

    let queries = [
        ("simple", "SELECT city, COUNT(*) FROM users GROUP city"),
        (
            "multi_agg",
            "SELECT city, COUNT(*), AVG(age) FROM users GROUP city",
        ),
        (
            "with_filter",
            "SELECT city, COUNT(*) FROM users WHERE active = true GROUP city",
        ),
    ];

    for (name, query) in queries {
        group.bench_with_input(BenchmarkId::new("group_by", name), &query, |b, q| {
            b.iter(|| db.execute_query(black_box(q)))
        });
    }

    group.finish();
}
