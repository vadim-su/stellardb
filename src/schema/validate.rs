//! Document validation against schema definitions.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::document::Value;
use crate::schema::{
    BindError, CollectionSchema, DefaultValue, FieldDef, FieldType, IndexType, SchemaMode,
};
use crate::util::suggestions::find_similar;

/// Maximum length for collection, index, and analyzer names.
/// This limit ensures keyspace names stay within filesystem and storage engine limits.
pub const MAX_NAME_LENGTH: usize = 120;

/// Validate a document value against a schema, return validated fields with defaults applied.
///
/// # Arguments
/// * `schema` - The collection schema to validate against
/// * `fields` - The document fields to validate
///
/// # Returns
/// * `Ok(HashMap<String, Value>)` - Validated fields with defaults applied
/// * `Err(BindError)` - Validation error
pub fn validate_document(
    schema: &CollectionSchema,
    fields: &HashMap<String, Value>,
) -> Result<HashMap<String, Value>, BindError> {
    let mut result = fields.clone();

    // Validate each schema field
    for field_def in &schema.fields {
        match fields.get(&field_def.name) {
            Some(value) => {
                // Field is present - check for null on required fields
                if field_def.required && matches!(value, Value::Null) {
                    return Err(BindError::RequiredFieldNull {
                        collection: schema.name.clone(),
                        field: field_def.name.clone(),
                    });
                }

                // Validate type (null is always OK for optional fields)
                if !matches!(value, Value::Null) {
                    // For Object fields, apply nested defaults
                    if let (
                        Value::Object(obj_fields),
                        FieldType::Object {
                            fields: nested_defs,
                            mode,
                        },
                    ) = (value, &field_def.field_type)
                    {
                        let validated = validate_and_default_object_fields(
                            &field_def.name,
                            obj_fields,
                            nested_defs,
                            mode.clone(),
                        )?;
                        result.insert(field_def.name.clone(), Value::Object(validated));
                    } else {
                        validate_field_type(&field_def.name, value, &field_def.field_type)?;
                    }
                }
            }
            None => {
                // Field is missing - check if required
                if field_def.required {
                    // Check for default value
                    if let Some(default) = &field_def.default {
                        result.insert(field_def.name.clone(), default_to_value(default));
                    } else {
                        return Err(BindError::RequiredFieldMissing {
                            collection: schema.name.clone(),
                            field: field_def.name.clone(),
                        });
                    }
                } else if let Some(default) = &field_def.default {
                    // Optional field with default - apply it
                    result.insert(field_def.name.clone(), default_to_value(default));
                }
            }
        }
    }

    // In strict mode: error on unknown fields (skip "id" - it's the document identifier)
    if matches!(schema.mode, SchemaMode::Strict) {
        for field_name in fields.keys() {
            // Skip "id" field - it's the document identifier, not a user field
            if field_name == "id" {
                continue;
            }
            if !schema.fields.iter().any(|f| &f.name == field_name) {
                let field_names: Vec<&str> =
                    schema.fields.iter().map(|f| f.name.as_str()).collect();
                return Err(BindError::UnknownField {
                    collection: schema.name.clone(),
                    field: field_name.clone(),
                    suggestions: find_similar(field_name, field_names.into_iter(), 3)
                        .into_iter()
                        .map(|s| s.to_string())
                        .collect(),
                });
            }
        }
    }

    Ok(result)
}

/// Validate a field value against an expected type.
pub fn validate_field_type(
    name: &str,
    value: &Value,
    expected: &FieldType,
) -> Result<(), BindError> {
    match (value, expected) {
        // Null is handled by the caller (required check)
        (Value::Null, _) => Ok(()),

        // Any type accepts everything
        (_, FieldType::Any) => Ok(()),

        // String matches String
        (Value::String(_), FieldType::String) => Ok(()),

        // Int matches Int
        (Value::Int(_), FieldType::Int) => Ok(()),

        // Float matches Float, Int can coerce to Float
        (Value::Float(_), FieldType::Float) => Ok(()),
        (Value::Int(_), FieldType::Float) => Ok(()), // Int -> Float coercion

        // Decimal matches Decimal, Int and Float can coerce to Decimal
        (Value::Decimal(_), FieldType::Decimal) => Ok(()),
        (Value::Int(_), FieldType::Decimal) => Ok(()), // Int -> Decimal coercion
        (Value::Float(_), FieldType::Decimal) => Ok(()), // Float -> Decimal coercion

        // Bool matches Bool
        (Value::Bool(_), FieldType::Bool) => Ok(()),

        // Datetime accepts Datetime, String (ISO 8601) or Int (timestamp)
        (Value::Datetime(_), FieldType::Datetime) => Ok(()),
        (Value::String(_), FieldType::Datetime) => Ok(()),
        (Value::Int(_), FieldType::Datetime) => Ok(()),

        // Duration matches Duration
        (Value::Duration(_), FieldType::Duration) => Ok(()),

        // Bytes matches Bytes
        (Value::Bytes(_), FieldType::Bytes) => Ok(()),

        // Object: recursively validate each field (defaults handled by validate_document)
        (
            Value::Object(obj_fields),
            FieldType::Object {
                fields: field_defs,
                mode,
            },
        ) => {
            validate_and_default_object_fields(name, obj_fields, field_defs, mode.clone())?;
            Ok(())
        }

        // Array: validate each element against inner type
        (Value::Array(elements), FieldType::Array(inner_type)) => {
            for (index, element) in elements.iter().enumerate() {
                if let Err(e) = validate_field_type("element", element, inner_type) {
                    return Err(BindError::ArrayElementError {
                        field: name.to_string(),
                        index,
                        error: Box::new(e),
                    });
                }
            }
            Ok(())
        }

        // AnyArray: accept any array without element validation
        (Value::Array(_), FieldType::AnyArray) => Ok(()),

        // Reference: validate that Value::Reference matches the expected collection (if specified)
        (Value::Reference(id), FieldType::Reference(expected_collection)) => {
            if let Some(expected) = expected_collection {
                // Check that reference starts with expected collection prefix
                if !id.starts_with(&format!("{}:", expected)) {
                    return Err(BindError::TypeMismatch {
                        field: name.to_string(),
                        expected: format!("Reference<{}>", expected),
                        got: format!("Reference({})", id),
                    });
                }
            }
            Ok(())
        }

        // Range: validate start and end against element type
        (Value::Range { start, end }, FieldType::Range(elem_type)) => {
            if !matches!(**start, Value::Null) {
                validate_field_type(name, start, elem_type)?;
            }
            if !matches!(**end, Value::Null) {
                validate_field_type(name, end, elem_type)?;
            }
            Ok(())
        }

        // Union: value must match at least one type in the union
        (_, FieldType::Union(types)) => {
            for t in types {
                if validate_field_type(name, value, t).is_ok() {
                    return Ok(());
                }
            }
            Err(BindError::TypeMismatch {
                field: name.to_string(),
                expected: format_type(expected),
                got: format_value_type(value),
            })
        }

        // Type mismatch
        _ => Err(BindError::TypeMismatch {
            field: name.to_string(),
            expected: format_type(expected),
            got: format_value_type(value),
        }),
    }
}

