//! Parser for StellarDB schema files (*.stellar).
//!
//! Converts pest grammar output into CollectionSchema types.

use pest::Parser;
use pest_derive::Parser;

use super::{CollectionSchema, DefaultValue, FieldDef, FieldType, IndexDef, IndexType, SchemaMode};

#[derive(Parser)]
#[grammar = "schema/schema.pest"]
struct SchemaParser;

/// Error type for schema parsing
#[derive(Debug, Clone)]
pub struct SchemaParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl std::fmt::Display for SchemaParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Parse error at {}:{}: {}",
            self.line, self.column, self.message
        )
    }
}

impl std::error::Error for SchemaParseError {}

impl SchemaParseError {
    fn new(message: impl Into<String>, line: usize, column: usize) -> Self {
        SchemaParseError {
            message: message.into(),
            line,
            column,
        }
    }

    fn from_pair(message: impl Into<String>, pair: &pest::iterators::Pair<Rule>) -> Self {
        let (line, column) = pair.line_col();
        SchemaParseError {
            message: message.into(),
            line,
            column,
        }
    }
}

/// Parse a schema file content into a list of CollectionSchema
pub fn parse_schema_file(input: &str) -> Result<Vec<CollectionSchema>, SchemaParseError> {
    let pairs = SchemaParser::parse(Rule::schema_file, input).map_err(|e| {
        let (line, column) = match e.line_col {
            pest::error::LineColLocation::Pos((l, c)) => (l, c),
            pest::error::LineColLocation::Span((l, c), _) => (l, c),
        };
        SchemaParseError {
            message: e.to_string(),
            line,
            column,
        }
    })?;

    let mut schemas = Vec::new();

    for pair in pairs {
        if pair.as_rule() == Rule::schema_file {
            for inner in pair.into_inner() {
                if inner.as_rule() == Rule::collection_def {
                    schemas.push(parse_collection(inner)?);
                }
            }
        }
    }

    Ok(schemas)
}

fn parse_collection(
    pair: pest::iterators::Pair<Rule>,
) -> Result<CollectionSchema, SchemaParseError> {
    let mut inner = pair.into_inner();

    // First element is the collection name (ident)
    let name_pair = inner
        .next()
        .ok_or_else(|| SchemaParseError::new("Expected collection name", 0, 0))?;
    let name = name_pair.as_str().to_string();

    // Default mode is Flexible
    let mut mode = SchemaMode::Flexible;
    let mut fields = Vec::new();
    let mut indexes = Vec::new();

    // Next is the collection_body
    let body_pair = inner
        .next()
        .ok_or_else(|| SchemaParseError::new("Expected collection body", 0, 0))?;

    for item in body_pair.into_inner() {
        match item.as_rule() {
            Rule::schema_mode => {
                mode = parse_schema_mode(item)?;
            }
            Rule::field_def => {
                fields.push(parse_field_def(item)?);
            }
            Rule::index_def => {
                indexes.push(parse_index_def(item)?);
            }
            _ => {}
        }
    }

    Ok(CollectionSchema {
        name,
        mode,
        fields,
        indexes,
    })
}

fn parse_schema_mode(pair: pest::iterators::Pair<Rule>) -> Result<SchemaMode, SchemaParseError> {
    let mode_str = pair.as_str();
    if mode_str.contains("strict") {
        Ok(SchemaMode::Strict)
    } else {
        Ok(SchemaMode::Flexible)
    }
}

fn parse_field_def(pair: pest::iterators::Pair<Rule>) -> Result<FieldDef, SchemaParseError> {
    let mut inner = pair.into_inner();

    // Field name (ident)
    let name_pair = inner
        .next()
        .ok_or_else(|| SchemaParseError::new("Expected field name", 0, 0))?;
    let name = name_pair.as_str().to_string();

    // Field type
    let type_pair = inner
        .next()
        .ok_or_else(|| SchemaParseError::new("Expected field type", 0, 0))?;
    let field_type = parse_field_type(type_pair)?;

    // Optional modifiers
    let mut required = false;
    let mut default = None;

    if let Some(modifiers_pair) = inner.next()
        && modifiers_pair.as_rule() == Rule::field_modifiers
    {
        for modifier in modifiers_pair.into_inner() {
            match modifier.as_rule() {
                Rule::required_mod => {
                    required = true;
                }
                Rule::default_mod => {
                    default = Some(parse_default_mod(modifier)?);
                }
                _ => {}
            }
        }
    }

    Ok(FieldDef {
        name,
        field_type,
        required,
        default,
    })
}

