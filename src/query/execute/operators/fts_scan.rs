//! FTS Index Scan Operator
//!
//! Volcano-style operator for full-text search index scans.
//! Returns documents with relevance scores attached.

use crate::document::{Document, Value};
use crate::query::ast::expr::{FtsOperator, FtsTarget};
use crate::query::error::ExecuteError;
use crate::query::execute::context::ExecutionContext;
use crate::query::execute::operators::operator::{Operator, Row};

/// Full-text search scan operator
///
/// Executes FTS queries against the Tantivy index and returns
/// matching documents with their relevance scores.
///
/// # Score Attachment
///
/// Each returned row includes a `$score` field containing the
/// BM25 relevance score from Tantivy. This enables ORDER BY $score.
pub struct FtsIndexScanOp<'a, Ctx: ExecutionContext> {
    ctx: &'a Ctx,
    collection: String,
    target: FtsTarget,
    query: String,
    limit: Option<usize>,

    // State
    results: Option<Vec<(String, f32)>>, // (doc_id, score)
    position: usize,
    opened: bool,
}

impl<'a, Ctx: ExecutionContext> FtsIndexScanOp<'a, Ctx> {
    /// Create a new FTS index scan operator
    ///
    /// # Arguments
    ///
    /// * `ctx` - Execution context for storage access
    /// * `collection` - Collection to search
    /// * `target` - Single field or MATCH clause target
    /// * `_operator` - Simple (@@ ) or named (@:name@) FTS operator (reserved for future use)
    /// * `query` - The search query string
    /// * `limit` - Optional result limit
    pub fn new(
        ctx: &'a Ctx,
        collection: String,
        target: FtsTarget,
        _operator: FtsOperator,
        query: String,
        limit: Option<usize>,
    ) -> Self {
        Self {
            ctx,
            collection,
            target,
            query,
            limit,
            results: None,
            position: 0,
            opened: false,
        }
    }

    /// Get the fields being searched
    #[cfg(test)]
    fn get_search_fields(&self) -> Vec<(String, f64)> {
        match &self.target {
            FtsTarget::Field(field) => vec![(field.clone(), 1.0)],
            FtsTarget::Match { fields, mode: _ } => fields
                .iter()
                .map(|f| (f.name.clone(), f.boost.unwrap_or(1.0)))
                .collect(),
        }
    }

    /// Find the FTS index name for the search target
    ///
    /// For single-field searches, looks up an FTS index that covers the field.
    /// For MATCH clauses, derives the index name from all specified fields.
    fn find_index_name(&self) -> Result<String, ExecuteError> {
        match &self.target {
            FtsTarget::Field(field) => {
                // Look up an FTS index that covers this field
                self.ctx
                    .find_fts_index_for_field(&self.collection, field)
                    .or_else(|| self.ctx.find_fts_index(&self.collection))
                    .ok_or_else(|| {
                        ExecuteError::Internal(format!(
                            "No FTS index found for field '{}' in collection '{}'",
                            field, self.collection
                        ))
                    })
            }
            FtsTarget::Match { fields, .. } => {
                // For MATCH with explicit fields, derive the index name
                let mut sorted_fields: Vec<String> =
                    fields.iter().map(|f| f.name.clone()).collect();
                sorted_fields.sort();
                Ok(format!("{}_fts", sorted_fields.join("_")))
            }
        }
    }

    /// Execute FTS search and return results
    ///
    /// Calls the appropriate ExecutionContext FTS method based on the target type.
    fn execute_fts_search(&self) -> Result<Vec<(String, f32)>, ExecuteError> {
        let index_name = self.find_index_name()?;
        let limit = self.limit.unwrap_or(1000); // Default limit for FTS

        match &self.target {
            FtsTarget::Field(_) => {
                // Simple single-field search
                self.ctx
                    .fts_search(&self.collection, &index_name, &self.query, limit)
                    .map_err(ExecuteError::Storage)
            }
            FtsTarget::Match { fields, .. } => {
                // Multi-field search with boosts
                let field_boosts: Vec<(String, f64)> = fields
                    .iter()
                    .map(|f| (f.name.clone(), f.boost.unwrap_or(1.0)))
                    .collect();
                self.ctx
                    .fts_search_with_boosts(
                        &self.collection,
                        &index_name,
                        &self.query,
                        &field_boosts,
                        limit,
                    )
                    .map_err(ExecuteError::Storage)
            }
        }
    }

    /// Fetch document by ID and attach score
    fn fetch_with_score(&self, doc_id: &str, score: f32) -> Result<Option<Row>, ExecuteError> {
        // Extract key from doc_id (format: "collection:key")
        let key = doc_id.split(':').next_back().unwrap_or(doc_id);

        let doc = self
            .ctx
            .get_document(&self.collection, key)
            .map_err(ExecuteError::Storage)?;

        Ok(doc.map(|d| {
            // Attach score to document fields
            let mut fields = d.fields;
            fields.insert("$score".to_string(), Value::Float(score as f64));
            Row::from_doc(Document { id: d.id, fields })
        }))
    }
}

