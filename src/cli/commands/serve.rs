#[cfg(feature = "server")]
use std::sync::Arc;

#[cfg(feature = "server")]
use rand::RngExt;
#[cfg(feature = "server")]
use tracing_subscriber::{EnvFilter, fmt};

#[cfg(feature = "server")]
use stellardb::metrics::{MetricsConfig, spawn_flush_task};
#[cfg(feature = "server")]
use stellardb::namespace::Namespace;
#[cfg(feature = "server")]
use stellardb::server::admission::AdmissionConfig;
#[cfg(all(feature = "server", feature = "grpc"))]
use stellardb::server::grpc::{GrpcConfig, GrpcServer};
#[cfg(feature = "tls")]
use stellardb::server::tls::TlsConfig;
#[cfg(feature = "server")]
use stellardb::server::{HttpServerConfig, create_router_with_config};
#[cfg(feature = "server")]
use stellardb::version::version_string;

#[cfg(feature = "server")]
#[allow(clippy::too_many_arguments)]
pub async fn run(
    port: u16,
    grpc_port: Option<u16>,
    login_body_limit: usize,
    http_body_limit: usize,
    grpc_max_message_size: usize,
    grpc_concurrency_limit: usize,
    grpc_max_concurrent_streams: u32,
    host: String,
    insecure_allow_remote: bool,
    tls_cert_file: Option<String>,
    tls_key_file: Option<String>,
    non_browser_api: bool,
    cors_allow_origins: Option<String>,
    data: String,
    verbose: u8,
    quiet: bool,
    query_timeout: String,
    jwt_secret_arg: Option<String>,
    jwt_secret_file: Option<String>,
    jwt_ttl: u64,
    anonymous_user: Option<String>,
    station_examples: Option<String>,
    max_db_concurrency: usize,
    db_deadline: String,
    index_build_chunk_size: usize,
    index_build_memory_budget: usize,
    index_build_workers: usize,
    max_login_concurrency: usize,
    login_rate_per_second: f64,
    login_burst: usize,
    login_client_rate_per_second: f64,
    login_client_burst: usize,
    max_login_clients: usize,
    login_client_retention: String,
    query_client_rate_per_second: f64,
    query_client_burst: usize,
    max_query_clients: usize,
    query_client_retention: String,
    trusted_proxy_hops: usize,
    metrics_max_query_stats: usize,
    metrics_query_retention_days: u64,
    metrics_max_index_stats: usize,
    metrics_index_retention_days: u64,
    metrics_max_collection_stats: usize,
    metrics_collection_retention_days: u64,
) -> anyhow::Result<()> {
    let transport_limits = build_transport_limits(
        login_body_limit,
        http_body_limit,
        grpc_max_message_size,
        grpc_concurrency_limit,
        grpc_max_concurrent_streams,
    )?;
    #[cfg(not(feature = "grpc"))]
    let _ = transport_limits;
    let tls = resolve_tls_config(tls_cert_file.as_deref(), tls_key_file.as_deref())?;
    let tls_enabled = tls.is_some();
    validate_bind_security(&host, insecure_allow_remote, tls_enabled)?;
    let cors = resolve_cors_config(
        &host,
        insecure_allow_remote,
        tls_enabled,
        non_browser_api,
        cors_allow_origins.as_deref(),
    )?;
    validate_jwt_ttl(jwt_ttl)?;
    let explicit_jwt_secret =
        resolve_explicit_jwt_secret(jwt_secret_arg.as_deref(), jwt_secret_file.as_deref())?;
    let admission_config = build_admission_config(
        max_db_concurrency,
        &db_deadline,
        max_login_concurrency,
        login_rate_per_second,
        login_burst,
        login_client_rate_per_second,
        login_client_burst,
        max_login_clients,
        &login_client_retention,
        query_client_rate_per_second,
        query_client_burst,
        max_query_clients,
        &query_client_retention,
    )?;
    if trusted_proxy_hops > 16 {
        anyhow::bail!("--trusted-proxy-hops must be between 0 and 16");
    }
    let metrics_config = build_metrics_config(
        metrics_max_query_stats,
        metrics_query_retention_days,
        metrics_max_index_stats,
        metrics_index_retention_days,
        metrics_max_collection_stats,
        metrics_collection_retention_days,
    )?;
    let http_addr = resolve_bind_address(&host, port).await?;
    #[cfg(feature = "grpc")]
    let grpc_addr = match grpc_port {
        Some(port) => Some(resolve_bind_address(&host, port).await?),
        None => None,
    };
    #[cfg(not(feature = "grpc"))]
    if let Some(port) = grpc_port {
        anyhow::bail!(
            "gRPC support is not enabled in this build; rebuild with --features grpc to use --grpc-port {port}"
        );
    }
    init_logging(verbose, quiet);

    let fresh = !std::path::Path::new(&data).join("databases").exists();
    let default_index_build = stellardb::IndexBuildConfig::default();
    let namespace = Namespace::open_with_index_build_config(
        data.as_ref(),
        stellardb::IndexBuildConfig {
            chunk_size: index_build_chunk_size,
            memory_budget_bytes: index_build_memory_budget,
            worker_count: if index_build_workers == 0 {
                default_index_build.worker_count
            } else {
                index_build_workers
            },
        },
    )?;

    // On first-ever startup, create a "default" database
    if fresh {
        namespace.create_database("default")?;
        tracing::info!("created default database");
    }

    let namespace = Arc::new(namespace);

    // Initialize _system database and auth
    let system_db = namespace.ensure_system_database()?;
    let jwt_secret = match explicit_jwt_secret {
        Some(secret) => secret,
        None => load_or_generate_jwt_secret(&system_db)?,
    };
    let (auth_service, root_password) =
        stellardb::auth::AuthService::init(system_db, &jwt_secret, jwt_ttl)
            .map_err(|e| anyhow::anyhow!(e))?;
    let auth_service = match anonymous_user.as_deref() {
        Some(username) => auth_service
            .with_anonymous_user(username)
            .map_err(|e| anyhow::anyhow!("--anonymous-user: {e}"))?,
        None => auth_service,
    };

    if let Some(pw) = root_password
        && !quiet
    {
        println!();
        println!("========================================");
        println!("  Root password (shown only once): {}", pw);
        println!("========================================");
        println!();
    }

    if let Some(username) = auth_service.anonymous_user() {
        if auth_service
            .anonymous_user_exists()
            .map_err(|e| anyhow::anyhow!(e))?
        {
            tracing::info!(username, "anonymous access enabled");
        } else {
            tracing::warn!(
                username,
                "anonymous access enabled but the user does not exist; unauthenticated requests are rejected until it is created"
            );
        }
    }

    let station_examples = match station_examples.as_deref() {
        Some(path) => {
            let examples = stellardb::server::station_examples::load(std::path::Path::new(path))
                .map_err(|e| anyhow::anyhow!("--station-examples: {e}"))?;
            tracing::info!(
                databases = examples.len(),
                "station examples loaded from {path}"
            );
            examples
        }
        None => Default::default(),
    };

    let auth_service = Arc::new(auth_service);

    // Print startup banner (unless quiet)
    if !quiet {
        let db_count = namespace.list_databases().len();
        print_banner(port, grpc_port, &host, &data, db_count, tls_enabled);
        if let Some(warning) = remote_http_warning(&host, tls_enabled) {
            println!("{}", warning);
            println!();
        }
    }

    let query_timeout_default = if query_timeout == "0" {
        None
    } else {
        let nanos = stellardb::query::parse::expr::parse_duration_nanos(&query_timeout)
            .map_err(|e| anyhow::anyhow!("invalid --query-timeout: {}", e))?;
        Some(std::time::Duration::from_nanos(nanos))
    };

    let (app, state) = create_router_with_config(
        namespace.clone(),
        query_timeout_default,
        Some(auth_service),
        HttpServerConfig {
            admission: admission_config,
            cors,
            metrics: metrics_config,
            trusted_proxy_hops,
            login_body_limit,
            http_body_limit,
            station_examples,
        },
    );

    // Spawn background task to cleanup timed out sessions
    let sessions_for_cleanup = state.sessions.clone();
    tokio::spawn(async move {
        let interval = sessions_for_cleanup.config().cleanup_interval;
        loop {
            tokio::time::sleep(interval).await;
            let cleaned = sessions_for_cleanup.cleanup_timed_out();
            if cleaned > 0 {
                tracing::debug!("Cleaned up {} timed out sessions", cleaned);
            }
        }
    });

    // Spawn metrics flush task
    let _flush_handle = spawn_flush_task(state.metrics.clone(), state.metrics.config());

    // Start gRPC server if port is specified
    if grpc_port.is_some() {
        #[cfg(feature = "grpc")]
        {
            let grpc_addr = grpc_addr.expect("configured gRPC port was resolved");
            let grpc_server = GrpcServer::with_config(
                state,
                GrpcConfig {
                    max_message_size: transport_limits.grpc_max_message_size,
                    concurrency_limit_per_connection: transport_limits.grpc_concurrency_limit,
                    max_concurrent_streams: transport_limits.grpc_max_concurrent_streams,
                    #[cfg(feature = "tls")]
                    tls: tls.clone(),
                },
            );
            tokio::spawn(async move {
                if let Err(e) = grpc_server.start(grpc_addr).await {
                    tracing::error!("gRPC server error: {}", e);
                }
            });
        }
    }

    let addr = http_addr;
    tracing::debug!("binding to {}", addr);

    #[cfg(feature = "tls")]
    if let Some(tls) = tls {
        let handle = axum_server::Handle::new();
        let shutdown_handle = handle.clone();
        tokio::spawn(async move {
            shutdown_signal(quiet).await;
            shutdown_handle.graceful_shutdown(Some(std::time::Duration::from_secs(30)));
        });
        axum_server::bind_rustls(addr, tls.http_config())
            .handle(handle)
            .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
            .await?;
    } else {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal(quiet))
        .await?;
    }

    #[cfg(not(feature = "tls"))]
    {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal(quiet))
        .await?;
    }

    if !quiet {
        println!("Shutdown complete");
    }

    Ok(())
}

