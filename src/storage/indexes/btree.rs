//! BTreeIndex - B-tree based secondary index implementation

use std::collections::HashMap;

use fjall::{OptimisticTxKeyspace, OptimisticWriteTx};

use crate::document::{Document, Value};
use crate::error::StorageError;
use crate::storage::encoding::*;

use super::traits::{IndexBackend, IndexType, RangeOp, ScanResult};

/// B-tree based index for scalar values.
/// Supports equality and range queries with ordered byte encoding.
pub struct BTreeIndex;

impl BTreeIndex {
    /// Create a new BTreeIndex
    pub fn new() -> Self {
        Self
    }

    /// Build index key for a document given the index fields.
    /// Returns None if any field is missing from the document.
    ///
    /// Format: [value₁][delim]...[0xFF][doc_key]
    pub(crate) fn build_index_key(&self, doc: &Document, fields: &[String]) -> Option<Vec<u8>> {
        let mut values = Vec::new();

        for field in fields {
            let value = if field == "id" {
                Some(Value::String(doc.id.clone()))
            } else if field.contains('.') {
                resolve_field_value(&doc.fields, field)
            } else {
                doc.fields.get(field).cloned()
            };

            values.push(value?);
        }

        let value_refs: Vec<&Value> = values.iter().collect();
        Some(encode_index_key(&value_refs, doc.key()))
    }

    /// Update index entries within a transaction.
    /// This ensures index updates are atomic with document changes.
    pub fn update_in_tx(
        &self,
        tx: &mut OptimisticWriteTx,
        keyspace: &OptimisticTxKeyspace,
        fields: &[String],
        old_doc: Option<&Document>,
        new_doc: Option<&Document>,
    ) -> Result<(), StorageError> {
        // Remove old index entry using tx.remove()
        if let Some(old) = old_doc
            && let Some(old_key) = self.build_index_key(old, fields)
        {
            tx.remove(keyspace, &old_key);
        }

        // Add new index entry using tx.insert()
        if let Some(new) = new_doc
            && let Some(new_key) = self.build_index_key(new, fields)
        {
            tx.insert(keyspace, &new_key, b"");
        }

        Ok(())
    }
}