impl<Ctx: ExecutionContext> Operator for FtsIndexScanOp<'_, Ctx> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        // Execute FTS search
        let results = self.execute_fts_search()?;

        // Apply limit if specified
        let results = if let Some(limit) = self.limit {
            results.into_iter().take(limit).collect()
        } else {
            results
        };

        self.results = Some(results);
        self.position = 0;
        self.opened = true;
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        let results = self
            .results
            .as_ref()
            .ok_or(ExecuteError::OperatorNotInitialized("FtsIndexScanOp"))?;

        // Check limit
        if let Some(limit) = self.limit
            && self.position >= limit
        {
            return Ok(None);
        }

        if self.position < results.len() {
            let (doc_id, score) = &results[self.position];
            self.position += 1;

            // Fetch document and attach score
            self.fetch_with_score(doc_id, *score)
        } else {
            Ok(None)
        }
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        self.results = None;
        self.position = 0;
        self.opened = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::ast::expr::{MatchField, MatchMode};
    use std::collections::HashMap as StdHashMap;

    /// Mock execution context for testing
    struct MockContext {
        documents: StdHashMap<String, Document>,
    }

    impl MockContext {
        fn new() -> Self {
            Self {
                documents: StdHashMap::new(),
            }
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
            Some("title_fts".to_string())
        }
        fn find_fts_index(&self, _: &str) -> Option<String> {
            Some("title_fts".to_string())
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
    fn test_fts_scan_empty_results() {
        let ctx = MockContext::new();
        let mut op = FtsIndexScanOp::new(
            &ctx,
            "articles".to_string(),
            FtsTarget::Field("title".to_string()),
            FtsOperator::Simple,
            "hello world".to_string(),
            None,
        );

        op.open().unwrap();
        assert!(op.next().unwrap().is_none());
        op.close().unwrap();
    }

    #[test]
    fn test_fts_scan_with_limit() {
        let ctx = MockContext::new();
        let mut op = FtsIndexScanOp::new(
            &ctx,
            "articles".to_string(),
            FtsTarget::Field("body".to_string()),
            FtsOperator::Simple,
            "rust programming".to_string(),
            Some(10),
        );

        op.open().unwrap();
        // With empty stub, should return None immediately
        assert!(op.next().unwrap().is_none());
        op.close().unwrap();
    }

    #[test]
    fn test_fts_scan_match_target() {
        let ctx = MockContext::new();
        let target = FtsTarget::Match {
            fields: vec![
                MatchField {
                    name: "title".to_string(),
                    boost: Some(2.0),
                },
                MatchField {
                    name: "body".to_string(),
                    boost: None,
                },
            ],
            mode: MatchMode::Sum,
        };

        let mut op = FtsIndexScanOp::new(
            &ctx,
            "articles".to_string(),
            target,
            FtsOperator::Simple,
            "database".to_string(),
            None,
        );

        op.open().unwrap();
        assert!(op.next().unwrap().is_none());
        op.close().unwrap();
    }

    #[test]
    fn test_fts_scan_named_operator() {
        let ctx = MockContext::new();
        let mut op = FtsIndexScanOp::new(
            &ctx,
            "articles".to_string(),
            FtsTarget::Field("content".to_string()),
            FtsOperator::Named("fuzzy".to_string()),
            "search query".to_string(),
            None,
        );

        op.open().unwrap();
        assert!(op.next().unwrap().is_none());
        op.close().unwrap();
    }

    #[test]
    fn test_get_search_fields_single() {
        let ctx = MockContext::new();
        let op = FtsIndexScanOp::new(
            &ctx,
            "test".to_string(),
            FtsTarget::Field("title".to_string()),
            FtsOperator::Simple,
            "query".to_string(),
            None,
        );

        let fields = op.get_search_fields();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0, "title");
        assert_eq!(fields[0].1, 1.0);
    }

    #[test]
    fn test_get_search_fields_match() {
        let ctx = MockContext::new();
        let target = FtsTarget::Match {
            fields: vec![
                MatchField {
                    name: "title".to_string(),
                    boost: Some(3.0),
                },
                MatchField {
                    name: "body".to_string(),
                    boost: None,
                },
            ],
            mode: MatchMode::Max { tie_breaker: None },
        };

        let op = FtsIndexScanOp::new(
            &ctx,
            "test".to_string(),
            target,
            FtsOperator::Simple,
            "query".to_string(),
            None,
        );

        let fields = op.get_search_fields();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].0, "title");
        assert_eq!(fields[0].1, 3.0);
        assert_eq!(fields[1].0, "body");
        assert_eq!(fields[1].1, 1.0);
    }
}
