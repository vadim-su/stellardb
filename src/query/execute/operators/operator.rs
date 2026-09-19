//! Volcano-style pull-based execution model

use std::time::Instant;

use crate::document::{Document, Value};
use crate::query::error::ExecuteError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadlineSource {
    QueryExecution,
    ServerDb,
    Request,
}

/// Absolute deadline and the limit that selected it. None means no timeout.
pub type QueryDeadline = Option<(Instant, DeadlineSource)>;

/// Check if query deadline has been exceeded.
#[inline]
pub fn check_deadline(deadline: QueryDeadline) -> Result<(), ExecuteError> {
    if let Some((dl, source)) = deadline
        && Instant::now() > dl
    {
        return Err(match source {
            DeadlineSource::QueryExecution => {
                ExecuteError::QueryTimeout("query execution deadline".to_string())
            }
            DeadlineSource::ServerDb => ExecuteError::ServerDeadlineExceeded,
            DeadlineSource::Request => {
                ExecuteError::QueryTimeout("request transport deadline".to_string())
            }
        });
    }
    Ok(())
}

/// Collect all results from an operator with deadline checking
pub fn collect_all_with_deadline<O: Operator + ?Sized>(
    op: &mut O,
    deadline: QueryDeadline,
) -> Result<Vec<Row>, ExecuteError> {
    op.open()?;
    let mut results = Vec::new();
    while let Some(row) = op.next()? {
        check_deadline(deadline)?;
        results.push(row);
    }
    op.close()?;
    Ok(results)
}

/// A row in the result set.
///
/// Represents either a document or a scalar value in the query execution pipeline.
///
/// # Invariants
///
/// - **Document row** (`scalar_value = None`): The `doc` field contains the document data.
///   Output is a JSON object with the document fields.
///
/// - **Scalar row** (`scalar_value = Some(v)`): The `scalar_value` is used for output.
///   The `doc` field may contain `$value` and alias fields for expression evaluation,
///   but `doc.fields` should be considered internal. Output is the scalar value directly.
///
/// # Usage
///
/// - Use `Row::from_doc(doc)` to create a document row
/// - Use `Row::scalar(value)` to create a scalar row
/// - Use `Row::scalar_with_alias(value, alias)` to create a scalar row with an alias
/// - Use `row.is_scalar()` to check the row type
/// - Use `row_to_json(&row)` to convert to JSON output
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The document data. For scalar rows, may contain internal fields like `$value`.
    pub doc: Document,
    /// If Some, this row outputs as a scalar value instead of a document.
    pub scalar_value: Option<Value>,
    /// Alias name for scalar_value (e.g., "x" from "FROM [...] AS x")
    pub scalar_alias: Option<String>,
}

impl Row {
    /// Create a document row (non-scalar).
    pub fn from_doc(doc: Document) -> Self {
        Self {
            doc,
            scalar_value: None,
            scalar_alias: None,
        }
    }

    /// Create a scalar row with a value.
    /// The `doc` field will be empty - use `doc.fields` only if you need to add
    /// internal fields like `$value` for expression evaluation.
    pub fn scalar(value: Value) -> Self {
        Self {
            doc: Document {
                id: String::new(),
                fields: std::collections::HashMap::new(),
            },
            scalar_value: Some(value),
            scalar_alias: None,
        }
    }

    /// Create a scalar row with a value and alias.
    pub fn scalar_with_alias(value: Value, alias: String) -> Self {
        Self {
            doc: Document {
                id: String::new(),
                fields: std::collections::HashMap::new(),
            },
            scalar_value: Some(value),
            scalar_alias: Some(alias),
        }
    }

    /// Check if this row is a scalar value.
    pub fn is_scalar(&self) -> bool {
        self.scalar_value.is_some()
    }

    /// Get the scalar value if this is a scalar row.
    pub fn scalar_value(&self) -> Option<&Value> {
        self.scalar_value.as_ref()
    }

    /// Get the document fields.
    pub fn fields(&self) -> &std::collections::HashMap<String, Value> {
        &self.doc.fields
    }

    /// Resolve a field name to a value.
    /// Priority: scalar_value ($value/alias) → doc.id → doc.fields
    pub fn get_field(&self, name: &str) -> Option<Value> {
        // 1. For scalar rows, check $value and alias
        if let Some(ref scalar) = self.scalar_value {
            if name == "$value" {
                return Some(scalar.clone());
            }
            if self.scalar_alias.as_deref() == Some(name) {
                return Some(scalar.clone());
            }
        }

        // 2. Special handling for id - returns Reference for FETCH support
        if name == "id" {
            return Some(Value::Reference(self.doc.id.clone()));
        }

        // 3. Document fields
        self.doc.fields.get(name).cloned()
    }

    /// Check if a field exists (for IS NONE / IS NOT NONE).
    /// Returns true if field would resolve to a value (not missing).
    pub fn has_field(&self, name: &str) -> bool {
        // Scalar $value and alias are always "present"
        if self.scalar_value.is_some()
            && (name == "$value" || self.scalar_alias.as_deref() == Some(name))
        {
            return true;
        }
        // id always exists
        if name == "id" {
            return true;
        }
        self.doc.fields.contains_key(name)
    }