#[cfg(not(feature = "server"))]
#[allow(clippy::too_many_arguments)]
pub async fn run(
    _port: u16,
    _grpc_port: Option<u16>,
    _login_body_limit: usize,
    _http_body_limit: usize,
    _grpc_max_message_size: usize,
    _grpc_concurrency_limit: usize,
    _grpc_max_concurrent_streams: u32,
    _host: String,
    _insecure_allow_remote: bool,
    _tls_cert_file: Option<String>,
    _tls_key_file: Option<String>,
    _non_browser_api: bool,
    _cors_allow_origins: Option<String>,
    _data: String,
    _verbose: u8,
    _quiet: bool,
    _query_timeout: String,
    _jwt_secret_arg: Option<String>,
    _jwt_secret_file: Option<String>,
    _jwt_ttl: u64,
    _anonymous_user: Option<String>,
    _station_examples: Option<String>,
    _max_db_concurrency: usize,
    _db_deadline: String,
    _index_build_chunk_size: usize,
    _index_build_memory_budget: usize,
    _index_build_workers: usize,
    _max_login_concurrency: usize,
    _login_rate_per_second: f64,
    _login_burst: usize,
    _login_client_rate_per_second: f64,
    _login_client_burst: usize,
    _max_login_clients: usize,
    _login_client_retention: String,
    _query_client_rate_per_second: f64,
    _query_client_burst: usize,
    _max_query_clients: usize,
    _query_client_retention: String,
    _trusted_proxy_hops: usize,
    _metrics_max_query_stats: usize,
    _metrics_query_retention_days: u64,
    _metrics_max_index_stats: usize,
    _metrics_index_retention_days: u64,
    _metrics_max_collection_stats: usize,
    _metrics_collection_retention_days: u64,
) -> anyhow::Result<()> {
    Err(anyhow::anyhow!(
        "server support is not enabled in this build; rebuild with --features server to use `stellar serve`"
    ))
}

