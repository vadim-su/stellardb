//! Schema data types for StellarDB's declarative schema system.

use rkyv::{
    Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize,
    rancor::{Fallible, Source},
    ser::{Allocator, Writer},
    validation::ArchiveContext,
};
use serde::{Deserialize, Serialize};

/// Type of index backend
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Default,
    Serialize,
    Deserialize,
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
)]
#[rkyv(
    serialize_bounds(__S: Writer + Allocator, <__S as Fallible>::Error: Source),
    deserialize_bounds(<__D as Fallible>::Error: Source),
    bytecheck(bounds(__C: ArchiveContext, <__C as Fallible>::Error: Source))
)]
pub enum IndexType {
    /// B-tree index for scalar values (default)
    #[default]
    BTree,
    /// HNSW index for vector similarity search
    Hnsw,
    /// Full-text search index
    FullText,
}

/// Distance metric for HNSW vector index
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
)]
#[rkyv(
    serialize_bounds(__S: Writer + Allocator, <__S as Fallible>::Error: Source),
    deserialize_bounds(<__D as Fallible>::Error: Source),
    bytecheck(bounds(__C: ArchiveContext, <__C as Fallible>::Error: Source))
)]
pub enum DistanceMetric {
    #[default]
    Cosine,
    Euclidean,
    Dot,
}

/// Parameters for HNSW vector index
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[rkyv(
    serialize_bounds(__S: Writer + Allocator, <__S as Fallible>::Error: Source),
    deserialize_bounds(<__D as Fallible>::Error: Source),
    bytecheck(bounds(__C: ArchiveContext, <__C as Fallible>::Error: Source))
)]
pub struct HnswParams {
    /// Vector dimension (required)
    pub dimension: usize,
    /// Distance metric (default: Cosine)
    #[rkyv(omit_bounds)]
    pub metric: DistanceMetric,
    /// Max connections per node (default: 16)
    pub m: usize,
    /// Ef construction parameter (default: 200)
    pub ef_construction: usize,
}

impl Default for HnswParams {
    fn default() -> Self {
        Self {
            dimension: 0,
            metric: DistanceMetric::Cosine,
            m: 16,
            ef_construction: 200,
        }
    }
}

impl HnswParams {
    pub fn new(dimension: usize) -> Self {
        Self {
            dimension,
            ..Default::default()
        }
    }
}

/// Schema enforcement mode for a collection.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Default,
    Serialize,
    Deserialize,
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
)]
#[rkyv(
    serialize_bounds(__S: Writer + Allocator, <__S as Fallible>::Error: Source),
    deserialize_bounds(<__D as Fallible>::Error: Source),
    bytecheck(bounds(__C: ArchiveContext, <__C as Fallible>::Error: Source))
)]
pub enum SchemaMode {
    /// Strict mode: only defined fields are allowed, types are enforced
    Strict,
    /// Flexible mode: additional fields are allowed, defined fields are validated
    #[default]
    Flexible,
}

