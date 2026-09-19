//! EdgeStore - Edge storage with keyspace-per-label architecture.
//!
//! Key features:
//! - Separate keyspace for each edge type (label)
//! - Support for Single/Multiple cardinality
//! - Support for Directed/Undirected edges
//! - Field indexes for efficient queries
//! - Compact storage for edges without fields

use std::collections::HashMap;
use std::sync::Arc;

use fjall::{
    KeyspaceCreateOptions, OptimisticTxDatabase, OptimisticTxKeyspace, OptimisticWriteTx, Readable,
};
use parking_lot::RwLock;
use rkyv::rancor;

use crate::edge::{
    EDGE_FORMAT_COMPACT, EDGE_FORMAT_FULL, Edge, EdgeCardinality, EdgeData, EdgeSchema,
    OnDeletePolicy,
};
use crate::error::StorageError;
use crate::schema::validate_name;
use crate::storage::encoding::*;

/// Keyspace name prefix for edge types
const EDGE_KEYSPACE_PREFIX: &str = "_edge:";

/// Keyspace name for edge metadata
const EDGE_META_KEYSPACE: &str = "_edge_meta";

/// Key for storing list of all edge labels
const META_LABELS_KEY: &[u8] = b"labels";

/// Prefix for schema storage
const META_SCHEMA_PREFIX: &str = "schema:";

// =============================================================================
// EdgeStore
// =============================================================================

/// Edge storage with keyspace-per-label architecture.
///
/// Each edge type (label) gets its own keyspace for optimal performance
/// and isolation. Supports both directed and undirected edges, as well
/// as single and multiple cardinality.
pub struct EdgeStore {
    /// Reference to the database
    db: Arc<OptimisticTxDatabase>,

    /// Metadata keyspace (schemas, labels list)
    meta: OptimisticTxKeyspace,

    /// Cached schemas (label -> schema)
    schemas: RwLock<HashMap<String, EdgeSchema>>,

    /// Cached keyspaces (label -> keyspace)
    keyspaces: RwLock<HashMap<String, OptimisticTxKeyspace>>,
}

#[allow(dead_code)] // Public API surface — methods will be wired to HTTP/gRPC handlers
impl EdgeStore {
    /// Create a new EdgeStore with the given database.
    pub fn new(db: Arc<OptimisticTxDatabase>) -> Result<Self, StorageError> {
        // Open or create metadata keyspace
        let meta = db.keyspace(EDGE_META_KEYSPACE, KeyspaceCreateOptions::default)?;

        let store = Self {
            db,
            meta,
            schemas: RwLock::new(HashMap::new()),
            keyspaces: RwLock::new(HashMap::new()),
        };

        // Load existing schemas
        store.load_schemas()?;

        Ok(store)
    }

    /// Load all schemas from metadata keyspace and open their keyspaces.
    fn load_schemas(&self) -> Result<(), StorageError> {
        let prefix = META_SCHEMA_PREFIX.as_bytes();
        let mut labels_to_open = Vec::new();

        {
            let mut schemas = self.schemas.write();

            for item in self.meta.inner().prefix(prefix) {
                let (key_bytes, value_bytes) = item.into_inner()?;
                let key = String::from_utf8_lossy(&key_bytes).to_string();
                if let Some(label) = key.strip_prefix(META_SCHEMA_PREFIX) {
                    let schema = rkyv::from_bytes::<EdgeSchema, rancor::Error>(&value_bytes)
                        .map_err(|e| StorageError::Serialization(e.to_string()))?;
                    schemas.insert(label.to_string(), schema.clone());
                    labels_to_open.push(label.to_string());
                }
            }
        }

        // Open keyspaces for all loaded edge types
        for label in labels_to_open {
            self.get_or_create_keyspace(&label)?;
        }

        Ok(())
    }

    /// Get or create keyspace for a label.
    fn get_or_create_keyspace(&self, label: &str) -> Result<OptimisticTxKeyspace, StorageError> {
        // Check cache first
        {
            let keyspaces = self.keyspaces.read();
            if let Some(ks) = keyspaces.get(label) {
                return Ok(ks.clone());
            }
        }

        // Create new keyspace
        let keyspace_name = format!("{}{}", EDGE_KEYSPACE_PREFIX, label);
        let ks = self
            .db
            .keyspace(&keyspace_name, KeyspaceCreateOptions::default)?;

        // Cache it
        {
            let mut keyspaces = self.keyspaces.write();
            keyspaces.insert(label.to_string(), ks.clone());
        }

        Ok(ks)
    }

    /// Get keyspace for a label (must exist).
    fn get_keyspace(&self, label: &str) -> Result<OptimisticTxKeyspace, StorageError> {
        let keyspaces = self.keyspaces.read();
        keyspaces
            .get(label)
            .cloned()
            .ok_or_else(|| StorageError::NotFound(format!("Edge type '{}' not defined", label)))
    }

    // =========================================================================
    // Schema Management
    // =========================================================================

    /// Define a new edge type with schema.
    pub fn define_edge_type(&self, schema: EdgeSchema) -> Result<(), StorageError> {
        let label = &schema.name;

        // Validate edge type name
        validate_name("edge type", label)
            .map_err(|e| StorageError::InvalidOperation(e.to_string()))?;

        // Check if already exists
        {
            let schemas = self.schemas.read();
            if schemas.contains_key(label) {
                return Err(StorageError::AlreadyExists(format!(
                    "Edge type '{}' already exists",
                    label
                )));
            }
        }

        // Serialize schema
        let schema_bytes = rkyv::to_bytes::<rancor::Error>(&schema)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        // Store in metadata
        let schema_key = format!("{}{}", META_SCHEMA_PREFIX, label);
        self.meta
            .insert(schema_key.as_bytes(), schema_bytes.as_slice())?;

        // Update labels list
        self.add_label_to_list(label)?;

        // Create keyspace
        self.get_or_create_keyspace(label)?;

        // Cache schema
        {
            let mut schemas = self.schemas.write();
            schemas.insert(label.to_string(), schema);
        }

        Ok(())
    }

    /// Get schema for an edge type.
    pub fn get_schema(&self, label: &str) -> Option<EdgeSchema> {
        let schemas = self.schemas.read();
        schemas.get(label).cloned()
    }

    /// List all defined edge types.
    pub fn list_edge_types(&self) -> Vec<String> {
        let schemas = self.schemas.read();
        schemas.keys().cloned().collect()
    }

    /// Drop an edge type and all its edges.
    pub fn drop_edge_type(&self, label: &str) -> Result<(), StorageError> {
        // Remove from cache
        {
            let mut schemas = self.schemas.write();
            schemas.remove(label);
        }
        {
            let mut keyspaces = self.keyspaces.write();
            keyspaces.remove(label);
        }

        // Remove schema from metadata
        let schema_key = format!("{}{}", META_SCHEMA_PREFIX, label);
        self.meta.remove(schema_key.as_bytes())?;

        // Remove from labels list
        self.remove_label_from_list(label)?;

        // Note: The keyspace itself will be dropped when no longer referenced
        // In production, you might want to explicitly delete the keyspace

        Ok(())
    }

    /// Add a label to the labels list.
    fn add_label_to_list(&self, label: &str) -> Result<(), StorageError> {
        let mut labels = self.get_labels_list()?;
        if !labels.contains(&label.to_string()) {
            labels.push(label.to_string());
            self.save_labels_list(&labels)?;
        }
        Ok(())
    }

