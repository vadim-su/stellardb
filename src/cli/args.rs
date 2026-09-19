use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "stellar")]
#[command(about = "StellarDB - multi-model database", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum Commands {
    /// Start the database server
    Serve {
        /// Port to listen on (HTTP)
        #[arg(short, long, default_value = "3000")]
        port: u16,

        /// gRPC port (optional, gRPC disabled if not set)
        #[arg(short = 'g', long)]
        grpc_port: Option<u16>,

        /// Maximum request body size for /auth/login, in bytes
        #[arg(long, env = "STELLARDB_LOGIN_BODY_LIMIT", default_value_t = 16 * 1024)]
        login_body_limit: usize,

        /// Maximum HTTP request body size, in bytes
        #[arg(long, env = "STELLARDB_HTTP_BODY_LIMIT", default_value_t = 10 * 1024 * 1024)]
        http_body_limit: usize,

        /// Maximum encoded or decoded gRPC message size, in bytes
        #[arg(long, env = "STELLARDB_GRPC_MAX_MESSAGE_SIZE", default_value_t = 4 * 1024 * 1024)]
        grpc_max_message_size: usize,

        /// Maximum concurrent gRPC requests per HTTP/2 connection
        #[arg(long, env = "STELLARDB_GRPC_CONCURRENCY_LIMIT", default_value_t = 64)]
        grpc_concurrency_limit: usize,

        /// Maximum concurrent HTTP/2 streams per gRPC connection
        #[arg(
            long,
            env = "STELLARDB_GRPC_MAX_CONCURRENT_STREAMS",
            default_value_t = 128
        )]
        grpc_max_concurrent_streams: u32,

        /// Host address to bind to
        #[arg(short = 'H', long, default_value = "127.0.0.1")]
        host: String,

        /// Allow plain HTTP/gRPC binding to a non-loopback interface.
        /// Use only behind a trusted TLS-terminating reverse proxy or in an isolated network.
        #[arg(long)]
        insecure_allow_remote: bool,

        /// PEM certificate chain used by both HTTP and optional gRPC TLS listeners.
        /// Must be provided together with --tls-key-file.
        #[arg(long, env = "STELLARDB_TLS_CERT_FILE")]
        tls_cert_file: Option<String>,

        /// PEM private key used by both HTTP and optional gRPC TLS listeners.
        /// The file must contain one unencrypted PKCS#8, PKCS#1, or SEC1 key.
        #[arg(long, env = "STELLARDB_TLS_KEY_FILE")]
        tls_key_file: Option<String>,

        /// Declare that a remote plain-HTTP deployment has no browser clients.
        /// Required when remote insecure mode has no explicit CORS allowlist.
        #[arg(long, env = "STELLARDB_NON_BROWSER_API")]
        non_browser_api: bool,

        /// Data directory
        #[arg(short, long, default_value = "./data")]
        data: String,

        /// Increase logging verbosity (-v, -vv, -vvv)
        #[arg(short, long, action = clap::ArgAction::Count)]
        verbose: u8,

        /// Decrease logging (only errors, no banner)
        #[arg(short, long)]
        quiet: bool,

        /// Default query timeout (e.g. 30s, 1m, 0 to disable)
        #[arg(long, default_value = "30s")]
        query_timeout: String,

        /// JWT signing secret. Prefer --jwt-secret-file to avoid exposing it in process arguments.
        #[arg(long, env = "STELLAR_JWT_SECRET", conflicts_with = "jwt_secret_file")]
        jwt_secret: Option<String>,

        /// File containing the JWT signing secret.
        #[arg(long, env = "STELLAR_JWT_SECRET_FILE", conflicts_with = "jwt_secret")]
        jwt_secret_file: Option<String>,

        /// JWT token lifetime in seconds (60..=31536000; default: 3600).
        #[arg(long, env = "STELLAR_JWT_TTL", default_value = "3600")]
        jwt_ttl: u64,

        /// Managed user that requests without credentials are resolved to.
        /// Its policies decide what anonymous clients may do; cannot be 'root'.
        #[arg(long, env = "STELLAR_ANONYMOUS_USER")]
        anonymous_user: Option<String>,

        /// JSON file with example queries per database, shown in Station's query workspace.
        #[arg(long, env = "STELLAR_STATION_EXAMPLES")]
        station_examples: Option<String>,

        /// Maximum number of concurrent blocking database operations
        #[arg(long, env = "STELLARDB_MAX_DB_CONCURRENCY", default_value_t = 64)]
        max_db_concurrency: usize,

        /// Hard DB deadline; caps query timeouts, while unfinished DDL becomes a tracked operation
        #[arg(long, env = "STELLARDB_DB_DEADLINE", default_value = "60s")]
        db_deadline: String,

        /// Documents prepared per B-tree backfill chunk
        #[arg(
            long,
            env = "STELLARDB_INDEX_BUILD_CHUNK_SIZE",
            default_value_t = 4_096
        )]
        index_build_chunk_size: usize,

        /// Memory budget in bytes for one prepared B-tree backfill batch
        #[arg(
            long,
            env = "STELLARDB_INDEX_BUILD_MEMORY_BUDGET",
            default_value_t = 64 * 1024 * 1024
        )]
        index_build_memory_budget: usize,

        /// CPU workers for B-tree backfill preparation (0 selects an automatic limit)
        #[arg(long, env = "STELLARDB_INDEX_BUILD_WORKERS", default_value_t = 0)]
        index_build_workers: usize,

        /// Maximum number of concurrent password verifications
        #[arg(long, env = "STELLARDB_MAX_LOGIN_CONCURRENCY", default_value_t = 4)]
        max_login_concurrency: usize,

        /// Process-wide login attempts allowed per second
        #[arg(long, env = "STELLARDB_LOGIN_RATE_PER_SECOND", default_value_t = 10.0)]
        login_rate_per_second: f64,

        /// Maximum burst size for the process-wide login rate limiter
        #[arg(long, env = "STELLARDB_LOGIN_BURST", default_value_t = 20)]
        login_burst: usize,

        /// Login attempts allowed per second for one resolved client IP
        #[arg(
            long,
            env = "STELLARDB_LOGIN_CLIENT_RATE_PER_SECOND",
            default_value_t = 5.0
        )]
        login_client_rate_per_second: f64,

        /// Maximum burst for one resolved client IP
        #[arg(long, env = "STELLARDB_LOGIN_CLIENT_BURST", default_value_t = 10)]
        login_client_burst: usize,

        /// Maximum client IP buckets retained (0 disables per-client limiting)
        #[arg(long, env = "STELLARDB_MAX_LOGIN_CLIENTS", default_value_t = 10_000)]
        max_login_clients: usize,

        /// Inactivity retention for per-client login buckets (e.g. 10m, 1h)
        #[arg(long, env = "STELLARDB_LOGIN_CLIENT_RETENTION", default_value = "10m")]
        login_client_retention: String,

        /// Queries allowed per second for one resolved client IP (0 disables)
        #[arg(
            long,
            env = "STELLARDB_QUERY_CLIENT_RATE_PER_SECOND",
            default_value_t = 0.0
        )]
        query_client_rate_per_second: f64,

        /// Maximum query burst for one resolved client IP
        #[arg(long, env = "STELLARDB_QUERY_CLIENT_BURST", default_value_t = 20)]
        query_client_burst: usize,

        /// Maximum query client IP buckets retained (0 disables per-client limiting)
        #[arg(long, env = "STELLARDB_MAX_QUERY_CLIENTS", default_value_t = 10_000)]
        max_query_clients: usize,

        /// Inactivity retention for per-client query buckets (e.g. 10m, 1h)
        #[arg(long, env = "STELLARDB_QUERY_CLIENT_RETENTION", default_value = "10m")]
        query_client_retention: String,

        /// Trusted proxy hops at the right side of X-Forwarded-For (0 ignores the header)
        #[arg(long, env = "STELLARDB_TRUSTED_PROXY_HOPS", default_value_t = 0)]
        trusted_proxy_hops: usize,

        /// Maximum retained normalized query metric entries (0 disables per-query entries)
        #[arg(
            long,
            env = "STELLARDB_METRICS_MAX_QUERY_STATS",
            default_value_t = 10_000
        )]
        metrics_max_query_stats: usize,

        /// Query metric inactivity retention in days
        #[arg(
            long,
            env = "STELLARDB_METRICS_QUERY_RETENTION_DAYS",
            default_value_t = 7
        )]
        metrics_query_retention_days: u64,

        /// Maximum retained index metric entries (0 disables per-index entries)
        #[arg(
            long,
            env = "STELLARDB_METRICS_MAX_INDEX_STATS",
            default_value_t = 4_096
        )]
        metrics_max_index_stats: usize,

        /// Index metric inactivity retention in days
        #[arg(
            long,
            env = "STELLARDB_METRICS_INDEX_RETENTION_DAYS",
            default_value_t = 7
        )]
        metrics_index_retention_days: u64,

        /// Maximum retained collection metric entries (0 disables per-collection entries)
        #[arg(
            long,
            env = "STELLARDB_METRICS_MAX_COLLECTION_STATS",
            default_value_t = 4_096
        )]
        metrics_max_collection_stats: usize,

        /// Collection metric inactivity retention in days
        #[arg(
            long,
            env = "STELLARDB_METRICS_COLLECTION_RETENTION_DAYS",
            default_value_t = 7
        )]
        metrics_collection_retention_days: u64,
    },

    /// Execute a StellarQL query
    Sql {
        /// The query to execute
        query: String,

        /// Server endpoint (host:port, http://..., or https://...)
        #[arg(long, default_value = "localhost:3000")]
        host: String,

        /// Target database name
        #[arg(short = 'D', long, default_value = "default")]
        database: String,

        /// Force plain JSON output (no colors)
        #[arg(long)]
        json: bool,

        /// Force pretty output (even in pipe)
        #[arg(long)]
        pretty: bool,
    },

    /// Manage collection schemas
    Schema {
        #[command(subcommand)]
        action: SchemaAction,
    },

    /// Back up a StellarDB data directory
    Backup {
        /// Data directory to back up
        #[arg(short, long, default_value = "./data")]
        data: String,

        /// Backup output directory; must not already contain files
        #[arg(short, long)]
        output: String,
    },

    /// Restore a StellarDB backup into a new data directory
    Restore {
        /// Backup input directory
        #[arg(short, long)]
        input: String,

        /// Restore destination data directory; must be missing or empty
        #[arg(short, long, default_value = "./data")]
        data: String,
    },
}

