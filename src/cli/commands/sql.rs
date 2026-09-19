use crate::cli::{endpoint::CliEndpoint, output};

pub async fn run(
    query: String,
    host: String,
    database: String,
    json: bool,
    pretty: bool,
) -> anyhow::Result<()> {
    let endpoint = CliEndpoint::parse(&host)?;
    let client = reqwest::Client::new();
    let res = client
        .post(endpoint.join("sql"))
        .header("X-Database", &database)
        .json(&serde_json::json!({"query": query}))
        .send()
        .await?;

    if res.status().is_success() {
        let body: serde_json::Value = res.json().await?;
        output::print_result(&body, json, pretty)?;
    } else {
        let err: serde_json::Value = res.json().await?;
        if let Some(msg) = err["error"].as_str() {
            eprintln!("{}", msg);
        } else {
            eprintln!("{}", err["error"]);
        }
        std::process::exit(1);
    }

    Ok(())
}
