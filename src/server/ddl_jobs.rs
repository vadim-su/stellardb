//! Durable background jobs for long-running DDL operations.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use fjall::{KeyspaceCreateOptions, OptimisticTxDatabase, PersistMode};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, StorageError};
use crate::namespace::Namespace;
use crate::query::ExecuteError;
use crate::query::execute::ddl::prepare_create_index;
use crate::schema::{IndexDef, IndexType};
use crate::server::admission::AdmissionController;
use crate::storage::IndexBuildEvent;

const JOBS_KEYSPACE: &str = "ddl_jobs";
pub const ASYNC_CREATE_INDEX_THRESHOLD: usize = 1_000;
const DEFAULT_LIST_LIMIT: usize = 50;
const MAX_LIST_LIMIT: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DdlJobState {
    Queued,
    Building,
    Ready,
    Failed,
    Cancelling,
    Cancelled,
}

impl DdlJobState {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Building | Self::Cancelling)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "QUEUED",
            Self::Building => "BUILDING",
            Self::Ready => "READY",
            Self::Failed => "FAILED",
            Self::Cancelling => "CANCELLING",
            Self::Cancelled => "CANCELLED",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DdlJobPhase {
    Queued,
    Recovering,
    Initializing,
    Building,
    ValidatingUnique,
    Committing,
    Registering,
    Persisting,
    CleaningUp,
    Complete,
}

impl DdlJobPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "QUEUED",
            Self::Recovering => "RECOVERING",
            Self::Initializing => "INITIALIZING",
            Self::Building => "BUILDING",
            Self::ValidatingUnique => "VALIDATING_UNIQUE",
            Self::Committing => "COMMITTING",
            Self::Registering => "REGISTERING",
            Self::Persisting => "PERSISTING",
            Self::CleaningUp => "CLEANING_UP",
            Self::Complete => "COMPLETE",
        }
    }

    fn cancellation_is_safe(self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::Recovering
                | Self::Initializing
                | Self::Building
                | Self::ValidatingUnique
                | Self::CleaningUp
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdlJobError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdlJob {
    pub operation_id: String,
    #[serde(default = "new_build_id")]
    pub build_id: String,
    pub operation_type: String,
    pub database: String,
    pub collection: String,
    pub index_name: String,
    pub index_type: String,
    pub state: DdlJobState,
    pub phase: DdlJobPhase,
    pub processed_items: usize,
    pub total_items: usize,
    pub progress_percent: f64,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<DdlJobError>,
    index: IndexDef,
}

impl DdlJob {
    fn new(
        operation_id: Option<String>,
        database: String,
        collection: String,
        index: IndexDef,
        total_items: usize,
    ) -> Self {
        Self {
            operation_id: operation_id.unwrap_or_else(|| format!("op_{}", nanoid::nanoid!(20))),
            build_id: new_build_id(),
            operation_type: "CREATE_INDEX".to_string(),
            database,
            collection,
            index_name: index.name.clone(),
            index_type: index_type_name(index.index_type).to_string(),
            state: DdlJobState::Queued,
            phase: DdlJobPhase::Queued,
            processed_items: 0,
            total_items,
            progress_percent: 0.0,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            error: None,
            index,
        }
    }

    fn refresh_progress(&mut self) {
        self.progress_percent = if self.total_items == 0 {
            if self.state == DdlJobState::Ready {
                100.0
            } else {
                0.0
            }
        } else {
            ((self.processed_items.min(self.total_items) as f64 / self.total_items as f64) * 100.0)
                .min(100.0)
        };
    }

    /// Stable HTTP representation. The replay-only `index` definition remains
    /// internal to the durable store.
    pub fn public_json(&self) -> serde_json::Value {
        serde_json::json!({
            "operation_id": self.operation_id,
            "build_id": self.build_id,
            "operation_type": self.operation_type,
            "database": self.database,
            "collection": self.collection,
            "index_name": self.index_name,
            "index_type": self.index_type,
            "state": self.state,
            "phase": self.phase,
            "processed_items": self.processed_items,
            "total_items": self.total_items,
            "progress_percent": self.progress_percent,
            "created_at": self.created_at,
            "started_at": self.started_at,
            "finished_at": self.finished_at,
            "error": self.error,
        })
    }

    pub fn matches_create_index(&self, database: &str, collection: &str, index: &IndexDef) -> bool {
        self.database == database && self.collection == collection && self.index == *index
    }
}

fn new_build_id() -> String {
    nanoid::nanoid!(20)
}

fn index_type_name(index_type: IndexType) -> &'static str {
    match index_type {
        IndexType::BTree => "BTREE",
        IndexType::Hnsw => "HNSW",
        IndexType::FullText => "FULLTEXT",
    }
}

