//! DDL (Data Definition Language) operations
//!
//! These are handled separately from DML because they:
//! - Don't compose (no iterator model)
//! - Aren't allowed in transactions
//! - Operate on metadata, not documents

use super::{Row, status_row};
use crate::Database;
use crate::document::Value;
use crate::query::error::ExecuteError;
use crate::schema::{
    CollectionSchema, DefaultValue, FieldDef, FieldType, HnswParams, IndexDef, IndexType,
    SchemaMode, validate_name, validate_schema,
};

/// Execute DEFINE COLLECTION
pub fn execute_define_collection(
    storage: &Database,
    name: String,
    mode: SchemaMode,
    fields: Vec<FieldDef>,
    indexes: Vec<IndexDef>,
) -> Result<Vec<Row>, ExecuteError> {
    let schema = CollectionSchema {
        name: name.clone(),
        mode,
        fields,
        indexes,
    };

    // Validate schema for duplicate fields and default value type mismatches
    validate_schema(&schema)?;

    storage.save_schema(&schema)?;

    // Acquire DDL lock for index modifications
    let _ddl_lock = storage.indexes().acquire_ddl_lock();

    // Sync indexes: remove extra, add missing
    let existing_indexes = storage.get_indexes(&name);

    // Remove indexes not in new schema
    for existing in &existing_indexes {
        if !schema.indexes.iter().any(|i| i.fields == existing.fields) {
            storage.drop_index(&name, existing)?;
            storage.index_registry().remove(&name, &existing.fields);
        }
    }

    // Add indexes from schema that don't exist
    for index in &schema.indexes {
        if !existing_indexes.iter().any(|i| i.fields == index.fields) {
            storage.build_index(&name, index)?;
            storage.index_registry().register(&name, index.clone());
        }
    }

    Ok(status_row(vec![
        ("status", Value::String("created".into())),
        ("collection", Value::String(name)),
    ]))
}

/// Execute DROP COLLECTION
pub fn execute_drop_collection(
    storage: &Database,
    name: &str,
    cascade: bool,
) -> Result<Vec<Row>, ExecuteError> {
    // Check if collection has documents
    let (docs, _) = storage.list_documents(name, None, 1)?;

    if !docs.is_empty() && !cascade {
        return Err(ExecuteError::CollectionNotEmpty(name.to_string()));
    }

    // delete_schema will:
    // 1. Clear all data from keyspaces in O(1) (no tombstones)
    // 2. Remove from registries
    // 3. Remove keyspaces from memory
    let existed = storage.delete_schema(name)?;

    Ok(status_row(vec![
        (
            "status",
            Value::String(if existed { "dropped" } else { "not_found" }.into()),
        ),
        ("collection", Value::String(name.to_string())),
    ]))
}

/// Execute DESCRIBE COLLECTION
pub fn execute_describe_collection(
    storage: &Database,
    name: &str,
) -> Result<Vec<Row>, ExecuteError> {
    match storage.get_schema(name) {
        Some(schema) => {
            let fields_value = Value::Array(schema.fields.iter().map(format_field_def).collect());

            let indexes_value = Value::Array(
                schema
                    .indexes
                    .iter()
                    .map(|i| {
                        let mut idx_map = std::collections::HashMap::new();
                        idx_map.insert(
                            "fields".to_string(),
                            Value::Array(
                                i.fields.iter().map(|s| Value::String(s.clone())).collect(),
                            ),
                        );
                        idx_map.insert("unique".to_string(), Value::Bool(i.unique));
                        idx_map.insert(
                            "index_type".to_string(),
                            Value::String(format!("{:?}", i.index_type)),
                        );
                        Value::Object(idx_map)
                    })
                    .collect(),
            );

            Ok(status_row(vec![
                ("name", Value::String(schema.name.clone())),
                (
                    "mode",
                    Value::String(
                        match schema.mode {
                            SchemaMode::Strict => "strict",
                            SchemaMode::Flexible => "flexible",
                        }
                        .into(),
                    ),
                ),
                ("fields", fields_value),
                ("indexes", indexes_value),
            ]))
        }
        None => {
            // Check if collection exists (has data but no schema)
            if storage.collection_exists(name) {
                Ok(status_row(vec![
                    ("name", Value::String(name.to_string())),
                    ("mode", Value::String("flexible".into())),
                    ("fields", Value::Array(vec![])),
                    ("indexes", Value::Array(vec![])),
                    (
                        "message",
                        Value::String("Collection exists but has no defined schema".into()),
                    ),
                ]))
            } else {
                Err(ExecuteError::CollectionNotFound(name.to_string()))
            }
        }
    }
}

