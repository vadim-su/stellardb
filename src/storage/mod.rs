//! Storage module with modular architecture
//!
//! # Module Structure
//!
//! - `database` - Database facade
//! - `keyspaces` - Keyspace management
//! - `documents` - Document CRUD operations
//! - `edges` - Edge CRUD operations
//! - `indexes` - Index backends and management
//! - `constraints` - Unique constraint handling
//! - `schema` - Schema persistence
//! - `encoding` - Key encoding utilities
//! - `lock_stripes` - Fine-grained locking
//!
//! # Directory Structure
//!
//! ```text
//! data/
//! ├── storage/           # Fjall database (keyspaces, journals)
//! └── indexes/
//!     ├── fts/           # Full-text search indexes (Tantivy)
//!     └── vector/        # Vector indexes (HNSW)
//! ```

/// Directory name for Fjall database storage
pub const DIR_STORAGE: &str = "storage";

/// Directory name for all indexes
pub const DIR_INDEXES: &str = "indexes";

/// Directory name for FTS indexes (under indexes/)
pub const DIR_FTS: &str = "fts";

/// Directory name for vector/HNSW indexes (under indexes/)
pub const DIR_VECTOR: &str = "vector";

mod constraints;
mod context_impl;
mod database;
mod documents;
mod edges;
mod encoding;
mod indexes;
mod keyspaces;
mod lock_stripes;
mod schema;

// Public API
pub use database::{Database, StorageStats};
pub use encoding::*;
pub use indexes::analyzers;
pub use indexes::{IndexBackend, IndexRegistry, IndexType, RangeOp, ScanResult};
pub use keyspaces::{CollectionKeyspaces, KeyspaceManager};
pub use lock_stripes::LockStripes;

/// Resource limits used while preparing B-tree index backfill batches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexBuildConfig {
    /// Maximum number of documents decoded for one preparation chunk.
    pub chunk_size: usize,
    /// Maximum target size of prepared keys in one Fjall write batch.
    pub memory_budget_bytes: usize,
    /// Maximum number of CPU workers used to decode and extract index keys.
    pub worker_count: usize,
}

impl Default for IndexBuildConfig {
    fn default() -> Self {
        Self {
            chunk_size: 4_096,
            memory_budget_bytes: 64 * 1024 * 1024,
            worker_count: std::thread::available_parallelism()
                .map_or(1, usize::from)
                .clamp(1, 8),
        }
    }
}

impl IndexBuildConfig {
    pub fn validate(self) -> Result<Self, crate::error::StorageError> {
        if self.chunk_size == 0 {
            return Err(crate::error::StorageError::InvalidConfig(
                "index build chunk size must be greater than zero".into(),
            ));
        }
        if self.memory_budget_bytes < 1024 {
            return Err(crate::error::StorageError::InvalidConfig(
                "index build memory budget must be at least 1024 bytes".into(),
            ));
        }
        if self.worker_count == 0 {
            return Err(crate::error::StorageError::InvalidConfig(
                "index build worker count must be greater than zero".into(),
            ));
        }
        Ok(self)
    }
}

/// Coarse-grained milestones emitted while a durable index build is running.
/// Callers may return `false` from their progress callback at any milestone to
/// request cancellation before the index is published in the schema registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexBuildEvent {
    Building { processed_items: usize },
    Committing,
    ValidatingUnique,
}

use crate::document::Document;
use crate::edge::Edge;

/// Atomic write operation for batch commits
#[derive(Debug, Clone)]
pub enum WriteOp {
    /// Insert or replace a document.
    InsertDoc(Document),
    /// Insert a document only if its key does not already exist.
    CreateDoc(Document),
    DeleteDoc {
        collection: String,
        key: String,
    },
    InsertEdge(Edge),
    DeleteEdge {
        from: String,
        label: String,
        to: String,
    },
}
