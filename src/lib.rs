#[cfg(all(feature = "server", any(not(feature = "fts"), not(feature = "hnsw"))))]
compile_error!("server builds require both the `fts` and `hnsw` features");

#[cfg(all(
    not(feature = "server"),
    not(feature = "embedded-lite"),
    any(not(feature = "fts"), not(feature = "hnsw"))
))]
compile_error!(
    "FTS and HNSW search may only be disabled in an explicit embedded-lite build; \
     use `--no-default-features --features embedded-lite`"
);

pub mod auth;
pub mod document;
pub mod edge;
pub mod embedded;
pub mod error;
#[cfg(feature = "fault-injection")]
pub mod fault_injection;
pub mod metrics;
pub mod namespace;
pub mod query;
pub mod schema;
#[cfg(feature = "server")]
pub mod server;
pub mod session;
pub mod storage;
pub mod transaction;
pub mod util;
pub mod version;

pub use document::{Document, Value};
pub use edge::Edge;
pub use embedded::{EmbeddedDatabase, EmbeddedError, EmbeddedQueryOptions, EmbeddedTransaction};
pub use query::execute::Row;
pub use schema::{
    CollectionSchema, DefaultValue, DistanceMetric, FieldDef, FieldType, HnswParams, IndexDef,
    IndexType, SchemaMode, SchemaRegistry, parse_schema_file,
};
pub use storage::{Database, IndexBuildConfig};

/// Helper for tests - runs query and converts result to JSON
pub fn query_json(storage: std::sync::Arc<Database>, sql: &str) -> serde_json::Value {
    use crate::query::execute::operators::operator::rows_to_json;
    use crate::query::{ExecuteContext, Query};
    let parsed = Query::parse(sql).expect("parse failed");
    let bound = parsed.bind(storage).expect("bind failed");
    let result = bound
        .execute(&ExecuteContext::none())
        .expect("execute failed")
        .single()
        .expect("expected single result");
    rows_to_json(&result.rows)
}
