//! Schema validation error types for StellarDB.

use std::fmt;

/// Errors that can occur during schema binding and validation.
#[derive(Debug, Clone, PartialEq)]
pub enum BindError {
    /// The specified collection does not exist.
    UnknownCollection {
        name: String,
        suggestions: Vec<String>,
    },

    /// The specified field does not exist in the collection schema.
    UnknownField {
        collection: String,
        field: String,
        suggestions: Vec<String>,
    },

    /// A field value has an incompatible type.
    TypeMismatch {
        field: String,
        expected: String,
        got: String,
    },

    /// A required field is missing from the document.
    RequiredFieldMissing { collection: String, field: String },

    /// A required field is present but has a null value.
    RequiredFieldNull { collection: String, field: String },

    /// An error occurred in a nested object field.
    NestedError { path: String, error: Box<BindError> },

    /// An error occurred in an array element.
    ArrayElementError {
        field: String,
        index: usize,
        error: Box<BindError>,
    },

    /// Duplicate field name in schema definition.
    DuplicateFieldName { collection: String, field: String },

    /// Default value type does not match field type.
    DefaultValueTypeMismatch {
        collection: String,
        field: String,
        field_type: String,
        default_type: String,
    },

    /// Aggregate function not found in registry
    UnknownFunction { namespace: String, name: String },

    /// Invalid argument for aggregate function (e.g., SUM(*))
    InvalidAggregateArg { function: String, reason: String },

    /// Cannot mix aggregate and non-aggregate items without GROUP
    MixedAggregateAndFields,

    /// SELECT with GROUP contains a field that is not in the GROUP list and is not an aggregate
    InvalidGroupField(String),

    /// Function called with wrong number of arguments
    ArityMismatch {
        function: String,
        expected: usize,
        got: usize,
    },

    /// $parent reference used outside of a correlated subquery in SELECT list
    ParentRefOutsideSubquery,

    /// Constant called as function: math::pi()
    ConstantCalledAsFunction { namespace: String, name: String },

    /// Function used without parentheses: math::sin
    FunctionUsedAsConstant { namespace: String, name: String },

    /// Unknown constant
    UnknownConstant {
        namespace: String,
        name: String,
        suggestions: Vec<String>,
    },

    /// Undefined variable
    UndefinedVariable(String),

    /// Type mismatch in function argument
    ArgumentTypeMismatch {
        function: String,
        param: String,
        expected: String,
        got: String,
    },

    /// FTS operators mixed: cannot use both @@ and @:name@ in same query
    FtsMixedOperators,

    /// FTS operator name is used more than once
    FtsDuplicateOperatorName(String),

    /// score() references an FTS operator name that doesn't exist
    FtsUnknownScoreName(String),

    /// Name exceeds maximum length
    NameTooLong {
        kind: String,
        name: String,
        max_length: usize,
    },

    /// Name is empty or contains invalid characters
    InvalidName { kind: String, reason: String },

    /// Duplicate index name in schema definition.
    DuplicateIndexName { collection: String, index: String },

    /// Index references a field that does not exist in the schema.
    IndexFieldNotFound {
        collection: String,
        index: String,
        field: String,
    },

    /// HNSW index is missing required parameters.
    InvalidIndexConfig {
        collection: String,
        index: String,
        reason: String,
    },
}