/// Execute DESCRIBE COLLECTIONS
pub fn execute_describe_collections(storage: &Database) -> Result<Vec<Row>, ExecuteError> {
    let collections = storage.list_collections();
    let count = collections.len();

    let collections_value = Value::Array(
        collections
            .iter()
            .map(|name| {
                let mode = storage
                    .get_schema(name)
                    .map(|s| match s.mode {
                        SchemaMode::Strict => "strict",
                        SchemaMode::Flexible => "flexible",
                    })
                    .unwrap_or("flexible");

                let mut coll_map = std::collections::HashMap::new();
                coll_map.insert("name".to_string(), Value::String(name.clone()));
                coll_map.insert("mode".to_string(), Value::String(mode.into()));
                Value::Object(coll_map)
            })
            .collect(),
    );

    Ok(status_row(vec![
        ("collections", collections_value),
        ("count", Value::Int(count as i64)),
    ]))
}

/// Execute CREATE INDEX
pub fn execute_create_index(
    storage: &Database,
    collection: &str,
    fields: Vec<String>,
    unique: bool,
    index_type: IndexType,
    hnsw_params: Option<HnswParams>,
    analyzer: Option<String>,
) -> Result<Vec<Row>, ExecuteError> {
    // Keep document mutations out of the build window so every document is
    // represented exactly once before the index becomes planner-visible.
    let _ddl_lock = storage.indexes().acquire_ddl_lock();
    let index_def = prepare_create_index(
        storage,
        collection,
        fields,
        unique,
        index_type,
        hnsw_params,
        analyzer,
    )?;
    let build_id = nanoid::nanoid!(20);
    storage.initialize_index_build_artifact(collection, &index_def, &build_id)?;

    let build_result =
        storage.build_index_artifact_with_progress(collection, &index_def, &build_id, |_| true);
    if let Err(error) = build_result {
        let _ = storage.cleanup_index_build_artifact(collection, &index_def, &build_id);
        return Err(error.into());
    }

    // Flush the isolated artifact before it can acquire a final physical name.
    storage.sync()?;
    storage.promote_index_build_artifact(collection, &index_def, &build_id)?;
    if let Err(error) = storage.publish_ready_index(collection, index_def.clone()) {
        let _ = storage.cleanup_index_build_artifact(collection, &index_def, &build_id);
        return Err(error.into());
    }
    let _ = storage.cleanup_index_build_artifact(collection, &index_def, &build_id);
    created_index_row(collection, &index_def)
}

/// Validate CREATE INDEX and produce the exact durable definition that a
/// background worker can persist and replay after restart.
pub fn prepare_create_index(
    storage: &Database,
    collection: &str,
    fields: Vec<String>,
    unique: bool,
    index_type: IndexType,
    hnsw_params: Option<HnswParams>,
    analyzer: Option<String>,
) -> Result<IndexDef, ExecuteError> {
    // Validate field names
    for field in &fields {
        validate_name("field", field)?;
    }

    // Check collection exists
    if !storage.collection_exists(collection) {
        return Err(ExecuteError::CollectionNotFound(collection.to_string()));
    }

    // Check index doesn't already exist
    if storage.has_index(collection, &fields) {
        let fields_str = fields.join(", ");
        return Err(ExecuteError::IndexAlreadyExists(
            collection.to_string(),
            fields_str,
        ));
    }

    // Validate analyzer
    let resolved_analyzer = if index_type == IndexType::FullText {
        // For FTS indexes, default to "standard" if not specified
        let analyzer_name = analyzer.as_deref().unwrap_or("standard");
        // Check both builtin and custom analyzers
        let analyzer_exists = storage
            .indexes()
            .fts_backend()
            .analyzer_registry()
            .exists(analyzer_name);
        if !analyzer_exists {
            return Err(ExecuteError::AnalyzerNotFound(analyzer_name.to_string()));
        }
        Some(analyzer_name.to_string())
    } else {
        // ANALYZER is only valid for FTS indexes
        if analyzer.is_some() {
            return Err(ExecuteError::Internal(
                "ANALYZER can only be specified for FULLTEXT indexes".to_string(),
            ));
        }
        None
    };

    // Create index definition with auto-generated name
    let index_name = crate::schema::generate_index_name(collection, index_type, &fields);

    // Validate generated index name
    validate_name("index", &index_name)?;

    let index_def = IndexDef {
        name: index_name,
        fields,
        unique,
        index_type,
        hnsw_params,
        analyzer: resolved_analyzer,
    };

    Ok(index_def)
}

