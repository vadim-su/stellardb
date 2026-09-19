//! JSON format output for /stats endpoint

use serde_json::{Value, json};

use crate::metrics::MetricsCollector;
use crate::storage::StorageStats;

/// Format metrics as JSON for /stats endpoint
pub fn format_json_stats(
    collector: &MetricsCollector,
    storage_stats: &StorageStats,
    top_queries_limit: usize,
) -> Value {
    let top_queries = collector.top_query_stats(top_queries_limit);
    let index_stats = collector.snapshot_index_stats();
    let collection_stats = collector.snapshot_collection_stats();

    let top_queries_json: Vec<Value> = top_queries
        .iter()
        .map(|q| {
            json!({
                "query_hash": format!("{:016x}", q.query_hash),
                "query_normalized": q.query_normalized,
                "query_example": q.query_example,
                "query_display": q.query_display,
                "query_variants": q.query_variants,
                "statement_kind": q.statement_kind,
                "risk": q.risk,
                "redaction_count": q.redaction_count,
                "sanitizer_version": q.sanitizer_version,
                "call_count": q.call_count,
                "total_time_us": q.total_time_us,
                "avg_time_us": q.total_time_us.checked_div(q.call_count).unwrap_or(0),
                "min_time_us": q.min_time_us,
                "max_time_us": q.max_time_us,
                "rows_returned": q.rows_returned,
                "rows_affected": q.rows_affected,
                "errors": q.errors,
                "first_seen": q.first_seen.to_rfc3339(),
                "last_seen": q.last_seen.to_rfc3339(),
            })
        })
        .collect();

    let indexes_json: Value = index_stats
        .iter()
        .map(|(name, stat)| {
            (
                name.clone(),
                json!({
                    "scans_count": stat.scans_count,
                    "rows_read": stat.rows_read,
                    "last_used": stat.last_used.to_rfc3339(),
                }),
            )
        })
        .collect();

    let collections_json: Value = collection_stats
        .iter()
        .map(|(name, stat)| {
            (
                name.clone(),
                json!({
                    "reads_count": stat.reads_count,
                    "writes_count": stat.writes_count,
                    "deletes_count": stat.deletes_count,
                    "last_read": stat.last_read.map(|t| t.to_rfc3339()),
                    "last_write": stat.last_write.map(|t| t.to_rfc3339()),
                }),
            )
        })
        .collect();

    json!({
        "server": {
            "uptime_secs": collector.uptime_secs(),
            "queries_total": collector.queries_total(),
            "queries_per_sec": (collector.queries_per_sec() * 100.0).round() / 100.0,
            "queries_errors": collector.queries_errors(),
            "queries_active": collector.active_queries(),
        },
        "storage": {
            "disk_size_bytes": storage_stats.disk_size,
            "collections_count": storage_stats.collections,
            "btree_indexes": storage_stats.btree_indexes,
            "vector_indexes": storage_stats.vector_indexes,
            "fts_indexes": storage_stats.fts_indexes,
        },
        "top_queries": top_queries_json,
        "indexes": indexes_json,
        "collections": collections_json,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{MetricsCollector, MetricsConfig};

    fn default_storage_stats() -> StorageStats {
        StorageStats {
            collections: 0,
            edge_labels: 0,
            btree_indexes: 0,
            vector_indexes: 0,
            fts_indexes: 0,
            disk_size: 0,
        }
    }

    #[test]
    fn test_format_json_stats() {
        let collector = MetricsCollector::new(MetricsConfig::default());
        collector.record_query_end(1234, 100_000, 10, 0, false);

        let storage_stats = StorageStats {
            collections: 5,
            edge_labels: 3,
            btree_indexes: 10,
            vector_indexes: 2,
            fts_indexes: 3,
            disk_size: 1024 * 1024 * 100,
        };

        let json = format_json_stats(&collector, &storage_stats, 10);

        assert!(json["server"]["uptime_secs"].is_number());
        assert_eq!(json["server"]["queries_total"], 1);
        assert!(json["storage"]["disk_size_bytes"].is_number());
        assert!(json["top_queries"].is_array());
    }

    #[test]
    fn stats_json_never_exposes_query_secrets() {
        let collector = MetricsCollector::new(MetricsConfig::default());
        let raw = "ALTER USER 'Мария Иванова' PASSWORD 'sk-live-secret-123'";
        let (hash, summary) = crate::metrics::normalize_query(raw);
        collector.record_query_end(hash, 50, 0, 7, true);
        collector.store_query_fingerprint(hash, summary.as_deref());

        let json = format_json_stats(&collector, &default_storage_stats(), 10);
        let serialized = json.to_string();
        for secret in ["Мария", "Иванова", "sk-live-secret-123", "123"] {
            assert!(
                !serialized.contains(secret),
                "leaked {secret}: {serialized}"
            );
        }

        let query = &json["top_queries"][0];
        assert_eq!(query["query_normalized"], "ALTER USER '?' PASSWORD '?'");
        assert_eq!(query["query_example"], "ALTER USER '?' PASSWORD '?'");
        assert_eq!(
            query["query_display"],
            "ALTER USER ‹string› PASSWORD ‹string›"
        );
        assert_eq!(query["statement_kind"], "ALTER USER");
        assert_eq!(query["risk"], "admin");
        assert_eq!(query["redaction_count"], 2);
        assert_eq!(
            query["sanitizer_version"],
            crate::metrics::QUERY_SANITIZER_VERSION
        );
        assert_eq!(query["call_count"], 1);
        assert_eq!(query["errors"], 1);
    }

    #[test]
    fn shape_stats_keep_bounded_safe_variants() {
        let collector = MetricsCollector::new(MetricsConfig::default());

        for limit in [1, 10, 100] {
            let raw =
                format!("SELECT * FROM users WHERE password = 'secret-{limit}' LIMIT {limit}");
            let (hash, template) = crate::metrics::normalize_query(&raw);
            collector.record_query_end(hash, 50, 1, 0, false);
            collector.store_query_fingerprint(hash, template.as_deref());
        }

        let json = format_json_stats(&collector, &default_storage_stats(), 10);
        let top = json["top_queries"].as_array().expect("top queries array");
        assert_eq!(top.len(), 1, "LIMIT variants should share one shape");
        assert_eq!(top[0]["call_count"], 3);
        assert_eq!(top[0]["query_variants"].as_array().unwrap().len(), 3);
        assert!(!json.to_string().contains("secret-"));
    }

    #[test]
    fn test_top_queries_sorted() {
        let collector = MetricsCollector::new(MetricsConfig::default());

        // Fast query
        collector.record_query_end(1, 100, 1, 0, false);
        collector.store_query_fingerprint(1, Some("SELECT"));

        // Slow query
        collector.record_query_end(2, 1000, 1, 0, false);
        collector.store_query_fingerprint(2, Some("SELECT"));

        let json = format_json_stats(&collector, &default_storage_stats(), 10);

        let top = json["top_queries"].as_array().unwrap();
        assert_eq!(top.len(), 2);
        // Sorted by total_time descending
        assert!(top[0]["total_time_us"].as_u64() > top[1]["total_time_us"].as_u64());
    }
}