impl fmt::Display for BindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindError::UnknownCollection { name, suggestions } => {
                write!(f, "unknown collection: '{}'", name)?;
                if !suggestions.is_empty() {
                    write!(f, ". Did you mean: {}?", suggestions.join(", "))?;
                }
                Ok(())
            }
            BindError::UnknownField {
                collection,
                field,
                suggestions,
            } => {
                write!(
                    f,
                    "unknown field '{}' in collection '{}'",
                    field, collection
                )?;
                if !suggestions.is_empty() {
                    write!(f, ". Did you mean: {}?", suggestions.join(", "))?;
                }
                Ok(())
            }
            BindError::TypeMismatch {
                field,
                expected,
                got,
            } => {
                write!(
                    f,
                    "type mismatch for field '{}': expected {}, got {}",
                    field, expected, got
                )
            }
            BindError::RequiredFieldMissing { collection, field } => {
                write!(
                    f,
                    "required field '{}' is missing in collection '{}'",
                    field, collection
                )
            }
            BindError::RequiredFieldNull { collection, field } => {
                write!(
                    f,
                    "required field '{}' cannot be null in collection '{}'",
                    field, collection
                )
            }
            BindError::NestedError { path, error } => {
                write!(f, "error at '{}': {}", path, error)
            }
            BindError::ArrayElementError {
                field,
                index,
                error,
            } => {
                write!(f, "error in '{}[{}]': {}", field, index, error)
            }
            BindError::DuplicateFieldName { collection, field } => {
                write!(
                    f,
                    "duplicate field name '{}' in collection '{}'",
                    field, collection
                )
            }
            BindError::DefaultValueTypeMismatch {
                collection,
                field,
                field_type,
                default_type,
            } => {
                write!(
                    f,
                    "default value type mismatch for field '{}' in collection '{}': field is {}, default is {}",
                    field, collection, field_type, default_type
                )
            }
            BindError::UnknownFunction { namespace, name } => {
                write!(f, "Unknown function: {}::{}", namespace, name)
            }
            BindError::InvalidAggregateArg { function, reason } => {
                write!(f, "Invalid argument for {}: {}", function, reason)
            }
            BindError::MixedAggregateAndFields => {
                write!(
                    f,
                    "Cannot mix aggregate functions with plain fields without GROUP"
                )
            }
            BindError::InvalidGroupField(field) => {
                write!(
                    f,
                    "Field '{}' must appear in GROUP clause or be used in an aggregate function",
                    field
                )
            }
            BindError::ArityMismatch {
                function,
                expected,
                got,
            } => {
                write!(
                    f,
                    "Function {} expects {} argument(s), got {}",
                    function, expected, got
                )
            }
            BindError::ParentRefOutsideSubquery => {
                write!(
                    f,
                    "$parent reference can only be used inside a subquery in SELECT list"
                )
            }
            BindError::ConstantCalledAsFunction { namespace, name } => {
                write!(
                    f,
                    "{}::{} is a constant, not a function - use without parentheses",
                    namespace, name
                )
            }
            BindError::FunctionUsedAsConstant { namespace, name } => {
                write!(
                    f,
                    "{}::{} is a function - must be called with parentheses",
                    namespace, name
                )
            }
            BindError::UnknownConstant {
                namespace,
                name,
                suggestions,
            } => {
                write!(f, "unknown constant: {}::{}", namespace, name)?;
                if !suggestions.is_empty() {
                    let formatted: Vec<String> = suggestions
                        .iter()
                        .map(|s| format!("{}::{}", namespace, s))
                        .collect();
                    write!(f, ". Did you mean: {}?", formatted.join(", "))?;
                }
                Ok(())
            }
            BindError::UndefinedVariable(name) => {
                write!(f, "undefined variable: ${}", name)
            }
            BindError::ArgumentTypeMismatch {
                function,
                param,
                expected,
                got,
            } => {
                write!(
                    f,
                    "type mismatch for parameter '{}' of function {}: expected {}, got {}",
                    param, function, expected, got
                )
            }
            BindError::FtsMixedOperators => {
                write!(
                    f,
                    "cannot mix simple FTS operator (@@) with named FTS operator (@:name@) in the same query"
                )
            }
            BindError::FtsDuplicateOperatorName(name) => {
                write!(
                    f,
                    "duplicate FTS operator name: '{}' is used more than once",
                    name
                )
            }
            BindError::FtsUnknownScoreName(name) => {
                write!(
                    f,
                    "score('{}') references an FTS operator name that doesn't exist in the query",
                    name
                )
            }
            BindError::NameTooLong {
                kind,
                name,
                max_length,
            } => {
                write!(
                    f,
                    "{} name '{}' exceeds maximum length of {} characters",
                    kind, name, max_length
                )
            }
            BindError::InvalidName { kind, reason } => {
                write!(f, "invalid {} name: {}", kind, reason)
            }
            BindError::DuplicateIndexName { collection, index } => {
                write!(
                    f,
                    "duplicate index name '{}' in collection '{}'",
                    index, collection
                )
            }
            BindError::IndexFieldNotFound {
                collection,
                index,
                field,
            } => {
                write!(
                    f,
                    "index '{}' in collection '{}' references unknown field '{}'",
                    index, collection, field
                )
            }
            BindError::InvalidIndexConfig {
                collection,
                index,
                reason,
            } => {
                write!(
                    f,
                    "invalid configuration for index '{}' in collection '{}': {}",
                    index, collection, reason
                )
            }
        }
    }
}

