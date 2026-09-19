//! Vector Index Scan Operator
//!
//! Volcano-style operator for KNN vector search.

use crate::document::{Document, Value};
use crate::query::ast::Expr;
use crate::query::error::ExecuteError;
use crate::query::execute::context::ExecutionContext;
use crate::query::execute::eval::eval_expr;
use crate::query::execute::operators::operator::{Operator, Row};

/// Vector index scan operator for KNN search
///
/// Executes KNN queries against the HNSW index and returns
/// matching documents with their distances.
///
/// # Distance Attachment
///
/// Each returned row includes a `$distance` field containing the
/// distance from the query vector. This enables ORDER BY $distance.
pub struct VectorIndexScanOp<'a, Ctx: ExecutionContext> {
    ctx: &'a Ctx,
    collection: String,
    field: String,
    vector: Vec<f32>,
    k: usize,
    effort: Option<usize>,

    // State
    results: Option<Vec<(String, f32)>>, // (doc_id, distance)
    position: usize,
    opened: bool,
}

impl<'a, Ctx: ExecutionContext> VectorIndexScanOp<'a, Ctx> {
    /// Create a new vector index scan operator
    ///
    /// # Arguments
    ///
    /// * `ctx` - Execution context for storage access
    /// * `collection` - Collection to search
    /// * `field` - Field containing vectors (used to find HNSW index)
    /// * `vector` - Query vector for KNN search
    /// * `k` - Number of nearest neighbors to return
    /// * `effort` - Optional search effort parameter (higher = more accurate but slower)
    pub fn new(
        ctx: &'a Ctx,
        collection: String,
        field: String,
        vector: Vec<f32>,
        k: usize,
        effort: Option<usize>,
    ) -> Self {
        Self {
            ctx,
            collection,
            field,
            vector,
            k,
            effort,
            results: None,
            position: 0,
            opened: false,
        }
    }

    /// Find the HNSW index name for the field
    fn find_index_name(&self) -> Result<String, ExecuteError> {
        self.ctx
            .find_hnsw_index_for_field(&self.collection, &self.field)
            .ok_or_else(|| {
                ExecuteError::Internal(format!(
                    "No HNSW index found for field '{}' in collection '{}'",
                    self.field, self.collection
                ))
            })
    }

    /// Fetch document by ID and attach distance
    fn fetch_with_distance(
        &self,
        doc_id: &str,
        distance: f32,
    ) -> Result<Option<Row>, ExecuteError> {
        // Extract key from doc_id (format: "collection:key")
        let key = doc_id.split(':').next_back().unwrap_or(doc_id);

        let doc = self
            .ctx
            .get_document(&self.collection, key)
            .map_err(ExecuteError::Storage)?;

        Ok(doc.map(|d| {
            // Attach distance to document fields
            let mut fields = d.fields;
            fields.insert("$distance".to_string(), Value::Float(distance as f64));
            Row::from_doc(Document { id: d.id, fields })
        }))
    }
}

impl<Ctx: ExecutionContext> Operator for VectorIndexScanOp<'_, Ctx> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        let index_name = self.find_index_name()?;

        // Execute vector search
        let results = self
            .ctx
            .vector_search(
                &self.collection,
                &index_name,
                &self.vector,
                self.k,
                self.effort,
            )
            .map_err(ExecuteError::Storage)?;

        self.results = Some(results);
        self.position = 0;
        self.opened = true;
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        let results = self
            .results
            .as_ref()
            .ok_or(ExecuteError::OperatorNotInitialized("VectorIndexScanOp"))?;

        if self.position >= results.len() {
            return Ok(None);
        }

        let (doc_id, distance) = &results[self.position];
        self.position += 1;

        // Fetch document and attach distance
        self.fetch_with_distance(doc_id, *distance)
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        self.results = None;
        self.position = 0;
        self.opened = false;
        Ok(())
    }
}