/// Validate and apply defaults to object fields against field definitions.
/// Returns the updated fields with defaults applied.
fn validate_and_default_object_fields(
    parent_name: &str,
    fields: &HashMap<String, Value>,
    field_defs: &[FieldDef],
    mode: SchemaMode,
) -> Result<HashMap<String, Value>, BindError> {
    let mut result = fields.clone();

    // In strict mode, reject unknown fields
    if mode == SchemaMode::Strict {
        let defined_fields: std::collections::HashSet<&str> =
            field_defs.iter().map(|f| f.name.as_str()).collect();
        for field_name in fields.keys() {
            if !defined_fields.contains(field_name.as_str()) {
                let field_names: Vec<&str> = field_defs.iter().map(|f| f.name.as_str()).collect();
                return Err(BindError::NestedError {
                    path: parent_name.to_string(),
                    error: Box::new(BindError::UnknownField {
                        collection: parent_name.to_string(),
                        field: field_name.clone(),
                        suggestions: find_similar(field_name, field_names.into_iter(), 3)
                            .into_iter()
                            .map(|s| s.to_string())
                            .collect(),
                    }),
                });
            }
        }
    }

    for field_def in field_defs {
        match fields.get(&field_def.name) {
            Some(value) => {
                // Check for null on required fields
                if field_def.required && matches!(value, Value::Null) {
                    return Err(BindError::NestedError {
                        path: parent_name.to_string(),
                        error: Box::new(BindError::TypeMismatch {
                            field: field_def.name.clone(),
                            expected: format_type(&field_def.field_type),
                            got: "Null".to_string(),
                        }),
                    });
                }

                // For nested objects, recursively validate and apply defaults
                if !matches!(value, Value::Null) {
                    if let (
                        Value::Object(nested_fields),
                        FieldType::Object {
                            fields: nested_defs,
                            mode: nested_mode,
                        },
                    ) = (value, &field_def.field_type)
                    {
                        let validated = validate_and_default_object_fields(
                            &field_def.name,
                            nested_fields,
                            nested_defs,
                            nested_mode.clone(),
                        )?;
                        result.insert(field_def.name.clone(), Value::Object(validated));
                    } else if let Err(e) =
                        validate_field_type(&field_def.name, value, &field_def.field_type)
                    {
                        return Err(BindError::NestedError {
                            path: parent_name.to_string(),
                            error: Box::new(e),
                        });
                    }
                }
            }
            None => {
                if let Some(default) = &field_def.default {
                    result.insert(field_def.name.clone(), default_to_value(default));
                } else if field_def.required {
                    return Err(BindError::NestedError {
                        path: parent_name.to_string(),
                        error: Box::new(BindError::TypeMismatch {
                            field: field_def.name.clone(),
                            expected: format_type(&field_def.field_type),
                            got: "missing".to_string(),
                        }),
                    });
                }
            }
        }
    }
    Ok(result)
}

/// Convert a DefaultValue to a Value.
fn default_to_value(default: &DefaultValue) -> Value {
    match default {
        DefaultValue::Null => Value::Null,
        DefaultValue::Bool(b) => Value::Bool(*b),
        DefaultValue::Int(i) => Value::Int(*i),
        DefaultValue::Float(f) => Value::Float(*f),
        DefaultValue::String(s) => Value::String(s.clone()),
        DefaultValue::Now => {
            // Current unix timestamp as Datetime
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            Value::Datetime(timestamp)
        }
    }
}

/// Format a FieldType as a string for error messages.
fn format_type(t: &FieldType) -> String {
    match t {
        FieldType::String => "String".to_string(),
        FieldType::Int => "Int".to_string(),
        FieldType::Float => "Float".to_string(),
        FieldType::Decimal => "Decimal".to_string(),
        FieldType::Bool => "Bool".to_string(),
        FieldType::Datetime => "Datetime".to_string(),
        FieldType::Duration => "Duration".to_string(),
        FieldType::Bytes => "Bytes".to_string(),
        FieldType::Object { .. } => "Object".to_string(),
        FieldType::Array(inner) => format!("Array<{}>", format_type(inner)),
        FieldType::AnyArray => "Array".to_string(),
        FieldType::Reference(Some(coll)) => format!("Reference<{}>", coll),
        FieldType::Reference(None) => "Reference".to_string(),
        FieldType::Any => "Any".to_string(),
        FieldType::Range(inner) => format!("Range<{}>", format_type(inner)),
        FieldType::Union(types) => types
            .iter()
            .map(format_type)
            .collect::<Vec<_>>()
            .join(" | "),
    }
}

