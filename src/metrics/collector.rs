use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use parking_lot::{Mutex, RwLock};

use super::config::MetricsConfig;
use super::normalize::normalize_query;

/// Sliding window rate tracker using a fixed-size ring buffer.
/// Each bucket holds a count for one second, indexed by epoch_second % WINDOW.
const RATE_WINDOW_SECS: usize = 60;

struct RateWindow {
    /// Query count per bucket
    counts: [u64; RATE_WINDOW_SECS],
    /// Epoch second each bucket represents (0 = unused)
    epochs: [u64; RATE_WINDOW_SECS],
}

impl RateWindow {
    fn new() -> Self {
        Self {
            counts: [0; RATE_WINDOW_SECS],
            epochs: [0; RATE_WINDOW_SECS],
        }
    }

    fn increment(&mut self) {
        let now = epoch_secs();
        let idx = now as usize % RATE_WINDOW_SECS;
        if self.epochs[idx] == now {
            self.counts[idx] += 1;
        } else {
            self.epochs[idx] = now;
            self.counts[idx] = 1;
        }
    }

    fn rate(&self) -> f64 {
        let now = epoch_secs();
        let cutoff = now.saturating_sub(RATE_WINDOW_SECS as u64 - 1);
        let total: u64 = self
            .counts
            .iter()
            .zip(self.epochs.iter())
            .filter(|&(_, &epoch)| epoch >= cutoff && epoch <= now)
            .map(|(&count, _)| count)
            .sum();
        total as f64 / RATE_WINDOW_SECS as f64
    }
}