impl Default for BTreeIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl IndexBackend for BTreeIndex {
    fn name(&self) -> &'static str {
        "btree"
    }

    fn index_type(&self) -> IndexType {
        IndexType::BTree
    }

    fn index_document(
        &self,
        keyspace: &OptimisticTxKeyspace,
        fields: &[String],
        doc: &Document,
    ) -> Result<(), StorageError> {
        if let Some(index_key) = self.build_index_key(doc, fields) {
            keyspace.insert(&index_key, b"")?;
        }
        Ok(())
    }

    fn update_document(
        &self,
        keyspace: &OptimisticTxKeyspace,
        fields: &[String],
        old_doc: Option<&Document>,
        new_doc: Option<&Document>,
    ) -> Result<(), StorageError> {
        // Remove old index entry
        if let Some(old) = old_doc
            && let Some(old_key) = self.build_index_key(old, fields)
        {
            keyspace.remove(&old_key)?;
        }

        // Add new index entry
        if let Some(new) = new_doc
            && let Some(new_key) = self.build_index_key(new, fields)
        {
            keyspace.insert(&new_key, b"")?;
        }

        Ok(())
    }

    fn scan_eq(
        &self,
        keyspace: &OptimisticTxKeyspace,
        _field: &str,
        value: &Value,
        limit: usize,
    ) -> Result<ScanResult, StorageError> {
        // Build prefix: [encoded_value][delimiter]
        let prefix = encode_index_prefix(&[value]);

        let mut doc_ids = Vec::new();

        for item in keyspace.inner().prefix(&prefix) {
            let (key, _): (fjall::Slice, fjall::Slice) = item.into_inner()?;
            // doc_key is after the 0xFF separator
            if let Some(doc_id) = extract_doc_key(&key) {
                doc_ids.push(doc_id);
                if doc_ids.len() > limit {
                    return Ok(ScanResult::new(doc_ids, true));
                }
            }
        }

        Ok(ScanResult::new(doc_ids, false))
    }

    fn scan_range(
        &self,
        keyspace: &OptimisticTxKeyspace,
        field: &str,
        op: &RangeOp,
        value: &Value,
        limit: usize,
    ) -> Result<ScanResult, StorageError> {
        // Handle Between separately
        if let RangeOp::Between {
            lower,
            upper,
            lower_inclusive,
            upper_inclusive,
        } = op
        {
            return self.scan_range_between(
                keyspace,
                field,
                lower,
                *lower_inclusive,
                upper,
                *upper_inclusive,
                limit,
            );
        }

        let query_encoded = encode_value_with_delimiter(value);
        let mut doc_ids = Vec::new();

        // Use range bounds to avoid O(N) full keyspace iteration.
        // Key format: [encoded_value][0xFF][doc_key]
        // We compute tight range bounds based on the operator.
        let iter: Box<dyn Iterator<Item = _>> = match op {
            // Gt/Gte: start from the encoded value, scan forward
            RangeOp::Gt | RangeOp::Gte => Box::new(keyspace.inner().range(query_encoded.clone()..)),
            // Lt: all keys with value < V. Keys with value V start with encode(V)
            // so ..encode(V) excludes them (correct for strict Lt).
            RangeOp::Lt => Box::new(
                keyspace
                    .inner()
                    .range::<Vec<u8>, _>(..query_encoded.clone()),
            ),
            // Lte: include keys with value == V. Their full keys are
            // [encode(V)][0xFF][doc_key] which are > encode(V), so we need
            // a bound past them. byte_successor(encode(V)) works because
            // all keys starting with encode(V) sort before successor(encode(V)).
            RangeOp::Lte => {
                let end = byte_successor(&query_encoded);
                Box::new(keyspace.inner().range::<Vec<u8>, _>(..end))
            }
            RangeOp::Between { .. } => unreachable!(),
        };

        for item in iter {
            let (key, _): (fjall::Slice, fjall::Slice) = item.into_inner()?;

            let separator_pos = match key.iter().rposition(|&b| b == 0xFF) {
                Some(pos) => pos,
                None => continue,
            };

            let key_value = &key[..separator_pos];

            // Lightweight boundary check — the range already handles most filtering,
            // but we need exact comparisons at the boundary.
            let matches = match op {
                RangeOp::Gt => key_value > query_encoded.as_slice(),
                RangeOp::Gte => true, // range start is inclusive, all keys match
                RangeOp::Lt => true,  // range end is exclusive at encode(V), all keys match
                RangeOp::Lte => true, // range end is exclusive at successor(encode(V)), all match
                RangeOp::Between { .. } => unreachable!(),
            };

            if matches && let Some(doc_id) = extract_doc_key(&key) {
                doc_ids.push(doc_id);
                if doc_ids.len() > limit {
                    return Ok(ScanResult::new(doc_ids, true));
                }
            }
        }

        Ok(ScanResult::new(doc_ids, false))
    }

    fn scan_range_between(
        &self,
        keyspace: &OptimisticTxKeyspace,
        _field: &str,
        lower: &Value,
        lower_inclusive: bool,
        upper: &Value,
        upper_inclusive: bool,
        limit: usize,
    ) -> Result<ScanResult, StorageError> {
        let lower_encoded = encode_value_with_delimiter(lower);
        let upper_encoded = encode_value_with_delimiter(upper);
        let mut doc_ids = Vec::new();

        // Compute range bounds to avoid full keyspace iteration.
        // Start: encoded lower value (keys with this value are >= start)
        // End: successor of upper value to include keys with upper value,
        //      or exact upper to exclude them.
        let range_end = if upper_inclusive {
            byte_successor(&upper_encoded)
        } else {
            // For exclusive upper, keys with value == upper start with
            // encode(upper) and their full keys are > encode(upper),
            // so ending at encode(upper) excludes them.
            upper_encoded.clone()
        };

        for item in keyspace.inner().range(lower_encoded.clone()..range_end) {
            let (key, _): (fjall::Slice, fjall::Slice) = item.into_inner()?;

            let separator_pos = match key.iter().rposition(|&b| b == 0xFF) {
                Some(pos) => pos,
                None => continue,
            };

            let key_value = &key[..separator_pos];

            // Check lower bound at boundary
            let lower_ok = if lower_inclusive {
                true // range start is inclusive
            } else {
                key_value > lower_encoded.as_slice()
            };

            if lower_ok && let Some(doc_id) = extract_doc_key(&key) {
                doc_ids.push(doc_id);
                if doc_ids.len() > limit {
                    return Ok(ScanResult::new(doc_ids, true));
                }
            }
        }

        Ok(ScanResult::new(doc_ids, false))
    }

    fn scan_compound_eq(
        &self,
        keyspace: &OptimisticTxKeyspace,
        field_values: &[(&str, &Value)],
        limit: usize,
    ) -> Result<ScanResult, StorageError> {
        // Build prefix from all values
        let values: Vec<&Value> = field_values.iter().map(|(_, v)| *v).collect();
        let prefix = encode_index_prefix(&values);

        let mut doc_ids = Vec::new();

        for item in keyspace.inner().prefix(&prefix) {
            let (key, _): (fjall::Slice, fjall::Slice) = item.into_inner()?;
            // Extract doc_key (after 0xFF separator)
            if let Some(doc_key) = extract_doc_key(&key) {
                doc_ids.push(doc_key);
                if doc_ids.len() > limit {
                    return Ok(ScanResult::new(doc_ids, true));
                }
            }
        }

        Ok(ScanResult::new(doc_ids, false))
    }
}

