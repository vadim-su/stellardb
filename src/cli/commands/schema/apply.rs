use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use crossterm::style::{Color, ResetColor, SetForegroundColor};

use crate::cli::endpoint::CliEndpoint;

use stellardb::schema::{
    CollectionSchema, DefaultValue, FieldDef, FieldType, IndexDef, SchemaMode, parse_schema_file,
};

pub async fn run(
    path: String,
    host: String,
    database: String,
    dry_run: bool,
) -> anyhow::Result<()> {
    let endpoint = CliEndpoint::parse(&host)?;
    let mut stdout = io::stdout();

    // 1. Parse schema files
    let file_schemas = load_schemas_from_path(&path)?;

    if file_schemas.is_empty() {
        writeln!(stdout, "No schema files found in {}", path)?;
        return Ok(());
    }

    writeln!(
        stdout,
        "{}Found {} schema(s) in files{}",
        SetForegroundColor(Color::Blue),
        file_schemas.len(),
        ResetColor
    )?;

    // 2. Fetch current schemas from DB
    let db_schemas = fetch_current_schemas(&endpoint, &database).await?;

    // 3. Compute diff
    let diff = compute_diff(&file_schemas, &db_schemas);

    if diff.is_empty() {
        writeln!(
            stdout,
            "{}No changes to apply{}",
            SetForegroundColor(Color::Green),
            ResetColor
        )?;
        return Ok(());
    }

    // 4. Print diff
    print_diff(&diff)?;

    // 5. Apply if not dry-run
    if dry_run {
        writeln!(stdout)?;
        writeln!(
            stdout,
            "{}Dry run - no changes applied{}",
            SetForegroundColor(Color::Yellow),
            ResetColor
        )?;
    } else {
        writeln!(stdout)?;
        apply_changes(&diff, &endpoint, &database).await?;
        writeln!(
            stdout,
            "{}Changes applied successfully{}",
            SetForegroundColor(Color::Green),
            ResetColor
        )?;
    }

    Ok(())
}

fn load_schemas_from_path(path: &str) -> anyhow::Result<HashMap<String, CollectionSchema>> {
    let path = Path::new(path);
    let mut schemas = HashMap::new();

    if path.is_file() {
        let content = fs::read_to_string(path)?;
        for schema in parse_schema_file(&content)? {
            schemas.insert(schema.name.clone(), schema);
        }
    } else if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let file_path = entry.path();
            if file_path.extension().is_some_and(|e| e == "stellar") {
                let content = fs::read_to_string(&file_path)?;
                for schema in parse_schema_file(&content)? {
                    schemas.insert(schema.name.clone(), schema);
                }
            }
        }
    } else {
        anyhow::bail!("Path does not exist: {}", path.display());
    }

    Ok(schemas)
}

async fn fetch_current_schemas(
    endpoint: &CliEndpoint,
    database: &str,
) -> anyhow::Result<HashMap<String, CollectionSchema>> {
    let client = reqwest::Client::new();
    let mut schemas = HashMap::new();

    // Get list of collections
    let res = client
        .post(endpoint.join("sql"))
        .header("X-Database", database)
        .json(&serde_json::json!({"query": "DESCRIBE COLLECTIONS"}))
        .send()
        .await?;

    if !res.status().is_success() {
        let err: serde_json::Value = res.json().await?;
        anyhow::bail!("Failed to fetch collections: {}", err["error"]);
    }

    let body: serde_json::Value = res.json().await?;
    let empty_vec = vec![];
    let collections = body["collections"].as_array().unwrap_or(&empty_vec);

    // Fetch each schema
    for coll in collections {
        let Some(name) = coll["name"].as_str() else {
            continue;
        };

        let res = client
            .post(endpoint.join("sql"))
            .header("X-Database", database)
            .json(&serde_json::json!({"query": format!("DESCRIBE COLLECTION {}", super::escape_ident(name))}))
            .send()
            .await?;

        if res.status().is_success() {
            let schema_json: serde_json::Value = res.json().await?;
            if let Some(schema) = json_to_schema(&schema_json) {
                schemas.insert(name.to_string(), schema);
            }
        }
    }

    Ok(schemas)
}

fn json_to_schema(json: &serde_json::Value) -> Option<CollectionSchema> {
    let name = json["name"].as_str()?.to_string();
    let mode = match json["mode"].as_str()? {
        "strict" => SchemaMode::Strict,
        _ => SchemaMode::Flexible,
    };

    let fields = json["fields"]
        .as_array()
        .map(|arr| arr.iter().filter_map(json_to_field_def).collect())
        .unwrap_or_default();

    let indexes = json["indexes"]
        .as_array()
        .map(|arr| arr.iter().filter_map(json_to_index_def).collect())
        .unwrap_or_default();

    Some(CollectionSchema {
        name,
        mode,
        fields,
        indexes,
    })
}

fn json_to_field_def(json: &serde_json::Value) -> Option<FieldDef> {
    let name = json["name"].as_str()?.to_string();
    let field_type = str_to_field_type(json["type"].as_str()?)?;
    let required = json["required"].as_bool().unwrap_or(false);
    let default = json_to_default(&json["default"]);

    Some(FieldDef {
        name,
        field_type,
        required,
        default,
    })
}

fn str_to_field_type(s: &str) -> Option<FieldType> {
    match s {
        "string" => Some(FieldType::String),
        "int" => Some(FieldType::Int),
        "float" => Some(FieldType::Float),
        "bool" => Some(FieldType::Bool),
        "datetime" => Some(FieldType::Datetime),
        s if s.starts_with('[') && s.ends_with(']') => {
            let inner = &s[1..s.len() - 1];
            Some(FieldType::Array(Box::new(str_to_field_type(inner)?)))
        }
        "object" => Some(FieldType::Object {
            fields: vec![],
            mode: SchemaMode::Strict,
        }),
        _ => None,
    }
}