/// Supported field types in a schema definition.
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[rkyv(
    serialize_bounds(__S: Writer + Allocator, <__S as Fallible>::Error: Source),
    deserialize_bounds(<__D as Fallible>::Error: Source),
    bytecheck(bounds(__C: ArchiveContext, <__C as Fallible>::Error: Source))
)]
pub enum FieldType {
    /// String value
    String,
    /// 64-bit signed integer
    Int,
    /// 64-bit floating point
    Float,
    /// Arbitrary precision decimal
    Decimal,
    /// Boolean value
    Bool,
    /// ISO 8601 datetime string
    Datetime,
    /// Duration (time span)
    Duration,
    /// Binary data
    Bytes,
    /// Nested object with its own field definitions and schema mode
    Object {
        #[rkyv(omit_bounds)]
        fields: Vec<FieldDef>,
        mode: SchemaMode,
    },
    /// Array of a specific type
    Array(#[rkyv(omit_bounds)] Box<FieldType>),
    /// Any array (no element type constraint)
    AnyArray,
    /// Reference to another document. None = any collection, Some("user") = only user collection
    Reference(Option<String>),
    /// Accepts any value (flexible type)
    Any,
    /// Range type with element type constraint (e.g., `range<int>`)
    Range(#[rkyv(omit_bounds)] Box<FieldType>),
    /// Union of multiple types (e.g., string | int | null)
    Union(#[rkyv(omit_bounds)] Vec<FieldType>),
}

/// Default value for a field when not provided.
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[rkyv(
    serialize_bounds(__S: Writer + Allocator, <__S as Fallible>::Error: Source),
    deserialize_bounds(<__D as Fallible>::Error: Source),
    bytecheck(bounds(__C: ArchiveContext, <__C as Fallible>::Error: Source))
)]
pub enum DefaultValue {
    /// Null value
    Null,
    /// Boolean default
    Bool(bool),
    /// Integer default
    Int(i64),
    /// Floating point default
    Float(f64),
    /// String default
    String(String),
    /// Current timestamp (evaluated at insert time)
    Now,
}

/// Definition of a field within a schema.
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[rkyv(
    serialize_bounds(__S: Writer + Allocator, <__S as Fallible>::Error: Source),
    deserialize_bounds(<__D as Fallible>::Error: Source),
    bytecheck(bounds(__C: ArchiveContext, <__C as Fallible>::Error: Source))
)]
pub struct FieldDef {
    /// Field name
    pub name: String,
    /// Field type
    #[rkyv(omit_bounds)]
    pub field_type: FieldType,
    /// Whether the field is required (must be present and non-null)
    pub required: bool,
    /// Default value when field is not provided
    #[rkyv(omit_bounds)]
    pub default: Option<DefaultValue>,
}

/// Definition of an index on a collection.
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[rkyv(
    serialize_bounds(__S: Writer + Allocator, <__S as Fallible>::Error: Source),
    deserialize_bounds(<__D as Fallible>::Error: Source),
    bytecheck(bounds(__C: ArchiveContext, <__C as Fallible>::Error: Source))
)]
pub struct IndexDef {
    /// Index name (unique within collection)
    pub name: String,
    /// Fields to index (in order for compound indexes)
    pub fields: Vec<String>,
    /// Whether the index enforces uniqueness
    pub unique: bool,
    /// Type of index backend
    #[rkyv(omit_bounds)]
    pub index_type: IndexType,
    /// HNSW parameters (only for Hnsw index type)
    #[rkyv(omit_bounds)]
    pub hnsw_params: Option<HnswParams>,
    /// Analyzer name for full-text search indexes
    pub analyzer: Option<String>,
}

/// Definition of a custom analyzer
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[rkyv(
    serialize_bounds(__S: Writer + Allocator, <__S as Fallible>::Error: Source),
    deserialize_bounds(<__D as Fallible>::Error: Source),
    bytecheck(bounds(__C: ArchiveContext, <__C as Fallible>::Error: Source))
)]
pub struct AnalyzerDef {
    pub name: String,
    #[rkyv(omit_bounds)]
    pub tokenizer: TokenizerConfig,
    #[rkyv(omit_bounds)]
    pub filters: Vec<FilterConfig>,
}

/// Tokenizer configuration for custom analyzers
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[rkyv(
    serialize_bounds(__S: Writer + Allocator, <__S as Fallible>::Error: Source),
    deserialize_bounds(<__D as Fallible>::Error: Source),
    bytecheck(bounds(__C: ArchiveContext, <__C as Fallible>::Error: Source))
)]
pub enum TokenizerConfig {
    /// Standard tokenizer (splits on whitespace and punctuation)
    Standard,
    /// Whitespace tokenizer (splits only on whitespace)
    Whitespace,
    /// N-gram tokenizer
    Ngram { min: u8, max: u8 },
    /// Regex pattern tokenizer
    Pattern { regex: String },
}

/// Filter configuration for custom analyzers
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[rkyv(
    serialize_bounds(__S: Writer + Allocator, <__S as Fallible>::Error: Source),
    deserialize_bounds(<__D as Fallible>::Error: Source),
    bytecheck(bounds(__C: ArchiveContext, <__C as Fallible>::Error: Source))
)]
pub enum FilterConfig {
    /// Convert to lowercase
    Lowercase,
    /// Apply language stemmer
    Stemmer { lang: String },
    /// Remove stopwords for language
    Stopwords { lang: String },
    /// Filter by token length
    Length { min: u8, max: u8 },
    /// ASCII folding (remove accents)
    AsciiFolding,
}

/// Generate an auto-generated index name from collection name, index type, and fields.
///
/// Format for BTree/HNSW: `idx_{collection}_{type}_{field1}_{field2}_...`
/// Format for FullText: `{sorted_fields}_fts` (consistent with FTS backend internal naming)
///
/// # Examples
/// - `generate_index_name("users", IndexType::BTree, &["email"])` -> `"idx_users_btree_email"`
/// - `generate_index_name("posts", IndexType::FullText, &["title", "body"])` -> `"body_title_fts"` (sorted)
pub fn generate_index_name(collection: &str, index_type: IndexType, fields: &[String]) -> String {
    match index_type {
        IndexType::FullText => {
            // FTS uses a specific format that matches the internal FTS backend naming
            // Fields are sorted alphabetically for consistency
            let mut sorted_fields: Vec<_> = fields.iter().map(|f| f.replace('.', "_")).collect();
            sorted_fields.sort();
            format!("{}_fts", sorted_fields.join("_"))
        }
        _ => {
            let type_prefix = match index_type {
                IndexType::BTree => "btree",
                IndexType::Hnsw => "hnsw",
                IndexType::FullText => unreachable!(),
            };
            // Replace dots in field paths with underscores for valid identifier
            let fields_part = fields
                .iter()
                .map(|f| f.replace('.', "_"))
                .collect::<Vec<_>>()
                .join("_");
            format!("idx_{collection}_{type_prefix}_{fields_part}")
        }
    }
}

impl IndexDef {
    /// Create an index with an explicit name.
    pub fn new_named(name: impl Into<String>, fields: Vec<String>) -> Self {
        Self {
            name: name.into(),
            fields,
            unique: false,
            index_type: IndexType::BTree,
            hnsw_params: None,
            analyzer: None,
        }
    }

    /// Create a new BTree index with auto-generated name.
    ///
    /// The name will be generated using `generate_index_name` with the provided collection name.
    pub fn new(collection: &str, fields: Vec<String>) -> Self {
        let name = generate_index_name(collection, IndexType::BTree, &fields);
        Self {
            name,
            fields,
            unique: false,
            index_type: IndexType::BTree,
            hnsw_params: None,
            analyzer: None,
        }
    }

    /// Create a unique BTree index with auto-generated name.
    pub fn unique(collection: &str, fields: Vec<String>) -> Self {
        let name = generate_index_name(collection, IndexType::BTree, &fields);
        Self {
            name,
            fields,
            unique: true,
            index_type: IndexType::BTree,
            hnsw_params: None,
            analyzer: None,
        }
    }

    /// Create a full-text search index with auto-generated name.
    pub fn fulltext(collection: &str, fields: Vec<String>) -> Self {
        let name = generate_index_name(collection, IndexType::FullText, &fields);
        Self {
            name,
            fields,
            unique: false,
            index_type: IndexType::FullText,
            hnsw_params: None,
            analyzer: None,
        }
    }

    /// Create an HNSW vector index with auto-generated name.
    pub fn hnsw(collection: &str, fields: Vec<String>, params: HnswParams) -> Self {
        let name = generate_index_name(collection, IndexType::Hnsw, &fields);
        Self {
            name,
            fields,
            unique: false,
            index_type: IndexType::Hnsw,
            hnsw_params: Some(params),
            analyzer: None,
        }
    }
}

/// Schema definition for a collection.
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[rkyv(
    serialize_bounds(__S: Writer + Allocator, <__S as Fallible>::Error: Source),
    deserialize_bounds(<__D as Fallible>::Error: Source),
    bytecheck(bounds(__C: ArchiveContext, <__C as Fallible>::Error: Source))
)]
pub struct CollectionSchema {
    /// Collection name
    pub name: String,
    /// Schema enforcement mode
    #[rkyv(omit_bounds)]
    pub mode: SchemaMode,
    /// Field definitions
    #[rkyv(omit_bounds)]
    pub fields: Vec<FieldDef>,
    /// Index definitions
    #[rkyv(omit_bounds)]
    pub indexes: Vec<IndexDef>,
}

impl CollectionSchema {
    /// Create a new schema with flexible mode (default).
    pub fn flexible(name: impl Into<String>) -> Self {
        CollectionSchema {
            name: name.into(),
            mode: SchemaMode::Flexible,
            fields: Vec::new(),
            indexes: Vec::new(),
        }
    }

    /// Create a new schema with strict mode.
    pub fn strict(name: impl Into<String>) -> Self {
        CollectionSchema {
            name: name.into(),
            mode: SchemaMode::Strict,
            fields: Vec::new(),
            indexes: Vec::new(),
        }
    }

    /// Get a field definition by name.
    pub fn get_field(&self, name: &str) -> Option<&FieldDef> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Check if the schema is in strict mode.
    pub fn is_strict(&self) -> bool {
        matches!(self.mode, SchemaMode::Strict)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_def_with_name() {
        // Test generate_index_name function
        assert_eq!(
            generate_index_name("users", IndexType::BTree, &["email".to_string()]),
            "idx_users_btree_email"
        );
        // FTS uses {sorted_fields}_fts format (sorted alphabetically)
        assert_eq!(
            generate_index_name(
                "posts",
                IndexType::FullText,
                &["title".to_string(), "body".to_string()]
            ),
            "body_title_fts" // sorted: body, title
        );
        assert_eq!(
            generate_index_name("items", IndexType::Hnsw, &["embedding".to_string()]),
            "idx_items_hnsw_embedding"
        );

        // Test new_named constructor (explicit name)
        let idx = IndexDef::new_named("my_custom_index", vec!["field1".to_string()]);
        assert_eq!(idx.name, "my_custom_index");
        assert_eq!(idx.fields, vec!["field1".to_string()]);
        assert_eq!(idx.index_type, IndexType::BTree);
        assert!(!idx.unique);
        assert!(idx.analyzer.is_none());

        // Test new constructor (auto-generated name)
        let idx = IndexDef::new("users", vec!["email".to_string()]);
        assert_eq!(idx.name, "idx_users_btree_email");
        assert_eq!(idx.index_type, IndexType::BTree);
        assert!(!idx.unique);

        // Test unique constructor
        let idx = IndexDef::unique("users", vec!["email".to_string()]);
        assert_eq!(idx.name, "idx_users_btree_email");
        assert!(idx.unique);

        // Test fulltext constructor (uses {sorted_fields}_fts format)
        let idx = IndexDef::fulltext("articles", vec!["content".to_string()]);
        assert_eq!(idx.name, "content_fts");
        assert_eq!(idx.index_type, IndexType::FullText);
        assert!(idx.analyzer.is_none());

        // Test hnsw constructor
        let params = HnswParams::new(128);
        let idx = IndexDef::hnsw("products", vec!["embedding".to_string()], params);
        assert_eq!(idx.name, "idx_products_hnsw_embedding");
        assert_eq!(idx.index_type, IndexType::Hnsw);
        assert!(idx.hnsw_params.is_some());
        assert_eq!(idx.hnsw_params.unwrap().dimension, 128);

        // Test compound index name generation
        let idx = IndexDef::new(
            "orders",
            vec!["customer_id".to_string(), "created_at".to_string()],
        );
        assert_eq!(idx.name, "idx_orders_btree_customer_id_created_at");
    }

    #[test]
    fn test_reference_field_type() {
        // Typed reference - only allows links to specific collection
        let typed = FieldType::Reference(Some("user".to_string()));
        assert!(matches!(typed, FieldType::Reference(Some(_))));

        // Untyped reference - allows links to any collection
        let untyped = FieldType::Reference(None);
        assert!(matches!(untyped, FieldType::Reference(None)));
    }
}
