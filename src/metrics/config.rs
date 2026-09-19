/// Configuration for metrics collection
#[derive(Debug, Clone)]
pub struct MetricsConfig {
    /// Enable/disable metrics collection
    pub enabled: bool,
    /// Flush interval in seconds (default: 60)
    pub flush_interval_secs: u64,
    /// Retention period in days (default: 7)
    pub retention_days: u64,
    /// Maximum number of distinct normalized query fingerprints retained.
    pub max_query_stats: usize,
    /// Maximum number of distinct index metric entries retained.
    pub max_index_stats: usize,
    /// Maximum number of distinct collection metric entries retained.
    pub max_collection_stats: usize,
    /// Index metric inactivity retention in days.
    pub index_retention_days: u64,
    /// Collection metric inactivity retention in days.
    pub collection_retention_days: u64,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            flush_interval_secs: 60,
            retention_days: 7,
            max_query_stats: 10_000,
            max_index_stats: 4_096,
            max_collection_stats: 4_096,
            index_retention_days: 7,
            collection_retention_days: 7,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = MetricsConfig::default();
        assert!(config.enabled);
        assert_eq!(config.flush_interval_secs, 60);
        assert_eq!(config.retention_days, 7);
        assert_eq!(config.max_query_stats, 10_000);
        assert_eq!(config.max_index_stats, 4_096);
        assert_eq!(config.max_collection_stats, 4_096);
        assert_eq!(config.index_retention_days, 7);
        assert_eq!(config.collection_retention_days, 7);
    }

    #[test]
    fn test_config_disabled() {
        let config = MetricsConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(!config.enabled);
    }
}
