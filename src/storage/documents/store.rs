//! DocumentStore - document CRUD operations

use fjall::OptimisticTxKeyspace;
use rkyv::rancor;

use crate::document::Document;
use crate::error::StorageError;
use crate::storage::KeyspaceManager;

/// Handles document storage operations
pub struct DocumentStore {
    keyspaces: KeyspaceManager,
}

impl DocumentStore {
    /// Create a new DocumentStore
    pub fn new(keyspaces: KeyspaceManager) -> Self {
        Self { keyspaces }
    }

    /// Get a document by collection and key
    pub fn get(&self, collection: &str, key: &str) -> Result<Option<Document>, StorageError> {
        let ks = match self.keyspaces.docs_keyspace(collection) {
            Some(ks) => ks,
            None => return Ok(None),
        };

        match ks.get(key.as_bytes())? {
            Some(bytes) => {
                let doc = rkyv::from_bytes::<Document, rancor::Error>(&bytes)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;
                Ok(Some(doc))
            }
            None => Ok(None),
        }
    }

    /// Store a document (raw, without constraint/index handling)
    /// Use Database.set_document for full operation with constraints
    pub fn set_raw(
        &self,
        keyspace: &OptimisticTxKeyspace,
        doc: &Document,
    ) -> Result<(), StorageError> {
        let key = doc.key();
        let bytes = rkyv::to_bytes::<rancor::Error>(doc)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        keyspace.insert(key.as_bytes(), bytes.as_slice())?;
        Ok(())
    }

    /// Delete a document (raw, without constraint/index handling)
    /// Returns true if document existed
    pub fn delete_raw(
        &self,
        keyspace: &OptimisticTxKeyspace,
        key: &str,
    ) -> Result<bool, StorageError> {
        let existed = keyspace.contains_key(key.as_bytes())?;
        if existed {
            keyspace.remove(key.as_bytes())?;
        }
        Ok(existed)
    }

    /// List documents in a collection with cursor-based pagination
    pub fn list(
        &self,
        collection: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Document>, Option<String>), StorageError> {
        let ks = match self.keyspaces.docs_keyspace(collection) {
            Some(ks) => ks,
            None => return Ok((vec![], None)),
        };

        let mut docs = Vec::with_capacity(limit.min(1024));

        let iter = match after {
            Some(cursor) => {
                let start = cursor.as_bytes().to_vec();
                ks.inner().range(start..)
            }
            None => ks.inner().range::<&[u8], _>(..),
        };

        // Skip first item if "after" was provided
        let skip_count = if after.is_some() { 1 } else { 0 };

        for item in iter.skip(skip_count).take(limit.saturating_add(1)) {
            let bytes: fjall::Slice = item.value()?;
            let doc = rkyv::from_bytes::<Document, rancor::Error>(&bytes)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            docs.push(doc);
        }

        let has_more = docs.len() > limit;
        if has_more {
            docs.pop();
        }

        let next_cursor = if has_more {
            docs.last().map(|d| d.key().to_string())
        } else {
            None
        };

        Ok((docs, next_cursor))
    }

    /// Count documents in a collection without deserializing document bodies.
    pub fn count(&self, collection: &str) -> Result<usize, StorageError> {
        let ks = match self.keyspaces.docs_keyspace(collection) {
            Some(ks) => ks,
            None => return Ok(0),
        };

        let mut count = 0;
        for _item in ks.inner().iter() {
            count += 1;
        }
        Ok(count)
    }

    /// Get multiple documents by their IDs
    pub fn get_by_ids(
        &self,
        collection: &str,
        doc_ids: &[String],
    ) -> Result<Vec<Document>, StorageError> {
        let mut docs = Vec::with_capacity(doc_ids.len());

        for doc_id in doc_ids {
            if let Some(doc) = self.get(collection, doc_id)? {
                docs.push(doc);
            }
        }

        Ok(docs)
    }

    /// Iterate over all documents in a collection
    pub fn iter(
        &self,
        collection: &str,
    ) -> Result<impl Iterator<Item = Result<Document, StorageError>>, StorageError> {
        let ks = self
            .keyspaces
            .docs_keyspace(collection)
            .ok_or_else(|| StorageError::CollectionNotLoaded("collection not found".into()))?;

        Ok(ks.inner().iter().map(|item| {
            let (_, value): (fjall::Slice, fjall::Slice) = item.into_inner()?;
            rkyv::from_bytes::<Document, rancor::Error>(&value)
                .map_err(|e| StorageError::Serialization(e.to_string()))
        }))
    }

    /// Iterate over serialized documents. B-tree backfill uses this form so it
    /// can bound reads before decoding and spread decoding across CPU workers.
    pub(crate) fn iter_serialized(
        &self,
        collection: &str,
    ) -> Result<impl Iterator<Item = Result<Vec<u8>, StorageError>>, StorageError> {
        let ks = self
            .keyspaces
            .docs_keyspace(collection)
            .ok_or_else(|| StorageError::CollectionNotLoaded("collection not found".into()))?;

        Ok(ks.inner().iter().map(|item| {
            let (_, value): (fjall::Slice, fjall::Slice) = item.into_inner()?;
            Ok(value.to_vec())
        }))
    }
}