/// Resolve a dot-separated field path against a document's fields map.
fn resolve_field_value(fields: &HashMap<String, Value>, path: &str) -> Option<Value> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that the key format correctly handles compound indexes with bool values.
    #[test]
    fn test_build_index_key_with_bool() {
        let btree = BTreeIndex::new();

        // Create a document with bool field
        let mut fields = HashMap::new();
        fields.insert("active".to_string(), Value::Bool(true));
        fields.insert("count".to_string(), Value::Int(42));

        let doc = Document {
            id: "test:doc1".to_string(),
            fields,
        };

        let index_fields = vec!["active".to_string(), "count".to_string()];
        let key = btree.build_index_key(&doc, &index_fields).unwrap();

        // Verify key structure: [value₁][delim][value₂][delim][0xFF][doc_key]
        let bool_encoded = encode_value_with_delimiter(&Value::Bool(true));

        // Check structure
        assert!(
            key.starts_with(&bool_encoded),
            "Key should start with bool value"
        );
        assert!(key.ends_with(b"doc1"), "Key should end with doc_key");

        // Verify the key can be parsed back
        let doc_key = extract_doc_key(&key).unwrap();
        assert_eq!(doc_key, "doc1");
    }

    /// Test that the key format correctly handles positive integers.
    #[test]
    fn test_build_index_key_with_positive_int() {
        let btree = BTreeIndex::new();

        let mut fields = HashMap::new();
        fields.insert("age".to_string(), Value::Int(25));

        let doc = Document {
            id: "user:alice".to_string(),
            fields,
        };

        let index_fields = vec!["age".to_string()];
        let key = btree.build_index_key(&doc, &index_fields).unwrap();

        // Parse the key
        let doc_key = extract_doc_key(&key).unwrap();
        assert_eq!(doc_key, "alice");
    }

    /// Test that key format works with nested field paths.
    #[test]
    fn test_build_index_key_with_nested_field() {
        let btree = BTreeIndex::new();

        let mut inner = HashMap::new();
        inner.insert("city".to_string(), Value::String("NYC".to_string()));

        let mut fields = HashMap::new();
        fields.insert("address".to_string(), Value::Object(inner));

        let doc = Document {
            id: "user:bob".to_string(),
            fields,
        };

        let index_fields = vec!["address.city".to_string()];
        let key = btree.build_index_key(&doc, &index_fields).unwrap();

        // Parse the key
        let doc_key = extract_doc_key(&key).unwrap();
        assert_eq!(doc_key, "bob");
    }

    /// Test that missing fields return None.
    #[test]
    fn test_build_index_key_missing_field() {
        let btree = BTreeIndex::new();

        let fields = HashMap::new();
        let doc = Document {
            id: "test:doc1".to_string(),
            fields,
        };

        let index_fields = vec!["missing".to_string()];
        let key = btree.build_index_key(&doc, &index_fields);

        assert!(key.is_none(), "Missing field should return None");
    }
}