    /// Remove a label from the labels list.
    fn remove_label_from_list(&self, label: &str) -> Result<(), StorageError> {
        let mut labels = self.get_labels_list()?;
        labels.retain(|l| l != label);
        self.save_labels_list(&labels)?;
        Ok(())
    }

    /// Get the labels list from metadata.
    fn get_labels_list(&self) -> Result<Vec<String>, StorageError> {
        match self.meta.get(META_LABELS_KEY)? {
            Some(bytes) => {
                let labels: Vec<String> = rkyv::from_bytes::<Vec<String>, rancor::Error>(&bytes)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;
                Ok(labels)
            }
            None => Ok(Vec::new()),
        }
    }

    /// Save the labels list to metadata.
    fn save_labels_list(&self, labels: &[String]) -> Result<(), StorageError> {
        // Convert to Vec for serialization
        let labels_vec: Vec<String> = labels.to_vec();
        let bytes = rkyv::to_bytes::<rancor::Error>(&labels_vec)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        self.meta.insert(META_LABELS_KEY, bytes.as_slice())?;
        Ok(())
    }

    // =========================================================================
    // CRUD Operations
    // =========================================================================

    /// Create a new edge.
    ///
    /// For Single cardinality: overwrites existing edge between same nodes.
    /// For Multiple cardinality: always creates a new edge.
    ///
    /// If the edge type is not defined, it will be auto-created with default schema
    /// (Directed, Single cardinality).
    pub fn create(&self, edge: &Edge) -> Result<(), StorageError> {
        // Auto-create edge type if not exists
        let schema = match self.get_schema(&edge.label) {
            Some(s) => s,
            None => {
                let default_schema = EdgeSchema::new(&edge.label);
                self.define_edge_type(default_schema.clone())?;
                default_schema
            }
        };

        let ks = self.get_keyspace(&edge.label)?;
        let edge_data = EdgeData::from_edge(edge);
        let data_bytes = self.serialize_edge_data(&edge_data)?;
        let edge_id = edge.nanoid();

        // Determine from/to based on direction
        let (from, to) = if schema.is_undirected() {
            // For undirected, ensure consistent ordering
            if edge.from <= edge.to {
                (&edge.from, &edge.to)
            } else {
                (&edge.to, &edge.from)
            }
        } else {
            (&edge.from, &edge.to)
        };

        match schema.cardinality {
            EdgeCardinality::Single => {
                // Single: use from|to as key, overwrite existing
                let out_key = encode_edge_out_key_single(from, to);
                ks.insert(&out_key, &data_bytes)?;

                if schema.is_undirected() {
                    // For undirected: also store reverse direction in OUT index
                    let reverse_out_key = encode_edge_out_key_single(to, from);
                    ks.insert(&reverse_out_key, &data_bytes)?;
                } else {
                    // IN index (only for directed edges)
                    let in_key = encode_edge_in_key_single(to, from);
                    ks.insert(&in_key, edge_id.as_bytes())?;
                }

                // ID lookup
                let id_key = encode_edge_id_key(edge_id);
                let id_value = encode_edge_id_value(from, to);
                ks.insert(&id_key, &id_value)?;
            }
            EdgeCardinality::Multiple => {
                // Multiple: include edge_id in key
                let out_key = encode_edge_out_key_multi(from, to, edge_id);
                ks.insert(&out_key, &data_bytes)?;

                if schema.is_undirected() {
                    // For undirected: also store reverse direction in OUT index
                    let reverse_out_key = encode_edge_out_key_multi(to, from, edge_id);
                    ks.insert(&reverse_out_key, &data_bytes)?;
                } else {
                    // IN index
                    let in_key = encode_edge_in_key_multi(to, from, edge_id);
                    ks.insert(&in_key, [])?; // Empty value, just for index
                }

                // ID lookup
                let id_key = encode_edge_id_key(edge_id);
                let id_value = encode_edge_id_value(from, to);
                ks.insert(&id_key, &id_value)?;
            }
        }

        // Update field indexes
        self.update_field_indexes(&ks, &schema, edge_id, &edge_data.fields, true)?;

        Ok(())
    }

    /// Create an edge within an existing transaction.
    /// This allows atomic edge creation with document operations.
    pub fn create_in_tx(
        &self,
        tx: &mut OptimisticWriteTx,
        edge: &Edge,
    ) -> Result<(), StorageError> {
        // Auto-create edge type if not exists
        let schema = match self.get_schema(&edge.label) {
            Some(s) => s,
            None => {
                let default_schema = EdgeSchema::new(&edge.label);
                self.define_edge_type(default_schema.clone())?;
                default_schema
            }
        };

        let ks = self.get_keyspace(&edge.label)?;
        let edge_data = EdgeData::from_edge(edge);
        let data_bytes = self.serialize_edge_data(&edge_data)?;
        let edge_id = edge.nanoid();

        // Determine from/to based on direction
        let (from, to) = if schema.is_undirected() {
            if edge.from <= edge.to {
                (&edge.from, &edge.to)
            } else {
                (&edge.to, &edge.from)
            }
        } else {
            (&edge.from, &edge.to)
        };

        match schema.cardinality {
            EdgeCardinality::Single => {
                let out_key = encode_edge_out_key_single(from, to);
                tx.insert(&ks, &out_key, &data_bytes);

                if schema.is_undirected() {
                    let reverse_out_key = encode_edge_out_key_single(to, from);
                    tx.insert(&ks, &reverse_out_key, &data_bytes);
                } else {
                    let in_key = encode_edge_in_key_single(to, from);
                    tx.insert(&ks, &in_key, edge_id.as_bytes());
                }

                let id_key = encode_edge_id_key(edge_id);
                let id_value = encode_edge_id_value(from, to);
                tx.insert(&ks, &id_key, &id_value);
            }
            EdgeCardinality::Multiple => {
                let out_key = encode_edge_out_key_multi(from, to, edge_id);
                tx.insert(&ks, &out_key, &data_bytes);

                if schema.is_undirected() {
                    let reverse_out_key = encode_edge_out_key_multi(to, from, edge_id);
                    tx.insert(&ks, &reverse_out_key, &data_bytes);
                } else {
                    let in_key = encode_edge_in_key_multi(to, from, edge_id);
                    tx.insert(&ks, &in_key, []);
                }

                let id_key = encode_edge_id_key(edge_id);
                let id_value = encode_edge_id_value(from, to);
                tx.insert(&ks, &id_key, &id_value);
            }
        }

        // Note: Field indexes are updated outside transaction for simplicity.
        // This is acceptable as they are secondary indexes.

        Ok(())
    }

    /// Get an edge by its ID (nanoid part).
    pub fn get_by_id(&self, label: &str, edge_id: &str) -> Result<Option<Edge>, StorageError> {
        let schema = self
            .get_schema(label)
            .ok_or_else(|| StorageError::NotFound(format!("Edge type '{}' not defined", label)))?;

        let ks = self.get_keyspace(label)?;

        // Look up from/to by ID
        let id_key = encode_edge_id_key(edge_id);
        let id_value = match ks.get(&id_key)? {
            Some(v) => v,
            None => return Ok(None),
        };

        let (from, to) = decode_edge_id_value(&id_value)
            .ok_or_else(|| StorageError::Serialization("Invalid ID value format".into()))?;

        // Get edge data
        let out_key = match schema.cardinality {
            EdgeCardinality::Single => encode_edge_out_key_single(&from, &to),
            EdgeCardinality::Multiple => encode_edge_out_key_multi(&from, &to, edge_id),
        };

        match ks.get(&out_key)? {
            Some(bytes) => {
                let edge_data = self.deserialize_edge_data(&bytes)?;
                Ok(Some(edge_data.to_edge(label, &from, &to)))
            }
            None => Ok(None),
        }
    }