/// Lazy vector index scan operator
///
/// Takes an expression for the query vector and evaluates it at open() time.
/// This allows the vector to be a literal, variable, or computed expression.
pub struct LazyVectorIndexScanOp<'a, Ctx: ExecutionContext> {
    ctx: &'a Ctx,
    collection: String,
    field: String,
    vector_expr: Expr,
    k: usize,
    effort: Option<usize>,

    // Inner operator, created at open() time
    inner: Option<VectorIndexScanOp<'a, Ctx>>,
}

impl<'a, Ctx: ExecutionContext> LazyVectorIndexScanOp<'a, Ctx> {
    /// Create a new lazy vector index scan operator
    pub fn new(
        ctx: &'a Ctx,
        collection: String,
        field: String,
        vector_expr: Expr,
        k: usize,
        effort: Option<usize>,
    ) -> Self {
        Self {
            ctx,
            collection,
            field,
            vector_expr,
            k,
            effort,
            inner: None,
        }
    }

    /// Evaluate the vector expression to get a Vec<f32>
    fn evaluate_vector(&self) -> Result<Vec<f32>, ExecuteError> {
        // Create empty row for evaluation context
        let empty_row = Row::from_doc(Document {
            id: String::new(),
            fields: std::collections::HashMap::new(),
        });

        // Pass variable scope from context for variable resolution (e.g., $vec)
        let value = eval_expr(
            &self.vector_expr,
            &empty_row,
            None,
            self.ctx.vars(),
            self.ctx,
        )?;

        // Convert Value to Vec<f32>
        match value {
            Value::Array(arr) => {
                let mut vector = Vec::with_capacity(arr.len());
                for v in arr.into_iter() {
                    match v {
                        Value::Float(f) => vector.push(f as f32),
                        Value::Int(n) => vector.push(n as f32),
                        other => {
                            return Err(ExecuteError::TypeError {
                                expected: "Float or Int",
                                got: format!("{:?}", other),
                                op: "KNN vector element",
                            });
                        }
                    }
                }
                Ok(vector)
            }
            other => Err(ExecuteError::TypeError {
                expected: "Array",
                got: format!("{:?}", other),
                op: "KNN vector",
            }),
        }
    }
}

