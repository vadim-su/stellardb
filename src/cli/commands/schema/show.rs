use crossterm::style::{Color, ResetColor, SetForegroundColor};
use std::io::{self, IsTerminal, Write};

use crate::cli::endpoint::CliEndpoint;

pub async fn run(
    collection: String,
    host: String,
    database: String,
    json: bool,
) -> anyhow::Result<()> {
    let endpoint = CliEndpoint::parse(&host)?;
    let client = reqwest::Client::new();
    let query = format!("DESCRIBE COLLECTION {}", super::escape_ident(&collection));

    let res = client
        .post(endpoint.join("sql"))
        .header("X-Database", &database)
        .json(&serde_json::json!({"query": query}))
        .send()
        .await?;

    if res.status().is_success() {
        let body: serde_json::Value = res.json().await?;

        if json || !io::stdout().is_terminal() {
            println!("{}", serde_json::to_string_pretty(&body)?);
        } else {
            print_schema_pretty(&body)?;
        }
    } else {
        let err: serde_json::Value = res.json().await?;
        eprintln!("Error: {}", err["error"]);
        std::process::exit(1);
    }

    Ok(())
}

fn print_schema_pretty(schema: &serde_json::Value) -> anyhow::Result<()> {
    let mut stdout = io::stdout();

    // Collection name and mode
    let name = schema["name"].as_str().unwrap_or("unknown");
    let mode = schema["mode"].as_str().unwrap_or("flexible");

    write!(
        stdout,
        "{}collection{} ",
        SetForegroundColor(Color::Blue),
        ResetColor
    )?;
    write!(
        stdout,
        "{}{}{}",
        SetForegroundColor(Color::Yellow),
        name,
        ResetColor
    )?;
    writeln!(stdout, " {{")?;

    // Schema mode
    write!(
        stdout,
        "    {}schema:{} ",
        SetForegroundColor(Color::Blue),
        ResetColor
    )?;
    writeln!(stdout, "{}", mode)?;
    writeln!(stdout)?;

    // Fields
    if let Some(fields) = schema["fields"].as_array() {
        for field in fields {
            let field_name = field["name"].as_str().unwrap_or("?");
            let field_type = field["type"].as_str().unwrap_or("?");
            let required = field["required"].as_bool().unwrap_or(false);
            let default = &field["default"];

            write!(
                stdout,
                "    {}field{} ",
                SetForegroundColor(Color::Blue),
                ResetColor
            )?;
            write!(
                stdout,
                "{}{}{}",
                SetForegroundColor(Color::Cyan),
                field_name,
                ResetColor
            )?;
            write!(stdout, ": {}", field_type)?;

            if required {
                write!(
                    stdout,
                    " {}required{}",
                    SetForegroundColor(Color::Magenta),
                    ResetColor
                )?;
            }

            if !default.is_null() {
                write!(stdout, " = ")?;
                write!(
                    stdout,
                    "{}{}{}",
                    SetForegroundColor(Color::Green),
                    default,
                    ResetColor
                )?;
            }

            writeln!(stdout)?;
        }
    }

    // Indexes
    if let Some(indexes) = schema["indexes"].as_array()
        && !indexes.is_empty()
    {
        writeln!(stdout)?;
        for index in indexes {
            let fields = &index["fields"];
            let unique = index["unique"].as_bool().unwrap_or(false);

            write!(
                stdout,
                "    {}index on{} ",
                SetForegroundColor(Color::Blue),
                ResetColor
            )?;

            if let Some(arr) = fields.as_array() {
                if arr.len() == 1 {
                    write!(stdout, "{}", arr[0].as_str().unwrap_or("?"))?;
                } else {
                    write!(stdout, "(")?;
                    for (i, f) in arr.iter().enumerate() {
                        if i > 0 {
                            write!(stdout, ", ")?;
                        }
                        write!(stdout, "{}", f.as_str().unwrap_or("?"))?;
                    }
                    write!(stdout, ")")?;
                }
            }

            if unique {
                write!(
                    stdout,
                    " {}unique{}",
                    SetForegroundColor(Color::Magenta),
                    ResetColor
                )?;
            }

            writeln!(stdout)?;
        }
    }

    writeln!(stdout, "}}")?;

    Ok(())
}
