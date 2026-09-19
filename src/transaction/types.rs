use std::collections::{HashMap, HashSet};
use std::time::Instant;

use parking_lot::RwLock;
use rkyv::rancor;

use super::error::TransactionError;
use super::pending::{PendingWrite, TransactionState};
use crate::document::Document;
use crate::edge::Edge;
use crate::error::StorageError;
use crate::storage::{Database, WriteOp};

/// Encode document key for pending_writes HashMap
fn encode_doc_key(collection: &str, key: &str) -> Vec<u8> {
    format!("doc:{}:{}", collection, key).into_bytes()
}

fn serialization_error(error: impl std::fmt::Display) -> TransactionError {
    TransactionError::Storage(StorageError::Serialization(error.to_string()))
}

/// Unique transaction identifier
pub type TransactionId = String;

/// An explicit database transaction
///
/// Uses interior mutability (RwLock) to allow write operations through &self,
/// enabling implementation of the ExecutionContext trait while being thread-safe.
pub struct Transaction {
    /// Unique identifier
    pub(crate) id: TransactionId,

    /// Reference to storage for snapshot reads
    pub(crate) storage: std::sync::Arc<Database>,

    /// Buffer of uncommitted writes: full_key -> PendingWrite
    /// Uses RwLock for thread-safe interior mutability to support ExecutionContext trait
    pub(crate) pending_writes: RwLock<HashMap<Vec<u8>, PendingWrite>>,

    /// Pending edge writes: (from, label, to) -> PendingWrite
    /// Uses RwLock for thread-safe interior mutability to support ExecutionContext trait
    pub(crate) pending_edges: RwLock<HashMap<(String, String, String), PendingWrite>>,

    /// Fast lookup for deleted doc_ids ("collection:key" format)
    pub(crate) pending_deletes: RwLock<HashSet<String>>,

    /// When the transaction was created (for timeout)
    pub(crate) created_at: Instant,

    /// Current state
    pub(crate) state: RwLock<TransactionState>,
}

impl Transaction {
    /// Create a new transaction
    pub fn new(id: TransactionId, storage: std::sync::Arc<Database>) -> Self {
        Self {
            id,
            storage,
            pending_writes: RwLock::new(HashMap::new()),
            pending_edges: RwLock::new(HashMap::new()),
            pending_deletes: RwLock::new(HashSet::new()),
            created_at: Instant::now(),
            state: RwLock::new(TransactionState::Active),
        }
    }

    /// Get the transaction ID
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Check if the transaction is still active
    pub fn is_active(&self) -> bool {
        *self.state.read() == TransactionState::Active
    }

    /// Get elapsed time since creation
    pub fn elapsed(&self) -> std::time::Duration {
        self.created_at.elapsed()
    }

    /// Ensure transaction is active, return error if not
    fn ensure_active(&self) -> Result<(), TransactionError> {
        if *self.state.read() != TransactionState::Active {
            return Err(TransactionError::AlreadyCompleted);
        }
        Ok(())
    }

    /// Check if a document is marked for deletion in this transaction
    pub(crate) fn is_pending_delete(&self, doc_id: &str) -> bool {
        self.pending_deletes.read().contains(doc_id)
    }

    /// Get all pending insert documents for a specific collection
    pub(crate) fn pending_inserts_for_collection(
        &self,
        collection: &str,
    ) -> Result<Vec<Document>, TransactionError> {
        let prefix = format!("doc:{}:", collection);
        let pending = self.pending_writes.read();
        let mut docs = Vec::new();

        for (full_key, write) in pending.iter() {
            let key_str = String::from_utf8_lossy(full_key);
            if key_str.starts_with(&prefix)
                && let PendingWrite::Insert(bytes) = write
            {
                let doc = rkyv::from_bytes::<Document, rancor::Error>(bytes)
                    .map_err(serialization_error)?;
                docs.push(doc);
            }
        }

        Ok(docs)
    }

    /// Get a document, checking pending writes first (read-your-writes)
    pub fn get(&self, collection: &str, key: &str) -> Result<Option<Document>, TransactionError> {
        self.ensure_active()?;

        let full_key = encode_doc_key(collection, key);

        // 1. Check pending writes first
        if let Some(pending) = self.pending_writes.read().get(&full_key) {
            return match pending {
                PendingWrite::Insert(bytes) => {
                    let doc = rkyv::from_bytes::<Document, rancor::Error>(bytes)
                        .map_err(serialization_error)?;
                    Ok(Some(doc))
                }
                PendingWrite::Delete => Ok(None),
            };
        }

        // 2. Fall back to storage snapshot
        self.storage
            .get_document(collection, key)
            .map_err(TransactionError::from)
    }

