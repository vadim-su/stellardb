//! Full pipeline benchmark (parse → bind → plan → execute)

use criterion::Criterion;
use std::hint::black_box;

use crate::helpers::TestDb;

pub fn bench_full_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline");

    let db = TestDb::with_users(100);

    let queries = [
        ("simple", "SELECT * FROM users"),
        (
            "filter_sort_limit",
            "SELECT name, age FROM users WHERE age > 30 ORDER age ASC LIMIT 10",
        ),
        (
            "aggregate",
            "SELECT city, COUNT(*), AVG(age) FROM users GROUP city",
        ),
    ];

    for (name, sql) in queries {
        group.bench_function(name, |b| b.iter(|| db.execute_query(black_box(sql))));
    }

    group.finish();
}
