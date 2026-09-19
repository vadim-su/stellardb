//! Edge types and structures for graph relationships.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use rkyv::{
    Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize,
    rancor::{Fallible, Source},
    ser::{Allocator, Writer},
    validation::ArchiveContext,
};
use serde::{Deserialize, Serialize};

use crate::document::Value;

/// Get current time in milliseconds since Unix epoch
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}

/// Generate a new nanoid for edge identification
pub fn generate_edge_id() -> String {
    nanoid::nanoid!()
}

// =============================================================================
// Edge
// =============================================================================

/// A graph edge connecting two documents.
///
/// Edges represent relationships between documents in the graph model.
/// Each edge has a unique ID in the format `{label}:{nanoid}`.
#[derive(Debug, Clone, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(
    serialize_bounds(__S: Writer + Allocator, <__S as Fallible>::Error: Source),
    deserialize_bounds(<__D as Fallible>::Error: Source),
    bytecheck(bounds(__C: ArchiveContext, <__C as Fallible>::Error: Source))
)]
pub struct Edge {
    /// Unique identifier: "{label}:{nanoid}"
    /// Example: "follows:V1StGXR8_Z5jdHi6B-myT"
    pub id: String,

    /// Source document ID (e.g., "user:alice")
    pub from: String,

    /// Target document ID (e.g., "user:bob")
    pub to: String,

    /// Edge type/label (e.g., "follows")
    pub label: String,

    /// Creation timestamp (Unix milliseconds)
    pub created_at: i64,

    /// Last update timestamp (None if never updated)
    pub updated_at: Option<i64>,

    /// User-defined properties
    #[rkyv(omit_bounds)]
    pub fields: HashMap<String, Value>,
}

impl Edge {
    /// Create a new directed edge with auto-generated nanoid.
    /// ID format: `<label>:<nanoid>` (e.g., `follows:V1StGXR8_Z5jdHi6B-myT`)
    pub fn new(from: String, label: String, to: String) -> Self {
        let nanoid = generate_edge_id();
        let id = format!("{}:{}", label, nanoid);
        Self {
            id,
            from,
            label,
            to,
            created_at: now_millis(),
            updated_at: None,
            fields: HashMap::new(),
        }
    }

    /// Create a new edge with specific timestamp (for testing/migration).
    pub fn with_timestamp(from: String, label: String, to: String, created_at: i64) -> Self {
        let nanoid = generate_edge_id();
        let id = format!("{}:{}", label, nanoid);
        Self {
            id,
            from,
            label,
            to,
            created_at,
            updated_at: None,
            fields: HashMap::new(),
        }
    }

    /// Create a new edge with a specific ID (for reconstruction from storage).
    pub fn with_id(id: String, from: String, label: String, to: String, created_at: i64) -> Self {
        Self {
            id,
            from,
            label,
            to,
            created_at,
            updated_at: None,
            fields: HashMap::new(),
        }
    }

    /// Create an undirected edge (nodes are sorted for consistent storage).
    /// ID format: `<label>:<nanoid>`
    pub fn new_undirected(node_a: String, label: String, node_b: String) -> Self {
        // Sort nodes lexicographically for consistent storage
        let (from, to) = if node_a <= node_b {
            (node_a, node_b)
        } else {
            (node_b, node_a)
        };
        Self::new(from, label, to)
    }

    /// Add fields to the edge (builder pattern).
    pub fn with_fields(mut self, fields: HashMap<String, Value>) -> Self {
        self.fields = fields;
        self
    }

    /// Add a single field to the edge (builder pattern).
    pub fn with_field(mut self, key: impl Into<String>, value: Value) -> Self {
        self.fields.insert(key.into(), value);
        self
    }

    /// Get the nanoid part of the edge ID (without label prefix).
    pub fn nanoid(&self) -> &str {
        self.id
            .strip_prefix(&self.label)
            .and_then(|s| s.strip_prefix(':'))
            .unwrap_or(&self.id)
    }

    /// Check if this edge has any user-defined fields.
    pub fn has_fields(&self) -> bool {
        !self.fields.is_empty()
    }

