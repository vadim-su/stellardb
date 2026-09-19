//! DML (Data Manipulation Language) operators
//!
//! Insert, Update, Delete operators for the Volcano execution model.

use std::collections::HashMap;

use crate::document::{Document, Value};
use crate::query::ast::{Assignment, ObjectLiteral};
use crate::query::error::ExecuteError;
use crate::query::execute::context::ExecutionContext;
use crate::query::execute::operators::operator::{Operator, Row};

/// Insert operator - inserts documents and returns them
pub struct InsertOp<'a, Ctx: ExecutionContext> {
    ctx: &'a Ctx,
    collection: String,
    documents: Vec<ObjectLiteral>,

    // State
    inserted: Vec<Document>,
    position: usize,
    executed: bool,
}

impl<'a, Ctx: ExecutionContext> InsertOp<'a, Ctx> {
    pub fn new(ctx: &'a Ctx, collection: String, documents: Vec<ObjectLiteral>) -> Self {
        Self {
            ctx,
            collection,
            documents,
            inserted: Vec::new(),
            position: 0,
            executed: false,
        }
    }
}

impl<Ctx: ExecutionContext> Operator for InsertOp<'_, Ctx> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        if self.executed {
            return Ok(());
        }

        let mut documents = Vec::with_capacity(self.documents.len());
        for obj in &self.documents {
            let key = extract_id_from_object(obj).unwrap_or_else(|| nanoid::nanoid!());

            // Validate ID
            validate_document_id(&key)?;

            let doc_id = format!("{}:{}", self.collection, key);

            let fields: HashMap<String, Value> = obj
                .fields
                .iter()
                .filter(|(k, _)| k != "id")
                .cloned()
                .collect();

            let doc = Document {
                id: doc_id.clone(),
                fields,
            };

            documents.push(doc);
        }

        // A multi-value SQL INSERT must reach the storage batch pipeline as a
        // single operation. Calling create_document in this loop would perform
        // one primary, external-index, and cleanup commit per row.
        let created = self.ctx.create_documents(&documents).map_err(|err| {
            if let crate::error::StorageError::AlreadyExists(id) = &err {
                ExecuteError::AlreadyExists(id.clone())
            } else {
                ExecuteError::Storage(err)
            }
        })?;

        for (doc, was_created) in documents.iter().zip(created) {
            if !was_created {
                return Err(ExecuteError::AlreadyExists(doc.id.clone()));
            }
        }

        self.inserted = documents;

        self.executed = true;
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        if self.position < self.inserted.len() {
            let doc = self.inserted[self.position].clone();
            self.position += 1;
            Ok(Some(Row::from_doc(doc)))
        } else {
            Ok(None)
        }
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        Ok(())
    }
}

/// Update operator - updates documents from input and returns updated versions
pub struct UpdateOp<'a, Ctx: ExecutionContext, I: Operator> {
    ctx: &'a Ctx,
    assignments: Vec<Assignment>,
    input: I,

    // State
    updated: Vec<Document>,
    position: usize,
    executed: bool,
}

impl<'a, Ctx: ExecutionContext, I: Operator> UpdateOp<'a, Ctx, I> {
    pub fn new(ctx: &'a Ctx, _collection: String, assignments: Vec<Assignment>, input: I) -> Self {
        Self {
            ctx,
            assignments,
            input,
            updated: Vec::new(),
            position: 0,
            executed: false,
        }
    }
}

impl<Ctx: ExecutionContext, I: Operator> Operator for UpdateOp<'_, Ctx, I> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        if self.executed {
            return Ok(());
        }

        self.input.open()?;

        while let Some(row) = self.input.next()? {
            let mut fields = row.doc.fields.clone();
            for assignment in &self.assignments {
                // Evaluate expression in context of current row
                let value = crate::query::execute::eval::eval_expr(
                    &assignment.expr,
                    &row,
                    None,
                    None,
                    self.ctx,
                )?;
                // Apply value at path (supports nested fields)
                crate::query::execute::eval::set_nested_field(
                    &mut fields,
                    &assignment.path,
                    value,
                )?;
            }

            let updated_doc = Document {
                id: row.doc.id.clone(),
                fields,
            };
            self.ctx
                .set_document(&updated_doc)
                .map_err(ExecuteError::Storage)?;
            self.updated.push(updated_doc);
        }

        self.input.close()?;
        self.executed = true;
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        if self.position < self.updated.len() {
            let doc = self.updated[self.position].clone();
            self.position += 1;
            Ok(Some(Row::from_doc(doc)))
        } else {
            Ok(None)
        }
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        Ok(())
    }
}