    /// Get the base value for $value references.
    /// For scalar rows, returns scalar_value.
    /// For document rows, checks for explicit "$value" field first,
    /// then falls back to the whole document as an Object.
    pub fn get_value(&self) -> Value {
        if let Some(ref scalar) = self.scalar_value {
            scalar.clone()
        } else {
            // For document rows, check for explicit $value field first
            // (e.g., FROM [{"$value": 42}] should return 42 for $value)
            self.doc
                .fields
                .get("$value")
                .cloned()
                .unwrap_or_else(|| Value::Object(self.doc.fields.clone()))
        }
    }
}

/// Volcano-style pull-based iterator
///
/// Each operator pulls data from its children on demand.
/// This enables lazy evaluation and early termination.
pub trait Operator {
    /// Initialize the operator (open cursors, prepare state)
    fn open(&mut self) -> Result<(), ExecuteError>;

    /// Get the next row, or None if exhausted
    fn next(&mut self) -> Result<Option<Row>, ExecuteError>;

    /// Clean up resources
    fn close(&mut self) -> Result<(), ExecuteError>;
}

/// Implement Operator for boxed trait objects to enable dynamic dispatch
impl<'a> Operator for Box<dyn Operator + 'a> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        (**self).open()
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        (**self).next()
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        (**self).close()
    }
}

/// Collect all results from an operator into a Vec
pub fn collect_all<O: Operator + ?Sized>(op: &mut O) -> Result<Vec<Row>, ExecuteError> {
    op.open()?;

    let mut results = Vec::new();
    while let Some(row) = op.next()? {
        results.push(row);
    }

    op.close()?;
    Ok(results)
}

#[cfg(test)]
mod deadline_tests {
    use super::*;

    #[test]
    fn deadline_reports_the_limit_that_won() {
        let past = Instant::now() - std::time::Duration::from_millis(1);
        assert!(matches!(
            check_deadline(Some((past, DeadlineSource::QueryExecution))),
            Err(ExecuteError::QueryTimeout(_))
        ));
        assert!(matches!(
            check_deadline(Some((past, DeadlineSource::ServerDb))),
            Err(ExecuteError::ServerDeadlineExceeded)
        ));
        assert!(matches!(
            check_deadline(Some((past, DeadlineSource::Request))),
            Err(ExecuteError::QueryTimeout(_))
        ));
    }
}

/// Convert document to JSON object (helper)
pub fn document_to_json(doc: &Document) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    if !doc.id.is_empty() {
        obj.insert("id".to_string(), serde_json::json!(doc.id));
    }

    for (k, v) in &doc.fields {
        obj.insert(k.clone(), serde_json::Value::from(v.clone()));
    }

    serde_json::Value::Object(obj)
}

/// Convert row to JSON value
/// If row is scalar, returns the scalar value directly.
/// Otherwise, returns the document as JSON object.
pub fn row_to_json(row: &Row) -> serde_json::Value {
    if let Some(ref scalar) = row.scalar_value {
        serde_json::Value::from(scalar.clone())
    } else {
        document_to_json(&row.doc)
    }
}

/// Convert a vector of rows to JSON array
pub fn rows_to_json(rows: &[Row]) -> serde_json::Value {
    let json_rows: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    serde_json::Value::Array(json_rows)
}

/// Execute operator and return JSON array
pub fn execute_to_json<O: Operator + ?Sized>(
    op: &mut O,
) -> Result<serde_json::Value, ExecuteError> {
    let rows = collect_all(op)?;
    let json_rows: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(serde_json::Value::Array(json_rows))
}

/// Shared test utilities for operator tests
#[cfg(test)]
pub mod test_util {
    use super::*;

    /// A mock operator that yields a fixed set of documents.
    pub struct MockOp {
        docs: Vec<Document>,
        index: usize,
    }

    impl MockOp {
        pub fn new(docs: Vec<Document>) -> Self {
            Self { docs, index: 0 }
        }
    }

    impl Operator for MockOp {
        fn open(&mut self) -> Result<(), ExecuteError> {
            self.index = 0;
            Ok(())
        }
        fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
            if self.index < self.docs.len() {
                let doc = self.docs[self.index].clone();
                self.index += 1;
                Ok(Some(Row::from_doc(doc)))
            } else {
                Ok(None)
            }
        }
        fn close(&mut self) -> Result<(), ExecuteError> {
            Ok(())
        }
    }

    /// Create a Document with the given fields.
    pub fn make_doc(fields: Vec<(&str, Value)>) -> Document {
        Document {
            id: String::new(),
            fields: fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
    }

    /// Create a Document with an ID and fields.
    pub fn make_doc_with_id(id: &str, fields: Vec<(&str, Value)>) -> Document {
        Document {
            id: id.to_string(),
            fields: fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
    }
}