    /// Get an edge by from/to (only works for Single cardinality).
    pub fn get(&self, from: &str, label: &str, to: &str) -> Result<Option<Edge>, StorageError> {
        let schema = match self.get_schema(label) {
            Some(s) => s,
            None => return Ok(None), // Edge type not defined = no edges
        };

        if schema.is_multiple() {
            return Err(StorageError::InvalidOperation(
                "Use get_by_id or list_out for Multiple cardinality edges".into(),
            ));
        }

        let ks = self.get_keyspace(label)?;

        // Normalize order for undirected
        let (from, to) = if schema.is_undirected() && from > to {
            (to, from)
        } else {
            (from, to)
        };

        let out_key = encode_edge_out_key_single(from, to);

        match ks.get(&out_key)? {
            Some(bytes) => {
                let edge_data = self.deserialize_edge_data(&bytes)?;
                Ok(Some(edge_data.to_edge(label, from, to)))
            }
            None => Ok(None),
        }
    }

    /// Delete an edge by ID.
    pub fn delete_by_id(&self, label: &str, edge_id: &str) -> Result<bool, StorageError> {
        let schema = match self.get_schema(label) {
            Some(s) => s,
            None => return Ok(false), // Edge type not defined = nothing to delete
        };

        let ks = self.get_keyspace(label)?;

        // Look up from/to by ID
        let id_key = encode_edge_id_key(edge_id);
        let id_value = match ks.get(&id_key)? {
            Some(v) => v,
            None => return Ok(false),
        };

        let (from, to) = decode_edge_id_value(&id_value)
            .ok_or_else(|| StorageError::Serialization("Invalid ID value format".into()))?;

        // Get edge data for field index cleanup
        let out_key = match schema.cardinality {
            EdgeCardinality::Single => encode_edge_out_key_single(&from, &to),
            EdgeCardinality::Multiple => encode_edge_out_key_multi(&from, &to, edge_id),
        };

        let edge_data = match ks.get(&out_key)? {
            Some(bytes) => Some(self.deserialize_edge_data(&bytes)?),
            None => None,
        };

        // Delete OUT
        ks.remove(&out_key)?;

        if schema.is_undirected() {
            // Delete reverse OUT for undirected edges
            let reverse_out_key = match schema.cardinality {
                EdgeCardinality::Single => encode_edge_out_key_single(&to, &from),
                EdgeCardinality::Multiple => encode_edge_out_key_multi(&to, &from, edge_id),
            };
            ks.remove(&reverse_out_key)?;
        } else {
            // Delete IN (for directed)
            let in_key = match schema.cardinality {
                EdgeCardinality::Single => encode_edge_in_key_single(&to, &from),
                EdgeCardinality::Multiple => encode_edge_in_key_multi(&to, &from, edge_id),
            };
            ks.remove(&in_key)?;
        }

        // Delete ID lookup
        ks.remove(&id_key)?;

        // Clean up field indexes
        if let Some(data) = edge_data {
            self.update_field_indexes(&ks, &schema, edge_id, &data.fields, false)?;
        }

        Ok(true)
    }

    /// Delete edge(s) by from/to.
    ///
    /// For Single cardinality: deletes the one edge.
    /// For Multiple cardinality: deletes all edges between the nodes.
    pub fn delete(&self, from: &str, label: &str, to: &str) -> Result<usize, StorageError> {
        let schema = match self.get_schema(label) {
            Some(s) => s,
            None => return Ok(0), // Edge type not defined = nothing to delete
        };

        let ks = self.get_keyspace(label)?;

        // Normalize for undirected
        let (from, to) = if schema.is_undirected() && from > to {
            (to, from)
        } else {
            (from, to)
        };

        match schema.cardinality {
            EdgeCardinality::Single => {
                let out_key = encode_edge_out_key_single(from, to);
                if ks.contains_key(&out_key)? {
                    // Get edge data for cleanup
                    if let Some(bytes) = ks.get(&out_key)? {
                        let data = self.deserialize_edge_data(&bytes)?;
                        self.update_field_indexes(&ks, &schema, &data.id, &data.fields, false)?;

                        // Delete ID lookup
                        let id_key = encode_edge_id_key(&data.id);
                        ks.remove(&id_key)?;
                    }

                    ks.remove(&out_key)?;

                    if schema.is_undirected() {
                        // Delete reverse OUT for undirected
                        let reverse_out_key = encode_edge_out_key_single(to, from);
                        ks.remove(&reverse_out_key)?;
                    } else {
                        let in_key = encode_edge_in_key_single(to, from);
                        ks.remove(&in_key)?;
                    }

                    Ok(1)
                } else {
                    Ok(0)
                }
            }
            EdgeCardinality::Multiple => {
                // Scan all edges between from and to
                let prefix = encode_edge_out_target_prefix(from, to);
                let prefix_end = encode_edge_out_target_prefix_end(from, to);

                let mut count = 0;
                let mut to_delete = Vec::new();

                for item in ks.inner().range(prefix..prefix_end) {
                    let (_key_bytes, value_bytes) = item.into_inner()?;
                    let data = self.deserialize_edge_data(&value_bytes)?;
                    to_delete.push((data.id.clone(), data.fields.clone()));
                    count += 1;
                }

                // Delete each edge
                for (edge_id, fields) in to_delete {
                    let out_key = encode_edge_out_key_multi(from, to, &edge_id);
                    ks.remove(&out_key)?;

                    if schema.is_undirected() {
                        // Delete reverse OUT for undirected
                        let reverse_out_key = encode_edge_out_key_multi(to, from, &edge_id);
                        ks.remove(&reverse_out_key)?;
                    } else {
                        let in_key = encode_edge_in_key_multi(to, from, &edge_id);
                        ks.remove(&in_key)?;
                    }

                    let id_key = encode_edge_id_key(&edge_id);
                    ks.remove(&id_key)?;

                    self.update_field_indexes(&ks, &schema, &edge_id, &fields, false)?;
                }

                Ok(count)
            }
        }
    }

    /// Delete edge(s) by from/to within an existing transaction.
    ///
    /// This mirrors `delete()` but writes removals into the caller's Fjall
    /// transaction, so document and edge batch operations commit or fail as a
    /// single unit.
    pub fn delete_in_tx(
        &self,
        tx: &mut OptimisticWriteTx,
        from: &str,
        label: &str,
        to: &str,
    ) -> Result<usize, StorageError> {
        let schema = match self.get_schema(label) {
            Some(s) => s,
            None => return Ok(0),
        };

        let ks = self.get_keyspace(label)?;

        let (from, to) = if schema.is_undirected() && from > to {
            (to, from)
        } else {
            (from, to)
        };

        match schema.cardinality {
            EdgeCardinality::Single => {
                let out_key = encode_edge_out_key_single(from, to);
                let Some(bytes) = tx.get(&ks, &out_key)? else {
                    return Ok(0);
                };

                let data = self.deserialize_edge_data(&bytes)?;
                self.delete_edge_keys_in_tx(tx, &ks, &schema, from, to, &data.id)?;
                Ok(1)
            }
            EdgeCardinality::Multiple => {
                let prefix = encode_edge_out_target_prefix(from, to);
                let prefix_end = encode_edge_out_target_prefix_end(from, to);

                let mut to_delete = Vec::new();
                for item in ks.inner().range(prefix..prefix_end) {
                    let (_key_bytes, value_bytes) = item.into_inner()?;
                    let data = self.deserialize_edge_data(&value_bytes)?;
                    to_delete.push(data.id.clone());
                }

                let count = to_delete.len();
                for edge_id in to_delete {
                    self.delete_edge_keys_in_tx(tx, &ks, &schema, from, to, &edge_id)?;
                }

                Ok(count)
            }
        }
    }