/// Delete operator - deletes documents from input, returns deleted documents
pub struct DeleteOp<'a, Ctx: ExecutionContext, I: Operator> {
    ctx: &'a Ctx,
    collection: String,
    input: I,

    // State
    deleted_docs: Vec<Document>,
    position: usize,
    executed: bool,
}

impl<'a, Ctx: ExecutionContext, I: Operator> DeleteOp<'a, Ctx, I> {
    pub fn new(ctx: &'a Ctx, collection: String, input: I) -> Self {
        Self {
            ctx,
            collection,
            input,
            deleted_docs: Vec::new(),
            position: 0,
            executed: false,
        }
    }

    /// Get the count of deleted documents (after execution)
    pub fn delete_count(&self) -> usize {
        self.deleted_docs.len()
    }
}

impl<Ctx: ExecutionContext, I: Operator> Operator for DeleteOp<'_, Ctx, I> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        if self.executed {
            return Ok(());
        }

        self.input.open()?;

        while let Some(row) = self.input.next()? {
            self.ctx
                .delete_document(&self.collection, row.doc.key())
                .map_err(ExecuteError::Storage)?;
            self.deleted_docs.push(row.doc);
        }

        self.input.close()?;
        self.executed = true;
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        // Return deleted documents so caller can see what was deleted
        if self.position < self.deleted_docs.len() {
            let doc = self.deleted_docs[self.position].clone();
            self.position += 1;
            Ok(Some(Row::from_doc(doc)))
        } else {
            Ok(None)
        }
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        Ok(())
    }
}

// Helper functions

fn extract_id_from_object(obj: &ObjectLiteral) -> Option<String> {
    obj.fields
        .iter()
        .find(|(k, _)| k == "id")
        .and_then(|(_, v)| match v {
            Value::String(s) => Some(s.clone()),
            Value::Int(n) => Some(n.to_string()),
            Value::Float(f) => Some(f.to_string()),
            _ => None,
        })
}

