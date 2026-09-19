//! Offset operator — skips the first N rows

use crate::query::error::ExecuteError;
use crate::query::execute::operators::operator::{Operator, Row};

pub struct OffsetOp<I: Operator> {
    input: I,
    offset: usize,
    skipped: usize,
}

impl<I: Operator> OffsetOp<I> {
    pub fn new(input: I, offset: usize) -> Self {
        Self {
            input,
            offset,
            skipped: 0,
        }
    }
}

impl<I: Operator> Operator for OffsetOp<I> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        self.skipped = 0;
        self.input.open()
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        // Skip first `offset` rows
        while self.skipped < self.offset {
            if self.input.next()?.is_none() {
                return Ok(None); // Input exhausted before reaching offset
            }
            self.skipped += 1;
        }
        self.input.next()
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        self.input.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Value;
    use crate::query::execute::operators::operator::collect_all;
    use crate::query::execute::operators::operator::test_util::{MockOp, make_doc};

    #[test]
    fn test_offset_skips_rows() {
        let docs: Vec<_> = (1..=5i64)
            .map(|i| make_doc(vec![("x", Value::Int(i))]))
            .collect();
        let mock = MockOp::new(docs);
        let mut op = OffsetOp::new(mock, 2);
        let rows = collect_all(&mut op).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].doc.fields.get("x"), Some(&Value::Int(3)));
        assert_eq!(rows[1].doc.fields.get("x"), Some(&Value::Int(4)));
        assert_eq!(rows[2].doc.fields.get("x"), Some(&Value::Int(5)));
    }

    #[test]
    fn test_offset_zero() {
        let docs: Vec<_> = (1..=3i64)
            .map(|i| make_doc(vec![("x", Value::Int(i))]))
            .collect();
        let mock = MockOp::new(docs);
        let mut op = OffsetOp::new(mock, 0);
        let rows = collect_all(&mut op).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn test_offset_exceeds_input() {
        let docs: Vec<_> = (1..=2i64)
            .map(|i| make_doc(vec![("x", Value::Int(i))]))
            .collect();
        let mock = MockOp::new(docs);
        let mut op = OffsetOp::new(mock, 10);
        let rows = collect_all(&mut op).unwrap();
        assert_eq!(rows.len(), 0);
    }
}
