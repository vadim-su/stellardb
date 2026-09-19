//! Index management and backends

#[cfg(feature = "fts")]
pub mod analyzers;
#[cfg(not(feature = "fts"))]
#[path = "analyzers_disabled.rs"]
pub mod analyzers;
mod btree;
#[cfg(feature = "fts")]
mod fts;
#[cfg(not(feature = "fts"))]
mod fts_disabled;
#[cfg(feature = "hnsw")]
mod hnsw;
#[cfg(not(feature = "hnsw"))]
mod hnsw_disabled;
mod manager;
mod registry;
mod traits;

#[allow(unused_imports)]
#[cfg(feature = "fts")]
pub use fts::FtsIndex;
#[allow(unused_imports)]
#[cfg(not(feature = "fts"))]
pub use fts_disabled::FtsIndex;
#[allow(unused_imports)]
#[cfg(feature = "hnsw")]
pub use hnsw::HnswIndex;
#[cfg(feature = "hnsw")]
pub(super) use hnsw::extract_vector;
#[allow(unused_imports)]
#[cfg(not(feature = "hnsw"))]
pub use hnsw_disabled::HnswIndex;
#[cfg(not(feature = "hnsw"))]
pub(super) use hnsw_disabled::extract_vector;
pub use manager::{IndexManager, QueryExecutionGuard};
pub use registry::IndexRegistry;
pub use traits::{IndexBackend, IndexType, RangeOp, ScanResult};

// Re-export analyzer registry and builder for external use
#[allow(unused_imports)]
#[cfg(not(feature = "fts"))]
pub use analyzers::AnalyzerRegistry;
#[allow(unused_imports)]
#[cfg(feature = "fts")]
pub use analyzers::{AnalyzerRegistry, build_analyzer};