impl<Ctx: ExecutionContext> Operator for LazyVectorIndexScanOp<'_, Ctx> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        // Evaluate the vector expression
        let vector = self.evaluate_vector()?;

        // Create the inner operator
        let mut inner = VectorIndexScanOp::new(
            self.ctx,
            self.collection.clone(),
            self.field.clone(),
            vector,
            self.k,
            self.effort,
        );
        inner.open()?;
        self.inner = Some(inner);
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        self.inner
            .as_mut()
            .ok_or(ExecuteError::OperatorNotInitialized(
                "LazyVectorIndexScanOp",
            ))?
            .next()
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        if let Some(ref mut inner) = self.inner {
            inner.close()?;
        }
        self.inner = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap as StdHashMap;

    /// Mock execution context for testing
    struct MockContext {
        documents: StdHashMap<String, Document>,
        hnsw_indexes: StdHashMap<(String, String), String>, // (collection, field) -> index_name
        search_results: Vec<(String, f32)>,
    }

    impl MockContext {
        fn new() -> Self {
            Self {
                documents: StdHashMap::new(),
                hnsw_indexes: StdHashMap::new(),
                search_results: Vec::new(),
            }
        }

        fn with_doc(mut self, id: &str, fields: StdHashMap<String, Value>) -> Self {
            self.documents.insert(
                id.to_string(),
                Document {
                    id: id.to_string(),
                    fields,
                },
            );
            self
        }

        fn with_hnsw_index(mut self, collection: &str, field: &str, index_name: &str) -> Self {
            self.hnsw_indexes.insert(
                (collection.to_string(), field.to_string()),
                index_name.to_string(),
            );
            self
        }

        fn with_search_results(mut self, results: Vec<(String, f32)>) -> Self {
            self.search_results = results;
            self
        }
    }

    impl crate::query::execute::DocumentOps for MockContext {
        fn get_document(
            &self,
            collection: &str,
            key: &str,
        ) -> Result<Option<Document>, crate::error::StorageError> {
            let full_id = format!("{}:{}", collection, key);
            Ok(self.documents.get(&full_id).cloned())
        }
        fn list_documents(
            &self,
            _: &str,
            _: usize,
        ) -> Result<Vec<Document>, crate::error::StorageError> {
            Ok(Vec::new())
        }
        fn list_documents_paged(
            &self,
            _: &str,
            _: Option<&str>,
            _: usize,
        ) -> Result<(Vec<Document>, Option<String>), crate::error::StorageError> {
            Ok((Vec::new(), None))
        }
        fn set_document(&self, _: &Document) -> Result<(), crate::error::StorageError> {
            Ok(())
        }
        fn create_document(&self, _: &Document) -> Result<bool, crate::error::StorageError> {
            Ok(true)
        }
        fn delete_document(&self, _: &str, _: &str) -> Result<bool, crate::error::StorageError> {
            Ok(false)
        }
        fn get_documents_by_ids(
            &self,
            _: &str,
            _: &[String],
        ) -> Result<Vec<Document>, crate::error::StorageError> {
            Ok(Vec::new())
        }
        fn collection_exists(&self, _: &str) -> bool {
            false
        }
    }

    impl crate::query::execute::IndexOps for MockContext {
        fn scan_index_eq(
            &self,
            _: &str,
            _: &str,
            _: &Value,
            _: usize,
        ) -> Result<(Vec<String>, bool), crate::error::StorageError> {
            Ok((Vec::new(), false))
        }
        fn scan_index_in(
            &self,
            _: &str,
            _: &str,
            _: &[Value],
            _: usize,
        ) -> Result<(Vec<String>, bool), crate::error::StorageError> {
            Ok((Vec::new(), false))
        }
        fn scan_index_range(
            &self,
            _: &str,
            _: &str,
            _: &Value,
            _: &str,
            _: usize,
        ) -> Result<(Vec<String>, bool), crate::error::StorageError> {
            Ok((Vec::new(), false))
        }
        fn scan_index_range_between(
            &self,
            _: &str,
            _: &str,
            _: &Value,
            _: bool,
            _: &Value,
            _: bool,
            _: usize,
        ) -> Result<(Vec<String>, bool), crate::error::StorageError> {
            Ok((Vec::new(), false))
        }
        fn scan_compound_index_eq(
            &self,
            _: &str,
            _: &[(&str, &Value)],
            _: usize,
        ) -> Result<(Vec<String>, bool), crate::error::StorageError> {
            Ok((Vec::new(), false))
        }
        fn scan_compound_index_eq_docs(
            &self,
            _: &str,
            _: &[(&str, &Value)],
            _: usize,
        ) -> Result<(Vec<Document>, bool), crate::error::StorageError> {
            Ok((Vec::new(), false))
        }
        fn find_index_for_field(&self, _: &str, _: &str) -> Option<Vec<String>> {
            None
        }
        fn find_compound_index(&self, _: &str, _: &[&str]) -> Option<Vec<String>> {
            None
        }
        fn find_fts_index_for_field(&self, _: &str, _: &str) -> Option<String> {
            None
        }
        fn find_fts_index(&self, _: &str) -> Option<String> {
            None
        }
        fn fts_search(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: usize,
        ) -> Result<Vec<(String, f32)>, crate::error::StorageError> {
            Ok(vec![])
        }
        fn fts_search_with_boosts(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &[(String, f64)],
            _: usize,
        ) -> Result<Vec<(String, f32)>, crate::error::StorageError> {
            Ok(vec![])
        }
        fn find_hnsw_index_for_field(&self, collection: &str, field: &str) -> Option<String> {
            self.hnsw_indexes
                .get(&(collection.to_string(), field.to_string()))
                .cloned()
        }
        fn vector_search(
            &self,
            _: &str,
            _: &str,
            _: &[f32],
            _: usize,
            _: Option<usize>,
        ) -> Result<Vec<(String, f32)>, crate::error::StorageError> {
            Ok(self.search_results.clone())
        }
    }

    impl crate::query::execute::EdgeOps for MockContext {
        fn create_edge(&self, _: &crate::edge::Edge) -> Result<(), crate::error::StorageError> {
            Ok(())
        }
        fn get_edge(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<Option<crate::edge::Edge>, crate::error::StorageError> {
            Ok(None)
        }
        fn delete_edge(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<bool, crate::error::StorageError> {
            Ok(false)
        }
        fn list_edges_out(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
            _: usize,
        ) -> Result<(Vec<crate::edge::Edge>, Option<String>), crate::error::StorageError> {
            Ok((Vec::new(), None))
        }
        fn list_edges_in(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
            _: usize,
        ) -> Result<(Vec<crate::edge::Edge>, Option<String>), crate::error::StorageError> {
            Ok((Vec::new(), None))
        }
    }

    impl ExecutionContext for MockContext {}

    #[test]
    fn test_vector_scan_no_index() {
        let ctx = MockContext::new();
        let mut op = VectorIndexScanOp::new(
            &ctx,
            "items".to_string(),
            "embedding".to_string(),
            vec![1.0, 2.0, 3.0],
            10,
            None,
        );

        // Should fail because no HNSW index exists
        let result = op.open();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No HNSW index found")
        );
    }

    #[test]
    fn test_vector_scan_empty_results() {
        let ctx = MockContext::new().with_hnsw_index("items", "embedding", "embedding_hnsw");

        let mut op = VectorIndexScanOp::new(
            &ctx,
            "items".to_string(),
            "embedding".to_string(),
            vec![1.0, 2.0, 3.0],
            10,
            None,
        );

        op.open().unwrap();
        assert!(op.next().unwrap().is_none());
        op.close().unwrap();
    }

    #[test]
    fn test_vector_scan_with_results() {
        let mut docs = StdHashMap::new();
        docs.insert("name".to_string(), Value::String("Item 1".to_string()));

        let ctx = MockContext::new()
            .with_hnsw_index("items", "embedding", "embedding_hnsw")
            .with_doc("items:item1", docs.clone())
            .with_search_results(vec![("items:item1".to_string(), 0.5)]);

        let mut op = VectorIndexScanOp::new(
            &ctx,
            "items".to_string(),
            "embedding".to_string(),
            vec![1.0, 2.0, 3.0],
            10,
            None,
        );

        op.open().unwrap();

        let row = op.next().unwrap();
        assert!(row.is_some());
        let row = row.unwrap();

        // Check that $distance is attached
        let distance = row.doc.fields.get("$distance");
        assert!(distance.is_some());
        if let Some(Value::Float(d)) = distance {
            assert!((d - 0.5).abs() < 0.001);
        } else {
            panic!("Expected Float distance");
        }

        // Check that original field is preserved
        let name = row.doc.fields.get("name");
        assert_eq!(name, Some(&Value::String("Item 1".to_string())));

        // No more results
        assert!(op.next().unwrap().is_none());

        op.close().unwrap();
    }

    #[test]
    fn test_vector_scan_with_effort() {
        let ctx = MockContext::new().with_hnsw_index("items", "embedding", "embedding_hnsw");

        let mut op = VectorIndexScanOp::new(
            &ctx,
            "items".to_string(),
            "embedding".to_string(),
            vec![1.0, 2.0, 3.0],
            10,
            Some(100), // effort parameter
        );

        op.open().unwrap();
        assert!(op.next().unwrap().is_none());
        op.close().unwrap();
    }

    #[test]
    fn test_vector_scan_not_opened() {
        let ctx = MockContext::new().with_hnsw_index("items", "embedding", "embedding_hnsw");

        let mut op = VectorIndexScanOp::new(
            &ctx,
            "items".to_string(),
            "embedding".to_string(),
            vec![1.0, 2.0, 3.0],
            10,
            None,
        );

        // Calling next() without open() should fail
        let result = op.next();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not initialized"));
    }
}
