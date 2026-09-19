//! SELECT query benchmarks

use criterion::{BenchmarkId, Criterion, Throughput};
use std::hint::black_box;

use crate::helpers::TestDb;

pub fn bench_select_all(c: &mut Criterion) {
    let mut group = c.benchmark_group("e2e_select_all");

    for count in [10, 100, 1000] {
        let db = TestDb::with_users(count);

        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::new("rows", count), &db, |b, db| {
            b.iter(|| db.execute_query(black_box("SELECT * FROM users")))
        });
    }

    group.finish();
}

pub fn bench_select_with_filter(c: &mut Criterion) {
    let mut group = c.benchmark_group("e2e_select_filter");

    let db = TestDb::with_users(1000);

    let queries = [
        ("eq", "SELECT * FROM users WHERE age = 30"),
        ("gt", "SELECT * FROM users WHERE age > 40"),
        (
            "and",
            "SELECT * FROM users WHERE age > 30 AND active = true",
        ),
        ("or", "SELECT * FROM users WHERE age < 25 OR age > 60"),
        (
            "in",
            "SELECT * FROM users WHERE city IN ['Moscow', 'Berlin']",
        ),
    ];

    for (name, query) in queries {
        group.bench_with_input(BenchmarkId::new("filter", name), &query, |b, q| {
            b.iter(|| db.execute_query(black_box(q)))
        });
    }

    group.finish();
}

pub fn bench_select_with_projection(c: &mut Criterion) {
    let mut group = c.benchmark_group("e2e_select_projection");

    let db = TestDb::with_users(500);

    let queries = [
        ("single_field", "SELECT name FROM users"),
        ("multi_field", "SELECT name, age, city FROM users"),
        ("computed", "SELECT name, age * 2 as double_age FROM users"),
        (
            "object_literal",
            "SELECT {'n': name, 'a': age} as info FROM users",
        ),
    ];

    for (name, query) in queries {
        group.bench_with_input(BenchmarkId::new("projection", name), &query, |b, q| {
            b.iter(|| db.execute_query(black_box(q)))
        });
    }

    group.finish();
}

pub fn bench_select_with_sort(c: &mut Criterion) {
    let mut group = c.benchmark_group("e2e_select_sort");

    for count in [100, 500, 1000] {
        let db = TestDb::with_users(count);

        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::new("sort_by_age", count), &db, |b, db| {
            b.iter(|| db.execute_query(black_box("SELECT * FROM users ORDER age ASC")))
        });
    }

    group.finish();
}

pub fn bench_select_with_limit(c: &mut Criterion) {
    let mut group = c.benchmark_group("e2e_select_limit");

    let db = TestDb::with_users(1000);

    let queries = [
        ("limit_10", "SELECT * FROM users LIMIT 10"),
        ("limit_100", "SELECT * FROM users LIMIT 100"),
        ("limit_offset", "SELECT * FROM users LIMIT 50 OFFSET 100"),
        ("sort_limit", "SELECT * FROM users ORDER age DESC LIMIT 20"),
    ];

    for (name, query) in queries {
        group.bench_with_input(BenchmarkId::new("limit", name), &query, |b, q| {
            b.iter(|| db.execute_query(black_box(q)))
        });
    }

    group.finish();
}

pub fn bench_key_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("e2e_key_lookup");

    let db = TestDb::with_users(1000);

    group.bench_function("single_key", |b| {
        b.iter(|| db.execute_query(black_box("SELECT * FROM users:user500")))
    });

    group.finish();
}