    // =========================================================================
    // List Operations
    // =========================================================================

    /// List outgoing edges from a node.
    pub fn list_out(
        &self,
        from: &str,
        label: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Edge>, Option<String>), StorageError> {
        // If edge type not defined, return empty list
        if self.get_schema(label).is_none() {
            return Ok((Vec::new(), None));
        }

        let ks = self.get_keyspace(label)?;
        let limit = limit.clamp(1, 1000);

        let start = encode_edge_out_prefix_v2(from);
        let end = encode_edge_out_prefix_end_v2(from);

        let mut edges = Vec::with_capacity(limit);
        let mut skip_until = after.map(|s| s.to_string());

        for item in ks.inner().range(start..end) {
            let (key_bytes, value_bytes) = item.into_inner()?;

            // Parse key to get 'to'
            let key = &key_bytes[1..]; // Skip prefix
            let to = self.parse_out_key_target(key)?;

            let data = self.deserialize_edge_data(&value_bytes)?;

            // Skip entries until we pass the cursor
            if let Some(ref cursor) = skip_until {
                if data.id == *cursor {
                    skip_until = None;
                }
                continue;
            }

            if edges.len() > limit {
                break;
            }

            edges.push(data.to_edge(label, from, &to));
        }

        let has_more = edges.len() > limit;
        if has_more {
            edges.pop();
        }

        let next_cursor = if has_more {
            edges.last().map(|e| e.nanoid().to_string())
        } else {
            None
        };

        Ok((edges, next_cursor))
    }

    /// List incoming edges to a node.
    pub fn list_in(
        &self,
        to: &str,
        label: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Edge>, Option<String>), StorageError> {
        let schema = match self.get_schema(label) {
            Some(s) => s,
            None => return Ok((Vec::new(), None)), // Edge type not defined = no edges
        };

        if schema.is_undirected() {
            // For undirected, list_in is the same as list_out
            return self.list_out(to, label, after, limit);
        }

        let ks = self.get_keyspace(label)?;
        let limit = limit.clamp(1, 1000);

        let start = encode_edge_in_prefix_v2(to);
        let end = encode_edge_in_prefix_end_v2(to);

        let mut edges = Vec::with_capacity(limit);
        let mut skip_until = after.map(|s| s.to_string());

        for item in ks.inner().range(start..end) {
            let (key_bytes, value_bytes) = item.into_inner()?;

            // For IN index, value contains edge_id (for single) or empty (for multi with id in key)
            let (from, edge_id) =
                self.parse_in_key_and_value(&key_bytes[1..], &value_bytes, &schema)?;

            // Skip until cursor
            if let Some(ref cursor) = skip_until {
                if edge_id == *cursor {
                    skip_until = None;
                }
                continue;
            }

            if edges.len() > limit {
                break;
            }

            // Get full edge data from OUT index
            let out_key = match schema.cardinality {
                EdgeCardinality::Single => encode_edge_out_key_single(&from, to),
                EdgeCardinality::Multiple => encode_edge_out_key_multi(&from, to, &edge_id),
            };

            if let Some(bytes) = ks.get(&out_key)? {
                let data = self.deserialize_edge_data(&bytes)?;
                edges.push(data.to_edge(label, &from, to));
            }
        }

        let has_more = edges.len() > limit;
        if has_more {
            edges.pop();
        }

        let next_cursor = if has_more {
            edges.last().map(|e| e.nanoid().to_string())
        } else {
            None
        };

        Ok((edges, next_cursor))
    }

    /// List all outgoing edges from a node (all labels).
    pub fn list_all_out(&self, from: &str, limit: usize) -> Result<Vec<Edge>, StorageError> {
        let labels = self.list_edge_types();
        let mut all_edges = Vec::new();

        for label in labels {
            let (edges, _) = self.list_out(from, &label, None, limit)?;
            all_edges.extend(edges);

            if all_edges.len() >= limit {
                all_edges.truncate(limit);
                break;
            }
        }

        Ok(all_edges)
    }

    /// List all incoming edges to a node (all labels).
    pub fn list_all_in(&self, to: &str, limit: usize) -> Result<Vec<Edge>, StorageError> {
        let labels = self.list_edge_types();
        let mut all_edges = Vec::new();

        for label in labels {
            let (edges, _) = self.list_in(to, &label, None, limit)?;
            all_edges.extend(edges);

            if all_edges.len() >= limit {
                all_edges.truncate(limit);
                break;
            }
        }

        Ok(all_edges)
    }

    // =========================================================================
    // Batch Operations
    // =========================================================================

    /// Create multiple edges in a batch.
    pub fn create_batch(&self, edges: &[Edge]) -> Result<(), StorageError> {
        // Group edges by label for efficiency
        let mut by_label: HashMap<&str, Vec<&Edge>> = HashMap::new();
        for edge in edges {
            by_label.entry(&edge.label).or_default().push(edge);
        }

        // Create each group
        for (_, label_edges) in by_label {
            for edge in label_edges {
                self.create(edge)?;
            }
        }

        Ok(())
    }

    /// Delete multiple edges by ID in a batch.
    pub fn delete_batch(&self, ids: &[(&str, &str)]) -> Result<usize, StorageError> {
        let mut count = 0;
        for (label, edge_id) in ids {
            if self.delete_by_id(label, edge_id)? {
                count += 1;
            }
        }
        Ok(count)
    }

    /// Batch load outgoing edges for multiple source nodes.
    /// Returns edges grouped by source node ID.
    pub fn list_out_batch(
        &self,
        from_nodes: &[&str],
        label: &str,
        limit_per_node: usize,
    ) -> Result<HashMap<String, Vec<Edge>>, StorageError> {
        let mut result = HashMap::with_capacity(from_nodes.len());
        for &from in from_nodes {
            let (edges, _) = self.list_out(from, label, None, limit_per_node)?;
            result.insert(from.to_string(), edges);
        }
        Ok(result)
    }

    /// Batch load incoming edges for multiple target nodes.
    /// Returns edges grouped by target node ID.
    pub fn list_in_batch(
        &self,
        to_nodes: &[&str],
        label: &str,
        limit_per_node: usize,
    ) -> Result<HashMap<String, Vec<Edge>>, StorageError> {
        let mut result = HashMap::with_capacity(to_nodes.len());
        for &to in to_nodes {
            let (edges, _) = self.list_in(to, label, None, limit_per_node)?;
            result.insert(to.to_string(), edges);
        }
        Ok(result)
    }

    // =========================================================================
    // Field Index Queries
    // =========================================================================

