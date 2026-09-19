//! gRPC DatabaseService implementation

use std::sync::Arc;

use tonic::{Request, Response, Status};

use super::interceptor::{authorize_request, extract_subject, storage_error_to_status};
use super::proto::{
    CreateDatabaseRequest, CreateDatabaseResponse, DatabaseStats, DropDatabaseRequest,
    ListDatabasesResponse, StatsRequest,
};
use crate::auth::Action;
use crate::server::AppState;

pub struct DatabaseServiceImpl {
    pub state: Arc<AppState>,
}

#[tonic::async_trait]
impl super::proto::database_service_server::DatabaseService for DatabaseServiceImpl {
    async fn list(&self, request: Request<()>) -> Result<Response<ListDatabasesResponse>, Status> {
        let subject = extract_subject(&self.state, request.metadata()).await?;
        authorize_request(&self.state, subject.as_ref(), Action::Manage, "", None).await?;
        let databases = self.state.namespace.list_databases();
        Ok(Response::new(ListDatabasesResponse { databases }))
    }

    async fn create(
        &self,
        request: Request<CreateDatabaseRequest>,
    ) -> Result<Response<CreateDatabaseResponse>, Status> {
        let subject = extract_subject(&self.state, request.metadata()).await?;
        authorize_request(
            &self.state,
            subject.as_ref(),
            Action::Manage,
            &request.get_ref().name,
            None,
        )
        .await?;
        let req = request.into_inner();
        let name = req.name;

        if name.is_empty() {
            return Err(Status::invalid_argument("Database name is required"));
        }

        let namespace = self.state.namespace.clone();
        let db_name = name.clone();
        self.state
            .admission
            .run_db(move || namespace.create_database(&db_name))
            .await
            .map_err(crate::server::grpc::interceptor::admission_error_to_status)?
            .map_err(storage_error_to_status)?;

        Ok(Response::new(CreateDatabaseResponse {
            name,
            status: "created".to_string(),
        }))
    }

    async fn drop(&self, request: Request<DropDatabaseRequest>) -> Result<Response<()>, Status> {
        let subject = extract_subject(&self.state, request.metadata()).await?;
        authorize_request(
            &self.state,
            subject.as_ref(),
            Action::Manage,
            &request.get_ref().name,
            None,
        )
        .await?;
        let req = request.into_inner();
        let name = req.name;

        if name.is_empty() {
            return Err(Status::invalid_argument("Database name is required"));
        }

        let namespace = self.state.namespace.clone();
        self.state
            .admission
            .run_db(move || namespace.drop_database(&name))
            .await
            .map_err(crate::server::grpc::interceptor::admission_error_to_status)?
            .map_err(storage_error_to_status)?;

        Ok(Response::new(()))
    }

    async fn stats(
        &self,
        request: Request<StatsRequest>,
    ) -> Result<Response<DatabaseStats>, Status> {
        let subject = extract_subject(&self.state, request.metadata()).await?;
        authorize_request(
            &self.state,
            subject.as_ref(),
            Action::Manage,
            &request.get_ref().name,
            None,
        )
        .await?;
        let req = request.into_inner();
        let name = req.name;

        if name.is_empty() {
            return Err(Status::invalid_argument("Database name is required"));
        }

        let stats = self
            .state
            .admission
            .run_db({
                let namespace = self.state.namespace.clone();
                let name = name.clone();
                move || {
                    namespace
                        .get_database(&name)
                        .map(|database| database.stats())
                }
            })
            .await
            .map_err(crate::server::grpc::interceptor::admission_error_to_status)?
            .map_err(storage_error_to_status)?;

        Ok(Response::new(DatabaseStats {
            name,
            collections: stats.collections as i32,
            edge_labels: stats.edge_labels as i32,
            btree_indexes: stats.btree_indexes as i32,
            vector_indexes: stats.vector_indexes as i32,
            fts_indexes: stats.fts_indexes as i32,
            disk_size: stats.disk_size,
            disk_size_human: stats.disk_size_human(),
        }))
    }
}