    /// Insert a document (buffered until commit)
    pub fn insert(&self, doc: Document) -> Result<(), TransactionError> {
        self.ensure_active()?;

        let collection = doc.collection();
        let key = doc.key();
        let full_key = encode_doc_key(collection, key);

        let bytes = rkyv::to_bytes::<rancor::Error>(&doc).map_err(serialization_error)?;

        self.pending_writes
            .write()
            .insert(full_key, PendingWrite::Insert(bytes.to_vec()));
        self.pending_deletes.write().remove(&doc.id);
        Ok(())
    }

    /// Update a document (same as insert - full replacement)
    pub fn update(&self, doc: Document) -> Result<(), TransactionError> {
        self.insert(doc)
    }

    /// Delete a document (buffered until commit)
    pub fn delete(&self, collection: &str, key: &str) -> Result<(), TransactionError> {
        self.ensure_active()?;

        let full_key = encode_doc_key(collection, key);
        let doc_id = format!("{}:{}", collection, key);

        self.pending_writes
            .write()
            .insert(full_key, PendingWrite::Delete);
        self.pending_deletes.write().insert(doc_id);
        Ok(())
    }

    /// List documents in a collection with read-your-writes semantics
    ///
    /// This merges pending writes with storage snapshot:
    /// - Documents in pending_writes (Insert) are included
    /// - Documents in pending_writes (Delete) are excluded
    /// - Documents from storage that aren't in pending_writes are included
    pub fn list_documents(
        &self,
        collection: &str,
        limit: usize,
    ) -> Result<Vec<Document>, TransactionError> {
        self.ensure_active()?;

        // 1. Get documents from storage snapshot
        let (storage_docs, _) = self.storage.list_documents(collection, None, 10000)?;

        // 2. Build result with read-your-writes semantics
        let mut result = Vec::new();
        let prefix = format!("doc:{}:", collection);

        // First, add documents from storage that aren't deleted in pending writes
        let pending_writes = self.pending_writes.read();
        for doc in storage_docs {
            let full_key = encode_doc_key(collection, doc.key());
            match pending_writes.get(&full_key) {
                Some(PendingWrite::Delete) => {
                    // Document is deleted in this transaction, skip it
                }
                Some(PendingWrite::Insert(_)) => {
                    // Document was updated in transaction, use pending version
                    // (will be added when we iterate pending_writes)
                }
                None => {
                    // Document not modified in transaction, include from storage
                    result.push(doc);
                }
            }
        }

        // Then, add/replace with pending inserts for this collection
        for (full_key, pending) in pending_writes.iter() {
            let key_str = String::from_utf8_lossy(full_key);
            if key_str.starts_with(&prefix)
                && let PendingWrite::Insert(bytes) = pending
            {
                let doc = rkyv::from_bytes::<Document, rancor::Error>(bytes)
                    .map_err(serialization_error)?;
                result.push(doc);
            }
        }

        // Sort by key for consistent ordering
        result.sort_by(|a, b| a.key().cmp(b.key()));

        // Apply limit
        result.truncate(limit);

        Ok(result)
    }

    /// Create an edge (buffered until commit)
    pub fn create_edge(&self, edge: Edge) -> Result<(), TransactionError> {
        self.ensure_active()?;

        let key = (edge.from.clone(), edge.label.clone(), edge.to.clone());
        let bytes = rkyv::to_bytes::<rancor::Error>(&edge).map_err(serialization_error)?;

        self.pending_edges
            .write()
            .insert(key, PendingWrite::Insert(bytes.to_vec()));
        Ok(())
    }

    /// Get an edge, checking pending writes first
    pub fn get_edge(
        &self,
        from: &str,
        label: &str,
        to: &str,
    ) -> Result<Option<Edge>, TransactionError> {
        self.ensure_active()?;

        let key = (from.to_string(), label.to_string(), to.to_string());

        // 1. Check pending edges first
        if let Some(pending) = self.pending_edges.read().get(&key) {
            return match pending {
                PendingWrite::Insert(bytes) => {
                    let edge = rkyv::from_bytes::<Edge, rancor::Error>(bytes)
                        .map_err(serialization_error)?;
                    Ok(Some(edge))
                }
                PendingWrite::Delete => Ok(None),
            };
        }

        // 2. Fall back to storage
        self.storage
            .get_edge(from, label, to)
            .map_err(TransactionError::from)
    }

