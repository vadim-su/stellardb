//! End-to-end benchmarks for StellarDB query execution
//!
//! These benchmarks measure the full query pipeline:
//! parse -> bind -> plan -> optimize -> execute

use criterion::{criterion_group, criterion_main};

mod aggregates;
mod complex;
mod dml;
mod helpers;
mod indexes;
mod pipeline;
mod select;
mod subquery;

criterion_group!(
    select_benches,
    select::bench_select_all,
    select::bench_select_with_filter,
    select::bench_select_with_projection,
    select::bench_select_with_sort,
    select::bench_select_with_limit,
    select::bench_key_lookup,
);

criterion_group!(
    aggregate_benches,
    aggregates::bench_aggregates,
    aggregates::bench_group_by,
);

criterion_group!(
    dml_benches,
    dml::bench_insert,
    dml::bench_update,
    dml::bench_delete,
);

criterion_group!(pipeline_benches, pipeline::bench_full_pipeline,);

criterion_group!(
    subquery_benches,
    subquery::bench_from_subquery,
    subquery::bench_correlated_subquery,
    subquery::bench_correlated_scaling,
);

criterion_group!(
    index_benches,
    indexes::bench_index_vs_scan,
    indexes::bench_index_scaling,
    indexes::bench_index_aggregates,
);

criterion_group!(
    complex_benches,
    complex::bench_complex_queries,
    complex::bench_computed_fields,
);

criterion_main!(
    select_benches,
    aggregate_benches,
    dml_benches,
    pipeline_benches,
    subquery_benches,
    index_benches,
    complex_benches
);