struct DurableJobStore {
    db: Arc<OptimisticTxDatabase>,
    jobs: fjall::OptimisticTxKeyspace,
}

impl DurableJobStore {
    fn open(data_dir: &Path) -> Result<Self, StorageError> {
        let path = data_dir.join("ddl_jobs");
        let db = Arc::new(OptimisticTxDatabase::builder(&path).open()?);
        let jobs = db.keyspace(JOBS_KEYSPACE, KeyspaceCreateOptions::default)?;
        Ok(Self { db, jobs })
    }

    fn load_all(&self) -> Result<HashMap<String, DdlJob>, StorageError> {
        let mut jobs = HashMap::new();
        for item in self.jobs.inner().iter() {
            let (key, value): (fjall::Slice, fjall::Slice) = item.into_inner()?;
            let job: DdlJob = serde_json::from_slice(&value)
                .map_err(|error| StorageError::Serialization(error.to_string()))?;
            let operation_id = String::from_utf8(key.to_vec())
                .map_err(|error| StorageError::Serialization(error.to_string()))?;
            jobs.insert(operation_id, job);
        }
        Ok(jobs)
    }

    fn save(&self, job: &DdlJob, durable: bool) -> Result<(), StorageError> {
        let bytes = serde_json::to_vec(job)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        self.jobs.insert(job.operation_id.as_bytes(), bytes)?;
        self.db.persist(if durable {
            PersistMode::SyncAll
        } else {
            PersistMode::Buffer
        })?;
        Ok(())
    }
}

pub struct DdlJobManager {
    namespace: Arc<Namespace>,
    admission: Arc<AdmissionController>,
    store: DurableJobStore,
    jobs: Mutex<HashMap<String, DdlJob>>,
    running: Mutex<HashSet<String>>,
}

impl DdlJobManager {
    pub fn open(
        namespace: Arc<Namespace>,
        admission: Arc<AdmissionController>,
    ) -> Result<Arc<Self>, StorageError> {
        let store = DurableJobStore::open(namespace.data_dir())?;
        let jobs = store.load_all()?;
        let manager = Arc::new(Self {
            namespace,
            admission,
            store,
            jobs: Mutex::new(jobs),
            running: Mutex::new(HashSet::new()),
        });
        manager.reconcile_job_publications()?;
        manager.reconcile_build_resources()?;
        manager.recover_unfinished()?;
        Ok(manager)
    }

    /// Resolve the crash window between durable schema publication and the
    /// durable job transition. This runs synchronously before the server can
    /// accept queries, so planner visibility and READY never diverge.
    fn reconcile_job_publications(&self) -> Result<(), StorageError> {
        let jobs: Vec<_> = self.jobs.lock().values().cloned().collect();
        for job in &jobs {
            let database = self.namespace.get_database(&job.database)?;
            let schema_contains_definition = database
                .get_indexes(&job.collection)
                .into_iter()
                .any(|index| index == job.index);
            if !schema_contains_definition {
                continue;
            }

            match job.state {
                DdlJobState::Queued | DdlJobState::Building | DdlJobState::Cancelling => {
                    self.ready(&job.operation_id)?;
                }
                DdlJobState::Failed | DdlJobState::Cancelled => {
                    let superseded_by_ready_job = jobs.iter().any(|candidate| {
                        candidate.state == DdlJobState::Ready
                            && candidate.database == job.database
                            && candidate.collection == job.collection
                            && candidate.index == job.index
                    });
                    if !superseded_by_ready_job {
                        database.cleanup_unpublished_index(&job.collection, &job.index)?;
                    }
                }
                DdlJobState::Ready => {}
            }
        }
        Ok(())
    }

