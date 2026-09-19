//! gRPC QueryService implementation.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::auth::Action;
use crate::query::{self, ExecuteContext, Query as QueryBuilder};
use crate::server::AppState;
use crate::server::ddl_jobs::CancelError;
use crate::server::grpc::convert;
use crate::server::grpc::interceptor::{
    authorize_request, extract_database, extract_database_name, extract_subject,
};

use super::proto;
use super::proto::query_service_server::QueryService;

pub struct QueryServiceImpl {
    pub state: Arc<AppState>,
}

fn grpc_transport_timeout(
    metadata: &tonic::metadata::MetadataMap,
) -> Result<Option<Duration>, Status> {
    let Some(value) = metadata.get("grpc-timeout") else {
        return Ok(None);
    };
    let raw = value
        .to_str()
        .map_err(|_| Status::invalid_argument("grpc-timeout must be ASCII"))?;
    if raw.is_empty() || raw.len() > 9 {
        return Err(Status::invalid_argument("invalid grpc-timeout"));
    }
    let (digits, unit) = raw.split_at(raw.len() - 1);
    if digits.is_empty() || digits.len() > 8 {
        return Err(Status::invalid_argument("invalid grpc-timeout"));
    }
    let value: u64 = digits
        .parse()
        .map_err(|_| Status::invalid_argument("invalid grpc-timeout"))?;
    let duration = match unit {
        "H" => Duration::from_secs(value.saturating_mul(60 * 60)),
        "M" => Duration::from_secs(value.saturating_mul(60)),
        "S" => Duration::from_secs(value),
        "m" => Duration::from_millis(value),
        "u" => Duration::from_micros(value),
        "n" => Duration::from_nanos(value),
        _ => return Err(Status::invalid_argument("invalid grpc-timeout")),
    };
    Ok(Some(duration))
}

fn min_deadline(
    first: std::time::Instant,
    second: Option<std::time::Instant>,
) -> std::time::Instant {
    second.map_or(first, |second| first.min(second))
}

fn parse_rpc_query_timeout(
    value: Option<&str>,
    default: Option<Duration>,
) -> Result<(Option<Duration>, bool), Status> {
    crate::server::parse_query_timeout(value, default)
        .map_err(|error| Status::invalid_argument(format!("[SDB-ST008] {error}")))
}