#[cfg(feature = "server")]
#[allow(clippy::too_many_arguments)]
fn build_admission_config(
    max_db_concurrency: usize,
    db_deadline: &str,
    max_login_concurrency: usize,
    login_rate_per_second: f64,
    login_burst: usize,
    login_client_rate_per_second: f64,
    login_client_burst: usize,
    max_login_clients: usize,
    login_client_retention: &str,
    query_client_rate_per_second: f64,
    query_client_burst: usize,
    max_query_clients: usize,
    query_client_retention: &str,
) -> anyhow::Result<AdmissionConfig> {
    if !(1..=4096).contains(&max_db_concurrency) {
        anyhow::bail!("--max-db-concurrency must be between 1 and 4096");
    }
    if !(1..=256).contains(&max_login_concurrency) {
        anyhow::bail!("--max-login-concurrency must be between 1 and 256");
    }
    if !login_rate_per_second.is_finite() || login_rate_per_second <= 0.0 {
        anyhow::bail!("--login-rate-per-second must be a positive finite number");
    }
    if !(1..=100_000).contains(&login_burst) {
        anyhow::bail!("--login-burst must be between 1 and 100000");
    }
    if !login_client_rate_per_second.is_finite() || login_client_rate_per_second <= 0.0 {
        anyhow::bail!("--login-client-rate-per-second must be a positive finite number");
    }
    if !(1..=100_000).contains(&login_client_burst) {
        anyhow::bail!("--login-client-burst must be between 1 and 100000");
    }
    if max_login_clients > 1_000_000 {
        anyhow::bail!("--max-login-clients must be between 0 and 1000000");
    }
    if !query_client_rate_per_second.is_finite() || query_client_rate_per_second < 0.0 {
        anyhow::bail!("--query-client-rate-per-second must be a non-negative finite number");
    }
    if !(1..=100_000).contains(&query_client_burst) {
        anyhow::bail!("--query-client-burst must be between 1 and 100000");
    }
    if max_query_clients > 1_000_000 {
        anyhow::bail!("--max-query-clients must be between 0 and 1000000");
    }

    let retention_nanos =
        stellardb::query::parse::expr::parse_duration_nanos(login_client_retention)
            .map_err(|error| anyhow::anyhow!("invalid --login-client-retention: {error}"))?;
    let login_client_retention = std::time::Duration::from_nanos(retention_nanos);
    if login_client_retention < std::time::Duration::from_secs(1)
        || login_client_retention > std::time::Duration::from_secs(24 * 60 * 60)
    {
        anyhow::bail!("--login-client-retention must be between 1s and 24h");
    }
    let query_retention_nanos =
        stellardb::query::parse::expr::parse_duration_nanos(query_client_retention)
            .map_err(|error| anyhow::anyhow!("invalid --query-client-retention: {error}"))?;
    let query_client_retention = std::time::Duration::from_nanos(query_retention_nanos);
    if query_client_retention < std::time::Duration::from_secs(1)
        || query_client_retention > std::time::Duration::from_secs(24 * 60 * 60)
    {
        anyhow::bail!("--query-client-retention must be between 1s and 24h");
    }

    let deadline_nanos = stellardb::query::parse::expr::parse_duration_nanos(db_deadline)
        .map_err(|error| anyhow::anyhow!("invalid --db-deadline: {error}"))?;
    let db_deadline = std::time::Duration::from_nanos(deadline_nanos);
    if db_deadline < std::time::Duration::from_millis(100)
        || db_deadline > std::time::Duration::from_secs(24 * 60 * 60)
    {
        anyhow::bail!("--db-deadline must be between 100ms and 24h");
    }

    Ok(AdmissionConfig {
        max_db_concurrency,
        db_deadline,
        max_login_concurrency,
        login_rate_per_second,
        login_burst,
        login_client_rate_per_second,
        login_client_burst,
        max_login_clients,
        login_client_retention,
        query_client_rate_per_second,
        query_client_burst,
        max_query_clients,
        query_client_retention,
    })
}

#[cfg(feature = "server")]
#[cfg_attr(not(feature = "grpc"), allow(dead_code))]
#[derive(Debug, Clone, Copy)]
struct TransportLimits {
    grpc_max_message_size: usize,
    grpc_concurrency_limit: usize,
    grpc_max_concurrent_streams: u32,
}

#[cfg(feature = "server")]
fn build_transport_limits(
    login_body_limit: usize,
    http_body_limit: usize,
    grpc_max_message_size: usize,
    grpc_concurrency_limit: usize,
    grpc_max_concurrent_streams: u32,
) -> anyhow::Result<TransportLimits> {
    if !(1_024..=1024 * 1024).contains(&login_body_limit) {
        anyhow::bail!("--login-body-limit must be between 1024 and 1048576 bytes");
    }
    if !(1_024..=1024 * 1024 * 1024).contains(&http_body_limit) {
        anyhow::bail!("--http-body-limit must be between 1024 and 1073741824 bytes");
    }
    if login_body_limit > http_body_limit {
        anyhow::bail!("--login-body-limit must not exceed --http-body-limit");
    }
    if !(1_024..=1024 * 1024 * 1024).contains(&grpc_max_message_size) {
        anyhow::bail!("--grpc-max-message-size must be between 1024 and 1073741824 bytes");
    }
    if !(1..=100_000).contains(&grpc_concurrency_limit) {
        anyhow::bail!("--grpc-concurrency-limit must be between 1 and 100000");
    }
    if !(1..=100_000).contains(&grpc_max_concurrent_streams) {
        anyhow::bail!("--grpc-max-concurrent-streams must be between 1 and 100000");
    }
    Ok(TransportLimits {
        grpc_max_message_size,
        grpc_concurrency_limit,
        grpc_max_concurrent_streams,
    })
}

#[cfg(feature = "server")]
fn build_metrics_config(
    max_query_stats: usize,
    query_retention_days: u64,
    max_index_stats: usize,
    index_retention_days: u64,
    max_collection_stats: usize,
    collection_retention_days: u64,
) -> anyhow::Result<MetricsConfig> {
    const MAX_CARDINALITY: usize = 1_000_000;
    for (name, value) in [
        ("--metrics-max-query-stats", max_query_stats),
        ("--metrics-max-index-stats", max_index_stats),
        ("--metrics-max-collection-stats", max_collection_stats),
    ] {
        if value > MAX_CARDINALITY {
            anyhow::bail!("{name} must be between 0 and {MAX_CARDINALITY}");
        }
    }
    for (name, value) in [
        ("--metrics-query-retention-days", query_retention_days),
        ("--metrics-index-retention-days", index_retention_days),
        (
            "--metrics-collection-retention-days",
            collection_retention_days,
        ),
    ] {
        if !(1..=3_650).contains(&value) {
            anyhow::bail!("{name} must be between 1 and 3650");
        }
    }

    Ok(MetricsConfig {
        max_query_stats,
        retention_days: query_retention_days,
        max_index_stats,
        index_retention_days,
        max_collection_stats,
        collection_retention_days,
        ..Default::default()
    })
}

