// src/query/execute/limit.rs

use crate::query::error::ExecuteError;
use crate::query::execute::operators::operator::{Operator, Row};

/// Limit operator - stops after N rows
pub struct LimitOp<I: Operator> {
    input: I,
    limit: usize,
    count: usize,
}

impl<I: Operator> LimitOp<I> {
    pub fn new(input: I, limit: usize) -> Self {
        Self {
            input,
            limit,
            count: 0,
        }
    }
}

impl<I: Operator> Operator for LimitOp<I> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        self.count = 0;
        self.input.open()
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        if self.count >= self.limit {
            return Ok(None);
        }

        if let Some(row) = self.input.next()? {
            self.count += 1;
            Ok(Some(row))
        } else {
            Ok(None)
        }
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        self.input.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, Value};
    use crate::query::execute::operators::operator::collect_all;
    use crate::query::execute::operators::operator::test_util::MockOp;
    use std::collections::HashMap;

    fn make_test_doc(id: &str) -> Document {
        Document {
            id: format!("test:{}", id),
            fields: HashMap::new(),
        }
    }

    #[test]
    fn test_limit_basic() {
        let docs: Vec<Document> = (1..=5).map(|i| make_test_doc(&i.to_string())).collect();
        let mock = MockOp::new(docs);
        let mut limit = LimitOp::new(mock, 3);

        let rows = collect_all(&mut limit).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].doc.id, "test:1");
        assert_eq!(rows[1].doc.id, "test:2");
        assert_eq!(rows[2].doc.id, "test:3");
    }

    #[test]
    fn test_limit_zero() {
        let docs: Vec<Document> = (1..=5).map(|i| make_test_doc(&i.to_string())).collect();
        let mock = MockOp::new(docs);
        let mut limit = LimitOp::new(mock, 0);

        let rows = collect_all(&mut limit).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_limit_greater_than_input() {
        let docs: Vec<Document> = (1..=3).map(|i| make_test_doc(&i.to_string())).collect();
        let mock = MockOp::new(docs);
        let mut limit = LimitOp::new(mock, 10);

        let rows = collect_all(&mut limit).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn test_limit_empty_input() {
        let mock = MockOp::new(vec![]);
        let mut limit = LimitOp::new(mock, 5);

        let rows = collect_all(&mut limit).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_limit_one() {
        let docs: Vec<Document> = (1..=5).map(|i| make_test_doc(&i.to_string())).collect();
        let mock = MockOp::new(docs);
        let mut limit = LimitOp::new(mock, 1);

        let rows = collect_all(&mut limit).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].doc.id, "test:1");
    }

    #[test]
    fn test_limit_reopen() {
        let docs: Vec<Document> = (1..=5).map(|i| make_test_doc(&i.to_string())).collect();
        let mock = MockOp::new(docs);
        let mut limit = LimitOp::new(mock, 2);

        // First iteration
        limit.open().unwrap();
        let _ = limit.next().unwrap();
        let _ = limit.next().unwrap();
        assert!(limit.next().unwrap().is_none());
        limit.close().unwrap();

        // Reopen should reset count
        limit.open().unwrap();
        let row1 = limit.next().unwrap();
        assert!(row1.is_some());
        assert_eq!(row1.unwrap().doc.id, "test:1");
        limit.close().unwrap();
    }

    #[test]
    fn test_limit_with_fields() {
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), Value::String("Alice".to_string()));
        fields.insert("age".to_string(), Value::Int(30));

        let docs = vec![
            Document {
                id: "test:1".to_string(),
                fields: fields.clone(),
            },
            Document {
                id: "test:2".to_string(),
                fields: fields.clone(),
            },
            Document {
                id: "test:3".to_string(),
                fields,
            },
        ];

        let mock = MockOp::new(docs);
        let mut limit = LimitOp::new(mock, 2);

        let rows = collect_all(&mut limit).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].doc.fields.contains_key("name"));
        assert!(rows[1].doc.fields.contains_key("age"));
    }
}
