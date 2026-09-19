//! Concurrency tests for StellarDB.
//!
//! These tests verify thread-safety and correct behavior under concurrent access.
//! They use multiple threads to simulate real-world concurrent workloads and
//! detect race conditions.
//!
//! ## Test Categories
//!
//! - `index/unique` - Race conditions in unique constraint validation
//! - `index/lifecycle` - CREATE/DROP INDEX during concurrent operations
//! - `index/reindex` - REINDEX during concurrent reads and writes
//! - `document/crud` - Concurrent document CRUD operations
//!
//! ## Running Concurrency Tests
//!
//! ```bash
//! cargo test concurrency
//! ```
//!
//! These tests may be flaky by nature (race conditions are timing-dependent).
//! Run multiple times to increase confidence:
//!
//! ```bash
//! for i in {1..10}; do cargo test concurrency || break; done
//! ```

#[path = "concurrency/common.rs"]
mod common;

// =============================================================================
// Index Operations
// =============================================================================

#[path = "concurrency/index/unique.rs"]
mod index_unique;

#[path = "concurrency/index/lifecycle.rs"]
mod index_lifecycle;

#[path = "concurrency/index/reindex.rs"]
mod index_reindex;

// =============================================================================
// Document Operations
// =============================================================================

#[path = "concurrency/document/crud.rs"]
mod document_crud;
