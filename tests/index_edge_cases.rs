//! Comprehensive edge case tests for secondary indexes.
//!
//! Organized by priority:
//! 1. Critical - bugs that would cause data corruption or silent failures
//! 2. Important - correctness issues with compound/nested indexes
//! 3. Useful - robustness tests for edge values and boundaries

#[path = "index_edge_cases_tests/mod.rs"]
mod index_edge_cases_tests;