fn parse_field_type(pair: pest::iterators::Pair<Rule>) -> Result<FieldType, SchemaParseError> {
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| SchemaParseError::new("Expected field type content", 0, 0))?;

    match inner.as_rule() {
        Rule::union_type => parse_union_type(inner),
        Rule::single_type => parse_single_type(inner),
        _ => Err(SchemaParseError::from_pair(
            format!("Unexpected field type rule: {:?}", inner.as_rule()),
            &inner,
        )),
    }
}

fn parse_single_type(pair: pest::iterators::Pair<Rule>) -> Result<FieldType, SchemaParseError> {
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| SchemaParseError::new("Expected single type content", 0, 0))?;

    match inner.as_rule() {
        Rule::primitive_type => parse_primitive_type(inner),
        Rule::object_type => parse_object_type(inner),
        Rule::array_type => parse_array_type(inner),
        Rule::reference_type => parse_reference_type(inner),
        Rule::range_type => parse_range_type(inner),
        _ => Err(SchemaParseError::from_pair(
            format!("Unexpected single type rule: {:?}", inner.as_rule()),
            &inner,
        )),
    }
}

fn parse_union_type(pair: pest::iterators::Pair<Rule>) -> Result<FieldType, SchemaParseError> {
    let mut variants = Vec::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::single_type {
            variants.push(parse_single_type(inner)?);
        }
    }
    Ok(FieldType::Union(variants))
}

fn parse_reference_type(pair: pest::iterators::Pair<Rule>) -> Result<FieldType, SchemaParseError> {
    let collection = pair.into_inner().next().map(|p| p.as_str().to_string());
    Ok(FieldType::Reference(collection))
}

fn parse_range_type(pair: pest::iterators::Pair<Rule>) -> Result<FieldType, SchemaParseError> {
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| SchemaParseError::new("Expected range element type", 0, 0))?;
    let element_type = parse_primitive_type(inner)?;
    Ok(FieldType::Range(Box::new(element_type)))
}

fn parse_primitive_type(pair: pest::iterators::Pair<Rule>) -> Result<FieldType, SchemaParseError> {
    let type_str = pair.as_str();
    match type_str {
        "string" => Ok(FieldType::String),
        "int" => Ok(FieldType::Int),
        "float" => Ok(FieldType::Float),
        "decimal" => Ok(FieldType::Decimal),
        "bool" => Ok(FieldType::Bool),
        "datetime" => Ok(FieldType::Datetime),
        "duration" => Ok(FieldType::Duration),
        "bytes" => Ok(FieldType::Bytes),
        "any" => Ok(FieldType::Any),
        _ => Err(SchemaParseError::from_pair(
            format!("Unknown primitive type: {}", type_str),
            &pair,
        )),
    }
}

fn parse_object_type(pair: pest::iterators::Pair<Rule>) -> Result<FieldType, SchemaParseError> {
    let mut fields = Vec::new();
    let mut mode = SchemaMode::Strict;

    for nested in pair.into_inner() {
        match nested.as_rule() {
            Rule::nested_field_def => {
                fields.push(parse_nested_field(nested)?);
            }
            Rule::schema_mode => {
                let mode_str = nested.as_str().to_uppercase();
                mode = if mode_str == "FLEXIBLE" {
                    SchemaMode::Flexible
                } else {
                    SchemaMode::Strict
                };
            }
            _ => {}
        }
    }

    Ok(FieldType::Object { fields, mode })
}

fn parse_array_type(pair: pest::iterators::Pair<Rule>) -> Result<FieldType, SchemaParseError> {
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| SchemaParseError::new("Expected array element type", 0, 0))?;

    let element_type = match inner.as_rule() {
        Rule::primitive_type => {
            let ft = parse_primitive_type(inner)?;
            if ft == FieldType::Any {
                return Ok(FieldType::AnyArray);
            }
            ft
        }
        Rule::object_type => parse_object_type(inner)?,
        Rule::reference_type => parse_reference_type(inner)?,
        Rule::range_type => parse_range_type(inner)?,
        _ => {
            return Err(SchemaParseError::from_pair(
                format!("Unexpected array element type: {:?}", inner.as_rule()),
                &inner,
            ));
        }
    };

    Ok(FieldType::Array(Box::new(element_type)))
}