#[derive(Subcommand)]
pub enum SchemaAction {
    /// Show schema for a collection
    Show {
        /// Collection name
        collection: String,

        /// Server endpoint (host:port, http://..., or https://...)
        #[arg(long, default_value = "localhost:3000")]
        host: String,

        /// Target database name
        #[arg(short = 'D', long, default_value = "default")]
        database: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List all collections
    List {
        /// Server endpoint (host:port, http://..., or https://...)
        #[arg(long, default_value = "localhost:3000")]
        host: String,

        /// Target database name
        #[arg(short = 'D', long, default_value = "default")]
        database: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Apply schema files to database
    Apply {
        /// Path to schema directory or file
        #[arg(short, long)]
        path: String,

        /// Server endpoint (host:port, http://..., or https://...)
        #[arg(long, default_value = "localhost:3000")]
        host: String,

        /// Target database name
        #[arg(short = 'D', long, default_value = "default")]
        database: String,

        /// Show diff without applying
        #[arg(long)]
        dry_run: bool,
    },

    /// Export schemas from database to files
    Export {
        /// Output directory
        #[arg(short, long)]
        path: String,

        /// Server endpoint (host:port, http://..., or https://...)
        #[arg(long, default_value = "localhost:3000")]
        host: String,

        /// Target database name
        #[arg(short = 'D', long, default_value = "default")]
        database: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn transport_limit_defaults_and_overrides_are_parsed() {
        let cli = Cli::parse_from(["stellar", "serve"]);
        match cli.command {
            Commands::Serve {
                login_body_limit,
                http_body_limit,
                grpc_max_message_size,
                grpc_concurrency_limit,
                grpc_max_concurrent_streams,
                ..
            } => {
                assert_eq!(login_body_limit, 16 * 1024);
                assert_eq!(http_body_limit, 10 * 1024 * 1024);
                assert_eq!(grpc_max_message_size, 4 * 1024 * 1024);
                assert_eq!(grpc_concurrency_limit, 64);
                assert_eq!(grpc_max_concurrent_streams, 128);
            }
            _ => panic!("expected Serve command"),
        }

        let cli = Cli::parse_from([
            "stellar",
            "serve",
            "--login-body-limit",
            "4096",
            "--http-body-limit",
            "8192",
            "--grpc-max-message-size",
            "16384",
            "--grpc-concurrency-limit",
            "32",
            "--grpc-max-concurrent-streams",
            "48",
        ]);
        match cli.command {
            Commands::Serve {
                login_body_limit,
                http_body_limit,
                grpc_max_message_size,
                grpc_concurrency_limit,
                grpc_max_concurrent_streams,
                ..
            } => {
                assert_eq!(login_body_limit, 4096);
                assert_eq!(http_body_limit, 8192);
                assert_eq!(grpc_max_message_size, 16384);
                assert_eq!(grpc_concurrency_limit, 32);
                assert_eq!(grpc_max_concurrent_streams, 48);
            }
            _ => panic!("expected Serve command"),
        }
    }

    #[test]
    fn test_serve_host_default() {
        let cli = Cli::parse_from(["stellar", "serve"]);
        match cli.command {
            Commands::Serve { host, .. } => assert_eq!(host, "127.0.0.1"),
            _ => panic!("expected Serve command"),
        }
    }

    #[test]
    fn test_serve_host_long() {
        let cli = Cli::parse_from(["stellar", "serve", "--host", "0.0.0.0"]);
        match cli.command {
            Commands::Serve { host, .. } => assert_eq!(host, "0.0.0.0"),
            _ => panic!("expected Serve command"),
        }
    }

    #[test]
    fn test_serve_host_short() {
        let cli = Cli::parse_from(["stellar", "serve", "-H", "192.168.1.1"]);
        match cli.command {
            Commands::Serve { host, .. } => assert_eq!(host, "192.168.1.1"),
            _ => panic!("expected Serve command"),
        }
    }

    #[test]
    fn test_insecure_remote_flag_defaults_off_and_can_be_enabled() {
        let cli = Cli::parse_from(["stellar", "serve"]);
        match cli.command {
            Commands::Serve {
                insecure_allow_remote,
                ..
            } => assert!(!insecure_allow_remote),
            _ => panic!("expected Serve command"),
        }

        let cli = Cli::parse_from(["stellar", "serve", "--insecure-allow-remote"]);
        match cli.command {
            Commands::Serve {
                insecure_allow_remote,
                ..
            } => assert!(insecure_allow_remote),
            _ => panic!("expected Serve command"),
        }
    }

    #[test]
    fn tls_file_flags_and_environment_contract_are_exposed() {
        use clap::CommandFactory;

        let cli = Cli::parse_from([
            "stellar",
            "serve",
            "--tls-cert-file",
            "certificate.pem",
            "--tls-key-file",
            "private-key.pem",
        ]);
        match cli.command {
            Commands::Serve {
                tls_cert_file,
                tls_key_file,
                ..
            } => {
                assert_eq!(tls_cert_file.as_deref(), Some("certificate.pem"));
                assert_eq!(tls_key_file.as_deref(), Some("private-key.pem"));
            }
            _ => panic!("expected Serve command"),
        }

        let command = Cli::command();
        let serve = command
            .get_subcommands()
            .find(|command| command.get_name() == "serve")
            .expect("serve command");
        let cert = serve
            .get_arguments()
            .find(|argument| argument.get_id() == "tls_cert_file")
            .expect("TLS certificate argument");
        let key = serve
            .get_arguments()
            .find(|argument| argument.get_id() == "tls_key_file")
            .expect("TLS key argument");
        assert_eq!(
            cert.get_env().and_then(|value| value.to_str()),
            Some("STELLARDB_TLS_CERT_FILE")
        );
        assert_eq!(
            key.get_env().and_then(|value| value.to_str()),
            Some("STELLARDB_TLS_KEY_FILE")
        );
    }

    #[test]
    fn jwt_secret_and_secret_file_conflict() {
        let error = match Cli::try_parse_from([
            "stellar",
            "serve",
            "--jwt-secret",
            "01234567890123456789012345678901",
            "--jwt-secret-file",
            "secret.txt",
        ]) {
            Ok(_) => panic!("secret sources must be mutually exclusive"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn jwt_secret_file_and_ttl_are_parsed() {
        let cli = Cli::parse_from([
            "stellar",
            "serve",
            "--jwt-secret-file",
            "secret.txt",
            "--jwt-ttl",
            "7200",
        ]);
        match cli.command {
            Commands::Serve {
                jwt_secret,
                jwt_secret_file,
                jwt_ttl,
                ..
            } => {
                assert!(jwt_secret.is_none());
                assert_eq!(jwt_secret_file.as_deref(), Some("secret.txt"));
                assert_eq!(jwt_ttl, 7200);
            }
            _ => panic!("expected Serve command"),
        }
    }

    #[test]
    fn test_admission_defaults_and_cli_overrides() {
        let cli = Cli::parse_from(["stellar", "serve"]);
        match cli.command {
            Commands::Serve {
                max_db_concurrency,
                db_deadline,
                max_login_concurrency,
                login_rate_per_second,
                login_burst,
                ..
            } => {
                assert_eq!(max_db_concurrency, 64);
                assert_eq!(db_deadline, "60s");
                assert_eq!(max_login_concurrency, 4);
                assert_eq!(login_rate_per_second, 10.0);
                assert_eq!(login_burst, 20);
            }
            _ => panic!("expected Serve command"),
        }

        let cli = Cli::parse_from([
            "stellar",
            "serve",
            "--max-db-concurrency",
            "128",
            "--db-deadline",
            "2m",
            "--max-login-concurrency",
            "8",
            "--login-rate-per-second",
            "25.5",
            "--login-burst",
            "50",
        ]);
        match cli.command {
            Commands::Serve {
                max_db_concurrency,
                db_deadline,
                max_login_concurrency,
                login_rate_per_second,
                login_burst,
                ..
            } => {
                assert_eq!(max_db_concurrency, 128);
                assert_eq!(db_deadline, "2m");
                assert_eq!(max_login_concurrency, 8);
                assert_eq!(login_rate_per_second, 25.5);
                assert_eq!(login_burst, 50);
            }
            _ => panic!("expected Serve command"),
        }
    }

    #[test]
    fn per_client_limits_and_proxy_defaults_and_overrides_are_parsed() {
        let cli = Cli::parse_from(["stellar", "serve"]);
        match cli.command {
            Commands::Serve {
                login_client_rate_per_second,
                login_client_burst,
                max_login_clients,
                login_client_retention,
                query_client_rate_per_second,
                query_client_burst,
                max_query_clients,
                query_client_retention,
                trusted_proxy_hops,
                ..
            } => {
                assert_eq!(login_client_rate_per_second, 5.0);
                assert_eq!(login_client_burst, 10);
                assert_eq!(max_login_clients, 10_000);
                assert_eq!(login_client_retention, "10m");
                assert_eq!(query_client_rate_per_second, 0.0);
                assert_eq!(query_client_burst, 20);
                assert_eq!(max_query_clients, 10_000);
                assert_eq!(query_client_retention, "10m");
                assert_eq!(trusted_proxy_hops, 0);
            }
            _ => panic!("expected Serve command"),
        }

        let cli = Cli::parse_from([
            "stellar",
            "serve",
            "--login-client-rate-per-second",
            "12.5",
            "--login-client-burst",
            "25",
            "--max-login-clients",
            "5000",
            "--login-client-retention",
            "1h",
            "--query-client-rate-per-second",
            "12.5",
            "--query-client-burst",
            "30",
            "--max-query-clients",
            "6000",
            "--query-client-retention",
            "30m",
            "--trusted-proxy-hops",
            "2",
        ]);
        match cli.command {
            Commands::Serve {
                login_client_rate_per_second,
                login_client_burst,
                max_login_clients,
                login_client_retention,
                query_client_rate_per_second,
                query_client_burst,
                max_query_clients,
                query_client_retention,
                trusted_proxy_hops,
                ..
            } => {
                assert_eq!(login_client_rate_per_second, 12.5);
                assert_eq!(login_client_burst, 25);
                assert_eq!(max_login_clients, 5_000);
                assert_eq!(login_client_retention, "1h");
                assert_eq!(query_client_rate_per_second, 12.5);
                assert_eq!(query_client_burst, 30);
                assert_eq!(max_query_clients, 6_000);
                assert_eq!(query_client_retention, "30m");
                assert_eq!(trusted_proxy_hops, 2);
            }
            _ => panic!("expected Serve command"),
        }
    }

    #[test]
    fn metrics_bounds_defaults_and_overrides_are_parsed() {
        let cli = Cli::parse_from(["stellar", "serve"]);
        match cli.command {
            Commands::Serve {
                metrics_max_query_stats,
                metrics_query_retention_days,
                metrics_max_index_stats,
                metrics_index_retention_days,
                metrics_max_collection_stats,
                metrics_collection_retention_days,
                ..
            } => {
                assert_eq!(metrics_max_query_stats, 10_000);
                assert_eq!(metrics_query_retention_days, 7);
                assert_eq!(metrics_max_index_stats, 4_096);
                assert_eq!(metrics_index_retention_days, 7);
                assert_eq!(metrics_max_collection_stats, 4_096);
                assert_eq!(metrics_collection_retention_days, 7);
            }
            _ => panic!("expected Serve command"),
        }

        let cli = Cli::parse_from([
            "stellar",
            "serve",
            "--metrics-max-query-stats",
            "20",
            "--metrics-query-retention-days",
            "30",
            "--metrics-max-index-stats",
            "10",
            "--metrics-index-retention-days",
            "14",
            "--metrics-max-collection-stats",
            "12",
            "--metrics-collection-retention-days",
            "21",
        ]);
        match cli.command {
            Commands::Serve {
                metrics_max_query_stats,
                metrics_query_retention_days,
                metrics_max_index_stats,
                metrics_index_retention_days,
                metrics_max_collection_stats,
                metrics_collection_retention_days,
                ..
            } => {
                assert_eq!(metrics_max_query_stats, 20);
                assert_eq!(metrics_query_retention_days, 30);
                assert_eq!(metrics_max_index_stats, 10);
                assert_eq!(metrics_index_retention_days, 14);
                assert_eq!(metrics_max_collection_stats, 12);
                assert_eq!(metrics_collection_retention_days, 21);
            }
            _ => panic!("expected Serve command"),
        }
    }

    #[test]
    fn test_sql_database_flag_default() {
        let cli = Cli::parse_from(["stellar", "sql", "SELECT 1"]);
        match cli.command {
            Commands::Sql { database, .. } => assert_eq!(database, "default"),
            _ => panic!("expected Sql command"),
        }
    }

    #[test]
    fn test_sql_database_flag_long() {
        let cli = Cli::parse_from(["stellar", "sql", "--database", "mydb", "SELECT 1"]);
        match cli.command {
            Commands::Sql { database, .. } => assert_eq!(database, "mydb"),
            _ => panic!("expected Sql command"),
        }
    }

    #[test]
    fn test_sql_database_flag_short() {
        let cli = Cli::parse_from(["stellar", "sql", "-D", "prod", "SELECT 1"]);
        match cli.command {
            Commands::Sql { database, .. } => assert_eq!(database, "prod"),
            _ => panic!("expected Sql command"),
        }
    }

    #[test]
    fn test_schema_show_database_flag() {
        let cli = Cli::parse_from(["stellar", "schema", "show", "-D", "testdb", "users"]);
        match cli.command {
            Commands::Schema {
                action: SchemaAction::Show { database, .. },
            } => assert_eq!(database, "testdb"),
            _ => panic!("expected Schema Show command"),
        }
    }

    #[test]
    fn test_schema_list_database_flag() {
        let cli = Cli::parse_from(["stellar", "schema", "list", "--database", "analytics"]);
        match cli.command {
            Commands::Schema {
                action: SchemaAction::List { database, .. },
            } => assert_eq!(database, "analytics"),
            _ => panic!("expected Schema List command"),
        }
    }

    #[test]
    fn test_schema_apply_database_flag() {
        let cli = Cli::parse_from([
            "stellar",
            "schema",
            "apply",
            "-p",
            "./schemas",
            "-D",
            "staging",
        ]);
        match cli.command {
            Commands::Schema {
                action: SchemaAction::Apply { database, .. },
            } => assert_eq!(database, "staging"),
            _ => panic!("expected Schema Apply command"),
        }
    }

    #[test]
    fn test_schema_export_database_flag() {
        let cli = Cli::parse_from(["stellar", "schema", "export", "-p", "./out", "-D", "backup"]);
        match cli.command {
            Commands::Schema {
                action: SchemaAction::Export { database, .. },
            } => assert_eq!(database, "backup"),
            _ => panic!("expected Schema Export command"),
        }
    }

    #[test]
    fn test_all_schema_commands_default_to_default_database() {
        let list = Cli::parse_from(["stellar", "schema", "list"]);
        match list.command {
            Commands::Schema {
                action: SchemaAction::List { database, .. },
            } => assert_eq!(database, "default"),
            _ => panic!("expected Schema List"),
        }

        let show = Cli::parse_from(["stellar", "schema", "show", "users"]);
        match show.command {
            Commands::Schema {
                action: SchemaAction::Show { database, .. },
            } => assert_eq!(database, "default"),
            _ => panic!("expected Schema Show"),
        }

        let apply = Cli::parse_from(["stellar", "schema", "apply", "-p", "./schemas"]);
        match apply.command {
            Commands::Schema {
                action: SchemaAction::Apply { database, .. },
            } => assert_eq!(database, "default"),
            _ => panic!("expected Schema Apply"),
        }

        let export = Cli::parse_from(["stellar", "schema", "export", "-p", "./out"]);
        match export.command {
            Commands::Schema {
                action: SchemaAction::Export { database, .. },
            } => assert_eq!(database, "default"),
            _ => panic!("expected Schema Export"),
        }
    }

    #[test]
    fn test_backup_command_paths() {
        let cli = Cli::parse_from([
            "stellar",
            "backup",
            "--data",
            "./data",
            "--output",
            "./backups/snapshot",
        ]);
        match cli.command {
            Commands::Backup { data, output } => {
                assert_eq!(data, "./data");
                assert_eq!(output, "./backups/snapshot");
            }
            _ => panic!("expected Backup command"),
        }
    }

    #[test]
    fn test_restore_command_paths() {
        let cli = Cli::parse_from([
            "stellar",
            "restore",
            "--input",
            "./backups/snapshot",
            "--data",
            "./restored-data",
        ]);
        match cli.command {
            Commands::Restore { input, data } => {
                assert_eq!(input, "./backups/snapshot");
                assert_eq!(data, "./restored-data");
            }
            _ => panic!("expected Restore command"),
        }
    }
}
