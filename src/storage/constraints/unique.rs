//! UniqueConstraintManager - handles unique constraint validation and enforcement
//!
//! Uses Fjall's `fetch_update` for atomic check-and-set operations, eliminating
//! race conditions between checking and claiming lock keys.

use std::cell::Cell;
use std::collections::HashMap;

use fjall::OptimisticTxKeyspace;
use xxhash_rust::xxh3::xxh3_128;

use crate::document::{Document, Value};
use crate::error::StorageError;
use crate::schema::IndexDef;
use crate::storage::encoding::encode_value_for_hash;

/// Manages unique constraint validation and enforcement using atomic operations.
///
/// All operations use `fetch_update` which provides:
/// - Atomic read-modify-write semantics
/// - Automatic retry on conflicts (OCC)
/// - No external locking required
pub struct UniqueConstraintManager;

impl UniqueConstraintManager {
    /// Create a new UniqueConstraintManager
    pub fn new() -> Self {
        Self
    }

    /// Validate and update unique constraints atomically.
    ///
    /// Uses `fetch_update` for atomic claim/release of lock keys:
    /// - For INSERT: atomically claims the new lock key
    /// - For UPDATE: claims new key first, then releases old key
    /// - For DELETE: atomically releases the lock key
    ///
    /// # Arguments
    /// * `ulock_keyspace` - Keyspace for storing unique lock entries
    /// * `indexes` - All indexes for the collection (filters to unique only)
    /// * `old_doc` - Previous document state (None for insert)
    /// * `new_doc` - New document state (None for delete)
    ///
    /// # Returns
    /// * `Ok(())` if constraints are satisfied and lock keys updated
    /// * `Err(StorageError::Conflict)` if a constraint would be violated
    pub fn validate_and_update(
        &self,
        ulock_keyspace: &OptimisticTxKeyspace,
        indexes: &[IndexDef],
        old_doc: Option<&Document>,
        new_doc: Option<&Document>,
    ) -> Result<(), StorageError> {
        let unique_indexes: Vec<_> = indexes.iter().filter(|idx| idx.unique).collect();

        if unique_indexes.is_empty() {
            return Ok(());
        }

        for index in &unique_indexes {
            let new_value = new_doc.and_then(|d| self.get_field_value_for_index(d, &index.fields));
            let old_value = old_doc.and_then(|d| self.get_field_value_for_index(d, &index.fields));

            // If value hasn't changed, no work needed for this index
            if new_value == old_value {
                continue;
            }

            // IMPORTANT: Claim new key FIRST, then release old key
            // This ensures we never leave a gap where another thread could steal the value

            // Claim new lock key if exists
            if let Some(ref new_val) = new_value {
                let new_doc_key = new_doc
                    .expect("new_doc must be Some when new_value is Some")
                    .key();
                self.claim_lock_key(ulock_keyspace, &index.fields, new_val, new_doc_key)?;
            }

            // Release old lock key if exists (only after new key is claimed)
            if let Some(ref old_val) = old_value {
                let old_doc_key = old_doc
                    .expect("old_doc must be Some when old_value is Some")
                    .key();
                self.release_lock_key(ulock_keyspace, &index.fields, old_val, old_doc_key)?;
            }
        }

        Ok(())
    }

    /// Atomically claim a lock key using fetch_update.
    ///
    /// The operation succeeds if:
    /// - The key doesn't exist (we claim it)
    /// - The key already belongs to this document (idempotent)
    ///
    /// Returns error if the key is claimed by another document.
    fn claim_lock_key(
        &self,
        ulock_keyspace: &OptimisticTxKeyspace,
        fields: &[String],
        value: &Value,
        doc_key: &str,
    ) -> Result<(), StorageError> {
        let lock_key = self.compute_unique_lock_key(&fields.join(","), value);
        let doc_key_bytes = doc_key.as_bytes().to_vec();

        // Cell to capture result from closure (closure may run multiple times on retry)
        let claimed = Cell::new(true);
        let fields_clone = fields.to_vec();

        ulock_keyspace.fetch_update(lock_key, |existing| {
            match existing {
                None => {
                    // Key doesn't exist - claim it
                    claimed.set(true);
                    Some(doc_key_bytes.clone().into())
                }
                Some(v) if *v == doc_key_bytes.as_slice() => {
                    // Already ours - keep it (idempotent)
                    claimed.set(true);
                    Some(doc_key_bytes.clone().into())
                }
                Some(v) => {
                    // Belongs to another document - keep their value, signal conflict
                    claimed.set(false);
                    Some(v.to_vec().into())
                }
            }
        })?;

        if !claimed.get() {
            return Err(StorageError::Conflict(format!(
                "Unique constraint violation on {:?}",
                fields_clone
            )));
        }

        Ok(())
    }