fn parse_nested_field(pair: pest::iterators::Pair<Rule>) -> Result<FieldDef, SchemaParseError> {
    let mut inner = pair.into_inner();

    // Field name (ident)
    let name_pair = inner
        .next()
        .ok_or_else(|| SchemaParseError::new("Expected nested field name", 0, 0))?;
    let name = name_pair.as_str().to_string();

    // Field type
    let type_pair = inner
        .next()
        .ok_or_else(|| SchemaParseError::new("Expected nested field type", 0, 0))?;
    let field_type = parse_field_type(type_pair)?;

    // Optional modifiers
    let mut required = false;
    let mut default = None;

    if let Some(modifiers_pair) = inner.next()
        && modifiers_pair.as_rule() == Rule::field_modifiers
    {
        for modifier in modifiers_pair.into_inner() {
            match modifier.as_rule() {
                Rule::required_mod => {
                    required = true;
                }
                Rule::default_mod => {
                    default = Some(parse_default_mod(modifier)?);
                }
                _ => {}
            }
        }
    }

    Ok(FieldDef {
        name,
        field_type,
        required,
        default,
    })
}

fn parse_default_mod(pair: pest::iterators::Pair<Rule>) -> Result<DefaultValue, SchemaParseError> {
    let value_pair = pair
        .into_inner()
        .next()
        .ok_or_else(|| SchemaParseError::new("Expected default value", 0, 0))?;

    parse_default_value(value_pair)
}

fn parse_default_value(
    pair: pest::iterators::Pair<Rule>,
) -> Result<DefaultValue, SchemaParseError> {
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| SchemaParseError::new("Expected default value content", 0, 0))?;

    match inner.as_rule() {
        Rule::function_call => {
            let func_name = inner.as_str();
            if func_name == "now()" {
                Ok(DefaultValue::Now)
            } else {
                Err(SchemaParseError::from_pair(
                    format!("Unknown function: {}", func_name),
                    &inner,
                ))
            }
        }
        Rule::string_literal => {
            let s = inner.as_str();
            // Strip quotes
            let stripped = &s[1..s.len() - 1];
            Ok(DefaultValue::String(stripped.to_string()))
        }
        Rule::number => {
            let num_str = inner.as_str();
            if num_str.contains('.') {
                let f: f64 = num_str.parse().map_err(|_| {
                    SchemaParseError::from_pair(format!("Invalid float: {}", num_str), &inner)
                })?;
                Ok(DefaultValue::Float(f))
            } else {
                let i: i64 = num_str.parse().map_err(|_| {
                    SchemaParseError::from_pair(format!("Invalid integer: {}", num_str), &inner)
                })?;
                Ok(DefaultValue::Int(i))
            }
        }
        Rule::boolean => {
            let b = inner.as_str() == "true";
            Ok(DefaultValue::Bool(b))
        }
        _ => Err(SchemaParseError::from_pair(
            format!("Unexpected default value rule: {:?}", inner.as_rule()),
            &inner,
        )),
    }
}