    fn reconcile_build_resources(&self) -> Result<(), StorageError> {
        for database_name in self.namespace.list_databases() {
            let database = self.namespace.get_database(&database_name)?;
            database.reconcile_index_build_artifacts()?;
        }
        Ok(())
    }

    pub fn enqueue_create_index(
        self: &Arc<Self>,
        database: String,
        collection: String,
        index: IndexDef,
        total_items: usize,
    ) -> Result<(DdlJob, bool), StorageError> {
        self.enqueue_create_index_with_operation_id(None, database, collection, index, total_items)
    }

    pub fn enqueue_create_index_with_operation_id(
        self: &Arc<Self>,
        operation_id: Option<String>,
        database: String,
        collection: String,
        index: IndexDef,
        total_items: usize,
    ) -> Result<(DdlJob, bool), StorageError> {
        let job = {
            let mut jobs = self.jobs.lock();
            if let Some(operation_id) = operation_id.as_deref()
                && let Some(existing) = jobs.get(operation_id)
            {
                if existing.database == database
                    && existing.collection == collection
                    && existing.index == index
                {
                    return Ok((existing.clone(), false));
                }
                return Err(StorageError::Conflict(format!(
                    "operation_id '{operation_id}' is already bound to a different operation"
                )));
            }
            if let Some(existing) = jobs.values().find(|job| {
                job.database == database
                    && job.collection == collection
                    && job.index == index
                    && job.state.is_active()
            }) {
                return Ok((existing.clone(), false));
            }

            let job = DdlJob::new(operation_id, database, collection, index, total_items);
            self.store.save(&job, true)?;
            jobs.insert(job.operation_id.clone(), job.clone());
            job
        };
        self.spawn(job.operation_id.clone());
        Ok((job, true))
    }

    pub fn get(&self, operation_id: &str) -> Option<DdlJob> {
        self.jobs.lock().get(operation_id).cloned()
    }

    pub fn list(
        &self,
        database: Option<&str>,
        active_only: bool,
        limit: Option<usize>,
    ) -> Vec<DdlJob> {
        let mut jobs: Vec<_> = self
            .jobs
            .lock()
            .values()
            .filter(|job| database.is_none_or(|database| job.database == database))
            .filter(|job| !active_only || job.state.is_active())
            .cloned()
            .collect();
        jobs.sort_by_key(|job| std::cmp::Reverse(job.created_at));
        jobs.truncate(limit.unwrap_or(DEFAULT_LIST_LIMIT).min(MAX_LIST_LIMIT));
        jobs
    }

    pub fn cancel(&self, operation_id: &str) -> Result<DdlJob, CancelError> {
        let mut jobs = self.jobs.lock();
        let job = jobs.get_mut(operation_id).ok_or(CancelError::NotFound)?;
        match job.state {
            DdlJobState::Queued => {
                job.state = DdlJobState::Cancelled;
                job.phase = DdlJobPhase::Complete;
                job.finished_at = Some(Utc::now());
            }
            DdlJobState::Building if job.phase.cancellation_is_safe() => {
                job.state = DdlJobState::Cancelling;
            }
            DdlJobState::Cancelling | DdlJobState::Cancelled => return Ok(job.clone()),
            DdlJobState::Building => {
                return Err(CancelError::UnsafePhase(job.phase));
            }
            DdlJobState::Ready | DdlJobState::Failed => {
                return Err(CancelError::Terminal(job.state));
            }
        }
        self.store.save(job, true).map_err(CancelError::Storage)?;
        Ok(job.clone())
    }

    fn recover_unfinished(self: &Arc<Self>) -> Result<(), StorageError> {
        let operation_ids = {
            let mut jobs = self.jobs.lock();
            let mut ids = Vec::new();
            for job in jobs.values_mut() {
                match job.state {
                    DdlJobState::Queued => ids.push(job.operation_id.clone()),
                    DdlJobState::Building => {
                        job.state = DdlJobState::Queued;
                        job.phase = DdlJobPhase::Recovering;
                        self.store.save(job, true)?;
                        ids.push(job.operation_id.clone());
                    }
                    DdlJobState::Cancelling => {
                        job.phase = DdlJobPhase::Recovering;
                        self.store.save(job, true)?;
                        ids.push(job.operation_id.clone());
                    }
                    DdlJobState::Ready | DdlJobState::Failed | DdlJobState::Cancelled => {}
                }
            }
            ids
        };
        for operation_id in operation_ids {
            self.spawn(operation_id);
        }
        Ok(())
    }