#[tonic::async_trait]
impl QueryService for QueryServiceImpl {
    /// Execute a query and return all results at once.
    async fn execute(
        &self,
        request: Request<proto::ExecuteRequest>,
    ) -> Result<Response<proto::ExecuteResponse>, Status> {
        let start = Instant::now();
        let transport_timeout = grpc_transport_timeout(request.metadata())?;
        let request_deadline = transport_timeout.and_then(|timeout| start.checked_add(timeout));
        let resolved_subject = extract_subject(&self.state, request.metadata()).await?;
        let database_name = extract_database_name(request.metadata())?.to_string();
        let database = extract_database(&self.state, &request).await?;
        let operation_id = request
            .metadata()
            .get("x-operation-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);

        let req = request.into_inner();
        let session_id = req.session_id;
        let (query_timeout, timeout_is_explicit) = parse_rpc_query_timeout(
            req.query_timeout.as_deref(),
            self.state.query_timeout_default,
        )?;
        let tracked_query_timeout = if timeout_is_explicit {
            query_timeout
        } else {
            crate::server::stored_session_query_timeout(
                &self.state,
                session_id.as_deref(),
                resolved_subject.as_ref(),
                &database_name,
            )
            .or(query_timeout)
        };
        let server_deadline = start + self.state.admission.db_deadline();
        let response_deadline = min_deadline(
            crate::server::effective_absolute_deadline(
                start,
                tracked_query_timeout,
                self.state.admission.db_deadline(),
            ),
            request_deadline,
        );

        let (launch_hash, launch_normalized) = crate::metrics::normalize_query(&req.query);
        let parsed = QueryBuilder::parse(&req.query).map_err(|error| {
            self.state.metrics.record_query_end(
                launch_hash,
                start.elapsed().as_micros() as u64,
                0,
                0,
                true,
            );
            self.state
                .metrics
                .store_query_fingerprint(launch_hash, launch_normalized.as_deref());
            crate::server::grpc::interceptor::query_error_to_status(&error.into())
        })?;
        if let Some((job, _created)) = crate::server::maybe_enqueue_create_index_job(
            &self.state,
            &parsed,
            database.clone(),
            &database_name,
            resolved_subject.as_ref(),
            transport_timeout.is_some(),
            operation_id,
        )
        .await
        .map_err(|error| crate::server::grpc::interceptor::app_error_to_status(&error))?
        {
            self.state.metrics.record_query_end(
                launch_hash,
                start.elapsed().as_micros() as u64,
                0,
                0,
                false,
            );
            self.state
                .metrics
                .store_query_fingerprint(launch_hash, launch_normalized.as_deref());
            return Ok(Response::new(proto::ExecuteResponse {
                results: Vec::new(),
                completed: 0,
                error: None,
                elapsed: format_elapsed(start.elapsed()),
                operation: Some(convert::ddl_job_to_proto(&job)),
            }));
        }

        let contains_ddl = parsed.contains_ddl();
        let sessions = self.state.sessions.clone();
        let auth = self.state.auth.clone();
        let sid = session_id.clone();
        let operation_scope = crate::server::admission::OperationScope::ddl(database_name.clone());
        let operation = move || {
            let mut ctx = ExecuteContext::with_session(&sessions, sid.as_deref())
                .with_database(database_name)
                .with_auth(auth.as_deref())
                .with_resolved_subject(resolved_subject)
                .with_server_db_deadline(start, server_deadline)
                .with_request_deadline(request_deadline);
            ctx = if timeout_is_explicit {
                ctx.with_explicit_query_timeout(query_timeout)
            } else {
                ctx.with_query_timeout(query_timeout)
            };
            parsed
                .bind(database)
                .map_err(query::QueryError::from)?
                .execute(&ctx)
                .map_err(query::QueryError::from)
        };
        let execution = if contains_ddl {
            let timeout = if transport_timeout.is_some() {
                Duration::ZERO
            } else {
                response_deadline.saturating_duration_since(Instant::now())
            };
            self.state
                .admission
                .run_db_with_timeout(timeout, operation_scope, operation, |result| match result {
                    Ok(_) => crate::server::admission::OperationOutcome::Completed { result: None },
                    Err(error) => crate::server::admission::OperationOutcome::Failed {
                        code: crate::error::ErrorCode::code(error).to_string(),
                        message: crate::error::safe_query_error_message(error),
                    },
                })
                .await
        } else {
            self.state.admission.run_db_cooperative(operation).await
        };
        let result = match execution {
            Ok(result) => result,
            Err(crate::server::admission::AdmissionError::OperationRunning(operation_id)) => {
                let operation = self
                    .state
                    .admission
                    .operation(&operation_id)
                    .ok_or_else(|| Status::internal("tracked DDL operation disappeared"))?;
                self.state.metrics.record_query_end(
                    launch_hash,
                    start.elapsed().as_micros() as u64,
                    0,
                    0,
                    false,
                );
                self.state
                    .metrics
                    .store_query_fingerprint(launch_hash, launch_normalized.as_deref());
                return Ok(Response::new(proto::ExecuteResponse {
                    results: Vec::new(),
                    completed: 0,
                    error: None,
                    elapsed: format_elapsed(start.elapsed()),
                    operation: Some(convert::admission_operation_to_proto(&operation)),
                }));
            }
            Err(error) => {
                return Err(crate::server::grpc::interceptor::admission_error_to_status(
                    error,
                ));
            }
        };
        let query_hash = launch_hash;
        let normalized = launch_normalized;

        // Record metrics
        let elapsed_us = start.elapsed().as_micros() as u64;
        let (rows_returned, is_error) = match &result {
            Ok(batch) => {
                let rows: u64 = batch.results.iter().map(|r| r.row_count() as u64).sum();
                (rows, false)
            }
            Err(_) => (0, true),
        };

        self.state
            .metrics
            .record_query_end(query_hash, elapsed_us, rows_returned, 0, is_error);
        self.state
            .metrics
            .store_query_fingerprint(query_hash, normalized.as_deref());

        let result = result
            .map_err(|error| crate::server::grpc::interceptor::query_error_to_status(&error))?;

        let elapsed = format_elapsed(start.elapsed());
        let response = convert::batch_result_to_proto(&result, elapsed);

        Ok(Response::new(response))
    }

    type ExecuteStreamStream = ReceiverStream<Result<proto::StreamRow, Status>>;

    /// Execute a query and stream results row by row.
    ///
    /// Stream format per statement: header -> row* -> footer
    async fn execute_stream(
        &self,
        request: Request<proto::ExecuteRequest>,
    ) -> Result<Response<Self::ExecuteStreamStream>, Status> {
        let start = Instant::now();
        let transport_timeout = grpc_transport_timeout(request.metadata())?;
        let request_deadline = transport_timeout.and_then(|timeout| start.checked_add(timeout));
        let subject = extract_subject(&self.state, request.metadata()).await?;
        let database_name = extract_database_name(request.metadata())?.to_string();
        let database = extract_database(&self.state, &request).await?;
        let operation_id = request
            .metadata()
            .get("x-operation-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);

        let req = request.into_inner();
        let session_id = req.session_id;
        let (query_timeout, timeout_is_explicit) = parse_rpc_query_timeout(
            req.query_timeout.as_deref(),
            self.state.query_timeout_default,
        )?;
        let tracked_query_timeout = if timeout_is_explicit {
            query_timeout
        } else {
            crate::server::stored_session_query_timeout(
                &self.state,
                session_id.as_deref(),
                subject.as_ref(),
                &database_name,
            )
            .or(query_timeout)
        };
        let server_deadline = start + self.state.admission.db_deadline();
        let response_deadline = min_deadline(
            crate::server::effective_absolute_deadline(
                start,
                tracked_query_timeout,
                self.state.admission.db_deadline(),
            ),
            request_deadline,
        );

        let (launch_hash, launch_normalized) = crate::metrics::normalize_query(&req.query);
        let parsed = QueryBuilder::parse(&req.query).map_err(|error| {
            self.state.metrics.record_query_end(
                launch_hash,
                start.elapsed().as_micros() as u64,
                0,
                0,
                true,
            );
            self.state
                .metrics
                .store_query_fingerprint(launch_hash, launch_normalized.as_deref());
            crate::server::grpc::interceptor::query_error_to_status(&error.into())
        })?;
        if let Some((job, _created)) = crate::server::maybe_enqueue_create_index_job(
            &self.state,
            &parsed,
            database.clone(),
            &database_name,
            subject.as_ref(),
            transport_timeout.is_some(),
            operation_id,
        )
        .await
        .map_err(|error| crate::server::grpc::interceptor::app_error_to_status(&error))?
        {
            self.state.metrics.record_query_end(
                launch_hash,
                start.elapsed().as_micros() as u64,
                0,
                0,
                false,
            );
            self.state
                .metrics
                .store_query_fingerprint(launch_hash, launch_normalized.as_deref());
            let (tx, rx) = mpsc::channel(1);
            tx.send(Ok(proto::StreamRow {
                payload: None,
                operation: Some(convert::ddl_job_to_proto(&job)),
            }))
            .await
            .map_err(|_| Status::internal("failed to create operation stream"))?;
            return Ok(Response::new(ReceiverStream::new(rx)));
        }

        let contains_ddl = parsed.contains_ddl();
        let sessions = self.state.sessions.clone();
        let auth = self.state.auth.clone();
        let sid = session_id.clone();
        let operation_scope = crate::server::admission::OperationScope::ddl(database_name.clone());
        let operation = move || {
            let mut ctx = ExecuteContext::with_session(&sessions, sid.as_deref())
                .with_database(database_name)
                .with_auth(auth.as_deref())
                .with_resolved_subject(subject)
                .with_server_db_deadline(start, server_deadline)
                .with_request_deadline(request_deadline);
            ctx = if timeout_is_explicit {
                ctx.with_explicit_query_timeout(query_timeout)
            } else {
                ctx.with_query_timeout(query_timeout)
            };
            parsed
                .bind(database)
                .map_err(query::QueryError::from)?
                .execute(&ctx)
                .map_err(query::QueryError::from)
        };
        let execution = if contains_ddl {
            let timeout = if transport_timeout.is_some() {
                Duration::ZERO
            } else {
                response_deadline.saturating_duration_since(Instant::now())
            };
            self.state
                .admission
                .run_db_with_timeout(timeout, operation_scope, operation, |result| match result {
                    Ok(_) => crate::server::admission::OperationOutcome::Completed { result: None },
                    Err(error) => crate::server::admission::OperationOutcome::Failed {
                        code: crate::error::ErrorCode::code(error).to_string(),
                        message: crate::error::safe_query_error_message(error),
                    },
                })
                .await
        } else {
            self.state.admission.run_db_cooperative(operation).await
        };
        let result = match execution {
            Ok(result) => result,
            Err(crate::server::admission::AdmissionError::OperationRunning(operation_id)) => {
                let operation = self
                    .state
                    .admission
                    .operation(&operation_id)
                    .ok_or_else(|| Status::internal("tracked DDL operation disappeared"))?;
                self.state.metrics.record_query_end(
                    launch_hash,
                    start.elapsed().as_micros() as u64,
                    0,
                    0,
                    false,
                );
                self.state
                    .metrics
                    .store_query_fingerprint(launch_hash, launch_normalized.as_deref());
                let (tx, rx) = mpsc::channel(1);
                tx.send(Ok(proto::StreamRow {
                    payload: None,
                    operation: Some(convert::admission_operation_to_proto(&operation)),
                }))
                .await
                .map_err(|_| Status::internal("failed to create operation stream"))?;
                return Ok(Response::new(ReceiverStream::new(rx)));
            }
            Err(error) => {
                return Err(crate::server::grpc::interceptor::admission_error_to_status(
                    error,
                ));
            }
        };
        let query_hash = launch_hash;
        let normalized = launch_normalized;

        // Record metrics
        let elapsed_us = start.elapsed().as_micros() as u64;
        let (rows_returned, is_error) = match &result {
            Ok(batch) => {
                let rows: u64 = batch.results.iter().map(|r| r.row_count() as u64).sum();
                (rows, false)
            }
            Err(_) => (0, true),
        };

        self.state
            .metrics
            .record_query_end(query_hash, elapsed_us, rows_returned, 0, is_error);
        self.state
            .metrics
            .store_query_fingerprint(query_hash, normalized.as_deref());

        let batch = result
            .map_err(|error| crate::server::grpc::interceptor::query_error_to_status(&error))?;

        let (tx, rx) = mpsc::channel(64);

        // Stream results in background task
        tokio::spawn(async move {
            let elapsed_str = format_elapsed(start.elapsed());

            for stmt in &batch.results {
                // Send header
                let columns: Vec<String> = if let Some(first_row) = stmt.rows.first() {
                    if first_row.is_scalar() {
                        vec![
                            first_row
                                .scalar_alias
                                .clone()
                                .unwrap_or_else(|| "$value".to_string()),
                        ]
                    } else {
                        let mut cols = Vec::new();
                        if !first_row.doc.id.is_empty() {
                            cols.push("id".to_string());
                        }
                        cols.extend(first_row.doc.fields.keys().cloned());
                        cols
                    }
                } else {
                    vec![]
                };

                let header = proto::StreamRow {
                    payload: Some(proto::stream_row::Payload::Header(proto::StatementHeader {
                        statement_index: stmt.statement_index as i32,
                        statement_type: stmt.statement_type.clone(),
                        columns,
                    })),
                    operation: None,
                };
                if tx.send(Ok(header)).await.is_err() {
                    return;
                }

                // Send rows
                for row in &stmt.rows {
                    let row_data = convert::row_to_proto(row);
                    let stream_row = proto::StreamRow {
                        payload: Some(proto::stream_row::Payload::Row(row_data)),
                        operation: None,
                    };
                    if tx.send(Ok(stream_row)).await.is_err() {
                        return;
                    }
                }

                // Send footer
                let footer = proto::StreamRow {
                    payload: Some(proto::stream_row::Payload::Footer(proto::StatementFooter {
                        statement_index: stmt.statement_index as i32,
                        rows_affected: stmt.rows_affected.map(|n| n as i64),
                        count: Some(stmt.rows.len() as i32),
                        session_id: stmt.session_id.clone(),
                        elapsed: elapsed_str.clone(),
                    })),
                    operation: None,
                };
                if tx.send(Ok(footer)).await.is_err() {
                    return;
                }
            }

            // Send batch error if present
            if let Some(ref err) = batch.error {
                let error_row = proto::StreamRow {
                    payload: Some(proto::stream_row::Payload::Error(
                        convert::batch_error_to_proto(err),
                    )),
                    operation: None,
                };
                let _ = tx.send(Ok(error_row)).await;
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn get_operation(
        &self,
        request: Request<proto::GetOperationRequest>,
    ) -> Result<Response<proto::DdlOperation>, Status> {
        let subject = extract_subject(&self.state, request.metadata()).await?;
        let operation_id = request.into_inner().operation_id;
        if let Some(job) = self.state.ddl_jobs.get(&operation_id) {
            authorize_request(
                &self.state,
                subject.as_ref(),
                Action::Create,
                &job.database,
                None,
            )
            .await?;
            return Ok(Response::new(convert::ddl_job_to_proto(&job)));
        }
        let operation = self
            .state
            .admission
            .operation(&operation_id)
            .ok_or_else(|| Status::not_found("DDL operation not found"))?;
        authorize_request(
            &self.state,
            subject.as_ref(),
            Action::Create,
            &operation.database,
            None,
        )
        .await?;
        Ok(Response::new(convert::admission_operation_to_proto(
            &operation,
        )))
    }

    async fn list_operations(
        &self,
        request: Request<proto::ListOperationsRequest>,
    ) -> Result<Response<proto::ListOperationsResponse>, Status> {
        let subject = extract_subject(&self.state, request.metadata()).await?;
        let database = extract_database_name(request.metadata())?.to_string();
        authorize_request(
            &self.state,
            subject.as_ref(),
            Action::Create,
            &database,
            None,
        )
        .await?;
        let request = request.into_inner();
        let jobs = self.state.ddl_jobs.list(
            Some(&database),
            request.active_only,
            request.limit.map(|value| value as usize),
        );
        let mut operations: Vec<_> = jobs.iter().map(convert::ddl_job_to_proto).collect();
        operations.extend(
            self.state
                .admission
                .list_operations(
                    &database,
                    request.active_only,
                    request.limit.map(|value| value as usize),
                )
                .iter()
                .map(convert::admission_operation_to_proto),
        );
        operations.truncate(request.limit.unwrap_or(100).min(1_000) as usize);
        Ok(Response::new(proto::ListOperationsResponse {
            count: operations.len() as u32,
            operations,
        }))
    }

    async fn cancel_operation(
        &self,
        request: Request<proto::CancelOperationRequest>,
    ) -> Result<Response<proto::DdlOperation>, Status> {
        let subject = extract_subject(&self.state, request.metadata()).await?;
        let operation_id = request.into_inner().operation_id;
        if let Some(job) = self.state.ddl_jobs.get(&operation_id) {
            authorize_request(
                &self.state,
                subject.as_ref(),
                Action::Create,
                &job.database,
                None,
            )
            .await?;
            let job = self
                .state
                .ddl_jobs
                .cancel(&operation_id)
                .map_err(cancel_error_to_status)?;
            return Ok(Response::new(convert::ddl_job_to_proto(&job)));
        }
        let operation = self
            .state
            .admission
            .operation(&operation_id)
            .ok_or_else(|| Status::not_found("DDL operation not found"))?;
        authorize_request(
            &self.state,
            subject.as_ref(),
            Action::Create,
            &operation.database,
            None,
        )
        .await?;
        Err(Status::failed_precondition(
            "DDL operation cannot be cancelled safely after blocking execution started",
        ))
    }
}

fn cancel_error_to_status(error: CancelError) -> Status {
    match error {
        CancelError::NotFound => Status::not_found("DDL operation not found"),
        CancelError::UnsafePhase(phase) => Status::failed_precondition(format!(
            "DDL operation cannot be cancelled safely during {}",
            phase.as_str()
        )),
        CancelError::Terminal(state) => Status::failed_precondition(format!(
            "DDL operation is already terminal ({})",
            state.as_str()
        )),
        CancelError::Storage(error) => {
            crate::server::grpc::interceptor::storage_error_to_status(error)
        }
    }
}

/// Format elapsed duration as a human-readable string.
fn format_elapsed(duration: Duration) -> String {
    let nanos = duration.as_nanos() as f64;
    if nanos < 1_000.0 {
        format!("{:.2}ns", nanos)
    } else if nanos < 1_000_000.0 {
        format!("{:.2}\u{00b5}s", nanos / 1_000.0)
    } else if nanos < 1_000_000_000.0 {
        format!("{:.2}ms", nanos / 1_000_000.0)
    } else {
        format!("{:.2}s", nanos / 1_000_000_000.0)
    }
}
