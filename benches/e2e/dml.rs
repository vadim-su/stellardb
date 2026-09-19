//! DML (INSERT, UPDATE, DELETE) benchmarks

use criterion::Criterion;
use std::hint::black_box;

use crate::helpers::TestDb;

pub fn bench_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("e2e_insert");

    group.bench_function("single_doc", |b| {
        b.iter_batched(
            || {
                let db = TestDb::new();
                db.execute_query("DEFINE COLLECTION users");
                db
            },
            |db| {
                db.execute_query(black_box(
                    "INSERT INTO users {id: 'test', name: 'Test', age: 25}",
                ))
            },
            criterion::BatchSize::SmallInput,
        )
    });

    group.bench_function("batch_10", |b| {
        b.iter_batched(
            || {
                let db = TestDb::new();
                db.execute_query("DEFINE COLLECTION users");
                db
            },
            |db| {
                for i in 0..10 {
                    db.execute_query(&format!(
                        "INSERT INTO users {{id: 'test{}', name: 'Test{}', age: {}}}",
                        i,
                        i,
                        20 + i
                    ));
                }
            },
            criterion::BatchSize::SmallInput,
        )
    });

    group.finish();
}

pub fn bench_update(c: &mut Criterion) {
    let mut group = c.benchmark_group("e2e_update");

    let db = TestDb::with_users(100);

    group.bench_function("single_field", |b| {
        b.iter(|| db.execute_query(black_box("UPDATE users SET age = 99 WHERE name = 'User50'")))
    });

    group.bench_function("multi_field", |b| {
        b.iter(|| {
            db.execute_query(black_box(
                "UPDATE users SET age = 99, active = false WHERE city = 'Moscow'",
            ))
        })
    });

    group.finish();
}

pub fn bench_delete(c: &mut Criterion) {
    let mut group = c.benchmark_group("e2e_delete");

    group.bench_function("with_filter", |b| {
        b.iter_batched(
            || TestDb::with_users(100),
            |db| db.execute_query(black_box("DELETE users WHERE age > 60")),
            criterion::BatchSize::SmallInput,
        )
    });

    group.finish();
}
