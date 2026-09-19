//! Complex query benchmarks combining multiple features

use criterion::{BenchmarkId, Criterion};
use std::hint::black_box;

use crate::helpers::TestDb;

/// Complex queries combining multiple SQL features
pub fn bench_complex_queries(c: &mut Criterion) {
    let mut group = c.benchmark_group("complex");

    let db = TestDb::with_users_and_orders(100, 5);

    let queries = [
        (
            "filter_sort_limit",
            "SELECT name, age, city FROM users WHERE active = true ORDER age DESC LIMIT 20",
        ),
        (
            "group_filter_sort",
            "SELECT city, COUNT(*) as cnt FROM users WHERE age > 25 GROUP city ORDER cnt DESC",
        ),
        (
            "multi_condition",
            "SELECT * FROM users WHERE (age > 30 AND city = 'Moscow') OR (age < 25 AND active = true) LIMIT 50",
        ),
        (
            "nested_subquery_agg",
            "SELECT * FROM (SELECT city, COUNT(*) as cnt, AVG(age) as avg FROM users GROUP city) WHERE cnt > 10",
        ),
        (
            "correlated_with_agg",
            "SELECT name, city, (SELECT SUM(amount) FROM orders WHERE user_name = $parent.name) as total FROM users WHERE active = true ORDER name LIMIT 20",
        ),
        (
            "multi_agg_group",
            "SELECT city, COUNT(*) as cnt, SUM(score) as total_score, AVG(age) as avg_age, MIN(age) as min_age, MAX(age) as max_age FROM users GROUP city ORDER cnt DESC",
        ),
    ];

    for (name, query) in queries {
        group.bench_with_input(BenchmarkId::new("query", name), &query, |b, q| {
            b.iter(|| db.execute_query(black_box(q)))
        });
    }

    group.finish();
}

/// Object literal and computed field benchmarks
pub fn bench_computed_fields(c: &mut Criterion) {
    let mut group = c.benchmark_group("computed");

    let db = TestDb::with_users(500);

    let queries = [
        (
            "object_literal",
            "SELECT {'name': name, 'info': {'age': age, 'city': city}} as user_data FROM users LIMIT 100",
        ),
        (
            "arithmetic",
            "SELECT name, age * 2 as double_age, score + 100 as bonus_score FROM users",
        ),
        (
            "conditional_proj",
            "SELECT name, age, city, score FROM users WHERE age > 30 OR score > 500",
        ),
    ];

    for (name, query) in queries {
        group.bench_with_input(BenchmarkId::new("computed", name), &query, |b, q| {
            b.iter(|| db.execute_query(black_box(q)))
        });
    }

    group.finish();
}
