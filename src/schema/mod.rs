mod error;
mod parser;
mod registry;
mod storage;
mod types;
mod validate;

pub use error::BindError;
pub use parser::{SchemaParseError, parse_schema_file};
pub use registry::SchemaRegistry;
pub use storage::{
    ANALYZER_PREFIX, COLLECTION_PREFIX, analyzer_key, analyzer_name_from_key, collection_key,
    collection_name_from_key, deserialize_analyzer, deserialize_schema, serialize_analyzer,
    serialize_schema,
};
pub use types::{
    AnalyzerDef, CollectionSchema, DefaultValue, DistanceMetric, FieldDef, FieldType, FilterConfig,
    HnswParams, IndexDef, IndexType, SchemaMode, TokenizerConfig, generate_index_name,
};
pub use validate::{
    MAX_NAME_LENGTH, validate_document, validate_field_type, validate_name, validate_name_length,
    validate_schema,
};

// Grammar validation test
#[cfg(test)]
mod grammar_test {
    use pest::Parser;
    use pest_derive::Parser;

    #[derive(Parser)]
    #[grammar = "schema/schema.pest"]
    struct SchemaParser;

    #[test]
    fn grammar_compiles() {
        // Just verifying the grammar is syntactically valid
        let input = r#"
            collection user {
                schema: strict
                field name: string required
                field email: string
                field age: int = 0
                index on email unique
            }
        "#;

        let result = SchemaParser::parse(Rule::schema_file, input);
        assert!(
            result.is_ok(),
            "Grammar should parse basic schema: {:?}",
            result.err()
        );
    }

    #[test]
    fn grammar_parses_nested_objects() {
        let input = r#"
            collection user {
                field address: {
                    street: string required
                    city: string required
                    zip: string
                    country: string = "US"
                }
            }
        "#;

        let result = SchemaParser::parse(Rule::schema_file, input);
        assert!(
            result.is_ok(),
            "Grammar should parse nested objects: {:?}",
            result.err()
        );
    }

    #[test]
    fn grammar_parses_arrays() {
        let input = r#"
            collection user {
                field tags: [string]
                field orders: [{
                    id: string required
                    total: float = 0.0
                }]
            }
        "#;

        let result = SchemaParser::parse(Rule::schema_file, input);
        assert!(
            result.is_ok(),
            "Grammar should parse arrays: {:?}",
            result.err()
        );
    }

    #[test]
    fn grammar_parses_datetime_with_now() {
        let input = r#"
            collection event {
                field created_at: datetime = now()
                field timestamp: datetime required
            }
        "#;

        let result = SchemaParser::parse(Rule::schema_file, input);
        assert!(
            result.is_ok(),
            "Grammar should parse datetime with now(): {:?}",
            result.err()
        );
    }

    #[test]
    fn grammar_parses_compound_indexes() {
        let input = r#"
            collection user {
                field name: string
                field created_at: datetime
                index on (name, created_at)
                index on address.city
            }
        "#;

        let result = SchemaParser::parse(Rule::schema_file, input);
        assert!(
            result.is_ok(),
            "Grammar should parse compound indexes: {:?}",
            result.err()
        );
    }

    #[test]
    fn grammar_parses_multiple_collections() {
        let input = r#"
            collection user {
                schema: strict
                field name: string required
            }

            collection event {
                schema: flexible
                field timestamp: datetime required
                field type: string required
            }
        "#;

        let result = SchemaParser::parse(Rule::schema_file, input);
        assert!(
            result.is_ok(),
            "Grammar should parse multiple collections: {:?}",
            result.err()
        );
    }

    #[test]
    fn grammar_parses_comments() {
        let input = r#"
            // This is a user collection
            collection user {
                schema: strict
                // User's full name
                field name: string required
                field email: string // Must be unique
                index on email unique
            }
        "#;

        let result = SchemaParser::parse(Rule::schema_file, input);
        assert!(
            result.is_ok(),
            "Grammar should ignore comments: {:?}",
            result.err()
        );
    }

    #[test]
    fn grammar_parses_full_example() {
        let input = r#"
            collection user {
                schema: strict

                field name: string required
                field email: string
                field age: int = 0
                field created_at: datetime = now()

                field address: {
                    street: string required
                    city: string required
                    zip: string
                    country: string = "US"
                }

                field tags: [string]

                field orders: [{
                    id: string required
                    total: float = 0.0
                }]

                index on email unique
                index on (name, created_at)
                index on address.city
            }

            collection event {
                schema: flexible

                field timestamp: datetime required
                field type: string required
            }
        "#;

        let result = SchemaParser::parse(Rule::schema_file, input);
        assert!(
            result.is_ok(),
            "Grammar should parse full example: {:?}",
            result.err()
        );
    }
}
