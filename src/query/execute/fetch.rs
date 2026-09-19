//! FETCH clause execution - resolves document references

use std::collections::HashMap;

use crate::document::{Document, Value};
use crate::query::ast::FetchItem;
use crate::query::error::ExecuteError;
use crate::query::execute::context::ExecutionContext;
use crate::query::execute::operators::operator::Row;

/// Execute FETCH post-processing on query results
///
/// For nested paths like `author.company`, this works in multiple passes:
/// 1. First expand the top-level reference (author)
/// 2. Then expand nested references within the expanded document (author.company)
pub fn execute_fetch<Ctx: ExecutionContext>(
    mut rows: Vec<Row>,
    fetch_items: &[FetchItem],
    ctx: &Ctx,
) -> Result<Vec<Row>, ExecuteError> {
    if fetch_items.is_empty() {
        return Ok(rows);
    }

    // Process each fetch item, handling nested paths by expanding level by level
    for item in fetch_items {
        // For a path like ["author", "company"], we need to:
        // 1. First expand "author" references
        // 2. Then expand "author.company" references within the expanded author docs

        for depth in 0..item.path.len() {
            let partial_path = &item.path[..=depth];

            // Collect refs at this depth
            let mut ref_ids: Vec<String> = Vec::new();
            for row in &rows {
                collect_refs_at_depth(&row.doc, partial_path, &mut ref_ids);
            }

            // Batch load documents for this depth
            let docs = batch_load_documents(&ref_ids, ctx)?;

            // Replace refs at this depth (only at the final element, not intermediate)
            for row in &mut rows {
                // At the final depth, use the alias if provided
                let alias = if depth == item.path.len() - 1 {
                    &item.alias
                } else {
                    &None
                };
                replace_refs_at_depth(&mut row.doc, partial_path, alias, &docs);
            }
        }
    }

    Ok(rows)
}

/// Collect references at a specific path depth
fn collect_refs_at_depth(doc: &Document, path: &[String], refs: &mut Vec<String>) {
    if path.is_empty() {
        return;
    }

    let field = &path[0];
    if let Some(value) = doc.fields.get(field) {
        collect_refs_at_depth_value(value, &path[1..], refs);
    }
}

fn collect_refs_at_depth_value(value: &Value, remaining_path: &[String], refs: &mut Vec<String>) {
    if remaining_path.is_empty() {
        // We've reached the target depth - collect references here
        match value {
            Value::Reference(id) => {
                if !refs.contains(id) {
                    refs.push(id.clone());
                }
            }
            // Also treat strings in "collection:key" format as references
            Value::String(s) if s.contains(':') => {
                if !refs.contains(s) {
                    refs.push(s.clone());
                }
            }
            Value::Array(arr) => {
                for item in arr {
                    collect_refs_at_depth_value(item, remaining_path, refs);
                }
            }
            _ => {}
        }
    } else {
        // Need to go deeper
        match value {
            Value::Object(obj) => {
                if let Some(next_value) = obj.get(&remaining_path[0]) {
                    collect_refs_at_depth_value(next_value, &remaining_path[1..], refs);
                }
            }
            Value::Array(arr) => {
                for item in arr {
                    collect_refs_at_depth_value(item, remaining_path, refs);
                }
            }
            _ => {}
        }
    }
}

/// Replace references at a specific path depth
fn replace_refs_at_depth(
    doc: &mut Document,
    path: &[String],
    alias: &Option<String>,
    docs: &HashMap<String, Document>,
) {
    if path.is_empty() {
        return;
    }

    let field = &path[0];

    if path.len() == 1 {
        // Final level - do the replacement
        if let Some(value) = doc.fields.get(field).cloned() {
            let replaced = replace_ref_value(value, docs);

            if let Some(alias_name) = alias {
                doc.fields.insert(alias_name.clone(), replaced);
            } else {
                doc.fields.insert(field.clone(), replaced);
            }
        }
    } else {
        // Need to go deeper
        if let Some(value) = doc.fields.get_mut(field) {
            replace_refs_at_depth_value(value, &path[1..], alias, docs);
        }
    }
}