/// Format a Value type as a string for error messages.
fn format_value_type(v: &Value) -> String {
    match v {
        Value::Null => "Null".to_string(),
        Value::Bool(_) => "Bool".to_string(),
        Value::Int(_) => "Int".to_string(),
        Value::Float(_) => "Float".to_string(),
        Value::Decimal(_) => "Decimal".to_string(),
        Value::String(_) => "String".to_string(),
        Value::Array(_) => "Array".to_string(),
        Value::Object(_) => "Object".to_string(),
        Value::Reference(_) => "Reference".to_string(),
        Value::Datetime(_) => "Datetime".to_string(),
        Value::Duration(_) => "Duration".to_string(),
        Value::Bytes(_) => "Bytes".to_string(),
        Value::Range { .. } => "Range".to_string(),
    }
}

/// Validate a schema definition for errors like duplicate fields or type mismatches.
///
/// # Arguments
/// * `schema` - The schema to validate
///
/// # Returns
/// * `Ok(())` - Schema is valid
/// * `Err(BindError)` - Validation error
pub fn validate_schema(schema: &CollectionSchema) -> Result<(), BindError> {
    validate_name("collection", &schema.name)?;
    validate_fields_no_duplicates(&schema.name, &schema.fields)?;
    validate_default_types(&schema.name, &schema.fields)?;
    validate_indexes(schema)?;
    Ok(())
}

/// Validate that a name does not exceed the maximum length.
pub fn validate_name_length(kind: &str, name: &str) -> Result<(), BindError> {
    if name.len() > MAX_NAME_LENGTH {
        return Err(BindError::NameTooLong {
            kind: kind.to_string(),
            name: name.to_string(),
            max_length: MAX_NAME_LENGTH,
        });
    }
    Ok(())
}

/// Validate name for basic requirements:
/// - Not empty
/// - No null bytes (breaks storage)
/// - No control characters
/// - No path traversal characters (`/`, `\`, `..`)
pub fn validate_name(kind: &str, name: &str) -> Result<(), BindError> {
    // Check empty
    if name.is_empty() {
        return Err(BindError::InvalidName {
            kind: kind.to_string(),
            reason: "name cannot be empty".to_string(),
        });
    }

    // Check for path traversal: reject `/`, `\`, and `..`
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(BindError::InvalidName {
            kind: kind.to_string(),
            reason: "name cannot contain path separators or '..'".to_string(),
        });
    }

    // Reject bare `.` (current directory reference)
    if name == "." {
        return Err(BindError::InvalidName {
            kind: kind.to_string(),
            reason: "name cannot be '.'".to_string(),
        });
    }

    // Check for null bytes (breaks keyspace names)
    if name.contains('\0') {
        return Err(BindError::InvalidName {
            kind: kind.to_string(),
            reason: "name cannot contain null bytes".to_string(),
        });
    }

    // Check for control characters (including tab, newline, carriage return)
    for c in name.chars() {
        if c.is_control() {
            return Err(BindError::InvalidName {
                kind: kind.to_string(),
                reason: "name cannot contain control characters".to_string(),
            });
        }
    }

    // Check length
    validate_name_length(kind, name)?;

    Ok(())
}

/// Check for duplicate field names in a list of fields (including nested objects).
fn validate_fields_no_duplicates(collection: &str, fields: &[FieldDef]) -> Result<(), BindError> {
    let mut seen = std::collections::HashSet::new();

    for field in fields {
        if !seen.insert(&field.name) {
            return Err(BindError::DuplicateFieldName {
                collection: collection.to_string(),
                field: field.name.clone(),
            });
        }

        // Check nested object fields recursively
        if let FieldType::Object {
            fields: nested_fields,
            ..
        } = &field.field_type
        {
            validate_fields_no_duplicates(collection, nested_fields)?;
        }

        // Check array element type if it's an object
        if let FieldType::Array(inner) = &field.field_type
            && let FieldType::Object {
                fields: nested_fields,
                ..
            } = inner.as_ref()
        {
            validate_fields_no_duplicates(collection, nested_fields)?;
        }
    }

    Ok(())
}

/// Check that default values match their field types.
fn validate_default_types(collection: &str, fields: &[FieldDef]) -> Result<(), BindError> {
    for field in fields {
        if let Some(default) = &field.default {
            let is_valid = match (&field.field_type, default) {
                (FieldType::String, DefaultValue::String(_)) => true,
                (FieldType::Int, DefaultValue::Int(_)) => true,
                (FieldType::Float, DefaultValue::Float(_)) => true,
                (FieldType::Float, DefaultValue::Int(_)) => true, // Int coerces to Float
                (FieldType::Decimal, DefaultValue::Float(_)) => true, // Float coerces to Decimal
                (FieldType::Decimal, DefaultValue::Int(_)) => true, // Int coerces to Decimal
                (FieldType::Bool, DefaultValue::Bool(_)) => true,
                (FieldType::Datetime, DefaultValue::Now) => true,
                (FieldType::Datetime, DefaultValue::Int(_)) => true, // Unix timestamp
                (FieldType::Datetime, DefaultValue::String(_)) => true, // ISO string
                (FieldType::Any, _) => true,                         // Any type accepts any default
                (_, DefaultValue::Null) => !field.required,          // Null OK for optional
                _ => false,
            };

            if !is_valid {
                return Err(BindError::DefaultValueTypeMismatch {
                    collection: collection.to_string(),
                    field: field.name.clone(),
                    field_type: format_type(&field.field_type),
                    default_type: format_default_type(default),
                });
            }
        }

        // Check nested object fields recursively
        if let FieldType::Object {
            fields: nested_fields,
            ..
        } = &field.field_type
        {
            validate_default_types(collection, nested_fields)?;
        }

        // Check array element type if it's an object
        if let FieldType::Array(inner) = &field.field_type
            && let FieldType::Object {
                fields: nested_fields,
                ..
            } = inner.as_ref()
        {
            validate_default_types(collection, nested_fields)?;
        }
    }

    Ok(())
}