#[cfg(feature = "server")]
async fn resolve_bind_address(host: &str, port: u16) -> anyhow::Result<std::net::SocketAddr> {
    if let Ok(address) = host.parse::<std::net::IpAddr>() {
        return Ok(std::net::SocketAddr::new(address, port));
    }

    tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| anyhow::anyhow!("failed to resolve --host"))?
        .next()
        .ok_or_else(|| anyhow::anyhow!("--host did not resolve to an address"))
}

#[cfg(feature = "tls")]
fn resolve_tls_config(
    certificate_file: Option<&str>,
    private_key_file: Option<&str>,
) -> anyhow::Result<Option<TlsConfig>> {
    match (certificate_file, private_key_file) {
        (None, None) => Ok(None),
        (Some(certificate_file), Some(private_key_file)) => {
            TlsConfig::from_files(certificate_file, private_key_file).map(Some)
        }
        _ => anyhow::bail!("--tls-cert-file and --tls-key-file must be provided together"),
    }
}

#[cfg(all(feature = "server", not(feature = "tls")))]
fn resolve_tls_config(
    certificate_file: Option<&str>,
    private_key_file: Option<&str>,
) -> anyhow::Result<Option<()>> {
    match (certificate_file, private_key_file) {
        (None, None) => Ok(None),
        (Some(_), Some(_)) => {
            anyhow::bail!("TLS support is not enabled in this build; rebuild with --features tls")
        }
        _ => anyhow::bail!("--tls-cert-file and --tls-key-file must be provided together"),
    }
}

#[cfg(feature = "server")]
fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .map(|address| address.is_loopback())
            .unwrap_or(false)
}

#[cfg(feature = "server")]
fn validate_bind_security(
    host: &str,
    insecure_allow_remote: bool,
    tls_enabled: bool,
) -> anyhow::Result<()> {
    if tls_enabled || insecure_allow_remote || is_loopback_host(host) {
        return Ok(());
    }

    Err(anyhow::anyhow!(
        "refusing plain remote bind to '{host}': configure --tls-cert-file and --tls-key-file, or use --insecure-allow-remote only behind a trusted TLS-terminating reverse proxy or in an isolated network"
    ))
}

#[cfg(feature = "server")]
fn resolve_cors_config(
    host: &str,
    insecure_allow_remote: bool,
    tls_enabled: bool,
    non_browser_api: bool,
    cors_allow_origins: Option<&str>,
) -> anyhow::Result<stellardb::server::CorsConfig> {
    if non_browser_api && cors_allow_origins.is_some() {
        anyhow::bail!(
            "--non-browser-api conflicts with STELLARDB_CORS_ALLOW_ORIGINS; choose exactly one browser contract"
        );
    }
    let cors = stellardb::server::CorsConfig::from_value(cors_allow_origins)
        .map_err(|error| anyhow::anyhow!("invalid STELLARDB_CORS_ALLOW_ORIGINS: {error}"))?;
    if !tls_enabled
        && !is_loopback_host(host)
        && insecure_allow_remote
        && !non_browser_api
        && cors.is_disabled()
    {
        anyhow::bail!(
            "remote insecure bind requires STELLARDB_CORS_ALLOW_ORIGINS or --non-browser-api; the latter explicitly disables browser access"
        );
    }
    Ok(cors)
}

#[cfg(feature = "server")]
fn init_logging(verbose: u8, quiet: bool) {
    let filter = if std::env::var("RUST_LOG").is_ok() && verbose == 0 && !quiet {
        // RUST_LOG is set and no CLI flags - use it
        EnvFilter::from_default_env()
    } else {
        // Build filter based on CLI flags
        let (stellar_level, deps_level) = match (quiet, verbose) {
            (true, _) => ("error", "error"),
            (_, 0) => ("info", "warn"),
            (_, 1) => ("debug", "warn"),
            (_, 2) => ("trace", "info"),
            _ => ("trace", "debug"),
        };
        EnvFilter::new(format!("{},stellar={}", deps_level, stellar_level))
    };

    fmt().with_env_filter(filter).init();
}

#[cfg(feature = "server")]
const STAR_FIELD_WIDTH: usize = 28;
#[cfg(feature = "server")]
const STAR_FIELD_HEIGHT: usize = 5;
#[cfg(feature = "server")]
const MIN_HORIZONTAL_DISTANCE: usize = 4; // 3 empty chars between stars horizontally
#[cfg(feature = "server")]
const MIN_STARS: usize = 12;

// Weighted distribution: dim dots common, medium stars less, bright stars rare
#[cfg(feature = "server")]
const STARS: &[char] = &['.', '.', '.', '.', '*', '*', '✦'];