fn json_to_default(json: &serde_json::Value) -> Option<DefaultValue> {
    match json {
        serde_json::Value::Null => None,
        serde_json::Value::Bool(b) => Some(DefaultValue::Bool(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(DefaultValue::Int(i))
            } else {
                n.as_f64().map(DefaultValue::Float)
            }
        }
        serde_json::Value::String(s) => {
            if s == "now()" {
                Some(DefaultValue::Now)
            } else {
                Some(DefaultValue::String(s.clone()))
            }
        }
        _ => None,
    }
}

fn json_to_index_def(json: &serde_json::Value) -> Option<IndexDef> {
    use stellardb::schema::IndexType;

    let fields = json["fields"]
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    let unique = json["unique"].as_bool().unwrap_or(false);
    let index_type = match json["index_type"].as_str() {
        Some("FullText") => IndexType::FullText,
        Some("Hnsw") => IndexType::Hnsw,
        _ => IndexType::BTree,
    };
    let name = json["name"].as_str().unwrap_or("").to_string();
    let analyzer = json["analyzer"].as_str().map(|s| s.to_string());
    Some(IndexDef {
        name,
        fields,
        unique,
        index_type,
        hnsw_params: None,
        analyzer,
    })
}

#[derive(Debug)]
enum SchemaDiff {
    Create(CollectionSchema),
    // Future: Update { name: String, changes: Vec<FieldChange> },
}

fn compute_diff(
    file_schemas: &HashMap<String, CollectionSchema>,
    db_schemas: &HashMap<String, CollectionSchema>,
) -> Vec<SchemaDiff> {
    let mut diff = Vec::new();

    for (name, file_schema) in file_schemas {
        if !db_schemas.contains_key(name) {
            diff.push(SchemaDiff::Create(file_schema.clone()));
        }
        // Future: check for field changes
    }

    diff
}

fn print_diff(diff: &[SchemaDiff]) -> anyhow::Result<()> {
    let mut stdout = io::stdout();

    writeln!(stdout)?;
    writeln!(
        stdout,
        "{}Changes to apply:{}",
        SetForegroundColor(Color::Blue),
        ResetColor
    )?;

    for change in diff {
        match change {
            SchemaDiff::Create(schema) => {
                writeln!(
                    stdout,
                    "  {}+ CREATE{} collection {}{}{}",
                    SetForegroundColor(Color::Green),
                    ResetColor,
                    SetForegroundColor(Color::Yellow),
                    schema.name,
                    ResetColor
                )?;
            }
        }
    }

    Ok(())
}

async fn apply_changes(
    diff: &[SchemaDiff],
    endpoint: &CliEndpoint,
    database: &str,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();

    for change in diff {
        match change {
            SchemaDiff::Create(schema) => {
                let sql = schema_to_define_sql(schema);

                let res = client
                    .post(endpoint.join("sql"))
                    .header("X-Database", database)
                    .json(&serde_json::json!({"query": sql}))
                    .send()
                    .await?;

                if !res.status().is_success() {
                    let err: serde_json::Value = res.json().await?;
                    anyhow::bail!("Failed to create {}: {}", schema.name, err["error"]);
                }
            }
        }
    }

    Ok(())
}

fn schema_to_define_sql(schema: &CollectionSchema) -> String {
    let escape = super::escape_ident;
    let mut parts = Vec::new();

    // Schema mode
    let mode = match schema.mode {
        SchemaMode::Strict => "STRICT",
        SchemaMode::Flexible => "FLEXIBLE",
    };
    parts.push(format!("SCHEMA {}", mode));

    // Fields
    for field in &schema.fields {
        let type_str = field_type_to_str(&field.field_type);
        let mut field_def = format!("{} {}", escape(&field.name), type_str);

        if field.required {
            field_def.push_str(" REQUIRED");
        }

        if let Some(default) = &field.default {
            field_def.push_str(&format!(" DEFAULT {}", default_to_str(default)));
        }

        parts.push(field_def);
    }

    // Indexes
    for index in &schema.indexes {
        let fields = if index.fields.len() == 1 {
            escape(&index.fields[0])
        } else {
            let escaped: Vec<String> = index.fields.iter().map(|f| escape(f)).collect();
            format!("({})", escaped.join(", "))
        };
        let unique = if index.unique { " UNIQUE" } else { "" };
        parts.push(format!("INDEX ON {}{}", fields, unique));
    }

    format!(
        "DEFINE COLLECTION {} ({})",
        escape(&schema.name),
        parts.join(", ")
    )
}

fn field_type_to_str(ft: &FieldType) -> String {
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
        FieldType::Array(inner) => format!("[{}]", field_type_to_str(inner)),
        FieldType::AnyArray => "array".to_string(),
        FieldType::Reference(Some(coll)) => format!("ref<{}>", coll),
        FieldType::Reference(None) => "ref".to_string(),
        FieldType::Any => "any".to_string(),
        FieldType::Range(inner) => format!("range<{}>", field_type_to_str(inner)),
        FieldType::Union(types) => types
            .iter()
            .map(field_type_to_str)
            .collect::<Vec<_>>()
            .join(" | "),
    }
}

fn default_to_str(d: &DefaultValue) -> String {
    match d {
        DefaultValue::Null => "null".to_string(),
        DefaultValue::Bool(b) => b.to_string(),
        DefaultValue::Int(i) => i.to_string(),
        DefaultValue::Float(f) => f.to_string(),
        DefaultValue::String(s) => {
            let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{}\"", escaped)
        }
        DefaultValue::Now => "now()".to_string(),
    }
}