    /// Mark the edge as updated with current timestamp.
    pub fn touch(&mut self) {
        self.updated_at = Some(now_millis());
    }
}

// =============================================================================
// EdgeSchema
// =============================================================================

/// Schema definition for an edge type.
///
/// Defines the structure, constraints, and behavior of edges with a specific label.
#[derive(Debug, Clone, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(
    serialize_bounds(__S: Writer + Allocator, <__S as Fallible>::Error: Source),
    deserialize_bounds(<__D as Fallible>::Error: Source),
    bytecheck(bounds(__C: ArchiveContext, <__C as Fallible>::Error: Source))
)]
pub struct EdgeSchema {
    /// Edge type name (e.g., "follows")
    pub name: String,

    /// Direction type (directed or undirected)
    pub direction: EdgeDirection,

    /// Cardinality constraint (single or multiple edges between same nodes)
    pub cardinality: EdgeCardinality,

    /// Policy for handling edges when connected node is deleted
    pub on_delete: OnDeletePolicy,

    /// Field definitions with types and constraints
    #[rkyv(omit_bounds)]
    pub fields: Vec<EdgeFieldDef>,

    /// Indexed fields for efficient queries
    #[rkyv(omit_bounds)]
    pub indexes: Vec<EdgeIndex>,
}

impl EdgeSchema {
    /// Create a new schema with default settings (directed, single cardinality).
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            direction: EdgeDirection::Directed,
            cardinality: EdgeCardinality::Single,
            on_delete: OnDeletePolicy::Cascade,
            fields: Vec::new(),
            indexes: Vec::new(),
        }
    }

    /// Set the edge direction.
    pub fn direction(mut self, direction: EdgeDirection) -> Self {
        self.direction = direction;
        self
    }

    /// Set the cardinality constraint.
    pub fn cardinality(mut self, cardinality: EdgeCardinality) -> Self {
        self.cardinality = cardinality;
        self
    }

    /// Set the on-delete policy.
    pub fn on_delete(mut self, policy: OnDeletePolicy) -> Self {
        self.on_delete = policy;
        self
    }

    /// Add a field definition.
    pub fn field(mut self, field: EdgeFieldDef) -> Self {
        self.fields.push(field);
        self
    }

    /// Add an index.
    pub fn index(mut self, index: EdgeIndex) -> Self {
        self.indexes.push(index);
        self
    }

    /// Check if this edge type is undirected.
    pub fn is_undirected(&self) -> bool {
        matches!(self.direction, EdgeDirection::Undirected)
    }

    /// Check if this edge type allows multiple edges between same nodes.
    pub fn is_multiple(&self) -> bool {
        matches!(self.cardinality, EdgeCardinality::Multiple)
    }
}

impl Default for EdgeSchema {
    fn default() -> Self {
        Self {
            name: String::new(),
            direction: EdgeDirection::Directed,
            cardinality: EdgeCardinality::Single,
            on_delete: OnDeletePolicy::Cascade,
            fields: Vec::new(),
            indexes: Vec::new(),
        }
    }
}

// =============================================================================
// OnDeletePolicy
// =============================================================================

/// Policy for handling edges when a connected node is deleted.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
)]
#[rkyv(compare(PartialEq))]
pub enum OnDeletePolicy {
    /// Delete all edges connected to the deleted node (default).
    #[default]
    Cascade,
    /// Prevent deletion if edges exist (return error).
    Restrict,
    /// Leave orphaned edges (legacy behavior).
    NoAction,
}

// =============================================================================
// EdgeDirection
// =============================================================================

/// Direction type for edges.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
)]
pub enum EdgeDirection {
    /// A -> B (stored with both OUT and IN indexes)
    #[default]
    Directed,

    /// A -- B (stored once, works both ways)
    Undirected,
}

// =============================================================================
// EdgeCardinality
// =============================================================================

/// Cardinality constraint for edges.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
)]
pub enum EdgeCardinality {
    /// Max one edge between any (from, to) pair.
    /// RELATE overwrites existing edge.
    #[default]
    Single,

