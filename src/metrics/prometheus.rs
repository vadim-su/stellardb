//! Prometheus text format output for metrics

use crate::metrics::MetricsCollector;
use crate::storage::StorageStats;

/// Format a counter metric
pub fn format_counter(name: &str, help: &str, value: u64) -> String {
    format!(
        "# HELP {} {}\n# TYPE {} counter\n{} {}\n",
        name, help, name, name, value
    )
}

/// Format a gauge metric
pub fn format_gauge(name: &str, help: &str, value: u64) -> String {
    format!(
        "# HELP {} {}\n# TYPE {} gauge\n{} {}\n",
        name, help, name, name, value
    )
}

/// Escape a label value per Prometheus exposition format:
/// backslash, double-quote, and newline must be escaped.
fn escape_label_value(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for ch in v.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out
}

/// Format labels string for a metric line
fn format_labels(labels: &[(&str, &str)]) -> String {
    labels
        .iter()
        .map(|(k, v)| format!("{}=\"{}\"", k, escape_label_value(v)))
        .collect::<Vec<_>>()
        .join(",")
}

/// Format a gauge metric with labels
#[cfg(test)]
pub fn format_gauge_with_labels(
    name: &str,
    help: &str,
    labels: &[(&str, &str)],
    value: u64,
) -> String {
    format!(
        "# HELP {} {}\n# TYPE {} gauge\n{}{{{}}} {}\n",
        name,
        help,
        name,
        name,
        format_labels(labels),
        value
    )
}

/// Format a counter metric with labels
#[cfg(test)]
pub fn format_counter_with_labels(
    name: &str,
    help: &str,
    labels: &[(&str, &str)],
    value: u64,
) -> String {
    format!(
        "# HELP {} {}\n# TYPE {} counter\n{}{{{}}} {}\n",
        name,
        help,
        name,
        name,
        format_labels(labels),
        value
    )
}

/// Format a metric header (HELP + TYPE), emitted once per metric name
fn format_header(name: &str, help: &str, metric_type: &str) -> String {
    format!(
        "# HELP {} {}\n# TYPE {} {}\n",
        name, help, name, metric_type
    )
}

/// Format a single labeled metric line (no header)
fn format_labeled_line(name: &str, labels: &[(&str, &str)], value: u64) -> String {
    format!("{}{{{}}} {}\n", name, format_labels(labels), value)
}

