//! Index vs full scan benchmarks

use criterion::{BenchmarkId, Criterion, Throughput};
use std::hint::black_box;

use crate::helpers::TestDb;

/// Compare query performance with and without indexes
pub fn bench_index_vs_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("index_vs_scan");

    let count = 1000;
    let db_no_index = TestDb::with_users(count);
    let db_indexed = TestDb::with_users_indexed(count);

    group.throughput(Throughput::Elements(count as u64));

    // Equality filter on indexed field
    group.bench_function("eq_no_index", |b| {
        b.iter(|| db_no_index.execute_query(black_box("SELECT * FROM users WHERE age = 35")))
    });
    group.bench_function("eq_indexed", |b| {
        b.iter(|| db_indexed.execute_query(black_box("SELECT * FROM users WHERE age = 35")))
    });

    // Range filter on indexed field
    group.bench_function("range_no_index", |b| {
        b.iter(|| {
            db_no_index.execute_query(black_box("SELECT * FROM users WHERE age > 30 AND age < 40"))
        })
    });
    group.bench_function("range_indexed", |b| {
        b.iter(|| {
            db_indexed.execute_query(black_box("SELECT * FROM users WHERE age > 30 AND age < 40"))
        })
    });

    // Unique index lookup
    group.bench_function("unique_no_index", |b| {
        b.iter(|| {
            db_no_index.execute_query(black_box(
                "SELECT * FROM users WHERE email = 'user500@example.com'",
            ))
        })
    });
    group.bench_function("unique_indexed", |b| {
        b.iter(|| {
            db_indexed.execute_query(black_box(
                "SELECT * FROM users WHERE email = 'user500@example.com'",
            ))
        })
    });

    // Compound index (city, age)
    group.bench_function("compound_no_index", |b| {
        b.iter(|| {
            db_no_index.execute_query(black_box(
                "SELECT * FROM users WHERE city = 'Moscow' AND age > 30",
            ))
        })
    });
    group.bench_function("compound_indexed", |b| {
        b.iter(|| {
            db_indexed.execute_query(black_box(
                "SELECT * FROM users WHERE city = 'Moscow' AND age > 30",
            ))
        })
    });

    group.finish();
}

/// Index performance at different data sizes
pub fn bench_index_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("index_scaling");

    for count in [100, 500, 1000, 5000] {
        let db = TestDb::with_users_indexed(count);

        group.throughput(Throughput::Elements(count as u64));

        // Point lookup via unique index
        group.bench_with_input(BenchmarkId::new("unique_lookup", count), &db, |b, db| {
            let email = format!("user{}@example.com", count / 2);
            let query = format!("SELECT * FROM users WHERE email = '{}'", email);
            b.iter(|| db.execute_query(black_box(&query)))
        });

        // Range scan via index
        group.bench_with_input(BenchmarkId::new("range_scan", count), &db, |b, db| {
            b.iter(|| db.execute_query(black_box("SELECT * FROM users WHERE age > 40")))
        });

        // Full scan for comparison
        group.bench_with_input(BenchmarkId::new("full_scan", count), &db, |b, db| {
            b.iter(|| db.execute_query(black_box("SELECT * FROM users")))
        });
    }

    group.finish();
}

/// Aggregate queries with indexes
pub fn bench_index_aggregates(c: &mut Criterion) {
    let mut group = c.benchmark_group("index_aggregates");

    let db_no_index = TestDb::with_users(1000);
    let db_indexed = TestDb::with_users_indexed(1000);

    // COUNT with filter
    group.bench_function("count_filter_no_index", |b| {
        b.iter(|| db_no_index.execute_query(black_box("SELECT COUNT(*) FROM users WHERE age > 40")))
    });
    group.bench_function("count_filter_indexed", |b| {
        b.iter(|| db_indexed.execute_query(black_box("SELECT COUNT(*) FROM users WHERE age > 40")))
    });

    // GROUP BY on indexed field
    group.bench_function("group_no_index", |b| {
        b.iter(|| {
            db_no_index.execute_query(black_box("SELECT city, COUNT(*) FROM users GROUP city"))
        })
    });
    group.bench_function("group_indexed", |b| {
        b.iter(|| {
            db_indexed.execute_query(black_box("SELECT city, COUNT(*) FROM users GROUP city"))
        })
    });

    // GROUP BY with filter
    group.bench_function("group_filter_no_index", |b| {
        b.iter(|| {
            db_no_index.execute_query(black_box(
                "SELECT city, AVG(age) FROM users WHERE active = true GROUP city",
            ))
        })
    });
    group.bench_function("group_filter_indexed", |b| {
        b.iter(|| {
            db_indexed.execute_query(black_box(
                "SELECT city, AVG(age) FROM users WHERE active = true GROUP city",
            ))
        })
    });

    group.finish();
}