fn replace_refs_at_depth_value(
    value: &mut Value,
    remaining_path: &[String],
    alias: &Option<String>,
    docs: &HashMap<String, Document>,
) {
    if remaining_path.is_empty() {
        return;
    }

    let field = &remaining_path[0];

    match value {
        Value::Object(obj) => {
            if remaining_path.len() == 1 {
                // Final level
                if let Some(inner) = obj.get(field).cloned() {
                    let replaced = replace_ref_value(inner, docs);
                    if let Some(alias_name) = alias {
                        obj.insert(alias_name.clone(), replaced);
                    } else {
                        obj.insert(field.clone(), replaced);
                    }
                }
            } else {
                // Go deeper
                if let Some(inner) = obj.get_mut(field) {
                    replace_refs_at_depth_value(inner, &remaining_path[1..], alias, docs);
                }
            }
        }
        Value::Array(arr) => {
            for item in arr {
                replace_refs_at_depth_value(item, remaining_path, alias, docs);
            }
        }
        _ => {}
    }
}

/// Replace a single reference value with its document
fn replace_ref_value(value: Value, docs: &HashMap<String, Document>) -> Value {
    match value {
        Value::Reference(id) => {
            if let Some(doc) = docs.get(&id) {
                document_to_value(doc)
            } else {
                Value::Null
            }
        }
        // Also treat strings in "collection:key" format as references
        Value::String(ref s) if s.contains(':') => {
            if let Some(doc) = docs.get(s) {
                document_to_value(doc)
            } else {
                value // Keep original string if not found
            }
        }
        Value::Array(arr) => Value::Array(
            arr.into_iter()
                .map(|v| replace_ref_value(v, docs))
                .collect(),
        ),
        other => other,
    }
}

fn batch_load_documents<Ctx: ExecutionContext>(
    ref_ids: &[String],
    ctx: &Ctx,
) -> Result<HashMap<String, Document>, ExecuteError> {
    let mut docs = HashMap::new();

    for id in ref_ids {
        if let Some((collection, key)) = id.split_once(':')
            && let Some(doc) = ctx
                .get_document(collection, key)
                .map_err(ExecuteError::Storage)?
        {
            docs.insert(id.clone(), doc);
        }
    }

    Ok(docs)
}