#[cfg(feature = "server")]
fn generate_star_field() -> Vec<String> {
    let mut rng = rand::rng();
    let mut grid: Vec<Vec<char>> = vec![vec![' '; STAR_FIELD_WIDTH]; STAR_FIELD_HEIGHT];
    let mut star_count = 0;

    // First pass: place stars with spacing rules
    for (_row, grid_row) in grid.iter_mut().enumerate().take(STAR_FIELD_HEIGHT) {
        let mut last_star_col: Option<usize> = None;

        for (col, cell) in grid_row.iter_mut().enumerate().take(STAR_FIELD_WIDTH) {
            // Check horizontal distance
            let h_ok = match last_star_col {
                Some(pos) => col - pos >= MIN_HORIZONTAL_DISTANCE,
                None => true,
            };

            // Check vertical neighbors - skip on this pass as grid isn't fully populated yet
            let v_ok = true;

            if h_ok && v_ok && rng.random::<f64>() < 0.12 {
                let star = STARS[rng.random_range(0..STARS.len())];
                *cell = star;
                last_star_col = Some(col);
                star_count += 1;
            }
        }
    }

    // Second pass: add more stars if below minimum (bounded to prevent infinite loop)
    let mut attempts = 0;
    while star_count < MIN_STARS && attempts < 1000 {
        attempts += 1;
        let row = rng.random_range(0..STAR_FIELD_HEIGHT);
        let col = rng.random_range(0..STAR_FIELD_WIDTH);

        if grid[row][col] != ' ' {
            continue;
        }

        // Check horizontal distance
        let h_ok = (0..STAR_FIELD_WIDTH).all(|c| {
            if grid[row][c] == ' ' {
                true
            } else {
                col.abs_diff(c) >= MIN_HORIZONTAL_DISTANCE
            }
        });

        // Check vertical neighbors
        let v_ok = (row == 0 || grid[row - 1][col] == ' ')
            && (row == STAR_FIELD_HEIGHT - 1 || grid[row + 1][col] == ' ');

        if h_ok && v_ok {
            let star = STARS[rng.random_range(0..STARS.len())];
            grid[row][col] = star;
            star_count += 1;
        }
    }

    grid.into_iter()
        .map(|row| row.into_iter().collect())
        .collect()
}

#[cfg(feature = "server")]
fn banner_info_lines(
    port: u16,
    grpc_port: Option<u16>,
    host: &str,
    data: &str,
    db_count: usize,
    tls_enabled: bool,
) -> Vec<String> {
    let version_line = format!("StellarDB {}", version_string());

    let data_line = if db_count == 0 {
        format!("Data: {} (no databases)", data)
    } else {
        let db_word = if db_count == 1 {
            "database"
        } else {
            "databases"
        };
        format!("Data: {} ({} {})", data, db_count, db_word)
    };

    let http_scheme = if tls_enabled { "https" } else { "http" };
    let grpc_scheme = if tls_enabled { "grpcs" } else { "grpc" };
    let listen_line = match grpc_port {
        Some(gp) => format!(
            "{}://{}:{} | {}://{}:{}",
            http_scheme, host, port, grpc_scheme, host, gp
        ),
        None => format!("{}://{}:{}", http_scheme, host, port),
    };

    #[cfg(feature = "ui")]
    let lines = vec![
        version_line,
        data_line,
        listen_line,
        format!("Station: {}://{}:{}/_station/", http_scheme, host, port),
    ];
    #[cfg(not(feature = "ui"))]
    let lines = vec![version_line, data_line, listen_line];
    lines
}

#[cfg(feature = "server")]
fn print_banner(
    port: u16,
    grpc_port: Option<u16>,
    host: &str,
    data: &str,
    db_count: usize,
    tls_enabled: bool,
) {
    let stars = generate_star_field();
    let info_lines = banner_info_lines(port, grpc_port, host, data, db_count, tls_enabled);

    for (i, star_line) in stars.iter().enumerate() {
        if let Some(info_line) = i.checked_sub(1).and_then(|index| info_lines.get(index)) {
            println!("{}    {}", star_line, info_line);
        } else {
            println!("{}", star_line);
        }
    }
    println!();
}

#[cfg(feature = "server")]
fn remote_http_warning(host: &str, tls_enabled: bool) -> Option<&'static str> {
    if tls_enabled || host.eq_ignore_ascii_case("localhost") {
        return None;
    }

    if host
        .parse::<std::net::IpAddr>()
        .map(|addr| addr.is_loopback())
        .unwrap_or(false)
    {
        return None;
    }

    Some(
        "WARNING: StellarDB serves plain HTTP. When binding to a remote interface, put it behind an HTTPS reverse proxy and configure STELLARDB_CORS_ALLOW_ORIGINS.",
    )
}

#[cfg(feature = "server")]
const MIN_JWT_SECRET_BYTES: usize = 32;
#[cfg(feature = "server")]
const MAX_JWT_SECRET_FILE_BYTES: u64 = 64 * 1024;
#[cfg(feature = "server")]
const MIN_JWT_TTL_SECS: u64 = 60;
#[cfg(feature = "server")]
const MAX_JWT_TTL_SECS: u64 = 365 * 24 * 60 * 60;

#[cfg(feature = "server")]
fn validate_jwt_ttl(ttl_secs: u64) -> anyhow::Result<()> {
    if !(MIN_JWT_TTL_SECS..=MAX_JWT_TTL_SECS).contains(&ttl_secs)
        || i64::try_from(ttl_secs).is_err()
    {
        anyhow::bail!(
            "--jwt-ttl must be between {MIN_JWT_TTL_SECS} and {MAX_JWT_TTL_SECS} seconds"
        );
    }
    Ok(())
}

#[cfg(feature = "server")]
fn validate_explicit_jwt_secret(secret: &[u8]) -> anyhow::Result<()> {
    if secret.len() < MIN_JWT_SECRET_BYTES {
        anyhow::bail!("JWT secret must contain at least {MIN_JWT_SECRET_BYTES} bytes");
    }
    Ok(())
}

#[cfg(feature = "server")]
fn resolve_explicit_jwt_secret(
    secret: Option<&str>,
    secret_file: Option<&str>,
) -> anyhow::Result<Option<Vec<u8>>> {
    if secret.is_some() && secret_file.is_some() {
        anyhow::bail!("--jwt-secret and --jwt-secret-file cannot be used together");
    }

    let secret = match (secret, secret_file) {
        (Some(value), None) => Some(value.as_bytes().to_vec()),
        (None, Some(path)) => Some(read_jwt_secret_file(path)?),
        (None, None) => None,
        (Some(_), Some(_)) => unreachable!(),
    };

    if let Some(secret) = &secret {
        validate_explicit_jwt_secret(secret)?;
    }
    Ok(secret)
}

