use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
#[cfg(test)]
use std::future::Future;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const OPERATION_RETENTION: Duration = Duration::from_secs(60 * 60);
const MAX_RETAINED_OPERATIONS: usize = 10_000;

#[derive(Debug, Clone)]
pub struct AdmissionConfig {
    pub max_db_concurrency: usize,
    pub db_deadline: Duration,
    pub max_login_concurrency: usize,
    pub login_rate_per_second: f64,
    pub login_burst: usize,
    pub login_client_rate_per_second: f64,
    pub login_client_burst: usize,
    pub max_login_clients: usize,
    pub login_client_retention: Duration,
    pub query_client_rate_per_second: f64,
    pub query_client_burst: usize,
    pub max_query_clients: usize,
    pub query_client_retention: Duration,
}

impl Default for AdmissionConfig {
    fn default() -> Self {
        Self {
            max_db_concurrency: 64,
            db_deadline: Duration::from_secs(60),
            max_login_concurrency: 4,
            login_rate_per_second: 10.0,
            login_burst: 20,
            login_client_rate_per_second: 5.0,
            login_client_burst: 10,
            max_login_clients: 10_000,
            login_client_retention: Duration::from_secs(10 * 60),
            query_client_rate_per_second: 0.0,
            query_client_burst: 20,
            max_query_clients: 10_000,
            query_client_retention: Duration::from_secs(10 * 60),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AdmissionError {
    #[error("request rate limit exceeded")]
    RateLimited,
    #[error("server is at its concurrency limit")]
    Overloaded,
    #[error("operation continues in the background ({0})")]
    OperationRunning(String),
    #[error("blocking operation failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

#[derive(Debug, Clone)]
pub enum OperationOutcome {
    Completed { result: Option<serde_json::Value> },
    Failed { code: String, message: String },
}

#[derive(Debug, Clone)]
pub enum OperationStatus {
    Running,
    Completed { result: Option<serde_json::Value> },
    Failed { code: String, message: String },
}

#[derive(Debug, Clone)]
pub struct OperationScope {
    pub operation_type: String,
    pub database: String,
}

impl OperationScope {
    pub fn ddl(database: impl Into<String>) -> Self {
        Self {
            operation_type: "DDL".to_string(),
            database: database.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TrackedOperationSnapshot {
    pub operation_id: String,
    pub operation_type: String,
    pub database: String,
    pub status: OperationStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

struct TrackedOperation {
    scope: OperationScope,
    status: OperationStatus,
    created_at: chrono::DateTime<chrono::Utc>,
    finished_at: Option<chrono::DateTime<chrono::Utc>>,
    updated_at: Instant,
}

struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
struct ClientOrder {
    last_seen: Instant,
    generation: u64,
    client: IpAddr,
}

struct ClientBucket {
    bucket: TokenBucket,
    last_seen: Instant,
    generation: u64,
}

#[derive(Default)]
struct ClientLimiterState {
    buckets: HashMap<IpAddr, ClientBucket>,
    order: BinaryHeap<Reverse<ClientOrder>>,
    next_generation: u64,
}

pub struct AdmissionController {
    config: AdmissionConfig,
    db: Arc<Semaphore>,
    login: Arc<Semaphore>,
    login_bucket: Mutex<TokenBucket>,
    login_clients: Mutex<ClientLimiterState>,
    query_clients: Mutex<ClientLimiterState>,
    operations: Arc<Mutex<HashMap<String, TrackedOperation>>>,
}

impl AdmissionController {
    pub fn new(config: AdmissionConfig) -> Self {
        Self {
            db: Arc::new(Semaphore::new(config.max_db_concurrency)),
            login: Arc::new(Semaphore::new(config.max_login_concurrency)),
            login_bucket: Mutex::new(TokenBucket {
                tokens: config.login_burst as f64,
                last_refill: Instant::now(),
            }),
            login_clients: Mutex::new(ClientLimiterState::default()),
            query_clients: Mutex::new(ClientLimiterState::default()),
            operations: Arc::new(Mutex::new(HashMap::new())),
            config,
        }
    }

    pub fn db_deadline(&self) -> Duration {
        self.config.db_deadline
    }

    pub fn operation_status(&self, operation_id: &str) -> Option<OperationStatus> {
        self.operation(operation_id)
            .map(|operation| operation.status)
    }

    pub fn operation(&self, operation_id: &str) -> Option<TrackedOperationSnapshot> {
        let mut operations = self.operations.lock();
        prune_operations(&mut operations, Instant::now());
        operations
            .get(operation_id)
            .map(|operation| operation_snapshot(operation_id, operation))
    }

    pub fn list_operations(
        &self,
        database: &str,
        active_only: bool,
        limit: Option<usize>,
    ) -> Vec<TrackedOperationSnapshot> {
        let mut operations = self.operations.lock();
        prune_operations(&mut operations, Instant::now());
        let mut result: Vec<_> = operations
            .iter()
            .filter(|(_, operation)| operation.scope.database == database)
            .filter(|(_, operation)| {
                !active_only || matches!(operation.status, OperationStatus::Running)
            })
            .map(|(id, operation)| operation_snapshot(id, operation))
            .collect();
        result.sort_by_key(|operation| std::cmp::Reverse(operation.created_at));
        result.truncate(limit.unwrap_or(100).min(1_000));
        result
    }

    pub fn try_acquire_login(&self) -> Result<OwnedSemaphorePermit, AdmissionError> {
        self.try_acquire_login_for(None)
    }

    pub fn try_acquire_login_for(
        &self,
        client: Option<IpAddr>,
    ) -> Result<OwnedSemaphorePermit, AdmissionError> {
        if let Some(client) = client
            && !self.try_take_client_login_token(client)
        {
            return Err(AdmissionError::RateLimited);
        }
        if !self.try_take_login_token() {
            return Err(AdmissionError::RateLimited);
        }
        self.login
            .clone()
            .try_acquire_owned()
            .map_err(|_| AdmissionError::Overloaded)
    }

    pub fn try_acquire_query_for(&self, client: Option<IpAddr>) -> Result<(), AdmissionError> {
        let Some(client) = client else {
            return Ok(());
        };
        if self.config.query_client_rate_per_second == 0.0 || self.config.max_query_clients == 0 {
            return Ok(());
        }
        if self.try_take_client_query_token(client) {
            Ok(())
        } else {
            Err(AdmissionError::RateLimited)
        }
    }

    pub async fn run_db<F, T>(&self, operation: F) -> Result<T, AdmissionError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.run_db_cooperative(operation).await
    }

    /// Run work that owns an absolute cooperative deadline. The future waits
    /// for the blocking task to acknowledge that deadline; dropping a
    /// `JoinHandle` is deliberately not treated as cancellation.
    pub async fn run_db_cooperative<F, T>(&self, operation: F) -> Result<T, AdmissionError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let permit = self
            .db
            .clone()
            .try_acquire_owned()
            .map_err(|_| AdmissionError::Overloaded)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation()
        })
        .await
        .map_err(AdmissionError::Join)
    }

    /// Run blocking DB work with an externally visible background-operation fallback.
    ///
    /// `spawn_blocking` cannot be cancelled safely. If `timeout` expires, this method
    /// therefore retains the join handle and returns an operation id instead of
    /// pretending that the operation failed. `completion` supplies the terminal
    /// representation exposed by the operation-status endpoint.
    pub async fn run_db_with_timeout<F, T, C>(
        &self,
        timeout: Duration,
        scope: OperationScope,
        operation: F,
        completion: C,
    ) -> Result<T, AdmissionError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
        C: FnOnce(&T) -> OperationOutcome + Send + 'static,
    {
        let permit = self
            .db
            .clone()
            .try_acquire_owned()
            .map_err(|_| AdmissionError::Overloaded)?;
        let mut task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation()
        });

        match tokio::time::timeout(timeout, &mut task).await {
            Ok(result) => result.map_err(AdmissionError::Join),
            Err(_) => {
                let operation_id = format!("op_{}", nanoid::nanoid!(20));
                {
                    let now = Instant::now();
                    let mut operations = self.operations.lock();
                    prune_operations(&mut operations, now);
                    if operations.len() >= MAX_RETAINED_OPERATIONS {
                        evict_oldest_operation(&mut operations);
                    }
                    operations.insert(
                        operation_id.clone(),
                        TrackedOperation {
                            scope,
                            status: OperationStatus::Running,
                            created_at: chrono::Utc::now(),
                            finished_at: None,
                            updated_at: now,
                        },
                    );
                }

                let operations = self.operations.clone();
                let tracked_id = operation_id.clone();
                tokio::spawn(async move {
                    let status = match task.await {
                        Ok(value) => match completion(&value) {
                            OperationOutcome::Completed { result } => {
                                OperationStatus::Completed { result }
                            }
                            OperationOutcome::Failed { code, message } => {
                                OperationStatus::Failed { code, message }
                            }
                        },
                        Err(error) => OperationStatus::Failed {
                            code: "SDB-ST001".to_string(),
                            message: format!("blocking operation failed: {error}"),
                        },
                    };
                    if let Some(operation) = operations.lock().get_mut(&tracked_id) {
                        operation.status = status;
                        operation.finished_at = Some(chrono::Utc::now());
                        operation.updated_at = Instant::now();
                    }
                });

                Err(AdmissionError::OperationRunning(operation_id))
            }
        }
    }

    pub async fn run_login<F, T>(
        &self,
        permit: OwnedSemaphorePermit,
        operation: F,
    ) -> Result<T, AdmissionError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation()
        });
        task.await.map_err(AdmissionError::Join)
    }

    fn try_take_login_token(&self) -> bool {
        take_token(
            &mut self.login_bucket.lock(),
            self.config.login_rate_per_second,
            self.config.login_burst,
            Instant::now(),
        )
    }

    fn try_take_client_login_token(&self, client: IpAddr) -> bool {
        if self.config.max_login_clients == 0 {
            return true;
        }
        let now = Instant::now();
        let mut state = self.login_clients.lock();
        prune_client_buckets(&mut state, now, self.config.login_client_retention);

        if state.buckets.contains_key(&client) {
            let allowed = {
                let entry = state.buckets.get_mut(&client).expect("entry exists");
                take_token(
                    &mut entry.bucket,
                    self.config.login_client_rate_per_second,
                    self.config.login_client_burst,
                    now,
                )
            };
            touch_client_bucket(&mut state, client, now);
            return allowed;
        }

        while state.buckets.len() >= self.config.max_login_clients {
            if !evict_oldest_client_bucket(&mut state) {
                return false;
            }
        }
        state.buckets.insert(
            client,
            ClientBucket {
                bucket: TokenBucket {
                    tokens: self.config.login_client_burst.saturating_sub(1) as f64,
                    last_refill: now,
                },
                last_seen: now,
                generation: 0,
            },
        );
        touch_client_bucket(&mut state, client, now);
        true
    }

    fn try_take_client_query_token(&self, client: IpAddr) -> bool {
        let now = Instant::now();
        let mut state = self.query_clients.lock();
        prune_client_buckets(&mut state, now, self.config.query_client_retention);

        if state.buckets.contains_key(&client) {
            let allowed = {
                let entry = state.buckets.get_mut(&client).expect("entry exists");
                take_token(
                    &mut entry.bucket,
                    self.config.query_client_rate_per_second,
                    self.config.query_client_burst,
                    now,
                )
            };
            touch_client_bucket(&mut state, client, now);
            return allowed;
        }

        while state.buckets.len() >= self.config.max_query_clients {
            if !evict_oldest_client_bucket(&mut state) {
                return false;
            }
        }
        state.buckets.insert(
            client,
            ClientBucket {
                bucket: TokenBucket {
                    tokens: self.config.query_client_burst.saturating_sub(1) as f64,
                    last_refill: now,
                },
                last_seen: now,
                generation: 0,
            },
        );
        touch_client_bucket(&mut state, client, now);
        true
    }

    #[cfg(test)]
    async fn hold_db_permit_until<F: Future<Output = ()>>(
        &self,
        ready: tokio::sync::oneshot::Sender<()>,
        release: F,
    ) {
        let permit = self.db.clone().acquire_owned().await.unwrap();
        let _ = ready.send(());
        release.await;
        drop(permit);
    }
}

