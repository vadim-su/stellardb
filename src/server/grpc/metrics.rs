//! gRPC MetricsService implementation

use std::sync::Arc;

use tonic::{Request, Response, Status};

use super::interceptor::{authorize_request, extract_subject};
use super::proto::{PrometheusResponse, StatsResponse};
use crate::auth::Action;
use crate::metrics::{format_json_stats, format_prometheus};
use crate::server::AppState;
use crate::storage::StorageStats;

pub struct MetricsServiceImpl {
    pub state: Arc<AppState>,
}

#[tonic::async_trait]
impl super::proto::metrics_service_server::MetricsService for MetricsServiceImpl {
    async fn get_stats(&self, request: Request<()>) -> Result<Response<StatsResponse>, Status> {
        let subject = extract_subject(&self.state, request.metadata()).await?;
        authorize_request(&self.state, subject.as_ref(), Action::Manage, "", None).await?;
        let storage_stats = StorageStats::default();
        let json = format_json_stats(&self.state.metrics, &storage_stats, 50);
        Ok(Response::new(StatsResponse {
            json: json.to_string(),
        }))
    }

    async fn get_prometheus(
        &self,
        request: Request<()>,
    ) -> Result<Response<PrometheusResponse>, Status> {
        let subject = extract_subject(&self.state, request.metadata()).await?;
        authorize_request(&self.state, subject.as_ref(), Action::Manage, "", None).await?;
        let storage_stats = StorageStats::default();
        let text = format_prometheus(&self.state.metrics, &storage_stats);
        Ok(Response::new(PrometheusResponse { text }))
    }
}