    /// Atomically release a lock key using fetch_update.
    ///
    /// Only releases the key if it belongs to the specified document.
    /// This prevents accidentally releasing a key that was already claimed by another document.
    fn release_lock_key(
        &self,
        ulock_keyspace: &OptimisticTxKeyspace,
        fields: &[String],
        value: &Value,
        doc_key: &str,
    ) -> Result<(), StorageError> {
        let lock_key = self.compute_unique_lock_key(&fields.join(","), value);
        let doc_key_bytes = doc_key.as_bytes().to_vec();

        ulock_keyspace.fetch_update(lock_key, |existing| {
            match existing {
                Some(v) if *v == doc_key_bytes.as_slice() => {
                    // Our key - delete it
                    None
                }
                None => {
                    // Already deleted - no-op
                    None
                }
                Some(v) => {
                    // Belongs to another document - don't touch it
                    Some(v.to_vec().into())
                }
            }
        })?;

        Ok(())
    }

    /// Rollback a previous `validate_and_update` call by reversing the operation.
    ///
    /// This is used when a transaction fails after constraints were already modified.
    /// Reverses the claim/release by swapping old_doc and new_doc parameters.
    /// Errors are silently ignored since this is best-effort compensation —
    /// if rollback fails (e.g., another thread claimed the value), the constraint
    /// state is still safe (the original transaction didn't commit).
    pub fn rollback_validate_and_update(
        &self,
        ulock_keyspace: &OptimisticTxKeyspace,
        indexes: &[IndexDef],
        old_doc: Option<&Document>,
        new_doc: Option<&Document>,
    ) {
        // Reverse: what was new becomes old (release), what was old becomes new (re-claim)
        let _ = self.validate_and_update(ulock_keyspace, indexes, new_doc, old_doc);
    }

    /// Remove unique constraint entries for a document being deleted.
    ///
    /// Uses atomic release to safely remove lock keys.
    pub fn remove_for_document(
        &self,
        ulock_keyspace: &OptimisticTxKeyspace,
        indexes: &[IndexDef],
        doc: &Document,
    ) -> Result<(), StorageError> {
        for index in indexes.iter().filter(|idx| idx.unique) {
            if let Some(value) = self.get_field_value_for_index(doc, &index.fields) {
                self.release_lock_key(ulock_keyspace, &index.fields, &value, doc.key())?;
            }
        }
        Ok(())
    }

    /// Remove all unique constraint entries for an index being dropped.
    pub fn drop_for_index(
        &self,
        ulock_keyspace: &OptimisticTxKeyspace,
        index: &IndexDef,
        docs: impl Iterator<Item = Result<Document, StorageError>>,
    ) -> Result<(), StorageError> {
        if !index.unique {
            return Ok(());
        }

        for doc_result in docs {
            let doc = doc_result?;
            if let Some(field_value) = self.get_field_value_for_index(&doc, &index.fields) {
                self.release_lock_key(ulock_keyspace, &index.fields, &field_value, doc.key())?;
            }
        }

        Ok(())
    }

    /// Compute unique lock key: xxh3_128(field_names + "\0" + encoded_value)
    pub(crate) fn compute_unique_lock_key(&self, fields: &str, value: &Value) -> [u8; 16] {
        let mut data = fields.as_bytes().to_vec();
        data.push(0x00);
        data.extend_from_slice(&encode_value_for_hash(value));
        xxh3_128(&data).to_le_bytes()
    }

    /// Get combined field value for an index (handles compound indexes)
    pub(crate) fn get_field_value_for_index(
        &self,
        doc: &Document,
        fields: &[String],
    ) -> Option<Value> {
        if fields.len() == 1 {
            // Single field index
            self.resolve_field_value(&doc.fields, &fields[0])
        } else {
            // Compound index - combine values into an array
            let values: Vec<Value> = fields
                .iter()
                .filter_map(|f| self.resolve_field_value(&doc.fields, f))
                .collect();
            if values.len() == fields.len() {
                Some(Value::Array(values))
            } else {
                None // Missing field(s)
            }
        }
    }

    /// Resolve a dot-separated field path against a document's fields map
    fn resolve_field_value(&self, fields: &HashMap<String, Value>, path: &str) -> Option<Value> {
        if path == "id" {
            // Special case: "id" field refers to document ID, but we don't have it here
            // This should be handled at a higher level
            return None;
        }

        let mut segments = path.splitn(2, '.');
        let root = segments.next()?;
        let value = fields.get(root)?;

        match segments.next() {
            None => Some(value.clone()),
            Some(rest) => {
                let mut current = value;
                for segment in rest.split('.') {
                    match current {
                        Value::Object(map) => current = map.get(segment)?,
                        _ => return None,
                    }
                }
                Some(current.clone())
            }
        }
    }
}

impl Default for UniqueConstraintManager {
    fn default() -> Self {
        Self::new()
    }
}