    /// Multiple edges allowed between same nodes.
    /// Each RELATE creates a new edge with unique ID.
    Multiple,
}

// =============================================================================
// EdgeFieldDef
// =============================================================================

/// Field definition for edge schema.
#[derive(Debug, Clone, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(
    serialize_bounds(__S: Writer + Allocator, <__S as Fallible>::Error: Source),
    deserialize_bounds(<__D as Fallible>::Error: Source),
    bytecheck(bounds(__C: ArchiveContext, <__C as Fallible>::Error: Source))
)]
pub struct EdgeFieldDef {
    /// Field name
    pub name: String,

    /// Expected value type
    pub value_type: EdgeValueType,

    /// Whether this field is required
    pub required: bool,

    /// Default value (if any)
    #[rkyv(omit_bounds)]
    pub default: Option<Value>,
}

impl EdgeFieldDef {
    /// Create a new optional field definition.
    pub fn new(name: impl Into<String>, value_type: EdgeValueType) -> Self {
        Self {
            name: name.into(),
            value_type,
            required: false,
            default: None,
        }
    }

    /// Create a required field.
    pub fn required(name: impl Into<String>, value_type: EdgeValueType) -> Self {
        Self {
            name: name.into(),
            value_type,
            required: true,
            default: None,
        }
    }

    /// Set a default value.
    pub fn with_default(mut self, default: Value) -> Self {
        self.default = Some(default);
        self
    }
}

// =============================================================================
// EdgeValueType
// =============================================================================

/// Value types for edge field definitions.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
)]
pub enum EdgeValueType {
    String,
    Int,
    Float,
    Bool,
    DateTime,
    Any,
}

// =============================================================================
// EdgeIndex
// =============================================================================

/// Index definition for edge fields.
#[derive(Debug, Clone, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
pub struct EdgeIndex {
    /// Index name
    pub name: String,

    /// Fields to index (in order for compound indexes)
    pub fields: Vec<String>,
}

impl EdgeIndex {
    /// Create a single-field index.
    pub fn new(name: impl Into<String>, field: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            fields: vec![field.into()],
        }
    }

    /// Create a compound index.
    pub fn compound(name: impl Into<String>, fields: Vec<String>) -> Self {
        Self {
            name: name.into(),
            fields,
        }
    }
}

// =============================================================================
// EdgeData (compact storage format)
// =============================================================================

/// Compact edge data for storage.
///
/// This is the serialized form stored in the database, optimized for
/// edges with no or few fields.
#[derive(Debug, Clone, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(
    serialize_bounds(__S: Writer + Allocator, <__S as Fallible>::Error: Source),
    deserialize_bounds(<__D as Fallible>::Error: Source),
    bytecheck(bounds(__C: ArchiveContext, <__C as Fallible>::Error: Source))
)]
pub struct EdgeData {
    /// Edge ID (nanoid part only, without label prefix)
    pub id: String,

    /// Creation timestamp (Unix milliseconds)
    pub created_at: i64,

    /// Update timestamp (0 = never updated)
    pub updated_at: i64,

    /// User-defined fields
    #[rkyv(omit_bounds)]
    pub fields: HashMap<String, Value>,
}

impl EdgeData {
    /// Create EdgeData from an Edge.
    pub fn from_edge(edge: &Edge) -> Self {
        Self {
            id: edge.nanoid().to_string(),
            created_at: edge.created_at,
            updated_at: edge.updated_at.unwrap_or(0),
            fields: edge.fields.clone(),
        }
    }

    /// Convert to Edge with additional context.
    pub fn to_edge(&self, label: &str, from: &str, to: &str) -> Edge {
        Edge {
            id: format!("{}:{}", label, self.id),
            from: from.to_string(),
            to: to.to_string(),
            label: label.to_string(),
            created_at: self.created_at,
            updated_at: if self.updated_at == 0 {
                None
            } else {
                Some(self.updated_at)
            },
            fields: self.fields.clone(),
        }
    }

