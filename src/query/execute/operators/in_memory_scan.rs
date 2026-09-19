//! In-memory scan operator — iterates over pre-materialized `Vec<Row>`

use crate::query::error::ExecuteError;
use crate::query::execute::operators::operator::{Operator, Row};

pub struct InMemoryScanOp {
    rows: Vec<Row>,
    position: usize,
}

impl InMemoryScanOp {
    pub fn new(rows: Vec<Row>) -> Self {
        Self { rows, position: 0 }
    }
}

impl Operator for InMemoryScanOp {
    fn open(&mut self) -> Result<(), ExecuteError> {
        self.position = 0;
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        if self.position < self.rows.len() {
            let row = self.rows[self.position].clone();
            self.position += 1;
            Ok(Some(row))
        } else {
            Ok(None)
        }
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        Ok(())
    }
}