    /// Query edges by field value.
    pub fn query_by_field(
        &self,
        label: &str,
        field: &str,
        value: &crate::document::Value,
    ) -> Result<Vec<Edge>, StorageError> {
        let schema = self
            .get_schema(label)
            .ok_or_else(|| StorageError::NotFound(format!("Edge type '{}' not defined", label)))?;

        // Check if field is indexed
        if !schema
            .indexes
            .iter()
            .any(|idx| idx.fields.contains(&field.to_string()))
        {
            return Err(StorageError::InvalidOperation(format!(
                "Field '{}' is not indexed on edge type '{}'",
                field, label
            )));
        }

        let ks = self.get_keyspace(label)?;
        let prefix = encode_edge_field_index_value_prefix(field, value);
        let prefix_end = {
            let mut end = prefix.clone();
            if let Some(last) = end.last_mut() {
                *last = 0x01;
            }
            end
        };

        let mut edges = Vec::new();

        for item in ks.inner().range(prefix..prefix_end) {
            let (key_bytes, _value_bytes) = item.into_inner()?;
            // Extract edge_id from key (last segment after 0x00)
            let key = &key_bytes[1..]; // Skip EDGE_PREFIX_IDX
            if let Some(edge_id) = self.extract_edge_id_from_index_key(key)
                && let Some(edge) = self.get_by_id(label, &edge_id)?
            {
                edges.push(edge);
            }
        }

        Ok(edges)
    }

    // =========================================================================
    // Helper Methods
    // =========================================================================

    /// Serialize EdgeData with compact format support.
    fn serialize_edge_data(&self, data: &EdgeData) -> Result<Vec<u8>, StorageError> {
        if data.is_compact() {
            // Compact format: marker + id + 0x00 + timestamp
            let mut buf = Vec::with_capacity(1 + data.id.len() + 1 + 8);
            buf.push(EDGE_FORMAT_COMPACT);
            buf.extend(data.id.as_bytes());
            buf.push(0x00);
            buf.extend(data.created_at.to_le_bytes());
            Ok(buf)
        } else {
            // Full format: marker + rkyv
            let mut buf = vec![EDGE_FORMAT_FULL];
            let rkyv_bytes = rkyv::to_bytes::<rancor::Error>(data)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            buf.extend(rkyv_bytes.as_slice());
            Ok(buf)
        }
    }

    /// Deserialize EdgeData from bytes.
    fn deserialize_edge_data(&self, bytes: &[u8]) -> Result<EdgeData, StorageError> {
        if bytes.is_empty() {
            return Err(StorageError::Serialization("Empty edge data".into()));
        }

        match bytes[0] {
            EDGE_FORMAT_COMPACT => {
                // Compact: id + 0x00 + timestamp
                let rest = &bytes[1..];
                let sep = rest
                    .iter()
                    .position(|&b| b == 0x00)
                    .ok_or_else(|| StorageError::Serialization("Invalid compact format".into()))?;
                let id = String::from_utf8_lossy(&rest[..sep]).to_string();
                let timestamp_bytes: [u8; 8] = rest[sep + 1..sep + 9]
                    .try_into()
                    .map_err(|_| StorageError::Serialization("Invalid timestamp".into()))?;
                let created_at = i64::from_le_bytes(timestamp_bytes);

                Ok(EdgeData {
                    id,
                    created_at,
                    updated_at: 0,
                    fields: HashMap::new(),
                })
            }
            EDGE_FORMAT_FULL => {
                // Full: rkyv
                rkyv::from_bytes::<EdgeData, rancor::Error>(&bytes[1..])
                    .map_err(|e| StorageError::Serialization(e.to_string()))
            }
            _ => Err(StorageError::Serialization(format!(
                "Unknown edge format marker: {}",
                bytes[0]
            ))),
        }
    }

    /// Parse OUT key to extract target node ID.
    fn parse_out_key_target(&self, key: &[u8]) -> Result<String, StorageError> {
        // Key format: from | 0x00 | to [| 0x00 | edge_id]
        let parts: Vec<&[u8]> = key.split(|&b| b == 0x00).collect();
        if parts.len() < 2 {
            return Err(StorageError::Serialization("Invalid OUT key format".into()));
        }
        Ok(String::from_utf8_lossy(parts[1]).to_string())
    }

    /// Parse IN key and value to extract source node and edge ID.
    fn parse_in_key_and_value(
        &self,
        key: &[u8],
        value: &[u8],
        schema: &EdgeSchema,
    ) -> Result<(String, String), StorageError> {
        // Key format: to | 0x00 | from [| 0x00 | edge_id]
        let parts: Vec<&[u8]> = key.split(|&b| b == 0x00).collect();
        if parts.len() < 2 {
            return Err(StorageError::Serialization("Invalid IN key format".into()));
        }

        let from = String::from_utf8_lossy(parts[1]).to_string();

        let edge_id = match schema.cardinality {
            EdgeCardinality::Single => {
                // Value contains edge_id
                String::from_utf8_lossy(value).to_string()
            }
            EdgeCardinality::Multiple => {
                // Edge ID is in key
                if parts.len() >= 3 {
                    String::from_utf8_lossy(parts[2]).to_string()
                } else {
                    return Err(StorageError::Serialization(
                        "Missing edge_id in multi key".into(),
                    ));
                }
            }
        };

        Ok((from, edge_id))
    }

    /// Extract edge_id from field index key.
    fn extract_edge_id_from_index_key(&self, key: &[u8]) -> Option<String> {
        // Key format: field | 0x00 | value | 0x00 | edge_id
        let parts: Vec<&[u8]> = key.split(|&b| b == 0x00).collect();
        parts
            .last()
            .map(|id| String::from_utf8_lossy(id).to_string())
    }

    /// Update field indexes for an edge.
    fn update_field_indexes(
        &self,
        ks: &OptimisticTxKeyspace,
        schema: &EdgeSchema,
        edge_id: &str,
        fields: &HashMap<String, crate::document::Value>,
        insert: bool,
    ) -> Result<(), StorageError> {
        for index in &schema.indexes {
            for field_name in &index.fields {
                if let Some(value) = fields.get(field_name) {
                    let key = encode_edge_field_index_key(field_name, value, edge_id);
                    if insert {
                        ks.insert(&key, [])?;
                    } else {
                        ks.remove(&key)?;
                    }
                }
            }
        }
        Ok(())
    }

    // =========================================================================
    // Delete Policy Methods
    // =========================================================================

    /// Delete all edges connected to a node (both outgoing and incoming).
    /// Returns the number of edges deleted.
    pub fn delete_edges_for_node(&self, node_id: &str) -> Result<usize, StorageError> {
        let labels = self.list_edge_types();
        let mut deleted_count = 0;

        for label in &labels {
            let schema = match self.get_schema(label) {
                Some(s) => s,
                None => continue,
            };

            match schema.on_delete {
                OnDeletePolicy::Cascade => {
                    // Delete outgoing edges
                    let (out_edges, _) = self.list_out(node_id, label, None, usize::MAX)?;
                    for edge in out_edges {
                        self.delete(&edge.from, &edge.label, &edge.to)?;
                        deleted_count += 1;
                    }

                    // Delete incoming edges
                    let (in_edges, _) = self.list_in(node_id, label, None, usize::MAX)?;
                    for edge in in_edges {
                        self.delete(&edge.from, &edge.label, &edge.to)?;
                        deleted_count += 1;
                    }
                }
                OnDeletePolicy::Restrict => {
                    // Check if any edges exist - if so, return error
                    let (out_edges, _) = self.list_out(node_id, label, None, 1)?;
                    if !out_edges.is_empty() {
                        return Err(StorageError::Conflict(format!(
                            "Cannot delete '{}': has outgoing '{}' edges (RESTRICT policy)",
                            node_id, label
                        )));
                    }

                    let (in_edges, _) = self.list_in(node_id, label, None, 1)?;
                    if !in_edges.is_empty() {
                        return Err(StorageError::Conflict(format!(
                            "Cannot delete '{}': has incoming '{}' edges (RESTRICT policy)",
                            node_id, label
                        )));
                    }
                }
                OnDeletePolicy::NoAction => {
                    // Skip - leave orphaned edges
                }
            }
        }

        Ok(deleted_count)
    }