fn document_to_value(doc: &Document) -> Value {
    let mut obj = doc.fields.clone();
    obj.insert("id".to_string(), Value::String(doc.id.clone()));
    Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_refs_simple() {
        let mut refs = Vec::new();
        let doc = Document {
            id: "post:1".to_string(),
            fields: [(
                "author".to_string(),
                Value::Reference("user:alice".to_string()),
            )]
            .into_iter()
            .collect(),
        };

        collect_refs_at_depth(&doc, &["author".to_string()], &mut refs);
        assert_eq!(refs, vec!["user:alice"]);
    }

    #[test]
    fn test_collect_refs_array() {
        let mut refs = Vec::new();
        let doc = Document {
            id: "post:1".to_string(),
            fields: [(
                "tags".to_string(),
                Value::Array(vec![
                    Value::Reference("tag:rust".to_string()),
                    Value::Reference("tag:db".to_string()),
                ]),
            )]
            .into_iter()
            .collect(),
        };

        collect_refs_at_depth(&doc, &["tags".to_string()], &mut refs);
        assert_eq!(refs, vec!["tag:rust", "tag:db"]);
    }

    #[test]
    fn test_collect_refs_dedup() {
        let mut refs = Vec::new();
        let doc = Document {
            id: "post:1".to_string(),
            fields: [(
                "mentions".to_string(),
                Value::Array(vec![
                    Value::Reference("user:alice".to_string()),
                    Value::Reference("user:alice".to_string()),
                ]),
            )]
            .into_iter()
            .collect(),
        };

        collect_refs_at_depth(&doc, &["mentions".to_string()], &mut refs);
        assert_eq!(refs, vec!["user:alice"]);
    }

    #[test]
    fn test_replace_ref_simple() {
        let docs: HashMap<String, Document> = [(
            "user:alice".to_string(),
            Document {
                id: "user:alice".to_string(),
                fields: [("name".to_string(), Value::String("Alice".to_string()))]
                    .into_iter()
                    .collect(),
            },
        )]
        .into_iter()
        .collect();

        let mut doc = Document {
            id: "post:1".to_string(),
            fields: [(
                "author".to_string(),
                Value::Reference("user:alice".to_string()),
            )]
            .into_iter()
            .collect(),
        };

        replace_refs_at_depth(&mut doc, &["author".to_string()], &None, &docs);

        match doc.fields.get("author") {
            Some(Value::Object(obj)) => {
                assert_eq!(
                    obj.get("id"),
                    Some(&Value::String("user:alice".to_string()))
                );
                assert_eq!(obj.get("name"), Some(&Value::String("Alice".to_string())));
            }
            other => panic!("Expected Object, got {:?}", other),
        }
    }

    #[test]
    fn test_replace_ref_with_alias() {
        let docs: HashMap<String, Document> = [(
            "user:alice".to_string(),
            Document {
                id: "user:alice".to_string(),
                fields: [("name".to_string(), Value::String("Alice".to_string()))]
                    .into_iter()
                    .collect(),
            },
        )]
        .into_iter()
        .collect();

        let mut doc = Document {
            id: "post:1".to_string(),
            fields: [(
                "author".to_string(),
                Value::Reference("user:alice".to_string()),
            )]
            .into_iter()
            .collect(),
        };

        replace_refs_at_depth(
            &mut doc,
            &["author".to_string()],
            &Some("author_doc".to_string()),
            &docs,
        );

        // Original reference preserved
        assert!(matches!(
            doc.fields.get("author"),
            Some(Value::Reference(_))
        ));

        // Alias contains resolved document
        match doc.fields.get("author_doc") {
            Some(Value::Object(obj)) => {
                assert_eq!(obj.get("name"), Some(&Value::String("Alice".to_string())));
            }
            other => panic!("Expected Object, got {:?}", other),
        }
    }

    #[test]
    fn test_replace_ref_not_found() {
        let docs: HashMap<String, Document> = HashMap::new();

        let mut doc = Document {
            id: "post:1".to_string(),
            fields: [(
                "author".to_string(),
                Value::Reference("user:missing".to_string()),
            )]
            .into_iter()
            .collect(),
        };

        replace_refs_at_depth(&mut doc, &["author".to_string()], &None, &docs);

        assert_eq!(doc.fields.get("author"), Some(&Value::Null));
    }

    #[test]
    fn test_replace_refs_in_array() {
        let docs: HashMap<String, Document> = [
            (
                "tag:rust".to_string(),
                Document {
                    id: "tag:rust".to_string(),
                    fields: [("label".to_string(), Value::String("Rust".to_string()))]
                        .into_iter()
                        .collect(),
                },
            ),
            (
                "tag:db".to_string(),
                Document {
                    id: "tag:db".to_string(),
                    fields: [("label".to_string(), Value::String("Database".to_string()))]
                        .into_iter()
                        .collect(),
                },
            ),
        ]
        .into_iter()
        .collect();

        let mut doc = Document {
            id: "post:1".to_string(),
            fields: [(
                "tags".to_string(),
                Value::Array(vec![
                    Value::Reference("tag:rust".to_string()),
                    Value::Reference("tag:db".to_string()),
                ]),
            )]
            .into_iter()
            .collect(),
        };

        replace_refs_at_depth(&mut doc, &["tags".to_string()], &None, &docs);

        match doc.fields.get("tags") {
            Some(Value::Array(arr)) => {
                assert_eq!(arr.len(), 2);
                match &arr[0] {
                    Value::Object(obj) => {
                        assert_eq!(obj.get("label"), Some(&Value::String("Rust".to_string())));
                    }
                    other => panic!("Expected Object, got {:?}", other),
                }
            }
            other => panic!("Expected Array, got {:?}", other),
        }
    }

    #[test]
    fn test_document_to_value() {
        let doc = Document {
            id: "user:alice".to_string(),
            fields: [("name".to_string(), Value::String("Alice".to_string()))]
                .into_iter()
                .collect(),
        };

        let value = document_to_value(&doc);

        match value {
            Value::Object(obj) => {
                assert_eq!(
                    obj.get("id"),
                    Some(&Value::String("user:alice".to_string()))
                );
                assert_eq!(obj.get("name"), Some(&Value::String("Alice".to_string())));
            }
            other => panic!("Expected Object, got {:?}", other),
        }
    }
}
