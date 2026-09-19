//! Subquery benchmarks

use criterion::{BenchmarkId, Criterion, Throughput};
use std::hint::black_box;

use crate::helpers::TestDb;

/// FROM subquery benchmarks
pub fn bench_from_subquery(c: &mut Criterion) {
    let mut group = c.benchmark_group("subquery_from");

    let db = TestDb::with_users(500);

    let queries = [
        ("simple", "SELECT * FROM (SELECT * FROM users)"),
        (
            "with_filter",
            "SELECT * FROM (SELECT * FROM users WHERE age > 30)",
        ),
        (
            "nested",
            "SELECT * FROM (SELECT * FROM (SELECT * FROM users WHERE active = true) WHERE age > 25)",
        ),
        (
            "with_projection",
            "SELECT name, age FROM (SELECT name, age, city FROM users WHERE age > 30)",
        ),
        (
            "aggregate_subquery",
            "SELECT * FROM (SELECT city, COUNT(*) as cnt, AVG(age) as avg_age FROM users GROUP city)",
        ),
    ];

    for (name, query) in queries {
        group.bench_with_input(BenchmarkId::new("from", name), &query, |b, q| {
            b.iter(|| db.execute_query(black_box(q)))
        });
    }

    group.finish();
}

/// Correlated subquery in SELECT projection benchmarks
pub fn bench_correlated_subquery(c: &mut Criterion) {
    let mut group = c.benchmark_group("subquery_correlated");

    // 50 users, 10 orders each = 500 orders total
    let db = TestDb::with_users_and_orders(50, 10);

    group.throughput(Throughput::Elements(50));

    let queries = [
        (
            "select_all_orders",
            "SELECT name, (SELECT * FROM orders WHERE user_name = $parent.name) AS orders FROM users",
        ),
        (
            "count_orders",
            "SELECT name, (SELECT COUNT(*) FROM orders WHERE user_name = $parent.name) AS order_count FROM users",
        ),
        (
            "sum_amount",
            "SELECT name, (SELECT SUM(amount) FROM orders WHERE user_name = $parent.name) AS total FROM users",
        ),
        (
            "filtered_orders",
            "SELECT name, (SELECT * FROM orders WHERE user_name = $parent.name AND status = 'completed') AS completed FROM users",
        ),
        (
            "with_outer_filter",
            "SELECT name, (SELECT COUNT(*) FROM orders WHERE user_name = $parent.name) AS cnt FROM users WHERE active = true",
        ),
    ];

    for (name, query) in queries {
        group.bench_with_input(BenchmarkId::new("correlated", name), &query, |b, q| {
            b.iter(|| db.execute_query(black_box(q)))
        });
    }

    group.finish();
}

/// Scaling test for correlated subqueries
pub fn bench_correlated_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("subquery_scaling");

    let configs = [
        (10, 5),   // 10 users, 5 orders each
        (50, 10),  // 50 users, 10 orders each
        (100, 10), // 100 users, 10 orders each
    ];

    for (users, orders) in configs {
        let db = TestDb::with_users_and_orders(users, orders);
        let label = format!("{}u_{}o", users, orders);

        group.throughput(Throughput::Elements(users as u64));
        group.bench_with_input(
            BenchmarkId::new("correlated_count", &label),
            &db,
            |b, db| {
                b.iter(|| {
                    db.execute_query(black_box(
                        "SELECT name, (SELECT COUNT(*) FROM orders WHERE user_name = $parent.name) AS cnt FROM users",
                    ))
                })
            },
        );
    }

    group.finish();
}