    /// Delete an edge (buffered until commit)
    pub fn delete_edge(&self, from: &str, label: &str, to: &str) -> Result<(), TransactionError> {
        self.ensure_active()?;

        let key = (from.to_string(), label.to_string(), to.to_string());
        self.pending_edges.write().insert(key, PendingWrite::Delete);
        Ok(())
    }

    /// Commit all pending changes to storage atomically
    pub fn commit(&self) -> Result<(), TransactionError> {
        let mut state = self.state.write();
        if *state != TransactionState::Active {
            return Err(TransactionError::AlreadyCompleted);
        }

        // Collect all operations
        let mut ops = Vec::new();
        let mut pending_writes = self.pending_writes.write();
        let mut pending_edges = self.pending_edges.write();

        // Document operations
        for (full_key, write) in pending_writes.iter() {
            let key_str = String::from_utf8_lossy(full_key);
            let parts: Vec<&str> = key_str.splitn(3, ':').collect();
            if parts.len() != 3 || parts[0] != "doc" {
                return Err(TransactionError::Internal(format!(
                    "malformed pending_writes key: expected 'doc:collection:key', got '{}'",
                    key_str
                )));
            }
            let (collection, key) = (parts[1].to_string(), parts[2].to_string());

            match write {
                PendingWrite::Insert(bytes) => {
                    let doc = rkyv::from_bytes::<Document, rancor::Error>(bytes)
                        .map_err(serialization_error)?;
                    ops.push(WriteOp::InsertDoc(doc));
                }
                PendingWrite::Delete => {
                    ops.push(WriteOp::DeleteDoc { collection, key });
                }
            }
        }

        // Edge operations
        for ((from, label, to), write) in pending_edges.iter() {
            match write {
                PendingWrite::Insert(bytes) => {
                    let edge = rkyv::from_bytes::<Edge, rancor::Error>(bytes)
                        .map_err(serialization_error)?;
                    ops.push(WriteOp::InsertEdge(edge));
                }
                PendingWrite::Delete => {
                    ops.push(WriteOp::DeleteEdge {
                        from: from.clone(),
                        label: label.clone(),
                        to: to.clone(),
                    });
                }
            }
        }

        // Atomic batch write
        self.storage.atomic_write_batch(ops)?;

        pending_writes.clear();
        pending_edges.clear();
        self.pending_deletes.write().clear();
        *state = TransactionState::Committed;
        Ok(())
    }

    /// Rollback the transaction, discarding all pending changes
    pub fn rollback(&self) -> Result<(), TransactionError> {
        self.ensure_active()?;

        self.pending_writes.write().clear();
        self.pending_edges.write().clear();
        self.pending_deletes.write().clear();
        *self.state.write() = TransactionState::RolledBack;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, std::sync::Arc<Database>) {
        let dir = TempDir::new().unwrap();
        let storage = std::sync::Arc::new(Database::open(dir.path()).unwrap());
        (dir, storage)
    }