fn epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn retention_cutoff(now: DateTime<Utc>, retention_days: u64) -> DateTime<Utc> {
    let max_days = i64::MAX / 86_400;
    let days = i64::try_from(retention_days)
        .unwrap_or(max_days)
        .min(max_days);
    now.checked_sub_signed(chrono::Duration::days(days))
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

/// Accumulated statistics for a normalized query
#[derive(Debug, Clone)]
pub struct QueryStatsAccum {
    pub query_hash: u64,
    pub query_normalized: String,
    pub query_example: String,
    pub query_display: String,
    pub query_variants: Vec<String>,
    pub statement_kind: String,
    pub risk: String,
    pub redaction_count: u64,
    pub sanitizer_version: u16,
    pub call_count: u64,
    pub total_time_us: u64,
    pub min_time_us: u64,
    pub max_time_us: u64,
    pub rows_returned: u64,
    pub rows_affected: u64,
    pub errors: u64,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
struct QueryOrder {
    last_seen: DateTime<Utc>,
    generation: u64,
    query_hash: u64,
}

#[derive(Default)]
struct QueryStatsState {
    stats: HashMap<u64, QueryStatsAccum>,
    generations: HashMap<u64, u64>,
    order: BinaryHeap<Reverse<QueryOrder>>,
    next_generation: u64,
    #[cfg(test)]
    rebuild_count: usize,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
struct NamedStatsOrder {
    last_seen: DateTime<Utc>,
    generation: u64,
    key: String,
}

struct NamedStatsState<T> {
    stats: HashMap<String, T>,
    generations: HashMap<String, u64>,
    last_seen: HashMap<String, DateTime<Utc>>,
    order: BinaryHeap<Reverse<NamedStatsOrder>>,
    next_generation: u64,
    #[cfg(test)]
    rebuild_count: usize,
}

impl<T> Default for NamedStatsState<T> {
    fn default() -> Self {
        Self {
            stats: HashMap::new(),
            generations: HashMap::new(),
            last_seen: HashMap::new(),
            order: BinaryHeap::new(),
            next_generation: 0,
            #[cfg(test)]
            rebuild_count: 0,
        }
    }
}

/// Accumulated statistics for an index
#[derive(Debug, Clone)]
pub struct IndexStatsAccum {
    pub index_name: String,
    pub scans_count: u64,
    pub rows_read: u64,
    pub last_used: DateTime<Utc>,
}

/// Accumulated statistics for a collection
#[derive(Debug, Clone)]
pub struct CollectionStatsAccum {
    pub collection: String,
    pub reads_count: u64,
    pub writes_count: u64,
    pub deletes_count: u64,
    pub last_read: Option<DateTime<Utc>>,
    pub last_write: Option<DateTime<Utc>>,
}

/// RAII guard for tracking active queries
pub struct QueryGuard<'a> {
    collector: &'a MetricsCollector,
    query_hash: u64,
    safe_summary: Option<String>,
    start: Instant,
}

impl<'a> QueryGuard<'a> {
    /// Finish the query and record stats
    pub fn finish(self, rows_returned: u64, rows_affected: u64, is_error: bool) {
        let elapsed_us = self.start.elapsed().as_micros() as u64;
        self.collector.record_query_end(
            self.query_hash,
            elapsed_us,
            rows_returned,
            rows_affected,
            is_error,
        );
        self.collector
            .store_query_fingerprint(self.query_hash, self.safe_summary.as_deref());
    }
}

impl Drop for QueryGuard<'_> {
    fn drop(&mut self) {
        self.collector
            .active_queries
            .fetch_sub(1, Ordering::Relaxed);
    }
}

/// Central metrics collector
pub struct MetricsCollector {
    config: MetricsConfig,
    pub(crate) active_queries: AtomicU64,
    pub(crate) queries_total: AtomicU64,
    pub(crate) queries_errors: AtomicU64,
    query_stats: RwLock<QueryStatsState>,
    index_stats: RwLock<NamedStatsState<IndexStatsAccum>>,
    collection_stats: RwLock<NamedStatsState<CollectionStatsAccum>>,
    rate_window: Mutex<RateWindow>,
    start_time: Instant,
}

impl MetricsCollector {
    pub fn new(config: MetricsConfig) -> Self {
        Self {
            config,
            active_queries: AtomicU64::new(0),
            queries_total: AtomicU64::new(0),
            queries_errors: AtomicU64::new(0),
            query_stats: RwLock::new(QueryStatsState::default()),
            index_stats: RwLock::new(NamedStatsState::default()),
            collection_stats: RwLock::new(NamedStatsState::default()),
            rate_window: Mutex::new(RateWindow::new()),
            start_time: Instant::now(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }
    pub fn config(&self) -> &MetricsConfig {
        &self.config
    }
    pub fn uptime_secs(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }
    pub fn active_queries(&self) -> u64 {
        self.active_queries.load(Ordering::Relaxed)
    }
    pub fn queries_total(&self) -> u64 {
        self.queries_total.load(Ordering::Relaxed)
    }
    pub fn queries_errors(&self) -> u64 {
        self.queries_errors.load(Ordering::Relaxed)
    }
    /// Average queries per second over the last 60 seconds.
    pub fn queries_per_sec(&self) -> f64 {
        self.rate_window.lock().rate()
    }

    /// Start tracking a query, returns guard for RAII.
    /// The raw query is normalized immediately and is never retained by the guard.
    pub fn query_start(&self, query: &str) -> QueryGuard<'_> {
        self.active_queries.fetch_add(1, Ordering::Relaxed);
        let (query_hash, safe_summary) = normalize_query(query);
        QueryGuard {
            collector: self,
            query_hash,
            safe_summary,
            start: Instant::now(),
        }
    }

    /// Record query completion
    pub fn record_query_end(
        &self,
        query_hash: u64,
        time_us: u64,
        rows_returned: u64,
        rows_affected: u64,
        is_error: bool,
    ) {
        if !self.config.enabled {
            return;
        }
        self.queries_total.fetch_add(1, Ordering::Relaxed);
        self.rate_window.lock().increment();
        if is_error {
            self.queries_errors.fetch_add(1, Ordering::Relaxed);
        }

        let now = Utc::now();
        let mut state = self.query_stats.write();
        let cutoff = self.query_stats_cutoff(now);
        let expired = state
            .stats
            .get(&query_hash)
            .is_some_and(|item| item.last_seen < cutoff);
        if expired {
            state.stats.remove(&query_hash);
            state.generations.remove(&query_hash);
        }

        if let Some(existing) = state.stats.get_mut(&query_hash) {
            existing.call_count += 1;
            existing.total_time_us += time_us;
            existing.min_time_us = existing.min_time_us.min(time_us);
            existing.max_time_us = existing.max_time_us.max(time_us);
            existing.rows_returned += rows_returned;
            existing.rows_affected += rows_affected;
            if is_error {
                existing.errors += 1;
            }
            existing.last_seen = now;
            self.touch_query_stats_locked(&mut state, query_hash, now);
            return;
        }

        if self.config.max_query_stats == 0 {
            return;
        }
        self.prune_query_stats_locked(&mut state, now);
        while state.stats.len() >= self.config.max_query_stats {
            if !self.evict_oldest_query_stats_locked(&mut state) {
                break;
            }
        }

        state.stats.insert(
            query_hash,
            QueryStatsAccum {
                query_hash,
                query_normalized: String::new(),
                query_example: String::new(),
                query_display: String::new(),
                query_variants: Vec::new(),
                statement_kind: "UNPARSED".to_string(),
                risk: "unknown".to_string(),
                redaction_count: 0,
                sanitizer_version: super::QUERY_SANITIZER_VERSION,
                call_count: 1,
                total_time_us: time_us,
                min_time_us: time_us,
                max_time_us: time_us,
                rows_returned,
                rows_affected,
                errors: if is_error { 1 } else { 0 },
                first_seen: now,
                last_seen: now,
            },
        );
        self.touch_query_stats_locked(&mut state, query_hash, now);
    }

    /// Store a parser-generated safe template and bounded replay variants.
    ///
    /// The API intentionally cannot accept a raw example. `None` is the fail-closed
    /// representation for malformed input and leaves the public string fields empty.
    pub fn store_query_fingerprint(&self, query_hash: u64, safe_summary: Option<&str>) {
        if !self.config.enabled {
            return;
        }
        let Some(safe_summary) = safe_summary else {
            return;
        };

        let metadata = super::sanitize_query(safe_summary);
        let mut state = self.query_stats.write();
        if let Some(s) = state.stats.get_mut(&query_hash) {
            if !s.query_variants.iter().any(|item| item == safe_summary) {
                const MAX_QUERY_VARIANTS: usize = 5;
                if s.query_variants.len() == MAX_QUERY_VARIANTS {
                    s.query_variants.remove(0);
                }
                s.query_variants.push(safe_summary.to_string());
            }
            s.query_example = safe_summary.to_string();
            s.query_normalized = safe_summary.to_string();
            s.query_display = metadata
                .display_template
                .unwrap_or_else(|| safe_summary.to_string());
            s.statement_kind = metadata.statement_kind.to_string();
            s.risk = metadata.risk.to_string();
            s.redaction_count = metadata.redaction_count as u64;
            s.sanitizer_version = metadata.sanitizer_version;
        }
    }

    /// Record index scan with bounded LRU metadata.
    pub fn record_index_scan(&self, index_name: &str, rows_read: u64) {
        if !self.config.enabled {
            return;
        }
        let now = Utc::now();
        let mut state = self.index_stats.write();
        if let Some(stat) = state.stats.get_mut(index_name) {
            stat.scans_count += 1;
            stat.rows_read += rows_read;
            stat.last_used = now;
            touch_named_stats(&mut state, index_name, now);
            return;
        }
        if self.config.max_index_stats == 0 {
            return;
        }
        prune_named_stats(
            &mut state,
            retention_cutoff(now, self.config.index_retention_days),
        );
        while state.stats.len() >= self.config.max_index_stats {
            if !evict_oldest_named_stats(&mut state) {
                return;
            }
        }
        state.stats.insert(
            index_name.to_string(),
            IndexStatsAccum {
                index_name: index_name.to_string(),
                scans_count: 1,
                rows_read,
                last_used: now,
            },
        );
        touch_named_stats(&mut state, index_name, now);
    }

    /// Record collection read.
    pub fn record_collection_read(&self, collection: &str) {
        self.record_collection_activity(collection, |stat, now| {
            stat.reads_count += 1;
            stat.last_read = Some(now);
        });
    }

    /// Record collection write.
    pub fn record_collection_write(&self, collection: &str) {
        self.record_collection_activity(collection, |stat, now| {
            stat.writes_count += 1;
            stat.last_write = Some(now);
        });
    }

    /// Record collection delete.
    pub fn record_collection_delete(&self, collection: &str) {
        self.record_collection_activity(collection, |stat, now| {
            stat.deletes_count += 1;
            stat.last_write = Some(now);
        });
    }

    fn record_collection_activity(
        &self,
        collection: &str,
        update: impl FnOnce(&mut CollectionStatsAccum, DateTime<Utc>),
    ) {
        if !self.config.enabled {
            return;
        }
        let now = Utc::now();
        let mut state = self.collection_stats.write();
        if let Some(stat) = state.stats.get_mut(collection) {
            update(stat, now);
            touch_named_stats(&mut state, collection, now);
            return;
        }
        if self.config.max_collection_stats == 0 {
            return;
        }
        prune_named_stats(
            &mut state,
            retention_cutoff(now, self.config.collection_retention_days),
        );
        while state.stats.len() >= self.config.max_collection_stats {
            if !evict_oldest_named_stats(&mut state) {
                return;
            }
        }
        let mut stat = CollectionStatsAccum {
            collection: collection.to_string(),
            reads_count: 0,
            writes_count: 0,
            deletes_count: 0,
            last_read: None,
            last_write: None,
        };
        update(&mut stat, now);
        state.stats.insert(collection.to_string(), stat);
        touch_named_stats(&mut state, collection, now);
    }

    fn query_stats_cutoff(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        retention_cutoff(now, self.config.retention_days)
    }

    fn touch_query_stats_locked(
        &self,
        state: &mut QueryStatsState,
        query_hash: u64,
        last_seen: DateTime<Utc>,
    ) {
        state.next_generation = state.next_generation.wrapping_add(1);
        if state.next_generation == 0 {
            self.rebuild_query_order_locked(state);
            state.next_generation = state.stats.len() as u64 + 1;
        }
        let generation = state.next_generation;
        state.generations.insert(query_hash, generation);
        state.order.push(Reverse(QueryOrder {
            last_seen,
            generation,
            query_hash,
        }));
        self.compact_query_order_if_needed_locked(state);
    }

    fn prune_query_stats_locked(&self, state: &mut QueryStatsState, now: DateTime<Utc>) {
        let cutoff = self.query_stats_cutoff(now);
        while let Some(Reverse(candidate)) = state.order.peek().cloned() {
            let current = state.generations.get(&candidate.query_hash).copied();
            if current != Some(candidate.generation) {
                state.order.pop();
                continue;
            }
            let expired = state
                .stats
                .get(&candidate.query_hash)
                .is_none_or(|item| item.last_seen < cutoff);
            if !expired {
                break;
            }
            state.order.pop();
            state.stats.remove(&candidate.query_hash);
            state.generations.remove(&candidate.query_hash);
        }
        self.compact_query_order_if_needed_locked(state);
    }

    fn evict_oldest_query_stats_locked(&self, state: &mut QueryStatsState) -> bool {
        while let Some(Reverse(candidate)) = state.order.pop() {
            if state.generations.get(&candidate.query_hash) != Some(&candidate.generation) {
                continue;
            }
            state.stats.remove(&candidate.query_hash);
            state.generations.remove(&candidate.query_hash);
            return true;
        }
        false
    }

    fn compact_query_order_if_needed_locked(&self, state: &mut QueryStatsState) {
        let threshold = state.stats.len().saturating_mul(4).max(64);
        if state.order.len() > threshold {
            self.rebuild_query_order_locked(state);
        }
    }

    fn rebuild_query_order_locked(&self, state: &mut QueryStatsState) {
        state.order = state
            .stats
            .iter()
            .filter_map(|(&query_hash, item)| {
                state
                    .generations
                    .get(&query_hash)
                    .copied()
                    .map(|generation| {
                        Reverse(QueryOrder {
                            last_seen: item.last_seen,
                            generation,
                            query_hash,
                        })
                    })
            })
            .collect();
        #[cfg(test)]
        {
            state.rebuild_count += 1;
        }
    }

    /// Take and reset query stats
    pub fn take_query_stats(&self) -> HashMap<u64, QueryStatsAccum> {
        let mut state = self.query_stats.write();
        self.prune_query_stats_locked(&mut state, Utc::now());
        state.generations.clear();
        state.order.clear();
        std::mem::take(&mut state.stats)
    }

    /// Take and reset index stats.
    pub fn take_index_stats(&self) -> HashMap<String, IndexStatsAccum> {
        let mut state = self.index_stats.write();
        prune_named_stats(
            &mut state,
            retention_cutoff(Utc::now(), self.config.index_retention_days),
        );
        state.generations.clear();
        state.last_seen.clear();
        state.order.clear();
        std::mem::take(&mut state.stats)
    }

    /// Take and reset collection stats.
    pub fn take_collection_stats(&self) -> HashMap<String, CollectionStatsAccum> {
        let mut state = self.collection_stats.write();
        prune_named_stats(
            &mut state,
            retention_cutoff(Utc::now(), self.config.collection_retention_days),
        );
        state.generations.clear();
        state.last_seen.clear();
        state.order.clear();
        std::mem::take(&mut state.stats)
    }

    /// Get a bounded snapshot of query stats without resetting.
    pub fn snapshot_query_stats(&self) -> HashMap<u64, QueryStatsAccum> {
        let mut state = self.query_stats.write();
        self.prune_query_stats_locked(&mut state, Utc::now());
        state.stats.clone()
    }

    /// Return only the top query entries without cloning the entire map.
    pub fn top_query_stats(&self, limit: usize) -> Vec<QueryStatsAccum> {
        let mut state = self.query_stats.write();
        self.prune_query_stats_locked(&mut state, Utc::now());
        let mut top: Vec<_> = state.stats.values().collect();
        top.sort_by_key(|item| Reverse(item.total_time_us));
        top.into_iter().take(limit).cloned().collect()
    }

    /// Get a bounded snapshot of index stats without resetting.
    pub fn snapshot_index_stats(&self) -> HashMap<String, IndexStatsAccum> {
        let mut state = self.index_stats.write();
        prune_named_stats(
            &mut state,
            retention_cutoff(Utc::now(), self.config.index_retention_days),
        );
        state.stats.clone()
    }

    /// Get a bounded snapshot of collection stats without resetting.
    pub fn snapshot_collection_stats(&self) -> HashMap<String, CollectionStatsAccum> {
        let mut state = self.collection_stats.write();
        prune_named_stats(
            &mut state,
            retention_cutoff(Utc::now(), self.config.collection_retention_days),
        );
        state.stats.clone()
    }
}

fn touch_named_stats<T>(state: &mut NamedStatsState<T>, key: &str, last_seen: DateTime<Utc>) {
    state.next_generation = state.next_generation.wrapping_add(1);
    if state.next_generation == 0 {
        rebuild_named_stats_order(state);
        state.next_generation = state.stats.len() as u64 + 1;
    }
    let generation = state.next_generation;
    state.generations.insert(key.to_string(), generation);
    state.last_seen.insert(key.to_string(), last_seen);
    state.order.push(Reverse(NamedStatsOrder {
        last_seen,
        generation,
        key: key.to_string(),
    }));
    compact_named_stats_order_if_needed(state);
}

fn prune_named_stats<T>(state: &mut NamedStatsState<T>, cutoff: DateTime<Utc>) {
    while let Some(Reverse(candidate)) = state.order.peek().cloned() {
        if state.generations.get(&candidate.key) != Some(&candidate.generation) {
            state.order.pop();
            continue;
        }
        if candidate.last_seen >= cutoff {
            break;
        }
        state.order.pop();
        state.stats.remove(&candidate.key);
        state.generations.remove(&candidate.key);
        state.last_seen.remove(&candidate.key);
    }
    compact_named_stats_order_if_needed(state);
}

fn evict_oldest_named_stats<T>(state: &mut NamedStatsState<T>) -> bool {
    while let Some(Reverse(candidate)) = state.order.pop() {
        if state.generations.get(&candidate.key) != Some(&candidate.generation) {
            continue;
        }
        state.stats.remove(&candidate.key);
        state.generations.remove(&candidate.key);
        state.last_seen.remove(&candidate.key);
        return true;
    }
    false
}

fn compact_named_stats_order_if_needed<T>(state: &mut NamedStatsState<T>) {
    let threshold = state.stats.len().saturating_mul(4).max(64);
    if state.order.len() > threshold {
        rebuild_named_stats_order(state);
    }
}

fn rebuild_named_stats_order<T>(state: &mut NamedStatsState<T>) {
    state.order = state
        .generations
        .iter()
        .filter_map(|(key, &generation)| {
            state.last_seen.get(key).copied().map(|last_seen| {
                Reverse(NamedStatsOrder {
                    last_seen,
                    generation,
                    key: key.clone(),
                })
            })
        })
        .collect();
    #[cfg(test)]
    {
        state.rebuild_count += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_stats_accumulation() {
        let collector = MetricsCollector::new(MetricsConfig::default());
        collector.record_query_end(1234, 100, 10, 0, false);
        collector.record_query_end(1234, 200, 20, 0, false);
        let stats = collector.take_query_stats();
        let stat = stats.get(&1234).unwrap();
        assert_eq!(stat.call_count, 2);
        assert_eq!(stat.total_time_us, 300);
        assert_eq!(stat.min_time_us, 100);
        assert_eq!(stat.max_time_us, 200);
        assert_eq!(stat.rows_returned, 30);
    }

    #[test]
    fn test_active_queries_gauge() {
        let collector = MetricsCollector::new(MetricsConfig::default());
        assert_eq!(collector.active_queries(), 0);
        let _guard1 = collector.query_start("SELECT 1");
        assert_eq!(collector.active_queries(), 1);
        let _guard2 = collector.query_start("SELECT 2");
        assert_eq!(collector.active_queries(), 2);
        drop(_guard1);
        assert_eq!(collector.active_queries(), 1);
        drop(_guard2);
        assert_eq!(collector.active_queries(), 0);
    }

    #[test]
    fn test_take_resets_stats() {
        let collector = MetricsCollector::new(MetricsConfig::default());
        collector.record_query_end(1234, 100, 10, 0, false);
        let stats1 = collector.take_query_stats();
        assert_eq!(stats1.len(), 1);
        let stats2 = collector.take_query_stats();
        assert!(stats2.is_empty());
    }

    #[test]
    fn test_disabled_collector() {
        let config = MetricsConfig {
            enabled: false,
            ..Default::default()
        };
        let collector = MetricsCollector::new(config);
        collector.record_query_end(1234, 100, 10, 0, false);
        let stats = collector.take_query_stats();
        assert!(stats.is_empty());
    }

    #[test]
    fn test_index_stats() {
        let collector = MetricsCollector::new(MetricsConfig::default());
        collector.record_index_scan("user.email_idx", 50);
        collector.record_index_scan("user.email_idx", 30);
        collector.record_index_scan("user.name_idx", 10);
        let stats = collector.take_index_stats();
        assert_eq!(stats["user.email_idx"].scans_count, 2);
        assert_eq!(stats["user.email_idx"].rows_read, 80);
        assert_eq!(stats["user.name_idx"].scans_count, 1);
    }

    #[test]
    fn test_collection_stats() {
        let collector = MetricsCollector::new(MetricsConfig::default());
        collector.record_collection_read("user");
        collector.record_collection_read("user");
        collector.record_collection_write("user");
        collector.record_collection_read("orders");
        let stats = collector.take_collection_stats();
        assert_eq!(stats["user"].reads_count, 2);
        assert_eq!(stats["user"].writes_count, 1);
        assert_eq!(stats["orders"].reads_count, 1);
    }

    #[test]
    fn test_concurrent_query_updates() {
        use std::sync::Arc;
        use std::thread;

        let collector = Arc::new(MetricsCollector::new(MetricsConfig::default()));
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let c = collector.clone();
                thread::spawn(move || {
                    for _ in 0..1000 {
                        c.record_query_end(1234, 100, 1, 0, false);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        let stats = collector.take_query_stats();
        let stat = stats.get(&1234).unwrap();
        assert_eq!(stat.call_count, 10_000);
    }

    #[test]
    fn query_stats_are_bounded_and_evict_oldest() {
        let collector = MetricsCollector::new(MetricsConfig {
            max_query_stats: 2,
            ..Default::default()
        });
        collector.record_query_end(1, 10, 0, 0, false);
        std::thread::sleep(std::time::Duration::from_millis(1));
        collector.record_query_end(2, 20, 0, 0, false);
        std::thread::sleep(std::time::Duration::from_millis(1));
        collector.record_query_end(3, 30, 0, 0, false);

        let stats = collector.snapshot_query_stats();
        assert_eq!(stats.len(), 2);
        assert!(!stats.contains_key(&1));
        assert!(stats.contains_key(&2));
        assert!(stats.contains_key(&3));
    }

    #[test]
    fn query_stats_refresh_updates_lru_order() {
        let collector = MetricsCollector::new(MetricsConfig {
            max_query_stats: 2,
            ..Default::default()
        });
        collector.record_query_end(1, 10, 0, 0, false);
        std::thread::sleep(std::time::Duration::from_millis(1));
        collector.record_query_end(2, 20, 0, 0, false);
        std::thread::sleep(std::time::Duration::from_millis(1));
        collector.record_query_end(1, 30, 0, 0, false);
        std::thread::sleep(std::time::Duration::from_millis(1));
        collector.record_query_end(3, 40, 0, 0, false);

        let stats = collector.snapshot_query_stats();
        assert_eq!(stats.len(), 2);
        assert!(stats.contains_key(&1));
        assert!(!stats.contains_key(&2));
        assert!(stats.contains_key(&3));
        assert_eq!(stats[&1].call_count, 2);
    }

    #[test]
    fn repeated_updates_do_not_full_prune_query_stats() {
        let collector = MetricsCollector::new(MetricsConfig::default());
        collector.record_query_end(1, 10, 0, 0, false);
        for _ in 0..32 {
            collector.record_query_end(1, 10, 0, 0, false);
        }

        let state = collector.query_stats.read();
        assert_eq!(state.rebuild_count, 0);
        assert_eq!(state.stats[&1].call_count, 33);
    }

    #[test]
    fn query_stats_retention_is_applied() {
        let collector = MetricsCollector::new(MetricsConfig {
            retention_days: 0,
            ..Default::default()
        });
        collector.record_query_end(1, 10, 0, 0, false);
        std::thread::sleep(std::time::Duration::from_millis(1));
        assert!(collector.snapshot_query_stats().is_empty());
    }

    #[test]
    fn top_query_stats_is_bounded_and_sorted() {
        let collector = MetricsCollector::new(MetricsConfig::default());
        collector.record_query_end(1, 10, 0, 0, false);
        collector.record_query_end(2, 30, 0, 0, false);
        collector.record_query_end(3, 20, 0, 0, false);

        let top = collector.top_query_stats(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].query_hash, 2);
        assert_eq!(top[1].query_hash, 3);
    }

    #[test]
    fn collector_never_retains_query_literals_or_malformed_input() {
        let collector = MetricsCollector::new(MetricsConfig::default());

        let valid = "CREATE USER 'Иван' PASSWORD 'super-secret-password'";
        let (valid_hash, valid_summary) = normalize_query(valid);
        collector.record_query_end(valid_hash, 10, 0, 0, true);
        collector.store_query_fingerprint(valid_hash, valid_summary.as_deref());

        let malformed = "SELECT * WHERE email = 'мария@example.com api_key=sk-secret";
        let (malformed_hash, malformed_summary) = normalize_query(malformed);
        collector.record_query_end(malformed_hash, 20, 0, 0, true);
        collector.store_query_fingerprint(malformed_hash, malformed_summary.as_deref());

        let stats = collector.snapshot_query_stats();
        let valid_stat = &stats[&valid_hash];
        assert_eq!(valid_stat.query_normalized, "CREATE USER '?' PASSWORD '?'");
        assert_eq!(valid_stat.query_example, "CREATE USER '?' PASSWORD '?'");
        assert_eq!(valid_stat.errors, 1);

        let malformed_stat = &stats[&malformed_hash];
        assert!(malformed_stat.query_normalized.is_empty());
        assert!(malformed_stat.query_example.is_empty());
        assert_eq!(malformed_stat.errors, 1);

        let debug = format!("{stats:?}");
        for secret in [
            "Иван",
            "super-secret-password",
            "мария@example.com",
            "sk-secret",
        ] {
            assert!(!debug.contains(secret));
        }
    }

    #[test]
    fn index_and_collection_stats_are_hard_bounded_and_evict_lru() {
        let collector = MetricsCollector::new(MetricsConfig {
            max_index_stats: 2,
            max_collection_stats: 2,
            ..Default::default()
        });
        collector.record_index_scan("idx-a", 1);
        collector.record_index_scan("idx-b", 1);
        collector.record_index_scan("idx-a", 1);
        collector.record_index_scan("idx-c", 1);
        let indexes = collector.snapshot_index_stats();
        assert_eq!(indexes.len(), 2);
        assert!(indexes.contains_key("idx-a"));
        assert!(!indexes.contains_key("idx-b"));
        assert!(indexes.contains_key("idx-c"));

        collector.record_collection_read("a");
        collector.record_collection_write("b");
        collector.record_collection_delete("a");
        collector.record_collection_read("c");
        let collections = collector.snapshot_collection_stats();
        assert_eq!(collections.len(), 2);
        assert!(collections.contains_key("a"));
        assert!(!collections.contains_key("b"));
        assert!(collections.contains_key("c"));
    }

    #[test]
    fn named_stats_retention_expires_inactive_entries() {
        let collector = MetricsCollector::new(MetricsConfig {
            index_retention_days: 0,
            collection_retention_days: 0,
            ..Default::default()
        });
        collector.record_index_scan("old-index", 1);
        collector.record_collection_read("old-collection");
        std::thread::sleep(std::time::Duration::from_millis(1));
        assert!(collector.snapshot_index_stats().is_empty());
        assert!(collector.snapshot_collection_stats().is_empty());
    }

    #[test]
    fn named_stats_eviction_metadata_stays_bounded_under_repeated_updates() {
        let collector = MetricsCollector::new(MetricsConfig {
            max_index_stats: 2,
            max_collection_stats: 2,
            ..Default::default()
        });
        for index in 0..1_000 {
            collector.record_index_scan(if index % 2 == 0 { "a" } else { "b" }, 1);
            collector.record_collection_read(if index % 2 == 0 { "a" } else { "b" });
        }
        let indexes = collector.index_stats.read();
        assert_eq!(indexes.stats.len(), 2);
        assert_eq!(indexes.generations.len(), 2);
        assert_eq!(indexes.last_seen.len(), 2);
        assert!(indexes.order.len() <= 64);
        drop(indexes);
        let collections = collector.collection_stats.read();
        assert_eq!(collections.stats.len(), 2);
        assert_eq!(collections.generations.len(), 2);
        assert_eq!(collections.last_seen.len(), 2);
        assert!(collections.order.len() <= 64);
    }

    #[test]
    fn zero_named_stats_limits_retain_no_entries_or_metadata() {
        let collector = MetricsCollector::new(MetricsConfig {
            max_index_stats: 0,
            max_collection_stats: 0,
            ..Default::default()
        });
        collector.record_index_scan("ignored", 1);
        collector.record_collection_read("ignored");
        let indexes = collector.index_stats.read();
        assert!(indexes.stats.is_empty() && indexes.order.is_empty());
        drop(indexes);
        let collections = collector.collection_stats.read();
        assert!(collections.stats.is_empty() && collections.order.is_empty());
    }

    #[test]
    fn test_concurrent_index_updates() {
        use std::sync::Arc;
        use std::thread;

        let collector = Arc::new(MetricsCollector::new(MetricsConfig::default()));
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let c = collector.clone();
                thread::spawn(move || {
                    for _ in 0..100 {
                        c.record_index_scan(&format!("idx_{}", i % 3), 10);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        let stats = collector.take_index_stats();
        let total_scans: u64 = stats.values().map(|s| s.scans_count).sum();
        assert_eq!(total_scans, 1000);
    }
}
