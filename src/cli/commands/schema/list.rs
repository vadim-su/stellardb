use crossterm::style::{Color, ResetColor, SetForegroundColor};
use std::io::{self, IsTerminal, Write};

use crate::cli::endpoint::CliEndpoint;

pub async fn run(host: String, database: String, json: bool) -> anyhow::Result<()> {
    let endpoint = CliEndpoint::parse(&host)?;
    let client = reqwest::Client::new();

    let res = client
        .post(endpoint.join("sql"))
        .header("X-Database", &database)
        .json(&serde_json::json!({"query": "DESCRIBE COLLECTIONS"}))
        .send()
        .await?;

    if res.status().is_success() {
        let body: serde_json::Value = res.json().await?;

        if json || !io::stdout().is_terminal() {
            println!("{}", serde_json::to_string_pretty(&body)?);
        } else {
            print_collections_pretty(&body)?;
        }
    } else {
        let err: serde_json::Value = res.json().await?;
        eprintln!("Error: {}", err["error"]);
        std::process::exit(1);
    }

    Ok(())
}

fn print_collections_pretty(data: &serde_json::Value) -> anyhow::Result<()> {
    let mut stdout = io::stdout();

    let collections = data["collections"].as_array();
    let count = data["count"].as_u64().unwrap_or(0);

    writeln!(
        stdout,
        "{}Collections ({}):{}",
        SetForegroundColor(Color::Blue),
        count,
        ResetColor
    )?;
    writeln!(stdout)?;

    if let Some(collections) = collections {
        // Find max name length for alignment
        let max_len = collections
            .iter()
            .map(|c| c["name"].as_str().unwrap_or("").len())
            .max()
            .unwrap_or(10);

        for coll in collections {
            let name = coll["name"].as_str().unwrap_or("?");
            let mode = coll["mode"].as_str().unwrap_or("flexible");

            write!(
                stdout,
                "  {}{:<width$}{}",
                SetForegroundColor(Color::Yellow),
                name,
                ResetColor,
                width = max_len + 2
            )?;

            let mode_color = if mode == "strict" {
                Color::Magenta
            } else {
                Color::Green
            };
            writeln!(
                stdout,
                "{}{}{}",
                SetForegroundColor(mode_color),
                mode,
                ResetColor
            )?;
        }
    }

    if count == 0 {
        writeln!(stdout, "  (no collections)")?;
    }

    Ok(())
}