/// Initialize external index backends before documents are scanned.
pub fn initialize_index_build(
    storage: &Database,
    collection: &str,
    index_def: &IndexDef,
) -> Result<(), ExecuteError> {
    // For HNSW indexes, initialize the instance before building
    if index_def.index_type == IndexType::Hnsw
        && let Some(params) = &index_def.hnsw_params
    {
        storage.init_hnsw_index(collection, &index_def.fields[0], params)?;
    }

    // For FTS indexes, initialize with the analyzer before building
    if index_def.index_type == IndexType::FullText {
        storage.init_fts_index(
            collection,
            &index_def.name,
            &index_def.fields,
            index_def.analyzer.as_deref(),
        )?;
    }

    Ok(())
}

/// Publish a fully built index atomically from the query planner's point of
/// view: registry visibility comes only after the build has completed.
pub fn publish_index(
    storage: &Database,
    collection: &str,
    index_def: IndexDef,
) -> Result<Vec<Row>, ExecuteError> {
    storage.publish_ready_index(collection, index_def.clone())?;
    created_index_row(collection, &index_def)
}

fn created_index_row(collection: &str, index_def: &IndexDef) -> Result<Vec<Row>, ExecuteError> {
    Ok(status_row(vec![
        ("status", Value::String("created".into())),
        ("index", Value::String(index_def.name.clone())),
        ("collection", Value::String(collection.to_string())),
        (
            "fields",
            Value::Array(
                index_def
                    .fields
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        ),
    ]))
}

/// Execute DROP INDEX by name
pub fn execute_drop_index(
    storage: &Database,
    collection: &str,
    index_name: &str,
) -> Result<Vec<Row>, ExecuteError> {
    // Acquire DDL lock to prevent queries from running during drop.
    // This ensures queries that planned to use the index complete before we remove it.
    let _ddl_lock = storage.indexes().acquire_ddl_lock();

    if !storage.collection_exists(collection) {
        return Err(ExecuteError::CollectionNotFound(collection.to_string()));
    }

    // Find index by name to get its fields
    let index = storage
        .get_indexes(collection)
        .into_iter()
        .find(|idx| idx.name == index_name);

    let index = match index {
        Some(idx) => idx,
        None => {
            return Err(ExecuteError::IndexNotFound(
                collection.to_string(),
                index_name.to_string(),
            ));
        }
    };

    let fields = index.fields.clone();

    // IMPORTANT: Order matters for concurrent query safety!
    //
    // 1. Remove from registry FIRST - this ensures new queries won't try to use
    //    the index. They'll fall back to full collection scan instead.
    storage.index_registry().remove(collection, &fields);

    // 2. Remove index entries - queries that started before registry removal
    //    might still be using the index, but they'll find entries that exist.
    //    New queries won't use the index at all.
    storage.drop_index(collection, &index)?;

    // 3. Remove from schema (persistence)
    storage.remove_index_from_schema(collection, &fields)?;

    Ok(status_row(vec![
        ("status", Value::String("dropped".into())),
        ("index", Value::String(index_name.to_string())),
        ("collection", Value::String(collection.to_string())),
    ]))
}

/// Execute REINDEX by index name
pub fn execute_reindex(
    storage: &Database,
    collection: &str,
    index_name: &str,
) -> Result<Vec<Row>, ExecuteError> {
    use std::time::Instant;

    let start = Instant::now();

    // NOTE: DDL lock is NOT acquired here. It's acquired inside finish_reindex
    // only during the atomic swap, allowing queries to run while the shadow
    // index is being built.

    // Verify collection exists
    if !storage.collection_exists(collection) {
        return Err(ExecuteError::CollectionNotFound(collection.to_string()));
    }

    // Find the index by name
    let index = storage
        .get_indexes(collection)
        .into_iter()
        .find(|idx| idx.name == index_name);

    let index = match index {
        Some(idx) => idx,
        None => {
            return Err(ExecuteError::IndexNotFound(
                collection.to_string(),
                index_name.to_string(),
            ));
        }
    };

    // Only FTS and HNSW indexes support REINDEX
    if !matches!(index.index_type, IndexType::FullText | IndexType::Hnsw) {
        return Err(ExecuteError::Internal(format!(
            "REINDEX only supports FULLTEXT and HNSW indexes, not {:?}",
            index.index_type
        )));
    }

    let docs_indexed = reindex_single(storage, collection, &index)?;

    let elapsed_ms = start.elapsed().as_millis() as u64;

    Ok(status_row(vec![
        ("status", Value::String("ok".into())),
        ("index", Value::String(index_name.to_string())),
        ("documents_indexed", Value::Int(docs_indexed as i64)),
        ("elapsed_ms", Value::Int(elapsed_ms as i64)),
    ]))
}

fn reindex_single(
    storage: &Database,
    collection: &str,
    index: &IndexDef,
) -> Result<usize, ExecuteError> {
    let mut sorted_fields = index.fields.clone();
    sorted_fields.sort();
    let index_name = match index.index_type {
        IndexType::FullText => format!("{}_fts", sorted_fields.join("_")),
        IndexType::Hnsw => sorted_fields[0].clone(),
        _ => return Ok(0),
    };

    // Start reindex (creates shadow)
    match index.index_type {
        IndexType::FullText => {
            storage.indexes().fts_backend().start_reindex(
                collection,
                &index_name,
                &index.fields,
                index.analyzer.as_deref(),
            )?;
        }
        IndexType::Hnsw => {
            let params = index
                .hnsw_params
                .as_ref()
                .ok_or_else(|| ExecuteError::Internal("HNSW params missing".into()))?;
            storage
                .indexes()
                .hnsw_backend()
                .start_reindex(collection, &index_name, params)?;
        }
        _ => return Ok(0),
    }

    // Build shadow from all documents
    let mut count = 0;
    let result: Result<usize, ExecuteError> = (|| {
        for doc_result in storage.documents().iter(collection)? {
            let doc = doc_result?;

            // Re-check if document still exists (handles concurrent deletes)
            if storage.documents().get(collection, doc.key())?.is_none() {
                continue; // Document was deleted concurrently, skip it
            }

            match index.index_type {
                IndexType::FullText => {
                    let indexed = storage.indexes().fts_backend().index_to_shadow(
                        collection,
                        &index_name,
                        &doc,
                        &index.fields,
                    )?;
                    if !indexed {
                        return Err(ExecuteError::Internal(format!(
                            "Failed to index document {} to shadow - state changed unexpectedly",
                            doc.id
                        )));
                    }
                }
                IndexType::Hnsw => {
                    // Extract vector from document
                    if let Some(crate::document::Value::Array(arr)) =
                        doc.fields.get(&index.fields[0])
                    {
                        let vec: Vec<f32> = arr
                            .iter()
                            .filter_map(|v| match v {
                                crate::document::Value::Float(f) => Some(*f as f32),
                                crate::document::Value::Int(i) => Some(*i as f32),
                                _ => None,
                            })
                            .collect();
                        if !vec.is_empty() {
                            storage.indexes().hnsw_backend().add_to_shadow(
                                collection,
                                &index_name,
                                &doc.id,
                                &vec,
                            )?;
                        }
                    }
                }
                _ => {}
            }
            count += 1;
        }
        Ok(count)
    })();

    // Finish or cancel based on result
    match result {
        Ok(count) => {
            match index.index_type {
                IndexType::FullText => {
                    let stats = storage.indexes().fts_backend().finish_reindex(
                        collection,
                        &index_name,
                        || storage.indexes().acquire_ddl_lock(),
                    )?;
                    tracing::info!(
                        "FTS REINDEX complete: {} indexed, {} skipped, {} concurrent modifications",
                        stats.docs_indexed,
                        stats.docs_skipped,
                        stats.concurrent_modifications
                    );
                }
                IndexType::Hnsw => {
                    storage.indexes().hnsw_backend().finish_reindex(
                        collection,
                        &index_name,
                        || storage.indexes().acquire_ddl_lock(),
                    )?;
                }
                _ => {}
            }
            Ok(count)
        }
        Err(e) => {
            // Cancel and cleanup
            match index.index_type {
                IndexType::FullText => {
                    let _ = storage
                        .indexes()
                        .fts_backend()
                        .cancel_reindex(collection, &index_name);
                }
                IndexType::Hnsw => {
                    let _ = storage
                        .indexes()
                        .hnsw_backend()
                        .cancel_reindex(collection, &index_name);
                }
                _ => {}
            }
            Err(e)
        }
    }
}

// === Helper functions ===

fn format_field_def(f: &FieldDef) -> Value {
    let mut field_map = std::collections::HashMap::new();
    field_map.insert("name".to_string(), Value::String(f.name.clone()));
    field_map.insert(
        "type".to_string(),
        Value::String(format_field_type(&f.field_type)),
    );
    field_map.insert("required".to_string(), Value::Bool(f.required));
    if let Some(default) = &f.default {
        field_map.insert("default".to_string(), format_default(default));
    }
    // Recursively include sub-fields for object types (including arrays of objects)
    match &f.field_type {
        FieldType::Object { fields, .. } => {
            field_map.insert(
                "fields".to_string(),
                Value::Array(fields.iter().map(format_field_def).collect()),
            );
        }
        FieldType::Array(inner) => {
            if let FieldType::Object { fields, .. } = inner.as_ref() {
                field_map.insert(
                    "fields".to_string(),
                    Value::Array(fields.iter().map(format_field_def).collect()),
                );
            }
        }
        _ => {}
    }
    Value::Object(field_map)
}

fn format_field_type(ft: &FieldType) -> String {
    match ft {
        FieldType::String => "string".to_string(),
        FieldType::Int => "int".to_string(),
        FieldType::Float => "float".to_string(),
        FieldType::Decimal => "decimal".to_string(),
        FieldType::Bool => "bool".to_string(),
        FieldType::Datetime => "datetime".to_string(),
        FieldType::Duration => "duration".to_string(),
        FieldType::Bytes => "bytes".to_string(),
        FieldType::Object { .. } => "object".to_string(),
        FieldType::Array(inner) => format!("[{}]", format_field_type(inner)),
        FieldType::AnyArray => "array".to_string(),
        FieldType::Reference(Some(coll)) => format!("ref<{}>", coll),
        FieldType::Reference(None) => "ref".to_string(),
        FieldType::Any => "any".to_string(),
        FieldType::Range(inner) => format!("range<{}>", format_field_type(inner)),
        FieldType::Union(types) => types
            .iter()
            .map(format_field_type)
            .collect::<Vec<_>>()
            .join(" | "),
    }
}

fn format_default(d: &DefaultValue) -> Value {
    match d {
        DefaultValue::Null => Value::Null,
        DefaultValue::Bool(b) => Value::Bool(*b),
        DefaultValue::Int(i) => Value::Int(*i),
        DefaultValue::Float(f) => Value::Float(*f),
        DefaultValue::String(s) => Value::String(s.clone()),
        DefaultValue::Now => Value::String("now()".into()),
    }
}

// === Analyzer DDL ===

use crate::schema::{AnalyzerDef, FilterConfig, TokenizerConfig};

/// Execute DEFINE ANALYZER
pub fn execute_define_analyzer(
    storage: &Database,
    name: String,
    tokenizer: TokenizerConfig,
    filters: Vec<FilterConfig>,
) -> Result<Vec<Row>, ExecuteError> {
    // Validate analyzer name
    validate_name("analyzer", &name)?;

    // Check if trying to redefine builtin
    if crate::storage::analyzers::is_builtin_analyzer(&name) {
        return Err(ExecuteError::Internal(format!(
            "Cannot redefine builtin analyzer: {}",
            name
        )));
    }

    // Check if already exists
    if storage.get_analyzer(&name).is_some() {
        return Err(ExecuteError::Internal(format!(
            "Analyzer already exists: {}",
            name
        )));
    }

    let def = AnalyzerDef {
        name: name.clone(),
        tokenizer,
        filters,
    };

    // Save to storage
    storage.save_analyzer(&def)?;

    // Register in memory (for use by FTS indexes)
    storage
        .indexes()
        .fts_backend()
        .analyzer_registry()
        .register(def)
        .map_err(ExecuteError::Internal)?;

    Ok(status_row(vec![
        ("status", Value::String("created".into())),
        ("analyzer", Value::String(name)),
    ]))
}

/// Execute DROP ANALYZER
pub fn execute_drop_analyzer(storage: &Database, name: &str) -> Result<Vec<Row>, ExecuteError> {
    // Check if builtin
    if crate::storage::analyzers::is_builtin_analyzer(name) {
        return Err(ExecuteError::Internal(format!(
            "Cannot drop builtin analyzer: {}",
            name
        )));
    }

    // Check if analyzer exists
    if storage.get_analyzer(name).is_none() {
        return Err(ExecuteError::AnalyzerNotFound(name.to_string()));
    }

    // Check if analyzer is in use by any index
    let collections = storage.list_collections();
    for coll in &collections {
        for index in storage.get_indexes(coll) {
            if index.analyzer.as_deref() == Some(name) {
                return Err(ExecuteError::Internal(format!(
                    "Cannot drop analyzer '{}': used by index '{}' on collection '{}'",
                    name, index.name, coll
                )));
            }
        }
    }

    // Remove from registry
    storage
        .indexes()
        .fts_backend()
        .analyzer_registry()
        .remove(name)
        .map_err(ExecuteError::Internal)?;

    // Delete from storage
    storage.delete_analyzer(name)?;

    Ok(status_row(vec![
        ("status", Value::String("dropped".into())),
        ("analyzer", Value::String(name.to_string())),
    ]))
}