fn take_token(bucket: &mut TokenBucket, rate: f64, burst: usize, now: Instant) -> bool {
    let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
    bucket.last_refill = now;
    bucket.tokens = (bucket.tokens + elapsed * rate).min(burst as f64);
    if bucket.tokens < 1.0 {
        false
    } else {
        bucket.tokens -= 1.0;
        true
    }
}

fn touch_client_bucket(state: &mut ClientLimiterState, client: IpAddr, now: Instant) {
    state.next_generation = state.next_generation.wrapping_add(1);
    if state.next_generation == 0 {
        rebuild_client_order(state);
        state.next_generation = state.buckets.len() as u64 + 1;
    }
    let generation = state.next_generation;
    if let Some(entry) = state.buckets.get_mut(&client) {
        entry.last_seen = now;
        entry.generation = generation;
    }
    state.order.push(Reverse(ClientOrder {
        last_seen: now,
        generation,
        client,
    }));
    compact_client_order_if_needed(state);
}

fn prune_client_buckets(state: &mut ClientLimiterState, now: Instant, retention: Duration) {
    while let Some(Reverse(candidate)) = state.order.peek().cloned() {
        let current = state.buckets.get(&candidate.client);
        if current.is_none_or(|entry| entry.generation != candidate.generation) {
            state.order.pop();
            continue;
        }
        if now.duration_since(candidate.last_seen) < retention {
            break;
        }
        state.order.pop();
        state.buckets.remove(&candidate.client);
    }
    compact_client_order_if_needed(state);
}

