//! gRPC server implementation for StellarDB.
//!
//! Provides a tonic-based gRPC server that runs alongside the HTTP/axum server,
//! sharing the same `AppState` (Namespace, SessionManager, MetricsCollector).

pub mod convert;
pub mod database;
pub mod document;
pub mod edge;
pub mod interceptor;
pub mod metrics;
pub mod query;

use std::net::SocketAddr;
use std::sync::Arc;

use tonic::transport::Server;

use crate::server::AppState;

/// Re-export generated protobuf types.
pub mod proto {
    tonic::include_proto!("stellar.v1");
}

#[derive(Debug, Clone)]
pub struct GrpcConfig {
    pub max_message_size: usize,
    pub concurrency_limit_per_connection: usize,
    pub max_concurrent_streams: u32,
    #[cfg(feature = "tls")]
    pub tls: Option<crate::server::tls::TlsConfig>,
}

impl Default for GrpcConfig {
    fn default() -> Self {
        Self {
            max_message_size: 4 * 1024 * 1024,
            concurrency_limit_per_connection: 64,
            max_concurrent_streams: 128,
            #[cfg(feature = "tls")]
            tls: None,
        }
    }
}

/// gRPC server that wraps tonic and registers all service implementations.
pub struct GrpcServer {
    state: Arc<AppState>,
    config: GrpcConfig,
}

impl GrpcServer {
    /// Create a new GrpcServer with the given shared application state.
    pub fn new(state: Arc<AppState>) -> Self {
        Self::with_config(state, GrpcConfig::default())
    }

    pub fn with_config(state: Arc<AppState>, config: GrpcConfig) -> Self {
        Self { state, config }
    }

    /// Start the gRPC server on the given address.
    ///
    /// Returns a future that resolves when the server shuts down.
    /// The server registers all service implementations.
    pub async fn start(self, addr: SocketAddr) -> Result<(), tonic::transport::Error> {
        let query_service = query::QueryServiceImpl {
            state: self.state.clone(),
        };
        let document_service = document::DocumentServiceImpl {
            state: self.state.clone(),
        };
        let edge_service = edge::EdgeServiceImpl {
            state: self.state.clone(),
        };
        let database_service = database::DatabaseServiceImpl {
            state: self.state.clone(),
        };
        let metrics_service = metrics::MetricsServiceImpl {
            state: self.state.clone(),
        };

        tracing::info!(
            "gRPC server listening on {} ({})",
            addr,
            if cfg!(feature = "tls") && self.tls_enabled() {
                "TLS"
            } else {
                "plaintext"
            }
        );
        let max_message_size = self.config.max_message_size;

        let builder = Server::builder()
            .concurrency_limit_per_connection(self.config.concurrency_limit_per_connection)
            .max_concurrent_streams(self.config.max_concurrent_streams);
        #[cfg(feature = "tls")]
        let mut builder = match &self.config.tls {
            Some(tls) => builder.tls_config(tls.grpc_server_config())?,
            None => builder,
        };

        #[cfg(not(feature = "tls"))]
        let mut builder = builder;

        builder
            .add_service(
                proto::query_service_server::QueryServiceServer::new(query_service)
                    .max_decoding_message_size(max_message_size)
                    .max_encoding_message_size(max_message_size),
            )
            .add_service(
                proto::document_service_server::DocumentServiceServer::new(document_service)
                    .max_decoding_message_size(max_message_size)
                    .max_encoding_message_size(max_message_size),
            )
            .add_service(
                proto::edge_service_server::EdgeServiceServer::new(edge_service)
                    .max_decoding_message_size(max_message_size)
                    .max_encoding_message_size(max_message_size),
            )
            .add_service(
                proto::database_service_server::DatabaseServiceServer::new(database_service)
                    .max_decoding_message_size(max_message_size)
                    .max_encoding_message_size(max_message_size),
            )
            .add_service(
                proto::metrics_service_server::MetricsServiceServer::new(metrics_service)
                    .max_decoding_message_size(max_message_size)
                    .max_encoding_message_size(max_message_size),
            )
            .serve(addr)
            .await
    }

    fn tls_enabled(&self) -> bool {
        #[cfg(feature = "tls")]
        {
            self.config.tls.is_some()
        }
        #[cfg(not(feature = "tls"))]
        {
            false
        }
    }
}
