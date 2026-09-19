//! Parent context stack for nested correlated subqueries.

use super::operators::operator::Row;

/// Stack of parent rows for nested correlated subqueries.
///
/// When executing `(SELECT ... WHERE x = $parent.parent.field)`,
/// we need access to multiple levels of parent context.
#[derive(Debug, Clone)]
pub struct ParentContext {
    /// Stack of parent rows.
    /// stack[0] = immediate $parent
    /// stack[1] = $parent.parent (grandparent)
    /// stack[2] = $parent.parent.parent, etc.
    stack: Vec<Row>,
}

impl ParentContext {
    /// Create a new context with a single parent row.
    pub fn new(row: Row) -> Self {
        Self { stack: vec![row] }
    }

    /// Push a new parent level, returning a new context.
    /// The new row becomes $parent, previous $parent becomes $parent.parent.
    pub fn push(&self, row: Row) -> Self {
        let mut new_stack = vec![row];
        new_stack.extend(self.stack.iter().cloned());
        Self { stack: new_stack }
    }

    /// Get parent at given depth.
    /// depth=0 → $parent, depth=1 → $parent.parent, etc.
    pub fn get(&self, depth: usize) -> Option<&Row> {
        self.stack.get(depth)
    }

    /// Number of available parent levels.
    pub fn len(&self) -> usize {
        self.stack.len()
    }

    /// Check if context is empty.
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, Value};
    use std::collections::HashMap;

    fn make_row(id: &str, fields: &[(&str, Value)]) -> Row {
        let mut field_map = HashMap::new();
        for (k, v) in fields {
            field_map.insert(k.to_string(), v.clone());
        }
        Row::from_doc(Document {
            id: id.to_string(),
            fields: field_map,
        })
    }

    #[test]
    fn test_single_parent() {
        let row = make_row("user:1", &[("name", Value::String("Alice".into()))]);
        let ctx = ParentContext::new(row);

        assert_eq!(ctx.len(), 1);
        assert!(ctx.get(0).is_some());
        assert!(ctx.get(1).is_none());
    }

    #[test]
    fn test_nested_parents() {
        let user = make_row("user:1", &[("name", Value::String("Alice".into()))]);
        let post = make_row("post:1", &[("title", Value::String("Hello".into()))]);
        let comment = make_row("comment:1", &[("text", Value::String("Nice!".into()))]);

        let ctx = ParentContext::new(user);
        let ctx = ctx.push(post);
        let ctx = ctx.push(comment);

        assert_eq!(ctx.len(), 3);

        // $parent = comment
        assert_eq!(ctx.get(0).unwrap().doc.id, "comment:1");
        // $parent.parent = post
        assert_eq!(ctx.get(1).unwrap().doc.id, "post:1");
        // $parent.parent.parent = user
        assert_eq!(ctx.get(2).unwrap().doc.id, "user:1");
        // Too deep
        assert!(ctx.get(3).is_none());
    }
}