fn evict_oldest_client_bucket(state: &mut ClientLimiterState) -> bool {
    while let Some(Reverse(candidate)) = state.order.pop() {
        if state
            .buckets
            .get(&candidate.client)
            .is_none_or(|entry| entry.generation != candidate.generation)
        {
            continue;
        }
        state.buckets.remove(&candidate.client);
        return true;
    }
    false
}

fn compact_client_order_if_needed(state: &mut ClientLimiterState) {
    let threshold = state.buckets.len().saturating_mul(4).max(64);
    if state.order.len() > threshold {
        rebuild_client_order(state);
    }
}

fn rebuild_client_order(state: &mut ClientLimiterState) {
    state.order = state
        .buckets
        .iter()
        .map(|(&client, entry)| {
            Reverse(ClientOrder {
                last_seen: entry.last_seen,
                generation: entry.generation,
                client,
            })
        })
        .collect();
}

fn prune_operations(operations: &mut HashMap<String, TrackedOperation>, now: Instant) {
    operations.retain(|_, operation| {
        matches!(operation.status, OperationStatus::Running)
            || now.duration_since(operation.updated_at) < OPERATION_RETENTION
    });
}

fn operation_snapshot(
    operation_id: &str,
    operation: &TrackedOperation,
) -> TrackedOperationSnapshot {
    TrackedOperationSnapshot {
        operation_id: operation_id.to_string(),
        operation_type: operation.scope.operation_type.clone(),
        database: operation.scope.database.clone(),
        status: operation.status.clone(),
        created_at: operation.created_at,
        finished_at: operation.finished_at,
    }
}