/// Validate index definitions in a schema.
fn validate_indexes(schema: &CollectionSchema) -> Result<(), BindError> {
    let mut seen_names = std::collections::HashSet::new();

    // Collect all known top-level field names for field-existence checks.
    // Only validate field references when the schema has explicit field definitions.
    let field_names: std::collections::HashSet<&str> =
        schema.fields.iter().map(|f| f.name.as_str()).collect();
    let has_fields = !schema.fields.is_empty();

    for index in &schema.indexes {
        // Index names may be empty at parse time (auto-generated later in DDL execution).
        // Only validate non-empty names.
        if !index.name.is_empty() {
            if !seen_names.insert(&index.name) {
                return Err(BindError::DuplicateIndexName {
                    collection: schema.name.clone(),
                    index: index.name.clone(),
                });
            }
            validate_name("index", &index.name)?;
        }

        // Check that indexed fields exist in the schema (only when fields are defined)
        if has_fields {
            for field in &index.fields {
                // For dot-separated paths, check the top-level segment
                let top_level = field.split('.').next().unwrap_or(field);
                if !field_names.contains(top_level) {
                    return Err(BindError::IndexFieldNotFound {
                        collection: schema.name.clone(),
                        index: index.name.clone(),
                        field: field.clone(),
                    });
                }
            }
        }

        // HNSW-specific validation
        if index.index_type == IndexType::Hnsw {
            match &index.hnsw_params {
                None => {
                    return Err(BindError::InvalidIndexConfig {
                        collection: schema.name.clone(),
                        index: index.name.clone(),
                        reason: "HNSW index requires hnsw_params with dimension > 0".to_string(),
                    });
                }
                Some(params) if params.dimension == 0 => {
                    return Err(BindError::InvalidIndexConfig {
                        collection: schema.name.clone(),
                        index: index.name.clone(),
                        reason: "HNSW dimension must be greater than 0".to_string(),
                    });
                }
                _ => {}
            }
        }
    }

    Ok(())
}

