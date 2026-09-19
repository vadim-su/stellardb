//! Background task for flushing metrics to _stats.* tables
//!
//! Note: Full implementation requires system tables support.
//! This is a placeholder that will be expanded when _stats.* tables are available.

use std::sync::Arc;
use std::time::Duration;

use tokio::time::interval;

use crate::metrics::{MetricsCollector, MetricsConfig};

/// Spawn the metrics flush background task
///
/// This task periodically flushes accumulated metrics to _stats.* tables.
/// Currently a stub - full implementation pending system tables support.
pub fn spawn_flush_task(
    collector: Arc<MetricsCollector>,
    config: &MetricsConfig,
) -> tokio::task::JoinHandle<()> {
    let flush_interval = Duration::from_secs(config.flush_interval_secs);

    tokio::spawn(async move {
        let mut ticker = interval(flush_interval);

        loop {
            ticker.tick().await;

            if !collector.is_enabled() {
                continue;
            }

            // TODO: Implement flush to _stats.* tables
            // Use non-destructive snapshots for logging so stats keep accumulating.
            let query_stats = collector.snapshot_query_stats();
            let index_stats = collector.snapshot_index_stats();
            let collection_stats = collector.snapshot_collection_stats();

            if !query_stats.is_empty() || !index_stats.is_empty() || !collection_stats.is_empty() {
                tracing::debug!(
                    "Metrics snapshot: {} queries, {} indexes, {} collections",
                    query_stats.len(),
                    index_stats.len(),
                    collection_stats.len()
                );
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_flush_task_runs() {
        let collector = Arc::new(MetricsCollector::new(MetricsConfig {
            flush_interval_secs: 1,
            ..Default::default()
        }));

        // Record some metrics
        collector.record_query_end(1234, 100, 10, 0, false);

        let collector_clone = collector.clone();
        let handle = tokio::spawn(async move {
            sleep(Duration::from_millis(100)).await;
            collector_clone.queries_total()
        });

        let total = handle.await.unwrap();
        assert_eq!(total, 1);
    }

    #[tokio::test]
    async fn test_flush_preserves_stats() {
        let collector = Arc::new(MetricsCollector::new(MetricsConfig {
            flush_interval_secs: 1,
            ..Default::default()
        }));

        // Record metrics before flush
        collector.record_query_end(1234, 100, 10, 0, false);
        collector.record_index_scan("idx_test", 50);
        collector.record_collection_read("users");

        // Spawn the flush task (uses snapshot_*, not take_*)
        let handle = spawn_flush_task(collector.clone(), collector.config());

        // Wait for at least one flush cycle
        sleep(Duration::from_secs(2)).await;

        // Stats should still be available after flush
        let query_stats = collector.snapshot_query_stats();
        assert_eq!(query_stats.len(), 1);
        assert_eq!(query_stats[&1234].call_count, 1);

        let index_stats = collector.snapshot_index_stats();
        assert_eq!(index_stats["idx_test"].scans_count, 1);

        let coll_stats = collector.snapshot_collection_stats();
        assert_eq!(coll_stats["users"].reads_count, 1);

        handle.abort();
    }
}