    fn spawn(self: &Arc<Self>, operation_id: String) {
        if !self.running.lock().insert(operation_id.clone()) {
            return;
        }
        let manager = self.clone();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            self.running.lock().remove(&operation_id);
            tracing::warn!(%operation_id, "DDL job queued without a running Tokio runtime");
            return;
        };
        runtime.spawn(async move {
            let worker = manager.clone();
            let worker_id = operation_id.clone();
            let outcome = manager
                .admission
                .run_db_cooperative(move || worker.run_blocking(&worker_id))
                .await;
            match outcome {
                Ok(Ok(())) => {}
                Ok(Err(error)) => manager.fail(&operation_id, error.code(), error.to_string()),
                Err(error) => manager.fail(&operation_id, "SDB-ST001", error.to_string()),
            }
            manager.running.lock().remove(&operation_id);
        });
    }

    fn run_blocking(&self, operation_id: &str) -> Result<(), StorageError> {
        let snapshot = self
            .get(operation_id)
            .ok_or_else(|| StorageError::NotFound(format!("DDL operation '{operation_id}'")))?;
        if snapshot.state == DdlJobState::Cancelled {
            return Ok(());
        }

        let database = self.namespace.get_database(&snapshot.database)?;
        if let Some(existing) =
            database
                .get_indexes(&snapshot.collection)
                .into_iter()
                .find(|index| {
                    index.name == snapshot.index_name || index.fields == snapshot.index.fields
                })
        {
            if existing == snapshot.index {
                self.ready(operation_id)?;
                return Ok(());
            }
            return Err(StorageError::AlreadyExists(format!(
                "index '{}' with a different definition",
                snapshot.index_name
            )));
        }

        let _ddl_guard = database.indexes().acquire_ddl_lock();
        let current = self
            .get(operation_id)
            .ok_or_else(|| StorageError::NotFound(format!("DDL operation '{operation_id}'")))?;
        if current.state == DdlJobState::Cancelled {
            return Ok(());
        }
        // Recovery always restarts the isolated artifact. Durable schema is the
        // only publication marker, so cleanup cannot remove a READY index.
        self.update(operation_id, true, |job| {
            if job.state != DdlJobState::Cancelling {
                job.state = DdlJobState::Building;
            }
            job.phase = DdlJobPhase::CleaningUp;
            job.started_at.get_or_insert_with(Utc::now);
        })?;
        self.cleanup(
            &database,
            &current.collection,
            &current.index,
            &current.build_id,
        )?;
        if self.is_cancelling(operation_id) {
            self.cancelled(operation_id)?;
            return Ok(());
        }

        let index = prepare_create_index(
            &database,
            &snapshot.collection,
            snapshot.index.fields.clone(),
            snapshot.index.unique,
            snapshot.index.index_type,
            snapshot.index.hnsw_params.clone(),
            snapshot.index.analyzer.clone(),
        )
        .map_err(execute_to_storage)?;

        let total_items = database.count_documents(&snapshot.collection)?;
        if !self.begin_build(operation_id, total_items)? {
            self.cleanup(&database, &snapshot.collection, &index, &snapshot.build_id)?;
            self.cancelled(operation_id)?;
            return Ok(());
        }
        if let Err(error) = database.initialize_index_build_artifact(
            &snapshot.collection,
            &index,
            &snapshot.build_id,
        ) {
            self.cleanup_after_error(
                operation_id,
                &database,
                &snapshot.collection,
                &index,
                &snapshot.build_id,
                &error,
            );
            return Err(error);
        }

        let completed = match database.build_index_artifact_with_progress(
            &snapshot.collection,
            &index,
            &snapshot.build_id,
            |event| self.record_build_event(operation_id, event),
        ) {
            Ok(completed) => completed,
            Err(error) => {
                self.cleanup_after_error(
                    operation_id,
                    &database,
                    &snapshot.collection,
                    &index,
                    &snapshot.build_id,
                    &error,
                );
                return Err(error);
            }
        };
        if !completed || self.is_cancelling(operation_id) {
            self.update(operation_id, true, |job| {
                job.state = DdlJobState::Cancelling;
                job.phase = DdlJobPhase::CleaningUp;
            })?;
            self.cleanup(&database, &snapshot.collection, &index, &snapshot.build_id)?;
            self.cancelled(operation_id)?;
            return Ok(());
        }

        // The temporary artifact must survive a power loss before it is allowed
        // to acquire the final physical name.
        self.update(operation_id, true, |job| {
            job.phase = DdlJobPhase::Persisting;
        })?;
        if let Err(error) = database.sync() {
            self.cleanup_after_error(
                operation_id,
                &database,
                &snapshot.collection,
                &index,
                &snapshot.build_id,
                &error,
            );
            return Err(error);
        }
        #[cfg(feature = "fault-injection")]
        crate::fault_injection::check("ddl_after_index_flush")?;

        // This atomic state transition closes the race between the final
        // cancellable checkpoint and schema publication.
        if !self.enter_noncancellable_phase(operation_id, DdlJobPhase::Registering)? {
            self.update(operation_id, true, |job| {
                job.phase = DdlJobPhase::CleaningUp;
            })?;
            self.cleanup(&database, &snapshot.collection, &index, &snapshot.build_id)?;
            self.cancelled(operation_id)?;
            return Ok(());
        }
        if let Err(error) =
            database.promote_index_build_artifact(&snapshot.collection, &index, &snapshot.build_id)
        {
            self.cleanup_after_error(
                operation_id,
                &database,
                &snapshot.collection,
                &index,
                &snapshot.build_id,
                &error,
            );
            return Err(error);
        }
        #[cfg(feature = "fault-injection")]
        crate::fault_injection::check("ddl_after_index_promote")?;

        if let Err(error) = database.publish_ready_index(&snapshot.collection, index.clone()) {
            self.cleanup_after_error(
                operation_id,
                &database,
                &snapshot.collection,
                &index,
                &snapshot.build_id,
                &error,
            );
            return Err(error);
        }
        // READY is persisted while queries are still excluded by the DDL lock.
        if let Err(error) = self.ready(operation_id) {
            if let Err(rollback) = database.cleanup_unpublished_index(&snapshot.collection, &index)
            {
                tracing::error!(
                    %operation_id,
                    error = %error,
                    rollback_error = %rollback,
                    "failed to roll back publication after READY persistence failed"
                );
            }
            return Err(error);
        }
        if let Err(error) =
            database.cleanup_index_build_artifact(&snapshot.collection, &index, &snapshot.build_id)
        {
            tracing::warn!(%operation_id, %error, "published index build cleanup deferred to startup");
        }
        Ok(())
    }

    fn record_build_event(&self, operation_id: &str, event: IndexBuildEvent) -> bool {
        if self.is_cancelling(operation_id) {
            return false;
        }
        let result = self.update(operation_id, false, |job| match event {
            IndexBuildEvent::Building { processed_items } => {
                job.phase = DdlJobPhase::Building;
                job.processed_items = processed_items;
                job.refresh_progress();
            }
            IndexBuildEvent::Committing => job.phase = DdlJobPhase::Committing,
            IndexBuildEvent::ValidatingUnique => job.phase = DdlJobPhase::ValidatingUnique,
        });
        if let Err(error) = result {
            tracing::error!(%operation_id, %error, "failed to persist DDL progress");
            return false;
        }
        !self.is_cancelling(operation_id)
    }

    fn cleanup(
        &self,
        database: &crate::storage::Database,
        collection: &str,
        index: &IndexDef,
        build_id: &str,
    ) -> Result<(), StorageError> {
        database.cleanup_index_build_artifact(collection, index, build_id)?;
        database.cleanup_unpublished_index(collection, index)
    }

    fn cleanup_after_error(
        &self,
        operation_id: &str,
        database: &crate::storage::Database,
        collection: &str,
        index: &IndexDef,
        build_id: &str,
        primary: &StorageError,
    ) {
        let _ = self.update(operation_id, true, |job| {
            job.phase = DdlJobPhase::CleaningUp;
        });
        if let Err(cleanup) = self.cleanup(database, collection, index, build_id) {
            tracing::error!(
                %operation_id,
                error = %primary,
                cleanup_error = %cleanup,
                "failed to clean up an unsuccessful DDL build"
            );
        }
    }

    fn enter_noncancellable_phase(
        &self,
        operation_id: &str,
        phase: DdlJobPhase,
    ) -> Result<bool, StorageError> {
        let mut jobs = self.jobs.lock();
        let job = jobs
            .get_mut(operation_id)
            .ok_or_else(|| StorageError::NotFound(format!("DDL operation '{operation_id}'")))?;
        if job.state == DdlJobState::Cancelling {
            return Ok(false);
        }
        job.phase = phase;
        self.store.save(job, true)?;
        Ok(true)
    }

    fn begin_build(&self, operation_id: &str, total_items: usize) -> Result<bool, StorageError> {
        let mut jobs = self.jobs.lock();
        let job = jobs
            .get_mut(operation_id)
            .ok_or_else(|| StorageError::NotFound(format!("DDL operation '{operation_id}'")))?;
        if job.state == DdlJobState::Cancelling {
            return Ok(false);
        }
        job.state = DdlJobState::Building;
        job.phase = DdlJobPhase::Initializing;
        job.started_at.get_or_insert_with(Utc::now);
        job.total_items = total_items;
        job.processed_items = 0;
        job.error = None;
        job.refresh_progress();
        self.store.save(job, true)?;
        Ok(true)
    }

    fn is_cancelling(&self, operation_id: &str) -> bool {
        self.jobs
            .lock()
            .get(operation_id)
            .is_some_and(|job| job.state == DdlJobState::Cancelling)
    }

    fn ready(&self, operation_id: &str) -> Result<(), StorageError> {
        self.update(operation_id, true, |job| {
            job.state = DdlJobState::Ready;
            job.phase = DdlJobPhase::Complete;
            job.processed_items = job.total_items;
            job.finished_at = Some(Utc::now());
            job.refresh_progress();
        })
    }

    fn cancelled(&self, operation_id: &str) -> Result<(), StorageError> {
        self.update(operation_id, true, |job| {
            job.state = DdlJobState::Cancelled;
            job.phase = DdlJobPhase::Complete;
            job.finished_at = Some(Utc::now());
        })
    }

    fn fail(&self, operation_id: &str, code: impl Into<String>, message: impl Into<String>) {
        let code = code.into();
        let message = message.into();
        if let Err(error) = self.update(operation_id, true, |job| {
            if matches!(job.state, DdlJobState::Ready | DdlJobState::Cancelled) {
                return;
            }
            job.state = DdlJobState::Failed;
            job.phase = DdlJobPhase::Complete;
            job.finished_at = Some(Utc::now());
            job.error = Some(DdlJobError { code, message });
        }) {
            tracing::error!(%operation_id, %error, "failed to persist DDL failure");
        }
    }

    fn update(
        &self,
        operation_id: &str,
        durable: bool,
        update: impl FnOnce(&mut DdlJob),
    ) -> Result<(), StorageError> {
        let mut jobs = self.jobs.lock();
        let job = jobs
            .get_mut(operation_id)
            .ok_or_else(|| StorageError::NotFound(format!("DDL operation '{operation_id}'")))?;
        update(job);
        self.store.save(job, durable)
    }
}

