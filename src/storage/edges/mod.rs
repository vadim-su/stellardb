//! Edge storage operations
//!
//! This module provides edge storage with keyspace-per-label architecture:
//!
//! - Separate keyspace for each edge type (label)
//! - Single/Multiple cardinality
//! - Directed/Undirected edges
//! - Field indexes
//! - Compact storage
//! - Batch operations

mod store;

pub use store::EdgeStore;