fn parse_index_def(pair: pest::iterators::Pair<Rule>) -> Result<IndexDef, SchemaParseError> {
    let mut inner = pair.into_inner();

    // index_fields
    let fields_pair = inner
        .next()
        .ok_or_else(|| SchemaParseError::new("Expected index fields", 0, 0))?;

    let mut fields = Vec::new();
    for field_path in fields_pair.into_inner() {
        if field_path.as_rule() == Rule::field_path {
            fields.push(field_path.as_str().to_string());
        }
    }

    // Optional unique modifier
    let unique = inner
        .next()
        .map(|p| p.as_rule() == Rule::unique_mod)
        .unwrap_or(false);

    Ok(IndexDef {
        name: String::new(), // Will be generated later with collection context
        fields,
        unique,
        index_type: IndexType::BTree,
        hnsw_params: None,
        analyzer: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_collection() {
        let input = r#"
            collection user {
                field name: string required
                field email: string
                field age: int
            }
        "#;

        let schemas = parse_schema_file(input).expect("Should parse simple collection");
        assert_eq!(schemas.len(), 1);

        let schema = &schemas[0];
        assert_eq!(schema.name, "user");
        assert_eq!(schema.mode, SchemaMode::Flexible); // Default mode
        assert_eq!(schema.fields.len(), 3);

        let name_field = &schema.fields[0];
        assert_eq!(name_field.name, "name");
        assert_eq!(name_field.field_type, FieldType::String);
        assert!(name_field.required);
        assert!(name_field.default.is_none());

        let email_field = &schema.fields[1];
        assert_eq!(email_field.name, "email");
        assert_eq!(email_field.field_type, FieldType::String);
        assert!(!email_field.required);

        let age_field = &schema.fields[2];
        assert_eq!(age_field.name, "age");
        assert_eq!(age_field.field_type, FieldType::Int);
    }

    #[test]
    fn test_parse_with_schema_mode() {
        let input = r#"
            collection user {
                schema: strict
                field name: string required
            }
        "#;

        let schemas = parse_schema_file(input).expect("Should parse with schema mode");
        assert_eq!(schemas.len(), 1);

        let schema = &schemas[0];
        assert_eq!(schema.mode, SchemaMode::Strict);
    }

    #[test]
    fn test_parse_flexible_mode() {
        let input = r#"
            collection user {
                schema: flexible
                field name: string
            }
        "#;

        let schemas = parse_schema_file(input).expect("Should parse flexible mode");
        assert_eq!(schemas[0].mode, SchemaMode::Flexible);
    }

    #[test]
    fn test_parse_nested_object() {
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

        let schemas = parse_schema_file(input).expect("Should parse nested object");
        let schema = &schemas[0];
        assert_eq!(schema.fields.len(), 1);

        let address_field = &schema.fields[0];
        assert_eq!(address_field.name, "address");

        if let FieldType::Object {
            fields: nested_fields,
            ..
        } = &address_field.field_type
        {
            assert_eq!(nested_fields.len(), 4);

            let street = &nested_fields[0];
            assert_eq!(street.name, "street");
            assert!(street.required);

            let country = &nested_fields[3];
            assert_eq!(country.name, "country");
            assert_eq!(
                country.default,
                Some(DefaultValue::String("US".to_string()))
            );
        } else {
            panic!("Expected Object type for address field");
        }
    }

    #[test]
    fn test_parse_array_types() {
        let input = r#"
            collection user {
                field tags: [string]
                field scores: [int]
                field orders: [{
                    id: string required
                    total: float = 0.0
                }]
            }
        "#;

        let schemas = parse_schema_file(input).expect("Should parse array types");
        let schema = &schemas[0];
        assert_eq!(schema.fields.len(), 3);

        // Check [string]
        let tags_field = &schema.fields[0];
        assert_eq!(tags_field.name, "tags");
        if let FieldType::Array(inner) = &tags_field.field_type {
            assert_eq!(**inner, FieldType::String);
        } else {
            panic!("Expected Array type for tags field");
        }

        // Check [int]
        let scores_field = &schema.fields[1];
        if let FieldType::Array(inner) = &scores_field.field_type {
            assert_eq!(**inner, FieldType::Int);
        } else {
            panic!("Expected Array type for scores field");
        }

        // Check array of objects
        let orders_field = &schema.fields[2];
        if let FieldType::Array(inner) = &orders_field.field_type {
            if let FieldType::Object { fields: nested, .. } = inner.as_ref() {
                assert_eq!(nested.len(), 2);
                assert_eq!(nested[0].name, "id");
                assert!(nested[0].required);
                assert_eq!(nested[1].name, "total");
                assert_eq!(nested[1].default, Some(DefaultValue::Float(0.0)));
            } else {
                panic!("Expected Object inside Array for orders field");
            }
        } else {
            panic!("Expected Array type for orders field");
        }
    }

    #[test]
    fn test_parse_defaults() {
        let input = r#"
            collection test {
                field str_default: string = "hello"
                field int_default: int = 42
                field float_default: float = 2.72
                field bool_default: bool = true
                field false_default: bool = false
                field datetime_default: datetime = now()
                field negative_int: int = -5
            }
        "#;

        let schemas = parse_schema_file(input).expect("Should parse defaults");
        let schema = &schemas[0];
        assert_eq!(schema.fields.len(), 7);

        assert_eq!(
            schema.fields[0].default,
            Some(DefaultValue::String("hello".to_string()))
        );
        assert_eq!(schema.fields[1].default, Some(DefaultValue::Int(42)));
        assert_eq!(schema.fields[2].default, Some(DefaultValue::Float(2.72)));
        assert_eq!(schema.fields[3].default, Some(DefaultValue::Bool(true)));
        assert_eq!(schema.fields[4].default, Some(DefaultValue::Bool(false)));
        assert_eq!(schema.fields[5].default, Some(DefaultValue::Now));
        assert_eq!(schema.fields[6].default, Some(DefaultValue::Int(-5)));
    }

    #[test]
    fn test_parse_indexes() {
        let input = r#"
            collection user {
                field name: string
                field email: string
                field created_at: datetime
                index on email unique
                index on name
                index on (name, created_at)
                index on address.city
            }
        "#;

        let schemas = parse_schema_file(input).expect("Should parse indexes");
        let schema = &schemas[0];
        assert_eq!(schema.indexes.len(), 4);

        // Single field unique index
        let email_index = &schema.indexes[0];
        assert_eq!(email_index.fields, vec!["email"]);
        assert!(email_index.unique);

        // Single field non-unique index
        let name_index = &schema.indexes[1];
        assert_eq!(name_index.fields, vec!["name"]);
        assert!(!name_index.unique);

        // Compound index
        let compound_index = &schema.indexes[2];
        assert_eq!(compound_index.fields, vec!["name", "created_at"]);
        assert!(!compound_index.unique);

        // Nested field index
        let nested_index = &schema.indexes[3];
        assert_eq!(nested_index.fields, vec!["address.city"]);
        assert!(!nested_index.unique);
    }

    #[test]
    fn test_parse_multiple_collections() {
        let input = r#"
            collection user {
                schema: strict
                field name: string required
                field email: string
            }

            collection event {
                schema: flexible
                field timestamp: datetime required
                field type: string required
            }

            collection product {
                field name: string
                field price: float = 0.0
            }
        "#;

        let schemas = parse_schema_file(input).expect("Should parse multiple collections");
        assert_eq!(schemas.len(), 3);

        assert_eq!(schemas[0].name, "user");
        assert_eq!(schemas[0].mode, SchemaMode::Strict);
        assert_eq!(schemas[0].fields.len(), 2);

        assert_eq!(schemas[1].name, "event");
        assert_eq!(schemas[1].mode, SchemaMode::Flexible);
        assert_eq!(schemas[1].fields.len(), 2);

        assert_eq!(schemas[2].name, "product");
        assert_eq!(schemas[2].mode, SchemaMode::Flexible); // Default
        assert_eq!(schemas[2].fields.len(), 2);
    }

    #[test]
    fn test_parse_with_comments() {
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

        let schemas = parse_schema_file(input).expect("Should parse with comments");
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].name, "user");
        assert_eq!(schemas[0].fields.len(), 2);
    }

    #[test]
    fn test_parse_required_with_default() {
        let input = r#"
            collection test {
                field a: string required = "default"
                field b: int = 0 required
            }
        "#;

        let schemas = parse_schema_file(input).expect("Should parse required with default");
        let schema = &schemas[0];

        // Field a: required first, then default
        assert!(schema.fields[0].required);
        assert_eq!(
            schema.fields[0].default,
            Some(DefaultValue::String("default".to_string()))
        );

        // Field b: default first, then required
        assert!(schema.fields[1].required);
        assert_eq!(schema.fields[1].default, Some(DefaultValue::Int(0)));
    }

    #[test]
    fn test_parse_all_primitive_types() {
        let input = r#"
            collection test {
                field s: string
                field i: int
                field f: float
                field b: bool
                field d: datetime
            }
        "#;

        let schemas = parse_schema_file(input).expect("Should parse all primitive types");
        let schema = &schemas[0];

        assert_eq!(schema.fields[0].field_type, FieldType::String);
        assert_eq!(schema.fields[1].field_type, FieldType::Int);
        assert_eq!(schema.fields[2].field_type, FieldType::Float);
        assert_eq!(schema.fields[3].field_type, FieldType::Bool);
        assert_eq!(schema.fields[4].field_type, FieldType::Datetime);
    }

    #[test]
    fn test_parse_empty_collection() {
        let input = r#"
            collection empty {
            }
        "#;

        let schemas = parse_schema_file(input).expect("Should parse empty collection");
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].name, "empty");
        assert!(schemas[0].fields.is_empty());
        assert!(schemas[0].indexes.is_empty());
    }

    #[test]
    fn test_parse_error_invalid_syntax() {
        let input = r#"
            collection {
                field name: string
            }
        "#;

        let result = parse_schema_file(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_full_example() {
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

        let schemas = parse_schema_file(input).expect("Should parse full example");
        assert_eq!(schemas.len(), 2);

        // Verify user collection
        let user = &schemas[0];
        assert_eq!(user.name, "user");
        assert_eq!(user.mode, SchemaMode::Strict);
        assert_eq!(user.fields.len(), 7);
        assert_eq!(user.indexes.len(), 3);

        // Verify event collection
        let event = &schemas[1];
        assert_eq!(event.name, "event");
        assert_eq!(event.mode, SchemaMode::Flexible);
        assert_eq!(event.fields.len(), 2);
    }

    #[test]
    fn test_parse_duration_type() {
        let input = r#"
            collection test {
                field ttl: duration
            }
        "#;
        let schemas = parse_schema_file(input).expect("Should parse duration type");
        assert_eq!(schemas[0].fields[0].field_type, FieldType::Duration);
    }

    #[test]
    fn test_parse_bytes_type() {
        let input = r#"
            collection test {
                field data: bytes
            }
        "#;
        let schemas = parse_schema_file(input).expect("Should parse bytes type");
        assert_eq!(schemas[0].fields[0].field_type, FieldType::Bytes);
    }

    #[test]
    fn test_parse_any_type() {
        let input = r#"
            collection test {
                field metadata: any
            }
        "#;
        let schemas = parse_schema_file(input).expect("Should parse any type");
        assert_eq!(schemas[0].fields[0].field_type, FieldType::Any);
    }

    #[test]
    fn test_parse_any_array_type() {
        let input = r#"
            collection test {
                field items: [any]
            }
        "#;
        let schemas = parse_schema_file(input).expect("Should parse [any] as AnyArray");
        assert_eq!(schemas[0].fields[0].field_type, FieldType::AnyArray);
    }

    #[test]
    fn test_parse_reference_untyped() {
        let input = r#"
            collection test {
                field link: ref
            }
        "#;
        let schemas = parse_schema_file(input).expect("Should parse untyped ref");
        assert_eq!(schemas[0].fields[0].field_type, FieldType::Reference(None));
    }

    #[test]
    fn test_parse_reference_typed() {
        let input = r#"
            collection test {
                field author: ref<user>
            }
        "#;
        let schemas = parse_schema_file(input).expect("Should parse typed ref");
        assert_eq!(
            schemas[0].fields[0].field_type,
            FieldType::Reference(Some("user".to_string()))
        );
    }

    #[test]
    fn test_parse_range_type() {
        let input = r#"
            collection test {
                field price_range: range<float>
                field age_range: range<int>
            }
        "#;
        let schemas = parse_schema_file(input).expect("Should parse range types");
        assert_eq!(
            schemas[0].fields[0].field_type,
            FieldType::Range(Box::new(FieldType::Float))
        );
        assert_eq!(
            schemas[0].fields[1].field_type,
            FieldType::Range(Box::new(FieldType::Int))
        );
    }

    #[test]
    fn test_parse_union_type() {
        let input = r#"
            collection test {
                field value: string | int | float
            }
        "#;
        let schemas = parse_schema_file(input).expect("Should parse union type");
        assert_eq!(
            schemas[0].fields[0].field_type,
            FieldType::Union(vec![FieldType::String, FieldType::Int, FieldType::Float])
        );
    }

    #[test]
    fn test_parse_union_with_ref() {
        let input = r#"
            collection test {
                field owner: string | ref<user>
            }
        "#;
        let schemas = parse_schema_file(input).expect("Should parse union with ref");
        assert_eq!(
            schemas[0].fields[0].field_type,
            FieldType::Union(vec![
                FieldType::String,
                FieldType::Reference(Some("user".to_string()))
            ])
        );
    }

    #[test]
    fn test_parse_array_of_ref() {
        let input = r#"
            collection test {
                field friends: [ref<user>]
            }
        "#;
        let schemas = parse_schema_file(input).expect("Should parse array of ref");
        assert_eq!(
            schemas[0].fields[0].field_type,
            FieldType::Array(Box::new(FieldType::Reference(Some("user".to_string()))))
        );
    }

    #[test]
    fn test_parse_new_primitive_types_in_all_positions() {
        let input = r#"
            collection test {
                field d: duration
                field b: bytes
                field a: any
                field arr_d: [duration]
                field arr_b: [bytes]
            }
        "#;
        let schemas =
            parse_schema_file(input).expect("Should parse new primitives in all positions");
        let fields = &schemas[0].fields;
        assert_eq!(fields[0].field_type, FieldType::Duration);
        assert_eq!(fields[1].field_type, FieldType::Bytes);
        assert_eq!(fields[2].field_type, FieldType::Any);
        assert_eq!(
            fields[3].field_type,
            FieldType::Array(Box::new(FieldType::Duration))
        );
        assert_eq!(
            fields[4].field_type,
            FieldType::Array(Box::new(FieldType::Bytes))
        );
    }
}