    #[test]
    fn test_transaction_get_from_pending_writes() {
        let (_dir, storage) = setup();
        let tx = Transaction::new("tx1".to_string(), storage);

        // Insert via pending writes
        let doc = crate::Document {
            id: "user:alice".to_string(),
            fields: [(
                "name".to_string(),
                crate::Value::String("Alice".to_string()),
            )]
            .into_iter()
            .collect(),
        };
        tx.insert(doc.clone()).unwrap();

        // Should read from pending writes
        let result = tx.get("user", "alice").unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "user:alice");
    }

    #[test]
    fn test_transaction_delete_then_get_returns_none() {
        let (_dir, storage) = setup();

        // First insert a document directly
        let doc = crate::Document {
            id: "user:bob".to_string(),
            fields: [("name".to_string(), crate::Value::String("Bob".to_string()))]
                .into_iter()
                .collect(),
        };
        storage.set_document(&doc).unwrap();

        // Start transaction and delete
        let tx = Transaction::new("tx1".to_string(), storage);
        tx.delete("user", "bob").unwrap();

        // Get should return None (read-your-writes)
        let result = tx.get("user", "bob").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_transaction_edge_operations() {
        let (_dir, storage) = setup();
        let tx = Transaction::new("tx1".to_string(), storage);

        // Create edge
        let edge = crate::Edge::new(
            "user:alice".to_string(),
            "follows".to_string(),
            "user:bob".to_string(),
        );
        tx.create_edge(edge.clone()).unwrap();

        // Get edge from pending
        let result = tx.get_edge("user:alice", "follows", "user:bob").unwrap();
        assert!(result.is_some());

        // Delete edge
        tx.delete_edge("user:alice", "follows", "user:bob").unwrap();

        // Get should return None
        let result = tx.get_edge("user:alice", "follows", "user:bob").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_transaction_commit_rejects_edge_to_missing_document() {
        let (_dir, storage) = setup();

        let alice = crate::Document {
            id: "user:alice".to_string(),
            fields: std::collections::HashMap::new(),
        };
        storage.create_document(&alice).unwrap();

        let tx = Transaction::new("tx1".to_string(), storage.clone());
        tx.create_edge(crate::Edge::new(
            "user:alice".to_string(),
            "follows".to_string(),
            "user:missing".to_string(),
        ))
        .unwrap();

        let err = tx.commit().unwrap_err();
        assert!(
            err.to_string().contains("does not exist"),
            "commit should reject dangling transaction edge, got: {err}"
        );
        assert!(
            storage
                .get_edge("user:alice", "follows", "user:missing")
                .unwrap()
                .is_none(),
            "failed transaction commit must not expose dangling edge"
        );
    }

    #[test]
    fn test_failed_transaction_commit_preserves_pending_writes() {
        let (_dir, storage) = setup();

        let alice = crate::Document {
            id: "user:alice".to_string(),
            fields: std::collections::HashMap::new(),
        };

        let tx = Transaction::new("tx1".to_string(), storage.clone());
        tx.insert(alice).unwrap();
        tx.create_edge(crate::Edge::new(
            "user:alice".to_string(),
            "follows".to_string(),
            "user:missing".to_string(),
        ))
        .unwrap();

        let err = tx.commit().unwrap_err();
        assert!(
            err.to_string().contains("does not exist"),
            "commit should reject dangling transaction edge, got: {err}"
        );
        assert!(
            tx.is_active(),
            "failed commit should leave the transaction active"
        );
        assert!(
            tx.get("user", "alice").unwrap().is_some(),
            "failed commit must preserve pending document writes"
        );
        assert!(
            tx.get_edge("user:alice", "follows", "user:missing")
                .unwrap()
                .is_some(),
            "failed commit must preserve pending edge writes"
        );
    }

    #[test]
    fn test_transaction_commit_persists_documents() {
        let (_dir, storage) = setup();

        // Insert in transaction and commit
        {
            let tx = Transaction::new("tx1".to_string(), storage.clone());
            let doc = crate::Document {
                id: "user:charlie".to_string(),
                fields: [(
                    "name".to_string(),
                    crate::Value::String("Charlie".to_string()),
                )]
                .into_iter()
                .collect(),
            };
            tx.insert(doc).unwrap();
            tx.commit().unwrap();
        }

        // Should be readable from storage
        let doc = storage.get_document("user", "charlie").unwrap();
        assert!(doc.is_some());
        assert_eq!(doc.unwrap().id, "user:charlie");
    }

    #[test]
    fn test_transaction_rollback_discards_changes() {
        let (_dir, storage) = setup();

        // Insert in transaction but rollback
        {
            let tx = Transaction::new("tx1".to_string(), storage.clone());
            let doc = crate::Document {
                id: "user:dave".to_string(),
                fields: [("name".to_string(), crate::Value::String("Dave".to_string()))]
                    .into_iter()
                    .collect(),
            };
            tx.insert(doc).unwrap();
            tx.rollback().unwrap();
        }

        // Should NOT be in storage
        let doc = storage.get_document("user", "dave").unwrap();
        assert!(doc.is_none());
    }

    #[test]
    fn test_transaction_cannot_use_after_commit() {
        let (_dir, storage) = setup();
        let tx = Transaction::new("tx1".to_string(), storage);

        tx.commit().unwrap();

        // Should fail
        let result = tx.get("user", "anyone");
        assert!(result.is_err());
    }
}
