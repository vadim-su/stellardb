//! Sort operator - buffers all input and sorts by expressions

use std::cmp::Ordering;

use crate::document::Value;
use crate::query::ast::{OrderDirection, OrderItem};
use crate::query::error::ExecuteError;
use crate::query::execute::VarScope;
use crate::query::execute::context::ExecutionContext;
use crate::query::execute::eval::eval_expr;
use crate::query::execute::operators::accumulator::compare_values;
use crate::query::execute::operators::operator::{Operator, Row};

/// Sort operator - buffers all rows, sorts, then yields.
/// Pre-computes sort keys for each row during open().
pub struct SortOp<'a, I: Operator, Ctx: ExecutionContext> {
    input: I,
    items: Vec<OrderItem>,
    vars: Option<&'a VarScope>,
    ctx: &'a Ctx,
    /// (row, sort_keys) pairs
    buffer: Vec<(Row, Vec<Value>)>,
    index: usize,
    opened: bool,
}

impl<'a, I: Operator, Ctx: ExecutionContext> SortOp<'a, I, Ctx> {
    pub fn new(input: I, items: Vec<OrderItem>, vars: Option<&'a VarScope>, ctx: &'a Ctx) -> Self {
        Self {
            input,
            items,
            vars,
            ctx,
            buffer: Vec::new(),
            index: 0,
            opened: false,
        }
    }
}

impl<I: Operator, Ctx: ExecutionContext> Operator for SortOp<'_, I, Ctx> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        self.input.open()?;

        // Buffer all rows and pre-compute sort keys
        while let Some(row) = self.input.next()? {
            let keys: Vec<Value> = self
                .items
                .iter()
                .map(|item| eval_expr(&item.expr, &row, None, self.vars, self.ctx))
                .collect::<Result<_, _>>()?;
            self.buffer.push((row, keys));
        }

        // Sort by pre-computed keys
        let directions: Vec<OrderDirection> =
            self.items.iter().map(|item| item.direction).collect();
        self.buffer
            .sort_by(|a, b| compare_keys(&a.1, &b.1, &directions));

        self.opened = true;
        self.index = 0;
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        if self.index < self.buffer.len() {
            let row = self.buffer[self.index].0.clone();
            self.index += 1;
            Ok(Some(row))
        } else {
            Ok(None)
        }
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        self.buffer.clear();
        self.index = 0;
        self.opened = false;
        self.input.close()
    }
}

