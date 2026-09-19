mod common;

fn header<'a>(response: &'a reqwest::Response, name: &str) -> Option<&'a str> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
}

#[tokio::test]
async fn cors_is_disabled_by_default_and_emits_no_preflight_headers() {
    let (addr, _tmp) = common::spawn_server().await;
    let response = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, format!("http://{addr}/sql"))
        .header("Origin", "https://app.example.com")
        .header("Access-Control-Request-Method", "POST")
        .header("Access-Control-Request-Headers", "content-type")
        .send()
        .await
        .unwrap();

    for name in [
        "access-control-allow-origin",
        "access-control-allow-methods",
        "access-control-allow-headers",
    ] {
        assert!(
            header(&response, name).is_none(),
            "unexpected {name} header"
        );
    }
}

#[tokio::test]
async fn explicit_origin_list_allows_only_listed_origins() {
    let (addr, _tmp) =
        common::spawn_server_with_cors(Some("https://app.example.com, https://admin.example.com"))
            .await;
    let client = reqwest::Client::new();

    let allowed = client
        .request(reqwest::Method::OPTIONS, format!("http://{addr}/v1/query"))
        .header("Origin", "https://app.example.com")
        .header("Access-Control-Request-Method", "POST")
        .header(
            "Access-Control-Request-Headers",
            "content-type, authorization, x-database",
        )
        .send()
        .await
        .unwrap();
    assert_eq!(allowed.status(), 200);
    assert_eq!(
        header(&allowed, "access-control-allow-origin"),
        Some("https://app.example.com")
    );
    let methods = header(&allowed, "access-control-allow-methods").unwrap_or("");
    assert!(
        methods
            .split(',')
            .map(str::trim)
            .any(|value| value == "POST")
    );
    let headers = header(&allowed, "access-control-allow-headers").unwrap_or("");
    for expected in ["content-type", "authorization", "x-database"] {
        assert!(
            headers
                .split(',')
                .map(str::trim)
                .any(|value| value.eq_ignore_ascii_case(expected))
        );
    }

    let denied = client
        .request(reqwest::Method::OPTIONS, format!("http://{addr}/v1/query"))
        .header("Origin", "https://evil.example.com")
        .header("Access-Control-Request-Method", "POST")
        .send()
        .await
        .unwrap();
    assert!(header(&denied, "access-control-allow-origin").is_none());
}

#[tokio::test]
async fn explicit_wildcard_is_available_for_development() {
    let (addr, _tmp) = common::spawn_server_with_cors(Some("*")).await;
    let response = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, format!("http://{addr}/v1/query"))
        .header("Origin", "https://any.example.com")
        .header("Access-Control-Request-Method", "POST")
        .send()
        .await
        .unwrap();
    assert_eq!(header(&response, "access-control-allow-origin"), Some("*"));
}

#[tokio::test]
async fn invalid_cors_configuration_fails_closed() {
    let (addr, _tmp) = common::spawn_server_with_cors(Some("https://ok.example/path")).await;
    let response = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, format!("http://{addr}/v1/query"))
        .header("Origin", "https://ok.example")
        .header("Access-Control-Request-Method", "POST")
        .send()
        .await
        .unwrap();
    assert!(header(&response, "access-control-allow-origin").is_none());
}

#[tokio::test]
async fn preflight_rejects_undocumented_methods_and_headers() {
    let (addr, _tmp) = common::spawn_server_with_cors(Some("*")).await;
    let response = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, format!("http://{addr}/sql"))
        .header("Origin", "https://app.example.com")
        .header("Access-Control-Request-Method", "TRACE")
        .header("Access-Control-Request-Headers", "x-evil")
        .send()
        .await
        .unwrap();

    let methods = header(&response, "access-control-allow-methods").unwrap_or("");
    assert!(
        !methods
            .split(',')
            .map(str::trim)
            .any(|method| method.eq_ignore_ascii_case("TRACE"))
    );
    let headers = header(&response, "access-control-allow-headers").unwrap_or("");
    assert!(
        !headers
            .split(',')
            .map(str::trim)
            .any(|name| name.eq_ignore_ascii_case("x-evil"))
    );
}
