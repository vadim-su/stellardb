//! gRPC DocumentService implementation.

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::auth::Action;
use crate::document::Document;
use crate::server::AppState;
use crate::server::grpc::convert;
use crate::server::grpc::interceptor::{
    authorize_database_request, extract_database, extract_subject, storage_error_to_status,
};

use super::proto;
use super::proto::document_service_server::DocumentService;

pub struct DocumentServiceImpl {
    pub state: Arc<AppState>,
}

#[tonic::async_trait]
impl DocumentService for DocumentServiceImpl {
    /// Get a single document by collection and key.
    async fn get(
        &self,
        request: Request<proto::GetDocumentRequest>,
    ) -> Result<Response<proto::Document>, Status> {
        let subject = extract_subject(&self.state, request.metadata()).await?;
        authorize_database_request(
            &self.state,
            subject.as_ref(),
            Action::Select,
            request.metadata(),
            Some(&request.get_ref().collection),
        )
        .await?;
        let database = extract_database(&self.state, &request).await?;
        let req = request.into_inner();

        if req.collection.is_empty() {
            return Err(Status::invalid_argument("Collection name cannot be empty"));
        }

        let collection = req.collection;
        let key = req.key;
        let coll_name = collection.clone();

        let doc = self
            .state
            .admission
            .run_db(move || database.get_document(&collection, &key))
            .await
            .map_err(crate::server::grpc::interceptor::admission_error_to_status)?
            .map_err(storage_error_to_status)?;

        self.state.metrics.record_collection_read(&coll_name);

        match doc {
            Some(doc) => Ok(Response::new(convert::document_to_proto(&doc))),
            None => Err(Status::not_found("Document not found")),
        }
    }

    /// Create a new document.
    async fn create(
        &self,
        request: Request<proto::CreateDocumentRequest>,
    ) -> Result<Response<proto::Document>, Status> {
        let subject = extract_subject(&self.state, request.metadata()).await?;
        authorize_database_request(
            &self.state,
            subject.as_ref(),
            Action::Insert,
            request.metadata(),
            Some(&request.get_ref().collection),
        )
        .await?;
        let database = extract_database(&self.state, &request).await?;
        let req = request.into_inner();

        if req.collection.is_empty() {
            return Err(Status::invalid_argument("Collection name cannot be empty"));
        }

        let key = nanoid::nanoid!();
        let id = format!("{}:{}", req.collection, key);
        let fields = convert::fields_from_proto(&req.fields);

        let doc = Document {
            id: id.clone(),
            fields,
        };

        let collection = req.collection;
        let created = self
            .state
            .admission
            .run_db({
                let doc = doc.clone();
                move || database.create_document(&doc)
            })
            .await
            .map_err(crate::server::grpc::interceptor::admission_error_to_status)?
            .map_err(storage_error_to_status)?;

        if created {
            self.state.metrics.record_collection_write(&collection);
            Ok(Response::new(convert::document_to_proto(&doc)))
        } else {
            Err(Status::already_exists(format!("{} already exists", id)))
        }
    }

    /// Update an existing document.
    async fn update(
        &self,
        request: Request<proto::UpdateDocumentRequest>,
    ) -> Result<Response<proto::Document>, Status> {
        let subject = extract_subject(&self.state, request.metadata()).await?;
        authorize_database_request(
            &self.state,
            subject.as_ref(),
            Action::Update,
            request.metadata(),
            Some(&request.get_ref().collection),
        )
        .await?;
        let database = extract_database(&self.state, &request).await?;
        let req = request.into_inner();

        if req.collection.is_empty() {
            return Err(Status::invalid_argument("Collection name cannot be empty"));
        }

        let id = format!("{}:{}", req.collection, req.key);
        let fields = convert::fields_from_proto(&req.fields);

        let doc = Document {
            id: id.clone(),
            fields,
        };

        let collection = req.collection;
        let updated = self
            .state
            .admission
            .run_db({
                let doc = doc.clone();
                move || database.update_document(&doc)
            })
            .await
            .map_err(crate::server::grpc::interceptor::admission_error_to_status)?
            .map_err(storage_error_to_status)?;

        if updated {
            self.state.metrics.record_collection_write(&collection);
            Ok(Response::new(convert::document_to_proto(&doc)))
        } else {
            Err(Status::not_found("Document not found"))
        }
    }

    /// Delete a document.
    async fn delete(
        &self,
        request: Request<proto::DeleteDocumentRequest>,
    ) -> Result<Response<()>, Status> {
        let subject = extract_subject(&self.state, request.metadata()).await?;
        authorize_database_request(
            &self.state,
            subject.as_ref(),
            Action::Delete,
            request.metadata(),
            Some(&request.get_ref().collection),
        )
        .await?;
        let database = extract_database(&self.state, &request).await?;
        let req = request.into_inner();

        if req.collection.is_empty() {
            return Err(Status::invalid_argument("Collection name cannot be empty"));
        }

        let collection = req.collection;
        let key = req.key;
        let coll_name = collection.clone();

        let deleted = self
            .state
            .admission
            .run_db(move || database.delete_document(&collection, &key))
            .await
            .map_err(crate::server::grpc::interceptor::admission_error_to_status)?
            .map_err(storage_error_to_status)?;

        if deleted {
            self.state.metrics.record_collection_delete(&coll_name);
            Ok(Response::new(()))
        } else {
            Err(Status::not_found("Document not found"))
        }
    }

    type ListStream = ReceiverStream<Result<proto::DocumentListItem, Status>>;

    /// List documents in a collection with pagination, streaming results.
    async fn list(
        &self,
        request: Request<proto::ListDocumentsRequest>,
    ) -> Result<Response<Self::ListStream>, Status> {
        let subject = extract_subject(&self.state, request.metadata()).await?;
        authorize_database_request(
            &self.state,
            subject.as_ref(),
            Action::Select,
            request.metadata(),
            Some(&request.get_ref().collection),
        )
        .await?;
        let database = extract_database(&self.state, &request).await?;
        let req = request.into_inner();

        if req.collection.is_empty() {
            return Err(Status::invalid_argument("Collection name cannot be empty"));
        }

        let collection = req.collection;
        let pagination = req.pagination.unwrap_or_default();
        let limit = (pagination.limit as usize).clamp(1, 1000);
        let after = pagination.after;
        let coll_name = collection.clone();

        let (docs, next_cursor) = self
            .state
            .admission
            .run_db({
                let after = after.clone();
                move || database.list_documents(&collection, after.as_deref(), limit)
            })
            .await
            .map_err(crate::server::grpc::interceptor::admission_error_to_status)?
            .map_err(storage_error_to_status)?;

        self.state.metrics.record_collection_read(&coll_name);

        let (tx, rx) = mpsc::channel(64);
        let has_more = next_cursor.is_some();

        tokio::spawn(async move {
            for doc in &docs {
                let item = proto::DocumentListItem {
                    payload: Some(proto::document_list_item::Payload::Document(
                        convert::document_to_proto(doc),
                    )),
                };
                if tx.send(Ok(item)).await.is_err() {
                    return;
                }
            }

            // Send has_more indicator
            let item = proto::DocumentListItem {
                payload: Some(proto::document_list_item::Payload::HasMore(has_more)),
            };
            let _ = tx.send(Ok(item)).await;
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}
