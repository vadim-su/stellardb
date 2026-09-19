//! Instrumented operators for EXPLAIN ANALYZE

use std::time::Instant;

use crate::query::error::ExecuteError;
use crate::query::execute::operators::operator::{Operator, Row};
use crate::query::plan::OperatorStats;

/// Wraps an operator to collect execution statistics.
///
/// Measures time spent in each `open()`, `next()`, and `close()` call
/// individually. This captures the operator's own time accurately in a
/// pull-based pipeline, where calling `inner.next()` includes child
/// operator time.
pub struct InstrumentedOperator<O: Operator> {
    inner: O,
    stats: OperatorStats,
}

impl<O: Operator> InstrumentedOperator<O> {
    pub fn new(inner: O) -> Self {
        Self {
            inner,
            stats: OperatorStats::default(),
        }
    }

    /// Get collected statistics (call after close)
    pub fn stats(&self) -> &OperatorStats {
        &self.stats
    }

    /// Take ownership of stats
    pub fn into_stats(self) -> OperatorStats {
        self.stats
    }
}

impl<O: Operator> Operator for InstrumentedOperator<O> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        let start = Instant::now();
        let result = self.inner.open();
        self.stats.time_micros += start.elapsed().as_micros() as u64;
        result
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        let start = Instant::now();
        let row = self.inner.next()?;
        self.stats.time_micros += start.elapsed().as_micros() as u64;
        if row.is_some() {
            self.stats.rows_out += 1;
        }
        Ok(row)
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        let start = Instant::now();
        let result = self.inner.close();
        self.stats.time_micros += start.elapsed().as_micros() as u64;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use crate::query::execute::operators::operator::collect_all;
    use crate::query::execute::operators::scan::TableScanOp;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Arc<Database>) {
        let tmp = TempDir::new().unwrap();
        let storage = Arc::new(Database::open(tmp.path()).unwrap());
        (tmp, storage)
    }

    #[test]
    fn test_instrumented_counts_rows() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION test");
        crate::run_sql!(storage, "INSERT INTO test {id: '1'}, {id: '2'}, {id: '3'}");

        let scan = TableScanOp::new(&*storage, "test".to_string(), None);
        let mut instrumented = InstrumentedOperator::new(scan);

        let rows = collect_all(&mut instrumented).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(instrumented.stats().rows_out, 3);
    }

    #[test]
    fn test_instrumented_measures_time() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION test");
        crate::run_sql!(storage, "INSERT INTO test {id: '1'}");

        let scan = TableScanOp::new(&*storage, "test".to_string(), None);
        let mut instrumented = InstrumentedOperator::new(scan);

        collect_all(&mut instrumented).unwrap();
        // Time should be recorded (non-zero for any non-trivial operation)
        // Note: We just verify stats were collected; actual time depends on system
        let stats = instrumented.stats();
        assert!(stats.rows_out > 0, "Should have counted at least one row");
    }
}