    /// Check if deleting a node would be blocked by RESTRICT policy.
    pub fn check_delete_allowed(&self, node_id: &str) -> Result<(), StorageError> {
        let labels = self.list_edge_types();

        for label in &labels {
            let schema = match self.get_schema(label) {
                Some(s) => s,
                None => continue,
            };

            if matches!(schema.on_delete, OnDeletePolicy::Restrict) {
                let (out_edges, _) = self.list_out(node_id, label, None, 1)?;
                if !out_edges.is_empty() {
                    return Err(StorageError::Conflict(format!(
                        "Cannot delete '{}': has outgoing '{}' edges (RESTRICT policy)",
                        node_id, label
                    )));
                }

                let (in_edges, _) = self.list_in(node_id, label, None, 1)?;
                if !in_edges.is_empty() {
                    return Err(StorageError::Conflict(format!(
                        "Cannot delete '{}': has incoming '{}' edges (RESTRICT policy)",
                        node_id, label
                    )));
                }
            }
        }

        Ok(())
    }

    /// Delete all edges connected to a node within a transaction.
    /// This version operates within an existing transaction for atomicity.
    ///
    /// Returns the number of edges scheduled for deletion.
    pub fn delete_edges_for_node_in_tx(
        &self,
        tx: &mut OptimisticWriteTx,
        node_id: &str,
    ) -> Result<usize, StorageError> {
        let labels = self.list_edge_types();
        let mut deleted_count = 0;

        for label in &labels {
            let schema = match self.get_schema(label) {
                Some(s) => s,
                None => continue,
            };

            // Skip non-CASCADE policies (RESTRICT already checked, NO_ACTION means skip)
            if !matches!(schema.on_delete, OnDeletePolicy::Cascade) {
                continue;
            }

            let ks = self.get_keyspace(label)?;

            // Collect outgoing edges
            let (out_edges, _) = self.list_out(node_id, label, None, usize::MAX)?;
            for edge in &out_edges {
                self.delete_edge_keys_in_tx(tx, &ks, &schema, &edge.from, &edge.to, edge.nanoid())?;
                deleted_count += 1;
            }

            // Collect incoming edges
            let (in_edges, _) = self.list_in(node_id, label, None, usize::MAX)?;
            for edge in &in_edges {
                self.delete_edge_keys_in_tx(tx, &ks, &schema, &edge.from, &edge.to, edge.nanoid())?;
                deleted_count += 1;
            }
        }

        Ok(deleted_count)
    }