fn evict_oldest_operation(operations: &mut HashMap<String, TrackedOperation>) {
    let oldest = operations
        .iter()
        .filter(|(_, operation)| !matches!(operation.status, OperationStatus::Running))
        .min_by_key(|(_, operation)| operation.updated_at)
        .map(|(id, _)| id.clone());
    if let Some(id) = oldest {
        operations.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> AdmissionConfig {
        AdmissionConfig {
            max_db_concurrency: 1,
            db_deadline: Duration::from_millis(20),
            max_login_concurrency: 1,
            login_rate_per_second: 0.0,
            login_burst: 2,
            login_client_rate_per_second: 0.0,
            login_client_burst: 1,
            max_login_clients: 2,
            login_client_retention: Duration::from_secs(60),
            query_client_rate_per_second: 0.0,
            query_client_burst: 1,
            max_query_clients: 2,
            query_client_retention: Duration::from_secs(60),
        }
    }

    #[tokio::test]
    async fn db_concurrency_is_bounded() {
        let controller = Arc::new(AdmissionController::new(config()));
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let held = controller.clone();
        let holder = tokio::spawn(async move {
            held.hold_db_permit_until(ready_tx, async {
                let _ = release_rx.await;
            })
            .await;
        });
        ready_rx.await.unwrap();

        let error = controller.run_db(|| ()).await.unwrap_err();
        assert!(matches!(error, AdmissionError::Overloaded));
        let _ = release_tx.send(());
        holder.await.unwrap();
    }

    #[tokio::test]
    async fn non_cancellable_db_deadline_returns_trackable_operation() {
        let controller = AdmissionController::new(config());
        let error = controller
            .run_db_with_timeout(
                Duration::from_millis(20),
                OperationScope::ddl("test"),
                || std::thread::sleep(Duration::from_millis(100)),
                |_| OperationOutcome::Completed { result: None },
            )
            .await
            .unwrap_err();
        let AdmissionError::OperationRunning(operation_id) = error else {
            panic!("expected a running operation id");
        };
        let operation = controller.operation(&operation_id).unwrap();
        assert_eq!(operation.database, "test");
        assert_eq!(operation.operation_type, "DDL");
        assert!(matches!(
            controller.operation_status(&operation_id),
            Some(OperationStatus::Running)
        ));
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(matches!(
            controller.operation_status(&operation_id),
            Some(OperationStatus::Completed { .. })
        ));
    }

    #[tokio::test]
    async fn cooperative_db_work_is_joined_not_detached() {
        let controller = AdmissionController::new(config());
        let started = Instant::now();
        controller
            .run_db_cooperative(|| std::thread::sleep(Duration::from_millis(40)))
            .await
            .unwrap();
        assert!(started.elapsed() >= Duration::from_millis(40));
    }

    #[test]
    fn per_client_login_rate_isolated_and_bounded() {
        let controller = AdmissionController::new(AdmissionConfig {
            max_login_concurrency: 2,
            login_rate_per_second: 0.0,
            login_burst: 10,
            ..config()
        });
        let first: IpAddr = "192.0.2.1".parse().unwrap();
        let second: IpAddr = "192.0.2.2".parse().unwrap();
        drop(controller.try_acquire_login_for(Some(first)).unwrap());
        assert!(matches!(
            controller.try_acquire_login_for(Some(first)).unwrap_err(),
            AdmissionError::RateLimited
        ));
        drop(controller.try_acquire_login_for(Some(second)).unwrap());
        assert_eq!(controller.login_clients.lock().buckets.len(), 2);

        let third: IpAddr = "192.0.2.3".parse().unwrap();
        drop(controller.try_acquire_login_for(Some(third)).unwrap());
        let state = controller.login_clients.lock();
        assert_eq!(state.buckets.len(), 2);
        assert!(state.order.len() <= 64);
    }

    #[test]
    fn per_client_query_rate_isolated_and_bounded() {
        let controller = AdmissionController::new(AdmissionConfig {
            query_client_rate_per_second: f64::EPSILON,
            ..config()
        });
        let first: IpAddr = "192.0.2.1".parse().unwrap();
        let second: IpAddr = "192.0.2.2".parse().unwrap();
        controller.try_acquire_query_for(Some(first)).unwrap();
        assert!(matches!(
            controller.try_acquire_query_for(Some(first)).unwrap_err(),
            AdmissionError::RateLimited
        ));
        controller.try_acquire_query_for(Some(second)).unwrap();

        let third: IpAddr = "192.0.2.3".parse().unwrap();
        controller.try_acquire_query_for(Some(third)).unwrap();
        let state = controller.query_clients.lock();
        assert_eq!(state.buckets.len(), 2);
        assert!(state.order.len() <= 64);
    }

    #[test]
    fn repeated_client_updates_do_not_create_unbounded_eviction_metadata() {
        let controller = AdmissionController::new(AdmissionConfig {
            login_client_rate_per_second: 1_000_000.0,
            login_client_burst: 10_000,
            login_rate_per_second: 1_000_000.0,
            login_burst: 10_000,
            max_login_concurrency: 1,
            ..config()
        });
        let client: IpAddr = "192.0.2.1".parse().unwrap();
        for _ in 0..1_000 {
            drop(controller.try_acquire_login_for(Some(client)).unwrap());
        }
        let state = controller.login_clients.lock();
        assert_eq!(state.buckets.len(), 1);
        assert!(state.order.len() <= 64);
    }

    #[test]
    fn login_rate_and_concurrency_are_bounded() {
        let controller = AdmissionController::new(config());
        let permit = controller.try_acquire_login().unwrap();
        assert!(matches!(
            controller.try_acquire_login().unwrap_err(),
            AdmissionError::Overloaded
        ));
        drop(permit);
        assert!(matches!(
            controller.try_acquire_login().unwrap_err(),
            AdmissionError::RateLimited
        ));
    }
}
