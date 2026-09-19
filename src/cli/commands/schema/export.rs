use crossterm::style::{Color, ResetColor, SetForegroundColor};
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use crate::cli::endpoint::CliEndpoint;

pub async fn run(path: String, host: String, database: String) -> anyhow::Result<()> {
    let endpoint = CliEndpoint::parse(&host)?;
    let mut stdout = io::stdout();

    // Ensure output directory exists
    let output_path = Path::new(&path);
    fs::create_dir_all(output_path)?;

    // Fetch all collections
    let client = reqwest::Client::new();
    let res = client
        .post(endpoint.join("sql"))
        .header("X-Database", &database)
        .json(&serde_json::json!({"query": "DESCRIBE COLLECTIONS"}))
        .send()
        .await?;

    if !res.status().is_success() {
        let err: serde_json::Value = res.json().await?;
        anyhow::bail!("Failed to fetch collections: {}", err["error"]);
    }

    let body: serde_json::Value = res.json().await?;
    let collections = body["collections"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid response"))?;

    if collections.is_empty() {
        writeln!(stdout, "No collections to export")?;
        return Ok(());
    }

    let mut exported = 0;

    for coll in collections {
        let name = match coll["name"].as_str() {
            Some(n) => n,
            None => continue,
        };

        // Fetch full schema
        let res = client
            .post(endpoint.join("sql"))
            .header("X-Database", &database)
            .json(&serde_json::json!({"query": format!("DESCRIBE COLLECTION {}", super::escape_ident(name))}))
            .send()
            .await?;

        if !res.status().is_success() {
            writeln!(
                stdout,
                "{}Warning: Could not fetch schema for {}{}",
                SetForegroundColor(Color::Yellow),
                name,
                ResetColor
            )?;
            continue;
        }

        let schema: serde_json::Value = res.json().await?;

        // Check if it has a real schema (not just message about no schema)
        if schema.get("message").is_some()
            && schema["fields"].as_array().is_none_or(|f| f.is_empty())
        {
            continue; // Skip schemaless collections
        }

        // Generate .stellar content
        let content = schema_to_stellar(&schema);

        // Write to file
        let file_path = output_path.join(format!("{}.stellar", name));
        fs::write(&file_path, &content)?;

        writeln!(
            stdout,
            "{}Exported{} {} -> {}",
            SetForegroundColor(Color::Green),
            ResetColor,
            name,
            file_path.display()
        )?;
        exported += 1;
    }

    writeln!(stdout)?;
    writeln!(stdout, "Exported {} collection(s) to {}", exported, path)?;

    Ok(())
}

fn schema_to_stellar(schema: &serde_json::Value) -> String {
    let mut lines = Vec::new();

    let name = schema["name"].as_str().unwrap_or("unknown");
    let mode = schema["mode"].as_str().unwrap_or("flexible");

    lines.push(format!("collection {} {{", name));
    lines.push(format!("    schema: {}", mode));
    lines.push(String::new());

    // Fields
    if let Some(fields) = schema["fields"].as_array() {
        for field in fields {
            let field_name = field["name"].as_str().unwrap_or("?");
            let field_type = field["type"].as_str().unwrap_or("string");
            let required = field["required"].as_bool().unwrap_or(false);
            let default = &field["default"];

            let mut line = format!("    field {}: {}", field_name, field_type);

            if required {
                line.push_str(" required");
            }

            if !default.is_null() {
                let default_str = match default {
                    serde_json::Value::String(s) if s == "now()" => "now()".to_string(),
                    serde_json::Value::String(s) => format!("\"{}\"", s),
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    _ => default.to_string(),
                };
                line.push_str(&format!(" = {}", default_str));
            }

            lines.push(line);
        }
    }

    // Indexes
    if let Some(indexes) = schema["indexes"].as_array()
        && !indexes.is_empty()
    {
        lines.push(String::new());
        for index in indexes {
            if let Some(fields) = index["fields"].as_array() {
                let unique = index["unique"].as_bool().unwrap_or(false);

                let fields_str = if fields.len() == 1 {
                    fields[0].as_str().unwrap_or("?").to_string()
                } else {
                    let names: Vec<&str> = fields.iter().filter_map(|f| f.as_str()).collect();
                    format!("({})", names.join(", "))
                };

                let mut line = format!("    index on {}", fields_str);
                if unique {
                    line.push_str(" unique");
                }
                lines.push(line);
            }
        }
    }

    lines.push("}".to_string());
    lines.push(String::new());

    lines.join("\n")
}
