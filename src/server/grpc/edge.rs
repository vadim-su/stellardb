//! gRPC EdgeService implementation

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use super::convert::{edge_to_proto, fields_from_proto};
use super::interceptor::{
    authorize_database_request, extract_database, extract_subject, storage_error_to_status,
};
use super::proto::{
    CreateEdgeRequest, DeleteEdgeRequest, Edge as ProtoEdge, EdgeListItem, GetEdgeRequest,
    ListEdgesRequest, edge_list_item::Payload as EdgeListPayload,
};
use crate::auth::Action;
use crate::edge::Edge;
use crate::server::AppState;

pub struct EdgeServiceImpl {
    pub state: Arc<AppState>,
}

#[tonic::async_trait]
impl super::proto::edge_service_server::EdgeService for EdgeServiceImpl {
    async fn create(
        &self,
        request: Request<CreateEdgeRequest>,
    ) -> Result<Response<ProtoEdge>, Status> {
        let subject = extract_subject(&self.state, request.metadata()).await?;
        authorize_database_request(
            &self.state,
            subject.as_ref(),
            Action::Traverse,
            request.metadata(),
            None,
        )
        .await?;
        let db = extract_database(&self.state, &request).await?;
        let req = request.into_inner();

        let fields = fields_from_proto(&req.fields);
        let edge = Edge::new(req.from, req.label, req.to).with_fields(fields);

        let edge_clone = edge.clone();
        self.state
            .admission
            .run_db(move || db.create_edge(&edge_clone))
            .await
            .map_err(crate::server::grpc::interceptor::admission_error_to_status)?
            .map_err(storage_error_to_status)?;

        Ok(Response::new(edge_to_proto(&edge)))
    }

    async fn get(&self, request: Request<GetEdgeRequest>) -> Result<Response<ProtoEdge>, Status> {
        let subject = extract_subject(&self.state, request.metadata()).await?;
        authorize_database_request(
            &self.state,
            subject.as_ref(),
            Action::Traverse,
            request.metadata(),
            None,
        )
        .await?;
        let db = extract_database(&self.state, &request).await?;
        let req = request.into_inner();

        let from = req.from;
        let label = req.label;
        let to = req.to;

        let edge = self
            .state
            .admission
            .run_db(move || db.get_edge(&from, &label, &to))
            .await
            .map_err(crate::server::grpc::interceptor::admission_error_to_status)?
            .map_err(storage_error_to_status)?;

        match edge {
            Some(e) => Ok(Response::new(edge_to_proto(&e))),
            None => Err(Status::not_found("Edge not found")),
        }
    }

    async fn delete(&self, request: Request<DeleteEdgeRequest>) -> Result<Response<()>, Status> {
        let subject = extract_subject(&self.state, request.metadata()).await?;
        authorize_database_request(
            &self.state,
            subject.as_ref(),
            Action::Delete,
            request.metadata(),
            None,
        )
        .await?;
        let db = extract_database(&self.state, &request).await?;
        let req = request.into_inner();

        let from = req.from;
        let label = req.label;
        let to = req.to;

        let deleted = self
            .state
            .admission
            .run_db(move || db.delete_edge(&from, &label, &to))
            .await
            .map_err(crate::server::grpc::interceptor::admission_error_to_status)?
            .map_err(storage_error_to_status)?;

        if deleted {
            Ok(Response::new(()))
        } else {
            Err(Status::not_found("Edge not found"))
        }
    }

    type ListOutStream = ReceiverStream<Result<EdgeListItem, Status>>;

    async fn list_out(
        &self,
        request: Request<ListEdgesRequest>,
    ) -> Result<Response<Self::ListOutStream>, Status> {
        let subject = extract_subject(&self.state, request.metadata()).await?;
        authorize_database_request(
            &self.state,
            subject.as_ref(),
            Action::Traverse,
            request.metadata(),
            None,
        )
        .await?;
        let db = extract_database(&self.state, &request).await?;
        let req = request.into_inner();

        let node_id = req.node_id;
        let label = req.label;
        let (limit, after) = parse_pagination(req.pagination);

        let (edges, _next_cursor) = self
            .state
            .admission
            .run_db(move || db.list_edges_out(&node_id, &label, after.as_deref(), limit))
            .await
            .map_err(crate::server::grpc::interceptor::admission_error_to_status)?
            .map_err(storage_error_to_status)?;

        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            let has_more = edges.len() == limit;
            for edge in &edges {
                let item = EdgeListItem {
                    payload: Some(EdgeListPayload::Edge(edge_to_proto(edge))),
                };
                if tx.send(Ok(item)).await.is_err() {
                    return;
                }
            }
            let _ = tx
                .send(Ok(EdgeListItem {
                    payload: Some(EdgeListPayload::HasMore(has_more)),
                }))
                .await;
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    type ListInStream = ReceiverStream<Result<EdgeListItem, Status>>;

    async fn list_in(
        &self,
        request: Request<ListEdgesRequest>,
    ) -> Result<Response<Self::ListInStream>, Status> {
        let subject = extract_subject(&self.state, request.metadata()).await?;
        authorize_database_request(
            &self.state,
            subject.as_ref(),
            Action::Traverse,
            request.metadata(),
            None,
        )
        .await?;
        let db = extract_database(&self.state, &request).await?;
        let req = request.into_inner();

        let node_id = req.node_id;
        let label = req.label;
        let (limit, after) = parse_pagination(req.pagination);

        let (edges, _next_cursor) = self
            .state
            .admission
            .run_db(move || db.list_edges_in(&node_id, &label, after.as_deref(), limit))
            .await
            .map_err(crate::server::grpc::interceptor::admission_error_to_status)?
            .map_err(storage_error_to_status)?;

        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            let has_more = edges.len() == limit;
            for edge in &edges {
                let item = EdgeListItem {
                    payload: Some(EdgeListPayload::Edge(edge_to_proto(edge))),
                };
                if tx.send(Ok(item)).await.is_err() {
                    return;
                }
            }
            let _ = tx
                .send(Ok(EdgeListItem {
                    payload: Some(EdgeListPayload::HasMore(has_more)),
                }))
                .await;
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

fn parse_pagination(
    pagination: Option<super::proto::PaginationRequest>,
) -> (usize, Option<String>) {
    match pagination {
        Some(p) => {
            let limit = (p.limit as usize).clamp(1, 1000);
            (limit, p.after)
        }
        None => (50, None),
    }
}