/// Format all metrics in Prometheus format
pub fn format_prometheus(collector: &MetricsCollector, storage_stats: &StorageStats) -> String {
    let mut output = String::new();

    // Server metrics
    output.push_str(&format_gauge(
        "stellardb_uptime_seconds",
        "Server uptime in seconds",
        collector.uptime_secs(),
    ));
    output.push_str(&format_counter(
        "stellardb_queries_total",
        "Total number of queries executed",
        collector.queries_total(),
    ));
    output.push_str(&format!(
        "# HELP stellardb_queries_per_second Average queries per second over last 60s\n\
         # TYPE stellardb_queries_per_second gauge\n\
         stellardb_queries_per_second {:.2}\n",
        collector.queries_per_sec(),
    ));
    output.push_str(&format_counter(
        "stellardb_queries_errors_total",
        "Total number of query errors",
        collector.queries_errors(),
    ));
    output.push_str(&format_gauge(
        "stellardb_queries_active",
        "Currently executing queries",
        collector.active_queries(),
    ));

    // Storage metrics
    output.push_str(&format_gauge(
        "stellardb_storage_disk_bytes",
        "Total disk space used",
        storage_stats.disk_size,
    ));
    output.push_str(&format_gauge(
        "stellardb_collections_total",
        "Number of collections",
        storage_stats.collections as u64,
    ));
    output.push_str(&format_gauge(
        "stellardb_indexes_btree_total",
        "Number of B-tree indexes",
        storage_stats.btree_indexes as u64,
    ));
    output.push_str(&format_gauge(
        "stellardb_indexes_vector_total",
        "Number of vector indexes",
        storage_stats.vector_indexes as u64,
    ));
    output.push_str(&format_gauge(
        "stellardb_indexes_fts_total",
        "Number of full-text search indexes",
        storage_stats.fts_indexes as u64,
    ));

    // Index usage stats — emit HELP/TYPE once, then one line per index
    let index_stats = collector.snapshot_index_stats();
    if !index_stats.is_empty() {
        output.push_str(&format_header(
            "stellardb_index_scans_total",
            "Number of index scans",
            "counter",
        ));
        for (name, stat) in &index_stats {
            output.push_str(&format_labeled_line(
                "stellardb_index_scans_total",
                &[("index", name)],
                stat.scans_count,
            ));
        }

        output.push_str(&format_header(
            "stellardb_index_rows_read_total",
            "Total rows read from index",
            "counter",
        ));
        for (name, stat) in &index_stats {
            output.push_str(&format_labeled_line(
                "stellardb_index_rows_read_total",
                &[("index", name)],
                stat.rows_read,
            ));
        }
    }

    // Collection stats — emit HELP/TYPE once, then one line per collection
    let collection_stats = collector.snapshot_collection_stats();
    if !collection_stats.is_empty() {
        output.push_str(&format_header(
            "stellardb_collection_reads_total",
            "Number of collection reads",
            "counter",
        ));
        for (name, stat) in &collection_stats {
            output.push_str(&format_labeled_line(
                "stellardb_collection_reads_total",
                &[("collection", name)],
                stat.reads_count,
            ));
        }

        output.push_str(&format_header(
            "stellardb_collection_writes_total",
            "Number of collection writes",
            "counter",
        ));
        for (name, stat) in &collection_stats {
            output.push_str(&format_labeled_line(
                "stellardb_collection_writes_total",
                &[("collection", name)],
                stat.writes_count,
            ));
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{MetricsCollector, MetricsConfig};

    #[test]
    fn test_format_counter() {
        let output = format_counter(
            "stellardb_queries_total",
            "Total number of queries executed",
            1234,
        );
        assert!(output.contains("# HELP stellardb_queries_total"));
        assert!(output.contains("# TYPE stellardb_queries_total counter"));
        assert!(output.contains("stellardb_queries_total 1234"));
    }

    #[test]
    fn test_format_gauge() {
        let output = format_gauge("stellardb_queries_active", "Currently executing queries", 5);
        assert!(output.contains("# TYPE stellardb_queries_active gauge"));
        assert!(output.contains("stellardb_queries_active 5"));
    }

    #[test]
    fn test_format_gauge_with_labels() {
        let output = format_gauge_with_labels(
            "stellardb_collection_documents",
            "Number of documents",
            &[("collection", "user")],
            5000,
        );
        assert!(output.contains("stellardb_collection_documents{collection=\"user\"} 5000"));
    }

    #[test]
    fn test_escape_label_value_backslash() {
        assert_eq!(escape_label_value(r#"a\b"#), r#"a\\b"#);
    }

    #[test]
    fn test_escape_label_value_quote() {
        assert_eq!(escape_label_value(r#"say "hi""#), r#"say \"hi\""#);
    }

    #[test]
    fn test_escape_label_value_newline() {
        assert_eq!(escape_label_value("line1\nline2"), "line1\\nline2");
    }

    #[test]
    fn test_escape_label_value_plain() {
        assert_eq!(escape_label_value("simple"), "simple");
    }

    #[test]
    fn test_labeled_metric_escapes_values() {
        let output = format_counter_with_labels("m", "help", &[("k", "val\"ue")], 1);
        assert!(output.contains(r#"k="val\"ue""#));
    }

    #[test]
    fn test_no_duplicate_help_type_lines() {
        let collector = MetricsCollector::new(MetricsConfig::default());
        // Record two different indexes
        collector.record_index_scan("idx_a", 10);
        collector.record_index_scan("idx_b", 20);

        let storage_stats = StorageStats {
            collections: 0,
            edge_labels: 0,
            btree_indexes: 2,
            vector_indexes: 0,
            fts_indexes: 0,
            disk_size: 0,
        };

        let output = format_prometheus(&collector, &storage_stats);

        // Each HELP/TYPE for index metrics should appear exactly once
        let scans_help_count = output.matches("# HELP stellardb_index_scans_total").count();
        let scans_type_count = output.matches("# TYPE stellardb_index_scans_total").count();
        assert_eq!(scans_help_count, 1, "HELP line duplicated for index_scans");
        assert_eq!(scans_type_count, 1, "TYPE line duplicated for index_scans");

        // But we should have two data lines
        let scans_data_count = output.matches("stellardb_index_scans_total{").count();
        assert_eq!(
            scans_data_count, 2,
            "expected two data lines for index_scans"
        );
    }

    #[test]
    fn test_format_metrics() {
        let collector = MetricsCollector::new(MetricsConfig::default());
        collector.record_query_end(1234, 100, 10, 0, false);

        let storage_stats = StorageStats {
            collections: 5,
            edge_labels: 3,
            btree_indexes: 10,
            vector_indexes: 2,
            fts_indexes: 3,
            disk_size: 1024 * 1024 * 100,
        };

        let output = format_prometheus(&collector, &storage_stats);
        assert!(output.contains("stellardb_queries_total"));
        assert!(output.contains("stellardb_queries_active"));
        assert!(output.contains("stellardb_uptime_seconds"));
        assert!(output.contains("stellardb_storage_disk_bytes"));
    }
}
