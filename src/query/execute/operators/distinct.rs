//! Distinct operator - removes duplicate rows

use std::collections::HashSet;

use crate::query::error::ExecuteError;
use crate::query::execute::operators::accumulator::serialize_value;
use crate::query::execute::operators::operator::{Operator, Row};

pub struct DistinctOp<I: Operator> {
    input: I,
    seen: HashSet<String>,
    opened: bool,
}

impl<I: Operator> DistinctOp<I> {
    pub fn new(input: I) -> Self {
        Self {
            input,
            seen: HashSet::new(),
            opened: false,
        }
    }
}

fn row_key(row: &Row) -> String {
    // For scalar rows, use the scalar value as the key
    if let Some(ref scalar) = row.scalar_value {
        return format!("$scalar:{}", serialize_value(scalar));
    }

    // DISTINCT compares only the projected fields, not the document ID.
    // Two rows with the same field values are duplicates even if they
    // came from different documents.
    let mut parts = Vec::new();
    let mut keys: Vec<_> = row.doc.fields.keys().collect();
    keys.sort();
    for k in keys {
        let v = row
            .doc
            .fields
            .get(k)
            .expect("field key from sorted keys must exist");
        parts.push(format!("{}:{}", k, serialize_value(v)));
    }
    parts.join("|")
}

impl<I: Operator> Operator for DistinctOp<I> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        self.input.open()?;
        self.seen.clear();
        self.opened = true;
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        while let Some(row) = self.input.next()? {
            let key = row_key(&row);
            if self.seen.insert(key) {
                return Ok(Some(row));
            }
            // Skip duplicate
        }
        Ok(None)
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        self.seen.clear();
        self.opened = false;
        self.input.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, Value};
    use crate::query::execute::operators::operator::collect_all;
    use crate::query::execute::operators::operator::test_util::MockOp;

    fn make_doc(id: &str, fields: Vec<(&str, Value)>) -> Document {
        Document {
            id: id.to_string(),
            fields: fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
    }

    #[test]
    fn test_distinct_removes_duplicates() {
        // DISTINCT compares only field content, not document IDs.
        // Same city values should dedupe even with different IDs.
        let docs = vec![
            make_doc("1", vec![("city", Value::String("NYC".into()))]),
            make_doc("2", vec![("city", Value::String("LA".into()))]),
            make_doc("3", vec![("city", Value::String("NYC".into()))]),
        ];
        let mock = MockOp::new(docs);
        let mut distinct = DistinctOp::new(mock);

        let rows = collect_all(&mut distinct).unwrap();
        // NYC and LA are the only unique city values
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn test_distinct_same_content() {
        // Same field content - should dedupe regardless of IDs
        let docs = vec![
            make_doc("1", vec![("city", Value::String("NYC".into()))]),
            make_doc("2", vec![("city", Value::String("NYC".into()))]),
            make_doc("3", vec![("city", Value::String("LA".into()))]),
        ];
        let mock = MockOp::new(docs);
        let mut distinct = DistinctOp::new(mock);

        let rows = collect_all(&mut distinct).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn test_distinct_empty_input() {
        let mock = MockOp::new(vec![]);
        let mut distinct = DistinctOp::new(mock);

        let rows = collect_all(&mut distinct).unwrap();
        assert!(rows.is_empty());
    }
}