fn execute_to_storage(error: ExecuteError) -> StorageError {
    match error {
        ExecuteError::Storage(error) => error,
        other => StorageError::InvalidOperation(other.to_string()),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CancelError {
    #[error("operation not found")]
    NotFound,
    #[error("operation cannot be cancelled during {0:?}")]
    UnsafePhase(DdlJobPhase),
    #[error("operation is already terminal ({0:?})")]
    Terminal(DdlJobState),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, Value};
    use crate::storage::WriteOp;

    fn manager_at(path: &Path) -> Arc<DdlJobManager> {
        let namespace = Arc::new(Namespace::open(path).unwrap());
        if !namespace.database_exists("test") {
            namespace.create_database("test").unwrap();
        }
        let database = namespace.get_database("test").unwrap();
        database.get_or_create_collection("items").unwrap();
        let admission = Arc::new(AdmissionController::new(Default::default()));
        DdlJobManager::open(namespace, admission).unwrap()
    }

    #[test]
    fn registration_is_durable_deduplicated_and_queue_cancellable() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = manager_at(tmp.path());
        let index = IndexDef::new("items", vec!["category".to_string()]);

        let (first, created) = manager
            .enqueue_create_index("test".to_string(), "items".to_string(), index.clone(), 12)
            .unwrap();
        assert!(created);
        let (duplicate, created) = manager
            .enqueue_create_index("test".to_string(), "items".to_string(), index, 12)
            .unwrap();
        assert!(!created);
        assert_eq!(duplicate.operation_id, first.operation_id);

        let cancelled = manager.cancel(&first.operation_id).unwrap();
        assert_eq!(cancelled.state, DdlJobState::Cancelled);
        drop(manager);

        let reopened = manager_at(tmp.path());
        let recovered = reopened.get(&first.operation_id).unwrap();
        assert_eq!(recovered.state, DdlJobState::Cancelled);
        assert_eq!(recovered.index.fields, vec!["category"]);
        assert!(recovered.finished_at.is_some());
    }

    #[test]
    fn operation_id_is_bound_to_definition_and_failed_retry_is_safe() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = manager_at(tmp.path());
        let index = IndexDef::new("items", vec!["category".to_string()]);
        let operation_id = "client-create-category".to_string();

        let (first, created) = manager
            .enqueue_create_index_with_operation_id(
                Some(operation_id.clone()),
                "test".to_string(),
                "items".to_string(),
                index.clone(),
                12,
            )
            .unwrap();
        assert!(created);
        manager.fail(&first.operation_id, "TEST", "injected failure");

        let (replayed, created) = manager
            .enqueue_create_index_with_operation_id(
                Some(operation_id.clone()),
                "test".to_string(),
                "items".to_string(),
                index.clone(),
                12,
            )
            .unwrap();
        assert!(!created);
        assert_eq!(replayed.state, DdlJobState::Failed);
        assert_eq!(replayed.operation_id, operation_id);

        let different = IndexDef::new("items", vec!["other".to_string()]);
        assert!(matches!(
            manager.enqueue_create_index_with_operation_id(
                Some(operation_id),
                "test".to_string(),
                "items".to_string(),
                different,
                12,
            ),
            Err(StorageError::Conflict(_))
        ));

        let (retry, created) = manager
            .enqueue_create_index_with_operation_id(
                Some("client-create-category-retry".to_string()),
                "test".to_string(),
                "items".to_string(),
                index,
                12,
            )
            .unwrap();
        assert!(created);
        assert_ne!(retry.build_id, first.build_id);
    }

    #[test]
    fn cancellation_is_rejected_after_commit_boundary() {
        assert!(DdlJobPhase::Building.cancellation_is_safe());
        assert!(DdlJobPhase::ValidatingUnique.cancellation_is_safe());
        assert!(!DdlJobPhase::Committing.cancellation_is_safe());
        assert!(!DdlJobPhase::Registering.cancellation_is_safe());
        assert!(!DdlJobPhase::Persisting.cancellation_is_safe());
    }

    #[test]
    fn build_artifact_is_invisible_until_durable_publication() {
        let tmp = tempfile::tempdir().unwrap();
        let namespace = Namespace::open(tmp.path()).unwrap();
        let database = namespace.create_database("test").unwrap();
        database.get_or_create_collection("items").unwrap();
        database
            .atomic_write_batch(vec![WriteOp::InsertDoc(Document {
                id: "items:one".to_string(),
                fields: [("category".to_string(), Value::String("books".into()))]
                    .into_iter()
                    .collect(),
            })])
            .unwrap();

        let index = IndexDef::new("items", vec!["category".to_string()]);
        let build_id = "visibility-test";
        database
            .initialize_index_build_artifact("items", &index, build_id)
            .unwrap();
        assert!(
            database
                .build_index_artifact_with_progress("items", &index, build_id, |_| true)
                .unwrap()
        );
        database.sync().unwrap();
        assert!(!database.has_index("items", &index.fields));

        database
            .promote_index_build_artifact("items", &index, build_id)
            .unwrap();
        assert!(!database.has_index("items", &index.fields));

        database
            .publish_ready_index("items", index.clone())
            .unwrap();
        assert!(database.has_index("items", &index.fields));
        database
            .cleanup_index_build_artifact("items", &index, build_id)
            .unwrap();
    }

    #[test]
    fn startup_reconciliation_deletes_orphaned_build_keyspaces() {
        let tmp = tempfile::tempdir().unwrap();
        let namespace = Namespace::open(tmp.path()).unwrap();
        let database = namespace.create_database("test").unwrap();
        database.get_or_create_collection("items").unwrap();
        let index = IndexDef::new("items", vec!["category".to_string()]);
        database
            .initialize_index_build_artifact("items", &index, "orphan-test")
            .unwrap();
        database.sync().unwrap();
        database.reconcile_index_build_artifacts().unwrap();
        drop(database);
        drop(namespace);

        let raw = OptimisticTxDatabase::builder(
            tmp.path().join("databases").join("test").join("storage"),
        )
        .open()
        .unwrap();
        assert!(
            raw.list_keyspace_names()
                .into_iter()
                .all(|name| { !name.to_string().contains("__build_") })
        );
    }

    #[tokio::test]
    async fn interrupted_build_is_recovered_from_durable_definition() {
        let tmp = tempfile::tempdir().unwrap();
        let namespace = Arc::new(Namespace::open(tmp.path()).unwrap());
        let database = namespace.create_database("test").unwrap();
        database.get_or_create_collection("items").unwrap();
        database
            .atomic_write_batch(
                (0..20)
                    .map(|number| {
                        WriteOp::InsertDoc(Document {
                            id: format!("items:{number}"),
                            fields: [("category".to_string(), Value::Int(number))]
                                .into_iter()
                                .collect(),
                        })
                    })
                    .collect(),
            )
            .unwrap();

        let index = IndexDef::new("items", vec!["category".to_string()]);
        let mut interrupted = DdlJob::new(
            None,
            "test".to_string(),
            "items".to_string(),
            index.clone(),
            20,
        );
        interrupted.state = DdlJobState::Building;
        interrupted.phase = DdlJobPhase::Building;
        interrupted.started_at = Some(Utc::now());
        let store = DurableJobStore::open(tmp.path()).unwrap();
        store.save(&interrupted, true).unwrap();
        drop(store);

        let admission = Arc::new(AdmissionController::new(Default::default()));
        let manager = DdlJobManager::open(namespace, admission).unwrap();
        let mut recovered = None;
        for _ in 0..200 {
            let job = manager.get(&interrupted.operation_id).unwrap();
            if !job.state.is_active() {
                recovered = Some(job);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let recovered = recovered.expect("recovered DDL job did not finish");
        assert_eq!(recovered.state, DdlJobState::Ready, "{recovered:?}");
        assert_eq!(recovered.processed_items, 20);
        assert!(database.get_indexes("items").contains(&index));
    }

    #[test]
    fn startup_finishes_job_when_durable_schema_is_the_commit_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let namespace = Arc::new(Namespace::open(tmp.path()).unwrap());
        let database = namespace.create_database("test").unwrap();
        database.get_or_create_collection("items").unwrap();
        let index = IndexDef::new("items", vec!["category".to_string()]);
        database
            .publish_ready_index("items", index.clone())
            .unwrap();

        let mut interrupted = DdlJob::new(
            None,
            "test".to_string(),
            "items".to_string(),
            index.clone(),
            0,
        );
        interrupted.state = DdlJobState::Building;
        interrupted.phase = DdlJobPhase::Registering;
        let store = DurableJobStore::open(tmp.path()).unwrap();
        store.save(&interrupted, true).unwrap();
        drop(store);

        let admission = Arc::new(AdmissionController::new(Default::default()));
        let manager = DdlJobManager::open(namespace, admission).unwrap();
        let reconciled = manager.get(&interrupted.operation_id).unwrap();
        assert_eq!(reconciled.state, DdlJobState::Ready);
        assert_eq!(reconciled.phase, DdlJobPhase::Complete);
        assert!(database.has_index("items", &index.fields));
    }
}