impl std::error::Error for BindError {}

use crate::error::ErrorCode;

impl ErrorCode for BindError {
    fn code(&self) -> &'static str {
        match self {
            Self::UnknownCollection { .. } => "SDB-QB001",
            Self::UnknownField { .. } => "SDB-QB002",
            Self::TypeMismatch { .. } => "SDB-QB003",
            Self::RequiredFieldMissing { .. } => "SDB-QB004",
            Self::RequiredFieldNull { .. } => "SDB-QB005",
            Self::NestedError { error, .. } => error.code(),
            Self::ArrayElementError { error, .. } => error.code(),
            Self::DuplicateFieldName { .. } => "SDB-QB008",
            Self::DefaultValueTypeMismatch { .. } => "SDB-QB009",
            Self::UnknownFunction { .. } => "SDB-QB010",
            Self::InvalidAggregateArg { .. } => "SDB-QB011",
            Self::MixedAggregateAndFields => "SDB-QB012",
            Self::InvalidGroupField(_) => "SDB-QB013",
            Self::ArityMismatch { .. } => "SDB-QB014",
            Self::ParentRefOutsideSubquery => "SDB-QB015",
            Self::ConstantCalledAsFunction { .. } => "SDB-QB016",
            Self::FunctionUsedAsConstant { .. } => "SDB-QB017",
            Self::UnknownConstant { .. } => "SDB-QB018",
            Self::UndefinedVariable(_) => "SDB-QB019",
            Self::ArgumentTypeMismatch { .. } => "SDB-QB020",
            Self::FtsMixedOperators => "SDB-QB021",
            Self::FtsDuplicateOperatorName(_) => "SDB-QB022",
            Self::FtsUnknownScoreName(_) => "SDB-QB023",
            Self::NameTooLong { .. } => "SDB-QB024",
            Self::InvalidName { .. } => "SDB-QB025",
            Self::DuplicateIndexName { .. } => "SDB-QB026",
            Self::IndexFieldNotFound { .. } => "SDB-QB027",
            Self::InvalidIndexConfig { .. } => "SDB-QB028",
        }
    }

    fn hint(&self) -> Option<String> {
        match self {
            Self::UnknownCollection { suggestions, .. } if !suggestions.is_empty() => {
                Some(format!("Did you mean: {}?", suggestions.join(", ")))
            }
            Self::UnknownField { suggestions, .. } if !suggestions.is_empty() => {
                Some(format!("Did you mean: {}?", suggestions.join(", ")))
            }
            Self::UnknownConstant {
                namespace,
                suggestions,
                ..
            } if !suggestions.is_empty() => {
                let formatted: Vec<String> = suggestions
                    .iter()
                    .map(|s| format!("{}::{}", namespace, s))
                    .collect();
                Some(format!("Did you mean: {}?", formatted.join(", ")))
            }
            Self::ConstantCalledAsFunction { namespace, name } => {
                Some(format!("Use {}::{} without parentheses", namespace, name))
            }
            Self::FunctionUsedAsConstant { namespace, name } => {
                Some(format!("Call as {}::{}()", namespace, name))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unknown_collection_display() {
        let err = BindError::UnknownCollection {
            name: "products".to_string(),
            suggestions: vec![],
        };
        assert_eq!(err.to_string(), "unknown collection: 'products'");
    }

    #[test]
    fn test_unknown_collection_with_suggestions() {
        let err = BindError::UnknownCollection {
            name: "prodcts".to_string(),
            suggestions: vec!["products".to_string()],
        };
        assert_eq!(
            err.to_string(),
            "unknown collection: 'prodcts'. Did you mean: products?"
        );
    }

    #[test]
    fn test_unknown_field_display() {
        let err = BindError::UnknownField {
            collection: "users".to_string(),
            field: "nonexistent".to_string(),
            suggestions: vec![],
        };
        assert_eq!(
            err.to_string(),
            "unknown field 'nonexistent' in collection 'users'"
        );
    }

    #[test]
    fn test_unknown_field_with_suggestions() {
        let err = BindError::UnknownField {
            collection: "users".to_string(),
            field: "nmae".to_string(),
            suggestions: vec!["name".to_string()],
        };
        assert_eq!(
            err.to_string(),
            "unknown field 'nmae' in collection 'users'. Did you mean: name?"
        );
    }

    #[test]
    fn test_type_mismatch_display() {
        let err = BindError::TypeMismatch {
            field: "age".to_string(),
            expected: "Int".to_string(),
            got: "String".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "type mismatch for field 'age': expected Int, got String"
        );
    }

    #[test]
    fn test_required_field_missing_display() {
        let err = BindError::RequiredFieldMissing {
            collection: "posts".to_string(),
            field: "title".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "required field 'title' is missing in collection 'posts'"
        );
    }

    #[test]
    fn test_required_field_null_display() {
        let err = BindError::RequiredFieldNull {
            collection: "comments".to_string(),
            field: "content".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "required field 'content' cannot be null in collection 'comments'"
        );
    }

    #[test]
    fn test_nested_error_display() {
        let inner = BindError::TypeMismatch {
            field: "zip".to_string(),
            expected: "String".to_string(),
            got: "Int".to_string(),
        };
        let err = BindError::NestedError {
            path: "address".to_string(),
            error: Box::new(inner),
        };
        assert_eq!(
            err.to_string(),
            "error at 'address': type mismatch for field 'zip': expected String, got Int"
        );
    }

    #[test]
    fn test_array_element_error_display() {
        let inner = BindError::TypeMismatch {
            field: "element".to_string(),
            expected: "String".to_string(),
            got: "Int".to_string(),
        };
        let err = BindError::ArrayElementError {
            field: "tags".to_string(),
            index: 2,
            error: Box::new(inner),
        };
        assert_eq!(
            err.to_string(),
            "error in 'tags[2]': type mismatch for field 'element': expected String, got Int"
        );
    }

    #[test]
    fn test_parent_ref_outside_subquery_display() {
        let err = BindError::ParentRefOutsideSubquery;
        assert_eq!(
            err.to_string(),
            "$parent reference can only be used inside a subquery in SELECT list"
        );
    }

    #[test]
    fn test_bind_error_equality() {
        let err1 = BindError::UnknownCollection {
            name: "test".to_string(),
            suggestions: vec![],
        };
        let err2 = BindError::UnknownCollection {
            name: "test".to_string(),
            suggestions: vec![],
        };
        let err3 = BindError::UnknownCollection {
            name: "other".to_string(),
            suggestions: vec![],
        };

        assert_eq!(err1, err2);
        assert_ne!(err1, err3);
    }

    #[test]
    fn test_bind_error_codes() {
        use crate::error::ErrorCode;

        assert_eq!(
            BindError::UnknownCollection {
                name: "x".into(),
                suggestions: vec![]
            }
            .code(),
            "SDB-QB001"
        );
        assert_eq!(
            BindError::TypeMismatch {
                field: "f".into(),
                expected: "Int".into(),
                got: "String".into()
            }
            .code(),
            "SDB-QB003"
        );
        assert_eq!(BindError::MixedAggregateAndFields.code(), "SDB-QB012");
        assert_eq!(BindError::ParentRefOutsideSubquery.code(), "SDB-QB015");
    }

    #[test]
    fn test_bind_error_hint() {
        use crate::error::ErrorCode;

        let err = BindError::UnknownCollection {
            name: "usr".into(),
            suggestions: vec!["users".into()],
        };
        assert_eq!(err.hint(), Some("Did you mean: users?".to_string()));

        let err = BindError::UnknownCollection {
            name: "x".into(),
            suggestions: vec![],
        };
        assert_eq!(err.hint(), None);
    }

    #[test]
    fn test_nested_error_delegates_code() {
        use crate::error::ErrorCode;

        let inner = BindError::TypeMismatch {
            field: "f".into(),
            expected: "Int".into(),
            got: "String".into(),
        };
        let outer = BindError::NestedError {
            path: "obj".into(),
            error: Box::new(inner),
        };
        assert_eq!(outer.code(), "SDB-QB003"); // delegates to inner
    }
}