#[cfg(feature = "server")]
fn read_jwt_secret_file(path: &str) -> anyhow::Result<Vec<u8>> {
    use std::io::Read;

    let path = std::path::Path::new(path);
    let path_metadata = std::fs::symlink_metadata(path)
        .map_err(|_| anyhow::anyhow!("failed to inspect JWT secret file"))?;
    if path_metadata.file_type().is_symlink() {
        anyhow::bail!("JWT secret file must not be a symbolic link");
    }

    // The post-open metadata check validates the opened target. Without a platform-specific
    // O_NOFOLLOW dependency there remains a narrow rename race between symlink_metadata and open.
    let file =
        std::fs::File::open(path).map_err(|_| anyhow::anyhow!("failed to open JWT secret file"))?;
    let metadata = file
        .metadata()
        .map_err(|_| anyhow::anyhow!("failed to inspect opened JWT secret file"))?;
    if !metadata.is_file() {
        anyhow::bail!("JWT secret file must be a regular file");
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            anyhow::bail!("JWT secret file must not be accessible by group or other users");
        }
    }

    let mut bytes = Vec::new();
    file.take(MAX_JWT_SECRET_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("failed to read JWT secret file"))?;
    if bytes.len() as u64 > MAX_JWT_SECRET_FILE_BYTES {
        anyhow::bail!("JWT secret file is too large");
    }

    if bytes.ends_with(b"\r\n") {
        bytes.truncate(bytes.len() - 2);
    } else if bytes.ends_with(b"\n") {
        bytes.truncate(bytes.len() - 1);
    }
    if bytes.iter().any(|byte| matches!(byte, b'\r' | b'\n')) {
        anyhow::bail!("JWT secret file must contain exactly one text line");
    }

    Ok(bytes)
}

#[cfg(feature = "server")]
fn load_or_generate_jwt_secret(
    system_db: &stellardb::storage::Database,
) -> anyhow::Result<Vec<u8>> {
    use std::collections::HashMap;
    use stellardb::document::{Document, Value};

    if let Some(doc) = system_db.get_document("config", "jwt_secret")? {
        let encoded = match doc.fields.get("secret") {
            Some(Value::String(value)) if !value.is_empty() => value,
            _ => anyhow::bail!("stored JWT secret is invalid"),
        };
        let bytes =
            hex::decode(encoded).map_err(|_| anyhow::anyhow!("stored JWT secret is invalid"))?;
        if bytes.len() < MIN_JWT_SECRET_BYTES {
            anyhow::bail!("stored JWT secret must contain at least {MIN_JWT_SECRET_BYTES} bytes");
        }
        return Ok(bytes);
    }

    let mut secret = [0u8; MIN_JWT_SECRET_BYTES];
    rand::rng().fill(&mut secret);

    let doc = Document {
        id: "config:jwt_secret".to_string(),
        fields: HashMap::from([("secret".to_string(), Value::String(hex::encode(secret)))]),
    };
    if !system_db.create_document(&doc)? {
        anyhow::bail!("failed to persist generated JWT secret");
    }
    Ok(secret.to_vec())
}

