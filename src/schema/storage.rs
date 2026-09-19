//! Schema serialization and storage helpers for StellarDB.

use rkyv::rancor;

use super::{AnalyzerDef, CollectionSchema};
use crate::error::StorageError;

/// Key prefix for collection schema entries.
pub const COLLECTION_PREFIX: &[u8] = b"coll\x00";

/// Key prefix for custom analyzer entries.
pub const ANALYZER_PREFIX: &[u8] = b"analyzer\x00";

/// Serialize a schema to bytes using rkyv.
pub fn serialize_schema(schema: &CollectionSchema) -> Result<Vec<u8>, StorageError> {
    rkyv::to_bytes::<rancor::Error>(schema)
        .map(|bytes| bytes.to_vec())
        .map_err(|e| StorageError::Serialization(e.to_string()))
}

/// Deserialize a schema from bytes using rkyv.
pub fn deserialize_schema(bytes: &[u8]) -> Result<CollectionSchema, StorageError> {
    rkyv::from_bytes::<CollectionSchema, rancor::Error>(bytes)
        .map_err(|e| StorageError::Serialization(e.to_string()))
}

/// Build a storage key for a collection schema.
pub fn collection_key(name: &str) -> Vec<u8> {
    let mut key = COLLECTION_PREFIX.to_vec();
    key.extend_from_slice(name.as_bytes());
    key
}

/// Extract the collection name from a storage key.
pub fn collection_name_from_key(key: &[u8]) -> Option<String> {
    if key.starts_with(COLLECTION_PREFIX) {
        let name_bytes = &key[COLLECTION_PREFIX.len()..];
        String::from_utf8(name_bytes.to_vec()).ok()
    } else {
        None
    }
}

/// Serialize an analyzer definition to bytes using rkyv.
pub fn serialize_analyzer(analyzer: &AnalyzerDef) -> Result<Vec<u8>, StorageError> {
    rkyv::to_bytes::<rancor::Error>(analyzer)
        .map(|bytes| bytes.to_vec())
        .map_err(|e| StorageError::Serialization(e.to_string()))
}

/// Deserialize an analyzer definition from bytes using rkyv.
pub fn deserialize_analyzer(bytes: &[u8]) -> Result<AnalyzerDef, StorageError> {
    rkyv::from_bytes::<AnalyzerDef, rancor::Error>(bytes)
        .map_err(|e| StorageError::Serialization(e.to_string()))
}

/// Build a storage key for a custom analyzer.
pub fn analyzer_key(name: &str) -> Vec<u8> {
    let mut key = ANALYZER_PREFIX.to_vec();
    key.extend_from_slice(name.as_bytes());
    key
}