fn compare_keys(a: &[Value], b: &[Value], directions: &[OrderDirection]) -> Ordering {
    for (i, dir) in directions.iter().enumerate() {
        let ord = compare_values(Some(&a[i]), Some(&b[i]));

        let ord = match dir {
            OrderDirection::Asc => ord,
            OrderDirection::Desc => ord.reverse(),
        };

        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::ast::Expr;
    use crate::query::execute::context::NullContext;
    use crate::query::execute::operators::operator::collect_all;
    use crate::query::execute::operators::operator::test_util::MockOp;
    use crate::query::execute::operators::operator::test_util::make_doc_with_id;

    #[test]
    fn test_sort_by_int_asc() {
        let docs = vec![
            make_doc_with_id("test:3", vec![("score", Value::Int(30))]),
            make_doc_with_id("test:1", vec![("score", Value::Int(10))]),
            make_doc_with_id("test:2", vec![("score", Value::Int(20))]),
        ];
        let mock = MockOp::new(docs);
        let mut sort = SortOp::new(
            mock,
            vec![OrderItem {
                expr: Expr::Field("score".to_string()),
                direction: OrderDirection::Asc,
            }],
            None,
            &NullContext,
        );

        let rows = collect_all(&mut sort).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].doc.id, "test:1");
        assert_eq!(rows[1].doc.id, "test:2");
        assert_eq!(rows[2].doc.id, "test:3");
    }

    #[test]
    fn test_sort_by_int_desc() {
        let docs = vec![
            make_doc_with_id("test:1", vec![("score", Value::Int(10))]),
            make_doc_with_id("test:3", vec![("score", Value::Int(30))]),
            make_doc_with_id("test:2", vec![("score", Value::Int(20))]),
        ];
        let mock = MockOp::new(docs);
        let mut sort = SortOp::new(
            mock,
            vec![OrderItem {
                expr: Expr::Field("score".to_string()),
                direction: OrderDirection::Desc,
            }],
            None,
            &NullContext,
        );

        let rows = collect_all(&mut sort).unwrap();
        assert_eq!(rows[0].doc.id, "test:3");
        assert_eq!(rows[1].doc.id, "test:2");
        assert_eq!(rows[2].doc.id, "test:1");
    }

    #[test]
    fn test_sort_by_string() {
        let docs = vec![
            make_doc_with_id(
                "test:c",
                vec![("name", Value::String("Charlie".to_string()))],
            ),
            make_doc_with_id("test:a", vec![("name", Value::String("Alice".to_string()))]),
            make_doc_with_id("test:b", vec![("name", Value::String("Bob".to_string()))]),
        ];
        let mock = MockOp::new(docs);
        let mut sort = SortOp::new(
            mock,
            vec![OrderItem {
                expr: Expr::Field("name".to_string()),
                direction: OrderDirection::Asc,
            }],
            None,
            &NullContext,
        );

        let rows = collect_all(&mut sort).unwrap();
        assert_eq!(rows[0].doc.id, "test:a");
        assert_eq!(rows[1].doc.id, "test:b");
        assert_eq!(rows[2].doc.id, "test:c");
    }

    #[test]
    fn test_sort_nulls_first() {
        let docs = vec![
            make_doc_with_id("test:2", vec![("score", Value::Int(20))]),
            make_doc_with_id("test:null", vec![]),
            make_doc_with_id("test:1", vec![("score", Value::Int(10))]),
        ];
        let mock = MockOp::new(docs);
        let mut sort = SortOp::new(
            mock,
            vec![OrderItem {
                expr: Expr::Field("score".to_string()),
                direction: OrderDirection::Asc,
            }],
            None,
            &NullContext,
        );

        let rows = collect_all(&mut sort).unwrap();
        assert_eq!(rows[0].doc.id, "test:null");
        assert_eq!(rows[1].doc.id, "test:1");
        assert_eq!(rows[2].doc.id, "test:2");
    }

    #[test]
    fn test_sort_multiple_fields() {
        let docs = vec![
            make_doc_with_id(
                "test:a2",
                vec![
                    ("status", Value::String("active".to_string())),
                    ("score", Value::Int(20)),
                ],
            ),
            make_doc_with_id(
                "test:a1",
                vec![
                    ("status", Value::String("active".to_string())),
                    ("score", Value::Int(10)),
                ],
            ),
            make_doc_with_id(
                "test:i1",
                vec![
                    ("status", Value::String("inactive".to_string())),
                    ("score", Value::Int(5)),
                ],
            ),
        ];
        let mock = MockOp::new(docs);
        let mut sort = SortOp::new(
            mock,
            vec![
                OrderItem {
                    expr: Expr::Field("status".to_string()),
                    direction: OrderDirection::Asc,
                },
                OrderItem {
                    expr: Expr::Field("score".to_string()),
                    direction: OrderDirection::Desc,
                },
            ],
            None,
            &NullContext,
        );

        let rows = collect_all(&mut sort).unwrap();
        assert_eq!(rows[0].doc.id, "test:a2");
        assert_eq!(rows[1].doc.id, "test:a1");
        assert_eq!(rows[2].doc.id, "test:i1");
    }

    #[test]
    fn test_sort_empty() {
        let mock = MockOp::new(vec![]);
        let mut sort = SortOp::new(
            mock,
            vec![OrderItem {
                expr: Expr::Field("score".to_string()),
                direction: OrderDirection::Asc,
            }],
            None,
            &NullContext,
        );

        let rows = collect_all(&mut sort).unwrap();
        assert!(rows.is_empty());
    }
}