/// Format a DefaultValue type as a string for error messages.
fn format_default_type(d: &DefaultValue) -> String {
    match d {
        DefaultValue::Null => "Null".to_string(),
        DefaultValue::Bool(_) => "Bool".to_string(),
        DefaultValue::Int(_) => "Int".to_string(),
        DefaultValue::Float(_) => "Float".to_string(),
        DefaultValue::String(_) => "String".to_string(),
        DefaultValue::Now => "Now()".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{HnswParams, IndexDef};

    fn make_schema(mode: SchemaMode, fields: Vec<FieldDef>) -> CollectionSchema {
        CollectionSchema {
            name: "test".to_string(),
            mode,
            fields,
            indexes: vec![],
        }
    }

    fn make_field(name: &str, field_type: FieldType, required: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            field_type,
            required,
            default: None,
        }
    }

    fn make_field_with_default(
        name: &str,
        field_type: FieldType,
        required: bool,
        default: DefaultValue,
    ) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            field_type,
            required,
            default: Some(default),
        }
    }

    #[test]
    fn test_valid_document() {
        let schema = make_schema(
            SchemaMode::Flexible,
            vec![
                make_field("name", FieldType::String, true),
                make_field("age", FieldType::Int, false),
                make_field("active", FieldType::Bool, false),
            ],
        );

        let mut fields = HashMap::new();
        fields.insert("name".to_string(), Value::String("Alice".to_string()));
        fields.insert("age".to_string(), Value::Int(30));
        fields.insert("active".to_string(), Value::Bool(true));

        let result = validate_document(&schema, &fields);
        assert!(result.is_ok());

        let validated = result.unwrap();
        assert_eq!(
            validated.get("name"),
            Some(&Value::String("Alice".to_string()))
        );
        assert_eq!(validated.get("age"), Some(&Value::Int(30)));
        assert_eq!(validated.get("active"), Some(&Value::Bool(true)));
    }

    #[test]
    fn test_missing_required_field() {
        let schema = make_schema(
            SchemaMode::Flexible,
            vec![
                make_field("name", FieldType::String, true),
                make_field("email", FieldType::String, true),
            ],
        );

        let mut fields = HashMap::new();
        fields.insert("name".to_string(), Value::String("Bob".to_string()));
        // Missing "email" field

        let result = validate_document(&schema, &fields);
        assert!(result.is_err());

        match result.unwrap_err() {
            BindError::RequiredFieldMissing { collection, field } => {
                assert_eq!(collection, "test");
                assert_eq!(field, "email");
            }
            _ => panic!("Expected RequiredFieldMissing error"),
        }
    }

    #[test]
    fn test_default_value_applied() {
        let schema = make_schema(
            SchemaMode::Flexible,
            vec![
                make_field("name", FieldType::String, true),
                make_field_with_default(
                    "status",
                    FieldType::String,
                    true,
                    DefaultValue::String("pending".to_string()),
                ),
                make_field_with_default("count", FieldType::Int, false, DefaultValue::Int(0)),
            ],
        );

        let mut fields = HashMap::new();
        fields.insert("name".to_string(), Value::String("Charlie".to_string()));
        // Missing "status" and "count" - should use defaults

        let result = validate_document(&schema, &fields);
        assert!(result.is_ok());

        let validated = result.unwrap();
        assert_eq!(
            validated.get("status"),
            Some(&Value::String("pending".to_string()))
        );
        assert_eq!(validated.get("count"), Some(&Value::Int(0)));
    }

    #[test]
    fn test_type_mismatch() {
        let schema = make_schema(
            SchemaMode::Flexible,
            vec![
                make_field("name", FieldType::String, true),
                make_field("age", FieldType::Int, true),
            ],
        );

        let mut fields = HashMap::new();
        fields.insert("name".to_string(), Value::String("Dave".to_string()));
        fields.insert("age".to_string(), Value::String("not a number".to_string())); // Wrong type

        let result = validate_document(&schema, &fields);
        assert!(result.is_err());

        match result.unwrap_err() {
            BindError::TypeMismatch {
                field,
                expected,
                got,
            } => {
                assert_eq!(field, "age");
                assert_eq!(expected, "Int");
                assert_eq!(got, "String");
            }
            _ => panic!("Expected TypeMismatch error"),
        }
    }

    #[test]
    fn test_unknown_field_strict() {
        let schema = make_schema(
            SchemaMode::Strict,
            vec![make_field("name", FieldType::String, true)],
        );

        let mut fields = HashMap::new();
        fields.insert("name".to_string(), Value::String("Eve".to_string()));
        fields.insert(
            "unknown_field".to_string(),
            Value::String("should fail".to_string()),
        );

        let result = validate_document(&schema, &fields);
        assert!(result.is_err());

        match result.unwrap_err() {
            BindError::UnknownField {
                collection, field, ..
            } => {
                assert_eq!(collection, "test");
                assert_eq!(field, "unknown_field");
            }
            _ => panic!("Expected UnknownField error"),
        }
    }

    #[test]
    fn test_flexible_allows_unknown() {
        let schema = make_schema(
            SchemaMode::Flexible,
            vec![make_field("name", FieldType::String, true)],
        );

        let mut fields = HashMap::new();
        fields.insert("name".to_string(), Value::String("Frank".to_string()));
        fields.insert(
            "extra_field".to_string(),
            Value::String("allowed".to_string()),
        );

        let result = validate_document(&schema, &fields);
        assert!(result.is_ok());

        let validated = result.unwrap();
        assert_eq!(
            validated.get("extra_field"),
            Some(&Value::String("allowed".to_string()))
        );
    }

    #[test]
    fn test_required_field_null() {
        let schema = make_schema(
            SchemaMode::Flexible,
            vec![
                make_field("name", FieldType::String, true),
                make_field("email", FieldType::String, true),
            ],
        );

        let mut fields = HashMap::new();
        fields.insert("name".to_string(), Value::String("Grace".to_string()));
        fields.insert("email".to_string(), Value::Null); // Null on required field

        let result = validate_document(&schema, &fields);
        assert!(result.is_err());

        match result.unwrap_err() {
            BindError::RequiredFieldNull { collection, field } => {
                assert_eq!(collection, "test");
                assert_eq!(field, "email");
            }
            _ => panic!("Expected RequiredFieldNull error"),
        }
    }

    #[test]
    fn test_int_to_float_coercion() {
        let schema = make_schema(
            SchemaMode::Flexible,
            vec![make_field("price", FieldType::Float, true)],
        );

        let mut fields = HashMap::new();
        fields.insert("price".to_string(), Value::Int(100)); // Int should coerce to Float

        let result = validate_document(&schema, &fields);
        assert!(result.is_ok());
    }

    #[test]
    fn test_datetime_accepts_string_and_int() {
        let schema = make_schema(
            SchemaMode::Flexible,
            vec![
                make_field("created_str", FieldType::Datetime, true),
                make_field("created_int", FieldType::Datetime, true),
            ],
        );

        let mut fields = HashMap::new();
        fields.insert(
            "created_str".to_string(),
            Value::String("2024-01-15T10:30:00Z".to_string()),
        );
        fields.insert("created_int".to_string(), Value::Int(1705315800));

        let result = validate_document(&schema, &fields);
        assert!(result.is_ok());
    }

    #[test]
    fn test_array_validation() {
        let schema = make_schema(
            SchemaMode::Flexible,
            vec![make_field(
                "tags",
                FieldType::Array(Box::new(FieldType::String)),
                true,
            )],
        );

        let mut fields = HashMap::new();
        fields.insert(
            "tags".to_string(),
            Value::Array(vec![
                Value::String("rust".to_string()),
                Value::String("database".to_string()),
            ]),
        );

        let result = validate_document(&schema, &fields);
        assert!(result.is_ok());
    }

    #[test]
    fn test_array_element_type_mismatch() {
        let schema = make_schema(
            SchemaMode::Flexible,
            vec![make_field(
                "scores",
                FieldType::Array(Box::new(FieldType::Int)),
                true,
            )],
        );

        let mut fields = HashMap::new();
        fields.insert(
            "scores".to_string(),
            Value::Array(vec![
                Value::Int(100),
                Value::String("not an int".to_string()), // Wrong type at index 1
                Value::Int(85),
            ]),
        );

        let result = validate_document(&schema, &fields);
        assert!(result.is_err());

        match result.unwrap_err() {
            BindError::ArrayElementError { field, index, .. } => {
                assert_eq!(field, "scores");
                assert_eq!(index, 1);
            }
            _ => panic!("Expected ArrayElementError"),
        }
    }

    #[test]
    fn test_nested_object_validation() {
        let address_fields = vec![
            make_field("city", FieldType::String, true),
            make_field("zip", FieldType::String, false),
        ];

        let schema = make_schema(
            SchemaMode::Flexible,
            vec![
                make_field("name", FieldType::String, true),
                make_field(
                    "address",
                    FieldType::Object {
                        fields: address_fields,
                        mode: SchemaMode::Strict,
                    },
                    true,
                ),
            ],
        );

        let mut address = HashMap::new();
        address.insert("city".to_string(), Value::String("New York".to_string()));
        address.insert("zip".to_string(), Value::String("10001".to_string()));

        let mut fields = HashMap::new();
        fields.insert("name".to_string(), Value::String("Henry".to_string()));
        fields.insert("address".to_string(), Value::Object(address));

        let result = validate_document(&schema, &fields);
        assert!(result.is_ok());
    }

    #[test]
    fn test_nested_object_type_mismatch() {
        let address_fields = vec![make_field("zip", FieldType::String, true)];

        let schema = make_schema(
            SchemaMode::Flexible,
            vec![make_field(
                "address",
                FieldType::Object {
                    fields: address_fields,
                    mode: SchemaMode::Strict,
                },
                true,
            )],
        );

        let mut address = HashMap::new();
        address.insert("zip".to_string(), Value::Int(12345)); // Wrong type

        let mut fields = HashMap::new();
        fields.insert("address".to_string(), Value::Object(address));

        let result = validate_document(&schema, &fields);
        assert!(result.is_err());

        match result.unwrap_err() {
            BindError::NestedError { path, error } => {
                assert_eq!(path, "address");
                match *error {
                    BindError::TypeMismatch { field, .. } => {
                        assert_eq!(field, "zip");
                    }
                    _ => panic!("Expected TypeMismatch in nested error"),
                }
            }
            _ => panic!("Expected NestedError"),
        }
    }

    #[test]
    fn test_default_now_returns_datetime() {
        let schema = make_schema(
            SchemaMode::Flexible,
            vec![make_field_with_default(
                "created_at",
                FieldType::Datetime,
                true,
                DefaultValue::Now,
            )],
        );

        let fields = HashMap::new();

        let result = validate_document(&schema, &fields);
        assert!(result.is_ok());

        let validated = result.unwrap();
        match validated.get("created_at") {
            Some(Value::Datetime(ts)) => {
                // Should be a reasonable recent timestamp (after 2020)
                assert!(*ts > 1577836800);
            }
            _ => panic!("Expected Datetime for DefaultValue::Now"),
        }
    }

    #[test]
    fn test_optional_field_accepts_null() {
        let schema = make_schema(
            SchemaMode::Flexible,
            vec![
                make_field("name", FieldType::String, true),
                make_field("nickname", FieldType::String, false), // Optional
            ],
        );

        let mut fields = HashMap::new();
        fields.insert("name".to_string(), Value::String("Ivy".to_string()));
        fields.insert("nickname".to_string(), Value::Null); // Null OK for optional

        let result = validate_document(&schema, &fields);
        assert!(result.is_ok());
    }

    #[test]
    fn test_reference_any_collection() {
        // Reference without collection constraint accepts any reference
        let schema = make_schema(
            SchemaMode::Flexible,
            vec![make_field("author", FieldType::Reference(None), true)],
        );

        let mut fields = HashMap::new();
        fields.insert(
            "author".to_string(),
            Value::Reference("user:alice".to_string()),
        );

        let result = validate_document(&schema, &fields);
        assert!(result.is_ok());

        // Also accepts other collections
        let mut fields2 = HashMap::new();
        fields2.insert(
            "author".to_string(),
            Value::Reference("organization:acme".to_string()),
        );

        let result2 = validate_document(&schema, &fields2);
        assert!(result2.is_ok());
    }

    #[test]
    fn test_reference_specific_collection() {
        // Reference with collection constraint only accepts matching collection
        let schema = make_schema(
            SchemaMode::Flexible,
            vec![make_field(
                "author",
                FieldType::Reference(Some("user".to_string())),
                true,
            )],
        );

        let mut fields = HashMap::new();
        fields.insert(
            "author".to_string(),
            Value::Reference("user:alice".to_string()),
        );

        let result = validate_document(&schema, &fields);
        assert!(result.is_ok());
    }

    #[test]
    fn test_reference_wrong_collection() {
        // Reference with collection constraint rejects wrong collection
        let schema = make_schema(
            SchemaMode::Flexible,
            vec![make_field(
                "author",
                FieldType::Reference(Some("user".to_string())),
                true,
            )],
        );

        let mut fields = HashMap::new();
        fields.insert(
            "author".to_string(),
            Value::Reference("organization:acme".to_string()),
        );

        let result = validate_document(&schema, &fields);
        assert!(result.is_err());

        match result.unwrap_err() {
            BindError::TypeMismatch {
                field,
                expected,
                got,
            } => {
                assert_eq!(field, "author");
                assert_eq!(expected, "Reference<user>");
                assert!(got.contains("organization:acme"));
            }
            _ => panic!("Expected TypeMismatch error"),
        }
    }

    #[test]
    fn test_reference_type_mismatch() {
        // String value when Reference is expected fails
        let schema = make_schema(
            SchemaMode::Flexible,
            vec![make_field("author", FieldType::Reference(None), true)],
        );

        let mut fields = HashMap::new();
        fields.insert(
            "author".to_string(),
            Value::String("user:alice".to_string()),
        );

        let result = validate_document(&schema, &fields);
        assert!(result.is_err());

        match result.unwrap_err() {
            BindError::TypeMismatch {
                field,
                expected,
                got,
            } => {
                assert_eq!(field, "author");
                assert_eq!(expected, "Reference");
                assert_eq!(got, "String");
            }
            _ => panic!("Expected TypeMismatch error"),
        }
    }

    #[test]
    fn test_optional_reference_accepts_null() {
        let schema = make_schema(
            SchemaMode::Flexible,
            vec![make_field("author", FieldType::Reference(None), false)],
        );

        let mut fields = HashMap::new();
        fields.insert("author".to_string(), Value::Null);

        let result = validate_document(&schema, &fields);
        assert!(result.is_ok());
    }

    // =========================================================================
    // Name validation tests
    // =========================================================================

    #[test]
    fn test_validate_name_empty() {
        let result = validate_name("collection", "");
        assert!(result.is_err());
        match result.unwrap_err() {
            BindError::InvalidName { kind, reason } => {
                assert_eq!(kind, "collection");
                assert!(reason.contains("empty"));
            }
            _ => panic!("Expected InvalidName error"),
        }
    }

    #[test]
    fn test_validate_name_null_bytes() {
        let result = validate_name("collection", "test\0name");
        assert!(result.is_err());
        match result.unwrap_err() {
            BindError::InvalidName { kind, reason } => {
                assert_eq!(kind, "collection");
                assert!(reason.contains("null"));
            }
            _ => panic!("Expected InvalidName error"),
        }
    }

    #[test]
    fn test_validate_name_control_chars() {
        // Bell character
        let result = validate_name("collection", "test\x07name");
        assert!(result.is_err());
        match result.unwrap_err() {
            BindError::InvalidName { kind, reason } => {
                assert_eq!(kind, "collection");
                assert!(reason.contains("control"));
            }
            _ => panic!("Expected InvalidName error"),
        }
    }

    #[test]
    fn test_validate_name_rejects_tab() {
        let result = validate_name("collection", "test\tname");
        assert!(result.is_err());
        match result.unwrap_err() {
            BindError::InvalidName { reason, .. } => {
                assert!(reason.contains("control"));
            }
            _ => panic!("Expected InvalidName error"),
        }
    }

    #[test]
    fn test_validate_name_rejects_newline() {
        let result = validate_name("collection", "test\nname");
        assert!(result.is_err());
        match result.unwrap_err() {
            BindError::InvalidName { reason, .. } => {
                assert!(reason.contains("control"));
            }
            _ => panic!("Expected InvalidName error"),
        }
    }

    #[test]
    fn test_validate_name_rejects_carriage_return() {
        let result = validate_name("collection", "test\rname");
        assert!(result.is_err());
        match result.unwrap_err() {
            BindError::InvalidName { reason, .. } => {
                assert!(reason.contains("control"));
            }
            _ => panic!("Expected InvalidName error"),
        }
    }

    #[test]
    fn test_validate_name_too_long() {
        let long_name = "a".repeat(121);
        let result = validate_name("collection", &long_name);
        assert!(result.is_err());
        match result.unwrap_err() {
            BindError::NameTooLong {
                kind, max_length, ..
            } => {
                assert_eq!(kind, "collection");
                assert_eq!(max_length, 120);
            }
            _ => panic!("Expected NameTooLong error"),
        }
    }

    #[test]
    fn test_validate_name_max_length_ok() {
        let max_name = "a".repeat(120);
        let result = validate_name("collection", &max_name);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_name_unicode_ok() {
        // Unicode names should be allowed
        assert!(validate_name("collection", "пользователи").is_ok());
        assert!(validate_name("collection", "用户").is_ok());
        assert!(validate_name("collection", "emoji_🎉_test").is_ok());
    }

    #[test]
    fn test_validate_name_numbers_ok() {
        // Numeric names should be allowed
        assert!(validate_name("collection", "123").is_ok());
        assert!(validate_name("collection", "1table").is_ok());
    }

    #[test]
    fn test_validate_name_special_chars_ok() {
        // Various special chars (except control chars and null)
        assert!(validate_name("collection", "my-collection").is_ok());
        assert!(validate_name("collection", "my.collection").is_ok());
        assert!(validate_name("collection", "my@collection").is_ok());
    }

    #[test]
    fn test_nested_object_defaults_applied() {
        let address_fields = vec![
            make_field("city", FieldType::String, true),
            make_field_with_default(
                "country",
                FieldType::String,
                true,
                DefaultValue::String("US".to_string()),
            ),
        ];

        let schema = make_schema(
            SchemaMode::Flexible,
            vec![
                make_field("name", FieldType::String, true),
                make_field(
                    "address",
                    FieldType::Object {
                        fields: address_fields,
                        mode: SchemaMode::Flexible,
                    },
                    true,
                ),
            ],
        );

        let mut address = HashMap::new();
        address.insert("city".to_string(), Value::String("New York".to_string()));
        // "country" is missing — should get default "US"

        let mut fields = HashMap::new();
        fields.insert("name".to_string(), Value::String("Alice".to_string()));
        fields.insert("address".to_string(), Value::Object(address));

        let result = validate_document(&schema, &fields);
        assert!(result.is_ok());

        let validated = result.unwrap();
        match validated.get("address") {
            Some(Value::Object(addr)) => {
                assert_eq!(addr.get("country"), Some(&Value::String("US".to_string())));
                assert_eq!(
                    addr.get("city"),
                    Some(&Value::String("New York".to_string()))
                );
            }
            _ => panic!("Expected Object for address"),
        }
    }

    #[test]
    fn test_deeply_nested_defaults_applied() {
        let inner_fields = vec![make_field_with_default(
            "active",
            FieldType::Bool,
            false,
            DefaultValue::Bool(true),
        )];

        let outer_fields = vec![
            make_field("label", FieldType::String, true),
            make_field(
                "meta",
                FieldType::Object {
                    fields: inner_fields,
                    mode: SchemaMode::Flexible,
                },
                false,
            ),
        ];

        let schema = make_schema(
            SchemaMode::Flexible,
            vec![make_field(
                "config",
                FieldType::Object {
                    fields: outer_fields,
                    mode: SchemaMode::Flexible,
                },
                true,
            )],
        );

        let meta = HashMap::new();
        // "active" is missing — should get default true

        let mut config = HashMap::new();
        config.insert("label".to_string(), Value::String("test".to_string()));
        config.insert("meta".to_string(), Value::Object(meta));

        let mut fields = HashMap::new();
        fields.insert("config".to_string(), Value::Object(config));

        let result = validate_document(&schema, &fields);
        assert!(result.is_ok());

        let validated = result.unwrap();
        match validated.get("config") {
            Some(Value::Object(cfg)) => match cfg.get("meta") {
                Some(Value::Object(m)) => {
                    assert_eq!(m.get("active"), Some(&Value::Bool(true)));
                }
                _ => panic!("Expected Object for meta"),
            },
            _ => panic!("Expected Object for config"),
        }
    }

    #[test]
    fn test_validate_name_rejects_path_traversal() {
        // Forward slash
        assert!(validate_name("database", "../../etc").is_err());
        assert!(validate_name("database", "foo/bar").is_err());

        // Backslash
        assert!(validate_name("database", "foo\\bar").is_err());
        assert!(validate_name("database", "..\\..\\etc").is_err());

        // Double dots
        assert!(validate_name("database", "..").is_err());
        assert!(validate_name("database", "a..b").is_err());

        // Single dot
        assert!(validate_name("database", ".").is_err());
    }

    // =========================================================================
    // Index validation tests
    // =========================================================================

    #[test]
    fn test_validate_schema_duplicate_index_names() {
        let schema = CollectionSchema {
            name: "test".to_string(),
            mode: SchemaMode::Flexible,
            fields: vec![make_field("email", FieldType::String, false)],
            indexes: vec![
                IndexDef::new_named("idx_email", vec!["email".to_string()]),
                IndexDef::new_named("idx_email", vec!["email".to_string()]),
            ],
        };

        let result = validate_schema(&schema);
        assert!(result.is_err());
        match result.unwrap_err() {
            BindError::DuplicateIndexName { collection, index } => {
                assert_eq!(collection, "test");
                assert_eq!(index, "idx_email");
            }
            e => panic!("Expected DuplicateIndexName, got: {:?}", e),
        }
    }

    #[test]
    fn test_validate_schema_index_field_not_found() {
        let schema = CollectionSchema {
            name: "test".to_string(),
            mode: SchemaMode::Flexible,
            fields: vec![make_field("name", FieldType::String, true)],
            indexes: vec![IndexDef::new_named(
                "idx_missing",
                vec!["nonexistent".to_string()],
            )],
        };

        let result = validate_schema(&schema);
        assert!(result.is_err());
        match result.unwrap_err() {
            BindError::IndexFieldNotFound {
                collection,
                index,
                field,
            } => {
                assert_eq!(collection, "test");
                assert_eq!(index, "idx_missing");
                assert_eq!(field, "nonexistent");
            }
            e => panic!("Expected IndexFieldNotFound, got: {:?}", e),
        }
    }

    #[test]
    fn test_validate_schema_hnsw_missing_params() {
        let schema = CollectionSchema {
            name: "test".to_string(),
            mode: SchemaMode::Flexible,
            fields: vec![],
            indexes: vec![IndexDef {
                name: "idx_vec".to_string(),
                fields: vec!["embedding".to_string()],
                unique: false,
                index_type: IndexType::Hnsw,
                hnsw_params: None,
                analyzer: None,
            }],
        };

        let result = validate_schema(&schema);
        assert!(result.is_err());
        match result.unwrap_err() {
            BindError::InvalidIndexConfig {
                collection,
                index,
                reason,
            } => {
                assert_eq!(collection, "test");
                assert_eq!(index, "idx_vec");
                assert!(reason.contains("hnsw_params"));
            }
            e => panic!("Expected InvalidIndexConfig, got: {:?}", e),
        }
    }

    #[test]
    fn test_validate_schema_hnsw_zero_dimension() {
        let schema = CollectionSchema {
            name: "test".to_string(),
            mode: SchemaMode::Flexible,
            fields: vec![],
            indexes: vec![IndexDef {
                name: "idx_vec".to_string(),
                fields: vec!["embedding".to_string()],
                unique: false,
                index_type: IndexType::Hnsw,
                hnsw_params: Some(HnswParams::new(0)),
                analyzer: None,
            }],
        };

        let result = validate_schema(&schema);
        assert!(result.is_err());
        match result.unwrap_err() {
            BindError::InvalidIndexConfig {
                collection,
                index,
                reason,
            } => {
                assert_eq!(collection, "test");
                assert_eq!(index, "idx_vec");
                assert!(reason.contains("dimension"));
            }
            e => panic!("Expected InvalidIndexConfig, got: {:?}", e),
        }
    }

    #[test]
    fn test_validate_schema_valid_indexes() {
        let schema = CollectionSchema {
            name: "test".to_string(),
            mode: SchemaMode::Flexible,
            fields: vec![
                make_field("email", FieldType::String, true),
                make_field("embedding", FieldType::Any, false),
            ],
            indexes: vec![
                IndexDef::new_named("idx_email", vec!["email".to_string()]),
                IndexDef {
                    name: "idx_vec".to_string(),
                    fields: vec!["embedding".to_string()],
                    unique: false,
                    index_type: IndexType::Hnsw,
                    hnsw_params: Some(HnswParams::new(128)),
                    analyzer: None,
                },
            ],
        };

        let result = validate_schema(&schema);
        assert!(result.is_ok());
    }
}