#[cfg(feature = "server")]
async fn shutdown_signal(quiet: bool) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            if !quiet {
                println!();  // newline after ^C
            }
            tracing::debug!("received Ctrl+C");
        }
        _ = terminate => {
            tracing::debug!("received SIGTERM");
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn make_secret_file_private(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[cfg(not(unix))]
    fn make_secret_file_private(_path: &std::path::Path) {}

    #[cfg(feature = "ui")]
    #[test]
    fn banner_includes_station_url_when_ui_is_enabled() {
        let lines = banner_info_lines(3000, None, "127.0.0.1", "./data", 2, false);

        assert_eq!(
            lines.last().unwrap(),
            "Station: http://127.0.0.1:3000/_station/"
        );
    }

    #[cfg(feature = "ui")]
    #[test]
    fn banner_uses_https_for_station_when_tls_is_enabled() {
        let lines = banner_info_lines(3443, None, "db.example.com", "./data", 1, true);

        assert_eq!(
            lines.last().unwrap(),
            "Station: https://db.example.com:3443/_station/"
        );
    }

    #[test]
    fn tls_files_are_required_together() {
        let error = resolve_tls_config(Some("certificate.pem"), None).unwrap_err();
        assert!(error.to_string().contains("must be provided together"));
        let error = resolve_tls_config(None, Some("private-key.pem")).unwrap_err();
        assert!(error.to_string().contains("must be provided together"));
    }

    #[test]
    fn remote_bind_requires_explicit_insecure_override() {
        assert!(validate_bind_security("127.0.0.1", false, false).is_ok());
        assert!(validate_bind_security("localhost", false, false).is_ok());
        assert!(validate_bind_security("::1", false, false).is_ok());
        assert!(validate_bind_security("0.0.0.0", false, false).is_err());
        assert!(validate_bind_security("::", false, false).is_err());
        assert!(validate_bind_security("192.168.1.10", false, false).is_err());
        assert!(validate_bind_security("0.0.0.0", true, false).is_ok());
        assert!(validate_bind_security("0.0.0.0", false, true).is_ok());
    }

    #[test]
    fn remote_insecure_browser_contract_is_explicit() {
        assert!(resolve_cors_config("127.0.0.1", true, false, false, None).is_ok());
        assert!(resolve_cors_config("0.0.0.0", true, false, true, None).is_ok());
        assert!(
            resolve_cors_config(
                "0.0.0.0",
                true,
                false,
                false,
                Some("https://app.example.com"),
            )
            .is_ok()
        );
        assert!(resolve_cors_config("0.0.0.0", true, false, false, Some("*")).is_ok());
        assert!(resolve_cors_config("0.0.0.0", true, false, false, None).is_err());
        assert!(resolve_cors_config("0.0.0.0", true, false, false, Some("not-an-origin")).is_err());
        assert!(resolve_cors_config("0.0.0.0", false, true, false, None).is_ok());
        assert!(
            resolve_cors_config(
                "127.0.0.1",
                false,
                false,
                true,
                Some("https://app.example.com"),
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn startup_rejects_remote_bind_before_opening_data() {
        let err = run(
            3000,
            None,
            16 * 1024,
            10 * 1024 * 1024,
            4 * 1024 * 1024,
            64,
            128,
            "0.0.0.0".to_string(),
            false,
            None,
            None,
            false,
            None,
            "./must-not-be-created-by-test".to_string(),
            0,
            true,
            "30s".to_string(),
            None,
            None,
            3600,
            None,
            None,
            64,
            "60s".to_string(),
            4_096,
            64 * 1024 * 1024,
            0,
            4,
            10.0,
            20,
            5.0,
            10,
            10_000,
            "10m".to_string(),
            0.0,
            20,
            10_000,
            "10m".to_string(),
            0,
            10_000,
            7,
            4_096,
            7,
            4_096,
            7,
        )
        .await
        .expect_err("remote bind must require an explicit insecure override");
        assert!(err.to_string().contains("--insecure-allow-remote"));
        assert!(!std::path::Path::new("./must-not-be-created-by-test").exists());
    }

    #[tokio::test]
    async fn startup_rejects_non_browser_cors_conflict_before_opening_data() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("must-not-exist");
        let error = run(
            3000,
            None,
            16 * 1024,
            10 * 1024 * 1024,
            4 * 1024 * 1024,
            64,
            128,
            "127.0.0.1".to_string(),
            false,
            None,
            None,
            true,
            Some("https://app.example.com".to_string()),
            data.to_string_lossy().into_owned(),
            0,
            true,
            "30s".to_string(),
            None,
            None,
            3600,
            None,
            None,
            64,
            "60s".to_string(),
            4_096,
            64 * 1024 * 1024,
            0,
            4,
            10.0,
            20,
            5.0,
            10,
            10_000,
            "10m".to_string(),
            0.0,
            20,
            10_000,
            "10m".to_string(),
            0,
            10_000,
            7,
            4_096,
            7,
            4_096,
            7,
        )
        .await
        .expect_err("non-browser mode and CORS allowlist must conflict");
        assert!(error.to_string().contains("conflicts"));
        assert!(!data.exists());
    }

    #[test]
    fn jwt_validation_rejects_weak_secrets_and_invalid_ttls() {
        assert!(validate_explicit_jwt_secret(b"").is_err());
        assert!(validate_explicit_jwt_secret(b"too-short").is_err());
        assert!(validate_explicit_jwt_secret(&[b'x'; MIN_JWT_SECRET_BYTES]).is_ok());
        assert!(validate_jwt_ttl(MIN_JWT_TTL_SECS - 1).is_err());
        assert!(validate_jwt_ttl(MIN_JWT_TTL_SECS).is_ok());
        assert!(validate_jwt_ttl(MAX_JWT_TTL_SECS).is_ok());
        assert!(validate_jwt_ttl(MAX_JWT_TTL_SECS + 1).is_err());
    }

    #[test]
    fn jwt_secret_file_is_read_without_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jwt-secret");
        std::fs::write(&path, format!("{}\n", "x".repeat(MIN_JWT_SECRET_BYTES))).unwrap();
        make_secret_file_private(&path);
        let secret = resolve_explicit_jwt_secret(None, path.to_str())
            .unwrap()
            .unwrap();
        assert_eq!(secret, vec![b'x'; MIN_JWT_SECRET_BYTES]);
    }

    #[test]
    fn jwt_secret_file_rejects_unsafe_inputs() {
        let dir = tempfile::tempdir().unwrap();

        let multiline = dir.path().join("multiline");
        std::fs::write(
            &multiline,
            format!("{}\n\n", "x".repeat(MIN_JWT_SECRET_BYTES)),
        )
        .unwrap();
        make_secret_file_private(&multiline);
        assert!(resolve_explicit_jwt_secret(None, multiline.to_str()).is_err());

        let oversized = dir.path().join("oversized");
        std::fs::write(
            &oversized,
            vec![b'x'; MAX_JWT_SECRET_FILE_BYTES as usize + 1],
        )
        .unwrap();
        make_secret_file_private(&oversized);
        assert!(resolve_explicit_jwt_secret(None, oversized.to_str()).is_err());

        assert!(resolve_explicit_jwt_secret(None, dir.path().to_str()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn jwt_secret_file_rejects_public_permissions_and_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::write(&target, vec![b'x'; MIN_JWT_SECRET_BYTES]).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(resolve_explicit_jwt_secret(None, target.to_str()).is_err());

        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join("link");
        symlink(&target, &link).unwrap();
        assert!(resolve_explicit_jwt_secret(None, link.to_str()).is_err());
    }

    #[test]
    fn generated_jwt_secret_is_durable_and_weak_stored_secret_fails() {
        use std::collections::HashMap;
        use stellardb::document::{Document, Value};

        let generated_dir = tempfile::tempdir().unwrap();
        let generated_db = stellardb::storage::Database::open(generated_dir.path()).unwrap();
        let first = load_or_generate_jwt_secret(&generated_db).unwrap();
        let second = load_or_generate_jwt_secret(&generated_db).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), MIN_JWT_SECRET_BYTES);

        let legacy_dir = tempfile::tempdir().unwrap();
        let legacy_db = stellardb::storage::Database::open(legacy_dir.path()).unwrap();
        let weak = b"legacy-weak-secret";
        let doc = Document {
            id: "config:jwt_secret".to_string(),
            fields: HashMap::from([("secret".to_string(), Value::String(hex::encode(weak)))]),
        };
        assert!(legacy_db.create_document(&doc).unwrap());
        assert!(load_or_generate_jwt_secret(&legacy_db).is_err());

        let invalid_dir = tempfile::tempdir().unwrap();
        let invalid_db = stellardb::storage::Database::open(invalid_dir.path()).unwrap();
        let invalid = Document {
            id: "config:jwt_secret".to_string(),
            fields: HashMap::from([("secret".to_string(), Value::String(String::new()))]),
        };
        assert!(invalid_db.create_document(&invalid).unwrap());
        assert!(load_or_generate_jwt_secret(&invalid_db).is_err());
    }

    #[tokio::test]
    async fn startup_rejects_invalid_jwt_before_opening_data() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("must-not-exist");
        let error = run(
            3000,
            None,
            16 * 1024,
            10 * 1024 * 1024,
            4 * 1024 * 1024,
            64,
            128,
            "127.0.0.1".to_string(),
            false,
            None,
            None,
            false,
            None,
            data.to_string_lossy().into_owned(),
            0,
            true,
            "30s".to_string(),
            Some("weak".to_string()),
            None,
            3600,
            None,
            None,
            64,
            "60s".to_string(),
            4_096,
            64 * 1024 * 1024,
            0,
            4,
            10.0,
            20,
            5.0,
            10,
            10_000,
            "10m".to_string(),
            0.0,
            20,
            10_000,
            "10m".to_string(),
            0,
            10_000,
            7,
            4_096,
            7,
            4_096,
            7,
        )
        .await
        .expect_err("weak secret must fail startup");
        assert!(error.to_string().contains("at least 32 bytes"));
        assert!(!error.to_string().contains("weak"));
        assert!(!data.exists());
    }

    #[test]
    fn admission_config_accepts_public_defaults_and_rejects_invalid_values() {
        let build = |max_db_concurrency,
                     db_deadline,
                     max_login_concurrency,
                     login_rate_per_second,
                     login_burst,
                     login_client_rate_per_second,
                     login_client_burst,
                     max_login_clients,
                     login_client_retention| {
            build_admission_config(
                max_db_concurrency,
                db_deadline,
                max_login_concurrency,
                login_rate_per_second,
                login_burst,
                login_client_rate_per_second,
                login_client_burst,
                max_login_clients,
                login_client_retention,
                0.0,
                20,
                10_000,
                "10m",
            )
        };
        let config = build(64, "60s", 4, 10.0, 20, 5.0, 10, 10_000, "10m").unwrap();
        assert_eq!(config.max_db_concurrency, 64);
        assert_eq!(config.db_deadline, std::time::Duration::from_secs(60));
        assert_eq!(config.max_login_concurrency, 4);
        assert_eq!(config.login_rate_per_second, 10.0);
        assert_eq!(config.login_burst, 20);
        assert_eq!(config.login_client_rate_per_second, 5.0);
        assert_eq!(config.login_client_burst, 10);
        assert_eq!(config.max_login_clients, 10_000);
        assert_eq!(
            config.login_client_retention,
            std::time::Duration::from_secs(600)
        );
        assert_eq!(config.query_client_rate_per_second, 0.0);

        assert!(build(0, "60s", 4, 10.0, 20, 5.0, 10, 10_000, "10m").is_err());
        assert!(build(64, "0", 4, 10.0, 20, 5.0, 10, 10_000, "10m").is_err());
        assert!(build(64, "60s", 0, 10.0, 20, 5.0, 10, 10_000, "10m").is_err());
        assert!(build(64, "60s", 4, 0.0, 20, 5.0, 10, 10_000, "10m").is_err());
        assert!(build(64, "60s", 4, f64::NAN, 20, 5.0, 10, 10_000, "10m").is_err());
        assert!(build(64, "60s", 4, 10.0, 0, 5.0, 10, 10_000, "10m").is_err());
        assert!(build(64, "60s", 4, 10.0, 20, 0.0, 10, 10_000, "10m").is_err());
        assert!(build(64, "60s", 4, 10.0, 20, 5.0, 0, 10_000, "10m").is_err());
        assert!(build(64, "60s", 4, 10.0, 20, 5.0, 10, 1_000_001, "10m").is_err());
        assert!(build(64, "60s", 4, 10.0, 20, 5.0, 10, 10_000, "0").is_err());
        assert!(
            build_admission_config(
                64, "60s", 4, 10.0, 20, 5.0, 10, 10_000, "10m", -1.0, 20, 10_000, "10m"
            )
            .is_err()
        );
    }

    #[test]
    fn transport_limits_validate_and_connect_grpc_settings() {
        let limits =
            build_transport_limits(16 * 1024, 10 * 1024 * 1024, 4 * 1024 * 1024, 64, 128).unwrap();
        assert_eq!(limits.grpc_max_message_size, 4 * 1024 * 1024);
        assert_eq!(limits.grpc_concurrency_limit, 64);
        assert_eq!(limits.grpc_max_concurrent_streams, 128);
        assert!(build_transport_limits(512, 4096, 4096, 1, 1).is_err());
        assert!(build_transport_limits(8192, 4096, 4096, 1, 1).is_err());
        assert!(build_transport_limits(1024, 4096, 512, 1, 1).is_err());
        assert!(build_transport_limits(1024, 4096, 4096, 0, 1).is_err());
        assert!(build_transport_limits(1024, 4096, 4096, 1, 0).is_err());
    }

    #[test]
    fn metrics_config_validates_and_connects_all_bounds() {
        let config = build_metrics_config(10, 30, 20, 14, 30, 21).unwrap();
        assert_eq!(config.max_query_stats, 10);
        assert_eq!(config.retention_days, 30);
        assert_eq!(config.max_index_stats, 20);
        assert_eq!(config.index_retention_days, 14);
        assert_eq!(config.max_collection_stats, 30);
        assert_eq!(config.collection_retention_days, 21);
        assert!(build_metrics_config(1_000_001, 7, 1, 7, 1, 7).is_err());
        assert!(build_metrics_config(1, 0, 1, 7, 1, 7).is_err());
        assert!(build_metrics_config(1, 7, 1, 3_651, 1, 7).is_err());
    }

    #[test]
    fn remote_http_warning_is_only_for_non_loopback_binds() {
        assert!(remote_http_warning("127.0.0.1", false).is_none());
        assert!(remote_http_warning("localhost", false).is_none());
        assert!(remote_http_warning("::1", false).is_none());
        assert!(remote_http_warning("0.0.0.0", true).is_none());

        let warning = remote_http_warning("0.0.0.0", false).expect("public bind should warn");
        assert!(warning.contains("HTTPS reverse proxy"));
        assert!(remote_http_warning("::", false).is_some());
        assert!(remote_http_warning("192.168.1.10", false).is_some());
    }
}

#[cfg(all(test, not(feature = "server")))]
mod disabled_tests {
    use super::*;

    #[tokio::test]
    async fn serve_reports_disabled_server_feature() {
        let err = run(
            3000,
            None,
            16 * 1024,
            10 * 1024 * 1024,
            4 * 1024 * 1024,
            64,
            128,
            "127.0.0.1".to_string(),
            false,
            None,
            None,
            false,
            None,
            "./data".to_string(),
            0,
            true,
            "30s".to_string(),
            None,
            None,
            3600,
            64,
            "60s".to_string(),
            4,
            10.0,
            20,
            5.0,
            10,
            10_000,
            "10m".to_string(),
            0,
            10_000,
            7,
            4_096,
            7,
            4_096,
            7,
        )
        .await
        .expect_err("serve should be unavailable without the server feature");

        assert!(err.to_string().contains("--features server"));
    }
}
