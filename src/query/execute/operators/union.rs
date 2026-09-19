//! Union operator — combines results from multiple inputs with optional deduplication

use std::collections::HashSet;

use crate::query::error::ExecuteError;
use crate::query::execute::operators::operator::{Operator, Row};

/// Union operator: pulls rows from multiple child operators sequentially.
/// Optionally deduplicates by document ID using a HashSet.
pub struct UnionOp<'a> {
    inputs: Vec<Box<dyn Operator + 'a>>,
    current_input: usize,
    seen_ids: Option<HashSet<String>>,
}

impl<'a> UnionOp<'a> {
    pub fn new(inputs: Vec<Box<dyn Operator + 'a>>, needs_dedup: bool) -> Self {
        Self {
            inputs,
            current_input: 0,
            seen_ids: if needs_dedup {
                Some(HashSet::new())
            } else {
                None
            },
        }
    }
}

impl Operator for UnionOp<'_> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        for input in &mut self.inputs {
            input.open()?;
        }
        self.current_input = 0;
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        loop {
            if self.current_input >= self.inputs.len() {
                return Ok(None);
            }

            match self.inputs[self.current_input].next()? {
                Some(row) => {
                    if let Some(seen) = &mut self.seen_ids
                        && !seen.insert(row.doc.id.clone())
                    {
                        continue;
                    }
                    return Ok(Some(row));
                }
                None => {
                    self.current_input += 1;
                }
            }
        }
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        for input in &mut self.inputs {
            input.close()?;
        }
        self.seen_ids = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use crate::document::Value;
    use crate::query::execute::operators::operator::collect_all;
    use crate::query::execute::operators::scan::IndexScanOp;
    use crate::query::plan::{IndexLookup, IndexRef};
    use std::sync::Arc;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Arc<Database>) {
        let tmp = TempDir::new().unwrap();
        let storage = Arc::new(Database::open(tmp.path()).unwrap());
        (tmp, storage)
    }

    #[test]
    fn test_union_sequences_inputs() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(status)");
        crate::run_sql!(
            storage,
            "INSERT INTO users {id: 'a', status: 'active'}, {id: 'b', status: 'inactive'}, {id: 'c', status: 'pending'}"
        );

        let input1: Box<dyn Operator + '_> = Box::new(IndexScanOp::new(
            &*storage,
            "users".to_string(),
            IndexRef {
                collection: "users".to_string(),
                fields: vec!["status".to_string()],
            },
            IndexLookup::Eq {
                field: "status".to_string(),
                value: Value::String("active".to_string()),
            },
        ));
        let input2: Box<dyn Operator + '_> = Box::new(IndexScanOp::new(
            &*storage,
            "users".to_string(),
            IndexRef {
                collection: "users".to_string(),
                fields: vec!["status".to_string()],
            },
            IndexLookup::Eq {
                field: "status".to_string(),
                value: Value::String("inactive".to_string()),
            },
        ));

        let mut union = UnionOp::new(vec![input1, input2], false);
        let rows = collect_all(&mut union).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn test_union_dedup_removes_duplicates() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(name)");
        crate::run_sql!(storage, "CREATE INDEX ON users(status)");
        crate::run_sql!(
            storage,
            "INSERT INTO users {id: 'alice', name: 'Alice', status: 'active'}, {id: 'bob', name: 'Bob', status: 'inactive'}"
        );

        let input1: Box<dyn Operator + '_> = Box::new(IndexScanOp::new(
            &*storage,
            "users".to_string(),
            IndexRef {
                collection: "users".to_string(),
                fields: vec!["name".to_string()],
            },
            IndexLookup::Eq {
                field: "name".to_string(),
                value: Value::String("Alice".to_string()),
            },
        ));
        let input2: Box<dyn Operator + '_> = Box::new(IndexScanOp::new(
            &*storage,
            "users".to_string(),
            IndexRef {
                collection: "users".to_string(),
                fields: vec!["status".to_string()],
            },
            IndexLookup::Eq {
                field: "status".to_string(),
                value: Value::String("active".to_string()),
            },
        ));

        let mut union = UnionOp::new(vec![input1, input2], true);
        let rows = collect_all(&mut union).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].doc.id, "users:alice");
    }

    #[test]
    fn test_union_without_dedup_allows_dupes() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(name)");
        crate::run_sql!(storage, "CREATE INDEX ON users(status)");
        crate::run_sql!(
            storage,
            "INSERT INTO users {id: 'alice', name: 'Alice', status: 'active'}"
        );

        let input1: Box<dyn Operator + '_> = Box::new(IndexScanOp::new(
            &*storage,
            "users".to_string(),
            IndexRef {
                collection: "users".to_string(),
                fields: vec!["name".to_string()],
            },
            IndexLookup::Eq {
                field: "name".to_string(),
                value: Value::String("Alice".to_string()),
            },
        ));
        let input2: Box<dyn Operator + '_> = Box::new(IndexScanOp::new(
            &*storage,
            "users".to_string(),
            IndexRef {
                collection: "users".to_string(),
                fields: vec!["status".to_string()],
            },
            IndexLookup::Eq {
                field: "status".to_string(),
                value: Value::String("active".to_string()),
            },
        ));

        let mut union = UnionOp::new(vec![input1, input2], false);
        let rows = collect_all(&mut union).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn test_union_empty_inputs() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION users");
        crate::run_sql!(storage, "CREATE INDEX ON users(name)");

        let input1: Box<dyn Operator + '_> = Box::new(IndexScanOp::new(
            &*storage,
            "users".to_string(),
            IndexRef {
                collection: "users".to_string(),
                fields: vec!["name".to_string()],
            },
            IndexLookup::Eq {
                field: "name".to_string(),
                value: Value::String("nobody".to_string()),
            },
        ));

        let mut union = UnionOp::new(vec![input1], true);
        let rows = collect_all(&mut union).unwrap();
        assert_eq!(rows.len(), 0);
    }
}