fn validate_document_id(key: &str) -> Result<(), ExecuteError> {
    if key.is_empty() {
        return Err(ExecuteError::InvalidId(
            "document ID cannot be empty. Example: CREATE user:alice SET name = 'Alice'"
                .to_string(),
        ));
    }
    if key.contains(':') {
        return Err(ExecuteError::InvalidId(format!(
            "document ID '{}' cannot contain ':' (the full format is collection:key, e.g., user:alice)",
            key
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use crate::query::ast::Expr;
    use crate::query::execute::operators::scan::TableScanOp;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Arc<Database>) {
        let tmp = TempDir::new().unwrap();
        let storage = Arc::new(Database::open(tmp.path()).unwrap());
        (tmp, storage)
    }

    #[test]
    fn test_insert_op() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION test");

        let docs = vec![ObjectLiteral {
            fields: vec![
                ("id".to_string(), Value::String("alice".to_string())),
                ("name".to_string(), Value::String("Alice".to_string())),
            ],
        }];

        let mut op = InsertOp::new(&*storage, "test".to_string(), docs);
        op.open().unwrap();

        let row = op.next().unwrap();
        assert!(row.is_some());
        let doc = row.unwrap();
        assert_eq!(doc.doc.id, "test:alice");

        assert!(op.next().unwrap().is_none());
        op.close().unwrap();

        // Verify persisted
        let found = storage.get_document("test", "alice").unwrap();
        assert!(found.is_some());
    }

    #[test]
    fn test_insert_op_auto_id() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION test");

        let docs = vec![ObjectLiteral {
            fields: vec![("name".to_string(), Value::String("Bob".to_string()))],
        }];

        let mut op = InsertOp::new(&*storage, "test".to_string(), docs);
        op.open().unwrap();

        let row = op.next().unwrap();
        assert!(row.is_some());
        let doc = row.unwrap();
        assert!(doc.doc.id.starts_with("test:"));
        assert!(doc.doc.id.len() > "test:".len()); // Has auto-generated ID

        op.close().unwrap();
    }

    #[test]
    fn test_insert_op_duplicate() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION test");
        crate::run_sql!(storage, "INSERT INTO test {id: 'alice'}");

        let docs = vec![ObjectLiteral {
            fields: vec![("id".to_string(), Value::String("alice".to_string()))],
        }];

        let mut op = InsertOp::new(&*storage, "test".to_string(), docs);
        let result = op.open();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ExecuteError::AlreadyExists(_)
        ));
    }

    #[test]
    fn test_insert_op_invalid_id() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION test");

        let docs = vec![ObjectLiteral {
            fields: vec![("id".to_string(), Value::String("bad:id".to_string()))],
        }];

        let mut op = InsertOp::new(&*storage, "test".to_string(), docs);
        let result = op.open();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ExecuteError::InvalidId(_)));
    }

    #[test]
    fn test_update_op() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION test");
        crate::run_sql!(
            storage,
            "INSERT INTO test {id: 'alice', name: 'Alice', age: 25}"
        );

        let scan = TableScanOp::new(&*storage, "test".to_string(), None);
        let assignments = vec![Assignment {
            path: vec!["age".to_string()],
            expr: Expr::Literal(Value::Int(30)),
        }];

        let mut op = UpdateOp::new(&*storage, "test".to_string(), assignments, scan);
        op.open().unwrap();

        let row = op.next().unwrap();
        assert!(row.is_some());
        let doc = row.unwrap();
        assert_eq!(doc.doc.fields.get("age"), Some(&Value::Int(30)));

        op.close().unwrap();

        // Verify persisted
        let found = storage.get_document("test", "alice").unwrap().unwrap();
        assert_eq!(found.fields.get("age"), Some(&Value::Int(30)));
    }

    #[test]
    fn test_update_op_multiple_fields() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION test");
        crate::run_sql!(
            storage,
            "INSERT INTO test {id: 'bob', name: 'Bob', age: 20}"
        );

        let scan = TableScanOp::new(&*storage, "test".to_string(), None);
        let assignments = vec![
            Assignment {
                path: vec!["age".to_string()],
                expr: Expr::Literal(Value::Int(25)),
            },
            Assignment {
                path: vec!["city".to_string()],
                expr: Expr::Literal(Value::String("NYC".to_string())),
            },
        ];

        let mut op = UpdateOp::new(&*storage, "test".to_string(), assignments, scan);
        op.open().unwrap();

        let row = op.next().unwrap();
        assert!(row.is_some());
        let doc = row.unwrap();
        assert_eq!(doc.doc.fields.get("age"), Some(&Value::Int(25)));
        assert_eq!(
            doc.doc.fields.get("city"),
            Some(&Value::String("NYC".to_string()))
        );

        op.close().unwrap();
    }

    #[test]
    fn test_delete_op() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION test");
        crate::run_sql!(storage, "INSERT INTO test {id: 'alice'}, {id: 'bob'}");

        let scan = TableScanOp::new(&*storage, "test".to_string(), None);
        let mut op = DeleteOp::new(&*storage, "test".to_string(), scan);
        op.open().unwrap();

        // Should return deleted docs
        assert!(op.next().unwrap().is_some());
        assert!(op.next().unwrap().is_some());
        assert!(op.next().unwrap().is_none());

        assert_eq!(op.delete_count(), 2);
        op.close().unwrap();

        // Verify deleted
        assert!(storage.get_document("test", "alice").unwrap().is_none());
        assert!(storage.get_document("test", "bob").unwrap().is_none());
    }

    #[test]
    fn test_delete_op_empty_collection() {
        let (_tmp, storage) = setup();
        crate::run_sql!(storage, "DEFINE COLLECTION test");

        let scan = TableScanOp::new(&*storage, "test".to_string(), None);
        let mut op = DeleteOp::new(&*storage, "test".to_string(), scan);
        op.open().unwrap();

        assert!(op.next().unwrap().is_none());
        assert_eq!(op.delete_count(), 0);

        op.close().unwrap();
    }
}