    /// Check if this edge data has no fields (can use compact format).
    pub fn is_compact(&self) -> bool {
        self.fields.is_empty() && self.updated_at == 0
    }
}

// =============================================================================
// Storage format markers
// =============================================================================

/// Marker for compact storage format (just id + timestamp).
pub const EDGE_FORMAT_COMPACT: u8 = 0x00;

/// Marker for full storage format (rkyv serialized EdgeData).
pub const EDGE_FORMAT_FULL: u8 = 0x01;

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edge_new() {
        let edge = Edge::new("user:alice".into(), "follows".into(), "user:bob".into());

        assert!(edge.id.starts_with("follows:"));
        assert_eq!(edge.from, "user:alice");
        assert_eq!(edge.to, "user:bob");
        assert_eq!(edge.label, "follows");
        assert!(edge.created_at > 0);
        assert!(edge.updated_at.is_none());
        assert!(edge.fields.is_empty());
    }

    #[test]
    fn test_edge_nanoid() {
        let edge = Edge::new("user:alice".into(), "follows".into(), "user:bob".into());
        let nanoid = edge.nanoid();

        assert!(!nanoid.is_empty());
        assert!(!nanoid.contains(':'));
        assert_eq!(edge.id, format!("follows:{}", nanoid));
    }

    #[test]
    fn test_edge_undirected() {
        // Order should be normalized
        let edge1 = Edge::new_undirected("user:bob".into(), "friends".into(), "user:alice".into());
        let edge2 = Edge::new_undirected("user:alice".into(), "friends".into(), "user:bob".into());

        // Both should have same from/to order (sorted)
        assert_eq!(edge1.from, "user:alice");
        assert_eq!(edge1.to, "user:bob");
        assert_eq!(edge2.from, "user:alice");
        assert_eq!(edge2.to, "user:bob");
    }

    #[test]
    fn test_edge_with_fields() {
        let edge = Edge::new("user:alice".into(), "follows".into(), "user:bob".into())
            .with_field("since", Value::Int(2024))
            .with_field("strength", Value::Float(0.8));

        assert_eq!(edge.fields.len(), 2);
        assert_eq!(edge.fields.get("since"), Some(&Value::Int(2024)));
        assert_eq!(edge.fields.get("strength"), Some(&Value::Float(0.8)));
        assert!(edge.has_fields());
    }

    #[test]
    fn test_edge_schema_builder() {
        let schema = EdgeSchema::new("transaction")
            .direction(EdgeDirection::Directed)
            .cardinality(EdgeCardinality::Multiple)
            .field(EdgeFieldDef::required("amount", EdgeValueType::Float))
            .field(EdgeFieldDef::new("memo", EdgeValueType::String))
            .index(EdgeIndex::new("idx_amount", "amount"));

        assert_eq!(schema.name, "transaction");
        assert!(!schema.is_undirected());
        assert!(schema.is_multiple());
        assert_eq!(schema.fields.len(), 2);
        assert_eq!(schema.indexes.len(), 1);
    }

    #[test]
    fn test_edge_data_conversion() {
        let edge = Edge::new("user:alice".into(), "follows".into(), "user:bob".into())
            .with_field("since", Value::Int(2024));

        let data = EdgeData::from_edge(&edge);
        let restored = data.to_edge("follows", "user:alice", "user:bob");

        assert_eq!(restored.id, edge.id);
        assert_eq!(restored.from, edge.from);
        assert_eq!(restored.to, edge.to);
        assert_eq!(restored.label, edge.label);
        assert_eq!(restored.created_at, edge.created_at);
        assert_eq!(restored.fields, edge.fields);
    }

    #[test]
    fn test_edge_data_compact() {
        let edge = Edge::new("user:alice".into(), "follows".into(), "user:bob".into());
        let data = EdgeData::from_edge(&edge);

        assert!(data.is_compact()); // No fields, no update

        let edge_with_fields = edge.with_field("x", Value::Int(1));
        let data_with_fields = EdgeData::from_edge(&edge_with_fields);

        assert!(!data_with_fields.is_compact()); // Has fields
    }
}