    /// Delete all keys for a single edge within a transaction.
    fn delete_edge_keys_in_tx(
        &self,
        tx: &mut OptimisticWriteTx,
        ks: &OptimisticTxKeyspace,
        schema: &EdgeSchema,
        from: &str,
        to: &str,
        edge_id: &str,
    ) -> Result<(), StorageError> {
        match schema.cardinality {
            EdgeCardinality::Single => {
                let out_key = encode_edge_out_key_single(from, to);
                tx.remove(ks, &out_key);

                if schema.is_undirected() {
                    let reverse_out_key = encode_edge_out_key_single(to, from);
                    tx.remove(ks, &reverse_out_key);
                } else {
                    let in_key = encode_edge_in_key_single(to, from);
                    tx.remove(ks, &in_key);
                }
            }
            EdgeCardinality::Multiple => {
                let out_key = encode_edge_out_key_multi(from, to, edge_id);
                tx.remove(ks, &out_key);

                if schema.is_undirected() {
                    let reverse_out_key = encode_edge_out_key_multi(to, from, edge_id);
                    tx.remove(ks, &reverse_out_key);
                } else {
                    let in_key = encode_edge_in_key_multi(to, from, edge_id);
                    tx.remove(ks, &in_key);
                }
            }
        }

        // Delete ID lookup
        let id_key = encode_edge_id_key(edge_id);
        tx.remove(ks, &id_key);

        // Note: Field indexes are not updated here for simplicity.
        // They will have stale entries that point to deleted edges.
        // This is acceptable as queries will filter out non-existent edges.

        Ok(())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Value;
    use crate::edge::{EdgeDirection, EdgeFieldDef, EdgeIndex, EdgeValueType, OnDeletePolicy};
    use tempfile::TempDir;

    fn create_test_store() -> (TempDir, EdgeStore) {
        let tmp = TempDir::new().unwrap();
        let db = Arc::new(
            fjall::OptimisticTxDatabase::builder(tmp.path())
                .open()
                .unwrap(),
        );
        let store = EdgeStore::new(db).unwrap();
        (tmp, store)
    }

    #[test]
    fn test_define_edge_type() {
        let (_tmp, store) = create_test_store();

        let schema = EdgeSchema::new("follows");
        store.define_edge_type(schema).unwrap();

        assert!(store.get_schema("follows").is_some());
        assert!(store.list_edge_types().contains(&"follows".to_string()));
    }

    #[test]
    fn test_create_and_get_single() {
        let (_tmp, store) = create_test_store();

        store.define_edge_type(EdgeSchema::new("follows")).unwrap();

        let edge = Edge::new("user:alice".into(), "follows".into(), "user:bob".into());
        let edge_id = edge.nanoid().to_string();
        store.create(&edge).unwrap();

        // Get by ID
        let found = store.get_by_id("follows", &edge_id).unwrap().unwrap();
        assert_eq!(found.from, "user:alice");
        assert_eq!(found.to, "user:bob");

        // Get by path
        let found = store
            .get("user:alice", "follows", "user:bob")
            .unwrap()
            .unwrap();
        assert_eq!(found.from, "user:alice");
    }

    #[test]
    fn test_single_cardinality_overwrite() {
        let (_tmp, store) = create_test_store();

        store.define_edge_type(EdgeSchema::new("follows")).unwrap();

        // Create first edge
        let edge1 = Edge::new("user:alice".into(), "follows".into(), "user:bob".into())
            .with_field("strength", Value::Float(0.5));
        store.create(&edge1).unwrap();

        // Create second edge (should overwrite)
        let edge2 = Edge::new("user:alice".into(), "follows".into(), "user:bob".into())
            .with_field("strength", Value::Float(0.9));
        store.create(&edge2).unwrap();

        // Should only have one edge
        let (edges, _) = store.list_out("user:alice", "follows", None, 10).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].fields.get("strength"), Some(&Value::Float(0.9)));
    }

    #[test]
    fn test_multiple_cardinality() {
        let (_tmp, store) = create_test_store();

        store
            .define_edge_type(EdgeSchema::new("transaction").cardinality(EdgeCardinality::Multiple))
            .unwrap();

        // Create multiple edges
        let edge1 = Edge::new("user:alice".into(), "transaction".into(), "user:bob".into())
            .with_field("amount", Value::Float(100.0));
        store.create(&edge1).unwrap();

        let edge2 = Edge::new("user:alice".into(), "transaction".into(), "user:bob".into())
            .with_field("amount", Value::Float(50.0));
        store.create(&edge2).unwrap();

        // Should have two edges
        let (edges, _) = store
            .list_out("user:alice", "transaction", None, 10)
            .unwrap();
        assert_eq!(edges.len(), 2);
    }

    #[test]
    fn test_undirected_edge() {
        let (_tmp, store) = create_test_store();

        store
            .define_edge_type(EdgeSchema::new("friends").direction(EdgeDirection::Undirected))
            .unwrap();

        // Create edge (order shouldn't matter)
        let edge = Edge::new_undirected("user:bob".into(), "friends".into(), "user:alice".into());
        store.create(&edge).unwrap();

        // Should find from both directions
        let (out_edges, _) = store.list_out("user:alice", "friends", None, 10).unwrap();
        assert_eq!(out_edges.len(), 1);

        // For undirected, list_in should also work
        let (in_edges, _) = store.list_in("user:bob", "friends", None, 10).unwrap();
        assert_eq!(in_edges.len(), 1);
    }

    #[test]
    fn test_delete_by_id() {
        let (_tmp, store) = create_test_store();

        store.define_edge_type(EdgeSchema::new("follows")).unwrap();

        let edge = Edge::new("user:alice".into(), "follows".into(), "user:bob".into());
        let edge_id = edge.nanoid().to_string();
        store.create(&edge).unwrap();

        // Delete
        let deleted = store.delete_by_id("follows", &edge_id).unwrap();
        assert!(deleted);

        // Should be gone
        let found = store.get_by_id("follows", &edge_id).unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn test_field_index() {
        let (_tmp, store) = create_test_store();

        store
            .define_edge_type(
                EdgeSchema::new("transaction")
                    .cardinality(EdgeCardinality::Multiple)
                    .field(EdgeFieldDef::new("amount", EdgeValueType::Float))
                    .index(EdgeIndex::new("idx_amount", "amount")),
            )
            .unwrap();

        // Create edges with different amounts
        let edge1 = Edge::new("user:alice".into(), "transaction".into(), "user:bob".into())
            .with_field("amount", Value::Float(100.0));
        store.create(&edge1).unwrap();

        let edge2 = Edge::new(
            "user:alice".into(),
            "transaction".into(),
            "user:carol".into(),
        )
        .with_field("amount", Value::Float(100.0));
        store.create(&edge2).unwrap();

        let edge3 = Edge::new(
            "user:alice".into(),
            "transaction".into(),
            "user:dave".into(),
        )
        .with_field("amount", Value::Float(50.0));
        store.create(&edge3).unwrap();

        // Query by amount = 100
        let results = store
            .query_by_field("transaction", "amount", &Value::Float(100.0))
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_compact_storage() {
        let (_tmp, store) = create_test_store();

        store.define_edge_type(EdgeSchema::new("follows")).unwrap();

        // Edge without fields should use compact storage
        let edge = Edge::new("user:alice".into(), "follows".into(), "user:bob".into());
        let data = EdgeData::from_edge(&edge);
        assert!(data.is_compact());

        let serialized = store.serialize_edge_data(&data).unwrap();
        assert_eq!(serialized[0], EDGE_FORMAT_COMPACT);

        // Deserialize and verify
        let deserialized = store.deserialize_edge_data(&serialized).unwrap();
        assert_eq!(deserialized.id, data.id);
        assert_eq!(deserialized.created_at, data.created_at);
    }

    #[test]
    fn test_batch_create() {
        let (_tmp, store) = create_test_store();

        store.define_edge_type(EdgeSchema::new("follows")).unwrap();

        let edges: Vec<Edge> = (0..10)
            .map(|i| {
                Edge::new(
                    "user:alice".into(),
                    "follows".into(),
                    format!("user:bob{}", i),
                )
            })
            .collect();

        store.create_batch(&edges).unwrap();

        let (found, _) = store.list_out("user:alice", "follows", None, 20).unwrap();
        assert_eq!(found.len(), 10);
    }

    #[test]
    fn test_batch_list_out_empty_nodes() {
        let (_tmp, store) = create_test_store();
        store.define_edge_type(EdgeSchema::new("follows")).unwrap();

        let result = store.list_out_batch(&[], "follows", 100).unwrap();
        assert!(result.is_empty(), "Empty input should return empty result");
    }

    #[test]
    fn test_batch_list_out_single_node() {
        let (_tmp, store) = create_test_store();
        store.define_edge_type(EdgeSchema::new("follows")).unwrap();

        let edge = Edge::new("user:alice".into(), "follows".into(), "user:bob".into());
        store.create(&edge).unwrap();

        let result = store
            .list_out_batch(&["user:alice"], "follows", 100)
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result.get("user:alice").unwrap().len(), 1);
    }

    #[test]
    fn test_batch_list_out_multiple_nodes() {
        let (_tmp, store) = create_test_store();
        store.define_edge_type(EdgeSchema::new("follows")).unwrap();

        // Alice follows Bob and Carol
        store
            .create(&Edge::new(
                "user:alice".into(),
                "follows".into(),
                "user:bob".into(),
            ))
            .unwrap();
        store
            .create(&Edge::new(
                "user:alice".into(),
                "follows".into(),
                "user:carol".into(),
            ))
            .unwrap();
        // Bob follows Carol
        store
            .create(&Edge::new(
                "user:bob".into(),
                "follows".into(),
                "user:carol".into(),
            ))
            .unwrap();

        let result = store
            .list_out_batch(&["user:alice", "user:bob"], "follows", 100)
            .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result.get("user:alice").unwrap().len(), 2);
        assert_eq!(result.get("user:bob").unwrap().len(), 1);
    }

    #[test]
    fn test_batch_list_out_nonexistent_node() {
        let (_tmp, store) = create_test_store();
        store.define_edge_type(EdgeSchema::new("follows")).unwrap();

        let result = store
            .list_out_batch(&["user:ghost"], "follows", 100)
            .unwrap();
        assert_eq!(result.len(), 1);
        assert!(
            result.get("user:ghost").unwrap().is_empty(),
            "Nonexistent node should return empty vec"
        );
    }

    #[test]
    fn test_batch_list_out_limit_respected() {
        let (_tmp, store) = create_test_store();
        store.define_edge_type(EdgeSchema::new("follows")).unwrap();

        // Alice follows 5 people
        for i in 0..5 {
            store
                .create(&Edge::new(
                    "user:alice".into(),
                    "follows".into(),
                    format!("user:friend{}", i),
                ))
                .unwrap();
        }

        let result = store.list_out_batch(&["user:alice"], "follows", 2).unwrap();
        assert_eq!(
            result.get("user:alice").unwrap().len(),
            2,
            "Limit should be respected"
        );
    }

    #[test]
    fn test_batch_list_out_mixed_degrees() {
        let (_tmp, store) = create_test_store();
        store.define_edge_type(EdgeSchema::new("follows")).unwrap();

        // Alice follows 3 people
        for i in 0..3 {
            store
                .create(&Edge::new(
                    "user:alice".into(),
                    "follows".into(),
                    format!("user:a{}", i),
                ))
                .unwrap();
        }
        // Bob follows 1 person
        store
            .create(&Edge::new(
                "user:bob".into(),
                "follows".into(),
                "user:carol".into(),
            ))
            .unwrap();
        // Carol follows nobody

        let result = store
            .list_out_batch(&["user:alice", "user:bob", "user:carol"], "follows", 100)
            .unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result.get("user:alice").unwrap().len(), 3);
        assert_eq!(result.get("user:bob").unwrap().len(), 1);
        assert_eq!(result.get("user:carol").unwrap().len(), 0);
    }

    #[test]
    fn test_batch_list_in_multiple_nodes() {
        let (_tmp, store) = create_test_store();
        store.define_edge_type(EdgeSchema::new("follows")).unwrap();

        // Alice and Bob follow Carol
        store
            .create(&Edge::new(
                "user:alice".into(),
                "follows".into(),
                "user:carol".into(),
            ))
            .unwrap();
        store
            .create(&Edge::new(
                "user:bob".into(),
                "follows".into(),
                "user:carol".into(),
            ))
            .unwrap();
        // Dave follows Alice
        store
            .create(&Edge::new(
                "user:dave".into(),
                "follows".into(),
                "user:alice".into(),
            ))
            .unwrap();

        let result = store
            .list_in_batch(&["user:carol", "user:alice"], "follows", 100)
            .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result.get("user:carol").unwrap().len(), 2);
        assert_eq!(result.get("user:alice").unwrap().len(), 1);
    }

    #[test]
    fn test_batch_list_in_empty_nodes() {
        let (_tmp, store) = create_test_store();
        store.define_edge_type(EdgeSchema::new("follows")).unwrap();

        let result = store.list_in_batch(&[], "follows", 100).unwrap();
        assert!(result.is_empty(), "Empty input should return empty result");
    }

    #[test]
    fn test_batch_list_in_nonexistent_node() {
        let (_tmp, store) = create_test_store();
        store.define_edge_type(EdgeSchema::new("follows")).unwrap();

        let result = store
            .list_in_batch(&["user:ghost"], "follows", 100)
            .unwrap();
        assert_eq!(result.len(), 1);
        assert!(
            result.get("user:ghost").unwrap().is_empty(),
            "Nonexistent node should return empty vec"
        );
    }

    #[test]
    fn test_batch_list_in_limit_respected() {
        let (_tmp, store) = create_test_store();
        store.define_edge_type(EdgeSchema::new("follows")).unwrap();

        // 5 people follow Alice
        for i in 0..5 {
            store
                .create(&Edge::new(
                    format!("user:follower{}", i),
                    "follows".into(),
                    "user:alice".into(),
                ))
                .unwrap();
        }

        let result = store.list_in_batch(&["user:alice"], "follows", 2).unwrap();
        assert_eq!(
            result.get("user:alice").unwrap().len(),
            2,
            "Limit should be respected"
        );
    }

    #[test]
    fn test_delete_edges_for_node_cascade() {
        let (_tmp, store) = create_test_store();

        // Default schema has Cascade policy
        store.define_edge_type(EdgeSchema::new("follows")).unwrap();
        store.define_edge_type(EdgeSchema::new("likes")).unwrap();

        // Create edges: alice -> bob, alice -> carol, dave -> alice
        store
            .create(&Edge::new(
                "user:alice".into(),
                "follows".into(),
                "user:bob".into(),
            ))
            .unwrap();
        store
            .create(&Edge::new(
                "user:alice".into(),
                "follows".into(),
                "user:carol".into(),
            ))
            .unwrap();
        store
            .create(&Edge::new(
                "user:dave".into(),
                "follows".into(),
                "user:alice".into(),
            ))
            .unwrap();
        store
            .create(&Edge::new(
                "user:alice".into(),
                "likes".into(),
                "post:p1".into(),
            ))
            .unwrap();

        // Delete all edges for alice
        let deleted = store.delete_edges_for_node("user:alice").unwrap();
        assert_eq!(
            deleted, 4,
            "Should delete 4 edges (2 out follows + 1 in follows + 1 likes)"
        );

        // Verify edges are gone
        let (out, _) = store.list_out("user:alice", "follows", None, 100).unwrap();
        assert!(
            out.is_empty(),
            "Alice should have no outgoing follows edges"
        );

        let (in_edges, _) = store.list_in("user:alice", "follows", None, 100).unwrap();
        assert!(
            in_edges.is_empty(),
            "Alice should have no incoming follows edges"
        );
    }

    #[test]
    fn test_delete_edges_for_node_restrict() {
        let (_tmp, store) = create_test_store();

        // Create edge type with RESTRICT policy
        store
            .define_edge_type(EdgeSchema::new("owns").on_delete(OnDeletePolicy::Restrict))
            .unwrap();

        // Create edge: alice owns item1
        store
            .create(&Edge::new(
                "user:alice".into(),
                "owns".into(),
                "item:1".into(),
            ))
            .unwrap();

        // Try to delete edges for alice - should fail due to RESTRICT
        let result = store.delete_edges_for_node("user:alice");
        assert!(result.is_err(), "Should fail due to RESTRICT policy");
        assert!(
            result.unwrap_err().to_string().contains("RESTRICT"),
            "Error should mention RESTRICT policy"
        );

        // Edge should still exist
        let (out, _) = store.list_out("user:alice", "owns", None, 100).unwrap();
        assert_eq!(out.len(), 1, "Edge should still exist");
    }

    #[test]
    fn test_delete_edges_for_node_no_action() {
        let (_tmp, store) = create_test_store();

        // Create edge type with NO_ACTION policy
        store
            .define_edge_type(EdgeSchema::new("mentions").on_delete(OnDeletePolicy::NoAction))
            .unwrap();

        // Create edge: post1 mentions alice
        store
            .create(&Edge::new(
                "post:1".into(),
                "mentions".into(),
                "user:alice".into(),
            ))
            .unwrap();

        // Delete edges for alice - should succeed but leave edge orphaned
        let deleted = store.delete_edges_for_node("user:alice").unwrap();
        assert_eq!(deleted, 0, "Should not delete any edges with NO_ACTION");

        // Edge should still exist (orphaned)
        let (out, _) = store.list_out("post:1", "mentions", None, 100).unwrap();
        assert_eq!(out.len(), 1, "Edge should still exist (orphaned)");
    }

    #[test]
    fn test_check_delete_allowed() {
        let (_tmp, store) = create_test_store();

        store.define_edge_type(EdgeSchema::new("follows")).unwrap(); // Cascade
        store
            .define_edge_type(EdgeSchema::new("owns").on_delete(OnDeletePolicy::Restrict))
            .unwrap();

        // Create edges
        store
            .create(&Edge::new(
                "user:alice".into(),
                "follows".into(),
                "user:bob".into(),
            ))
            .unwrap();
        store
            .create(&Edge::new(
                "user:alice".into(),
                "owns".into(),
                "item:1".into(),
            ))
            .unwrap();

        // Check for bob - should be allowed (only cascade edges)
        assert!(store.check_delete_allowed("user:bob").is_ok());

        // Check for alice - should be blocked (has RESTRICT edge)
        assert!(store.check_delete_allowed("user:alice").is_err());

        // Check for carol - should be allowed (no edges)
        assert!(store.check_delete_allowed("user:carol").is_ok());
    }
}