/// Extract the analyzer name from a storage key.
pub fn analyzer_name_from_key(key: &[u8]) -> Option<String> {
    if key.starts_with(ANALYZER_PREFIX) {
        let name_bytes = &key[ANALYZER_PREFIX.len()..];
        String::from_utf8(name_bytes.to_vec()).ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{FieldDef, FieldType, FilterConfig, IndexDef, SchemaMode, TokenizerConfig};

    fn create_test_schema() -> CollectionSchema {
        CollectionSchema {
            name: "products".to_string(),
            mode: SchemaMode::Strict,
            fields: vec![
                FieldDef {
                    name: "name".to_string(),
                    field_type: FieldType::String,
                    required: true,
                    default: None,
                },
                FieldDef {
                    name: "price".to_string(),
                    field_type: FieldType::Float,
                    required: true,
                    default: None,
                },
                FieldDef {
                    name: "tags".to_string(),
                    field_type: FieldType::Array(Box::new(FieldType::String)),
                    required: false,
                    default: None,
                },
            ],
            indexes: vec![IndexDef::unique("products", vec!["name".to_string()])],
        }
    }

    #[test]
    fn test_serialize_deserialize() {
        let schema = create_test_schema();

        let bytes = serialize_schema(&schema).expect("serialization should succeed");
        assert!(!bytes.is_empty());

        let deserialized = deserialize_schema(&bytes).expect("deserialization should succeed");

        assert_eq!(deserialized.name, schema.name);
        assert_eq!(deserialized.mode, schema.mode);
        assert_eq!(deserialized.fields.len(), schema.fields.len());
        assert_eq!(deserialized.indexes.len(), schema.indexes.len());

        // Verify field details
        assert_eq!(deserialized.fields[0].name, "name");
        assert_eq!(deserialized.fields[0].field_type, FieldType::String);
        assert!(deserialized.fields[0].required);

        assert_eq!(deserialized.fields[1].name, "price");
        assert_eq!(deserialized.fields[1].field_type, FieldType::Float);

        // Verify index
        assert!(deserialized.indexes[0].unique);
        assert_eq!(deserialized.indexes[0].fields, vec!["name"]);
    }

    #[test]
    fn test_collection_key() {
        let key = collection_key("users");
        assert!(key.starts_with(COLLECTION_PREFIX));
        assert_eq!(&key[COLLECTION_PREFIX.len()..], b"users");

        let key2 = collection_key("my_collection");
        assert_eq!(&key2[COLLECTION_PREFIX.len()..], b"my_collection");
    }

    #[test]
    fn test_collection_name_from_key() {
        let key = collection_key("posts");
        let name = collection_name_from_key(&key);
        assert_eq!(name, Some("posts".to_string()));

        let key2 = collection_key("my-special-collection");
        let name2 = collection_name_from_key(&key2);
        assert_eq!(name2, Some("my-special-collection".to_string()));

        // Invalid prefix returns None
        let invalid_key = b"invalid\x00posts";
        assert_eq!(collection_name_from_key(invalid_key), None);

        // Empty name
        let empty_name_key = collection_key("");
        assert_eq!(
            collection_name_from_key(&empty_name_key),
            Some("".to_string())
        );
    }

    fn create_test_analyzer() -> AnalyzerDef {
        AnalyzerDef {
            name: "my_analyzer".to_string(),
            tokenizer: TokenizerConfig::Whitespace,
            filters: vec![
                FilterConfig::Lowercase,
                FilterConfig::Stemmer {
                    lang: "english".to_string(),
                },
            ],
        }
    }

    #[test]
    fn test_analyzer_serialize_deserialize() {
        let analyzer = create_test_analyzer();

        let bytes = serialize_analyzer(&analyzer).expect("serialization should succeed");
        assert!(!bytes.is_empty());

        let deserialized = deserialize_analyzer(&bytes).expect("deserialization should succeed");

        assert_eq!(deserialized.name, analyzer.name);
        assert_eq!(deserialized.tokenizer, TokenizerConfig::Whitespace);
        assert_eq!(deserialized.filters.len(), 2);
        assert_eq!(deserialized.filters[0], FilterConfig::Lowercase);
        assert_eq!(
            deserialized.filters[1],
            FilterConfig::Stemmer {
                lang: "english".to_string()
            }
        );
    }

    #[test]
    fn test_analyzer_key() {
        let key = analyzer_key("my_analyzer");
        assert!(key.starts_with(ANALYZER_PREFIX));
        assert_eq!(&key[ANALYZER_PREFIX.len()..], b"my_analyzer");

        let key2 = analyzer_key("custom-ngram");
        assert_eq!(&key2[ANALYZER_PREFIX.len()..], b"custom-ngram");
    }

    #[test]
    fn test_analyzer_name_from_key() {
        let key = analyzer_key("my_analyzer");
        let name = analyzer_name_from_key(&key);
        assert_eq!(name, Some("my_analyzer".to_string()));

        let key2 = analyzer_key("special-analyzer");
        let name2 = analyzer_name_from_key(&key2);
        assert_eq!(name2, Some("special-analyzer".to_string()));

        // Invalid prefix returns None
        let invalid_key = b"invalid\x00my_analyzer";
        assert_eq!(analyzer_name_from_key(invalid_key), None);

        // Empty name
        let empty_name_key = analyzer_key("");
        assert_eq!(
            analyzer_name_from_key(&empty_name_key),
            Some("".to_string())
        );
    }

    #[test]
    fn test_analyzer_with_ngram_tokenizer() {
        let analyzer = AnalyzerDef {
            name: "ngram_analyzer".to_string(),
            tokenizer: TokenizerConfig::Ngram { min: 2, max: 4 },
            filters: vec![FilterConfig::Lowercase, FilterConfig::AsciiFolding],
        };

        let bytes = serialize_analyzer(&analyzer).expect("serialization should succeed");
        let deserialized = deserialize_analyzer(&bytes).expect("deserialization should succeed");

        assert_eq!(
            deserialized.tokenizer,
            TokenizerConfig::Ngram { min: 2, max: 4 }
        );
        assert_eq!(deserialized.filters.len(), 2);
    }
}
