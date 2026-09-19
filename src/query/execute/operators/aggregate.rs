//! Aggregate operator - computes aggregate functions over input

use std::collections::HashMap;

use crate::document::Document;
use crate::query::ast::AggregateArg;
use crate::query::error::ExecuteError;
use crate::query::execute::operators::accumulator::Accumulator;
use crate::query::execute::operators::operator::{Operator, Row};
use crate::query::plan::logical::ResolvedAggregate;

/// Aggregate operator: buffers all input, computes aggregates, yields one row
pub struct AggregateOp<I: Operator> {
    input: I,
    aggregates: Vec<ResolvedAggregate>,
    result: Option<Row>,
    returned: bool,
}

impl<I: Operator> AggregateOp<I> {
    pub fn new(input: I, aggregates: Vec<ResolvedAggregate>) -> Self {
        Self {
            input,
            aggregates,
            result: None,
            returned: false,
        }
    }
}

impl<I: Operator> Operator for AggregateOp<I> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        self.input.open()?;

        let mut accumulators: Vec<Accumulator> = self
            .aggregates
            .iter()
            .map(|a| Accumulator::new(a.function, a.distinct))
            .collect();

        while let Some(row) = self.input.next()? {
            for (acc, agg) in accumulators.iter_mut().zip(&self.aggregates) {
                match &agg.arg {
                    AggregateArg::Wildcard => acc.feed_wildcard(),
                    AggregateArg::Field(f) => acc.feed(row.get_field(f).as_ref()),
                };
            }
        }

        let mut fields = HashMap::new();
        for (acc, agg) in accumulators.iter().zip(&self.aggregates) {
            fields.insert(agg.alias.clone(), acc.result());
        }

        self.result = Some(Row::from_doc(Document {
            id: String::new(),
            fields,
        }));
        self.returned = false;
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        if self.returned {
            return Ok(None);
        }
        self.returned = true;
        Ok(self.result.take())
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        self.result = None;
        self.returned = false;
        self.input.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Value;
    use crate::query::execute::operators::operator::collect_all;
    use crate::query::execute::operators::operator::test_util::{MockOp, make_doc};
    use crate::query::function::AggregateFunction;
    use rust_decimal::Decimal;

    #[test]
    fn test_count_star() {
        let docs = vec![
            make_doc(vec![("x", Value::Int(1))]),
            make_doc(vec![("x", Value::Int(2))]),
            make_doc(vec![("x", Value::Int(3))]),
        ];
        let mock = MockOp::new(docs);
        let mut agg = AggregateOp::new(
            mock,
            vec![ResolvedAggregate {
                function: AggregateFunction::Count,
                arg: AggregateArg::Wildcard,
                alias: "count(*)".to_string(),
                distinct: false,
            }],
        );

        let rows = collect_all(&mut agg).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].doc.fields.get("count(*)"), Some(&Value::Int(3)));
    }

    #[test]
    fn test_count_field_skips_null() {
        let docs = vec![
            make_doc(vec![("x", Value::Int(1))]),
            make_doc(vec![("x", Value::Null)]),
            make_doc(vec![]),
        ];
        let mock = MockOp::new(docs);
        let mut agg = AggregateOp::new(
            mock,
            vec![ResolvedAggregate {
                function: AggregateFunction::Count,
                arg: AggregateArg::Field("x".to_string()),
                alias: "count(x)".to_string(),
                distinct: false,
            }],
        );

        let rows = collect_all(&mut agg).unwrap();
        assert_eq!(rows[0].doc.fields.get("count(x)"), Some(&Value::Int(1)));
    }

    #[test]
    fn test_sum() {
        let docs = vec![
            make_doc(vec![("score", Value::Int(10))]),
            make_doc(vec![("score", Value::Int(20))]),
            make_doc(vec![("score", Value::Float(30.5))]),
        ];
        let mock = MockOp::new(docs);
        let mut agg = AggregateOp::new(
            mock,
            vec![ResolvedAggregate {
                function: AggregateFunction::Sum,
                arg: AggregateArg::Field("score".to_string()),
                alias: "sum(score)".to_string(),
                distinct: false,
            }],
        );

        let rows = collect_all(&mut agg).unwrap();
        match rows[0].doc.fields.get("sum(score)") {
            Some(Value::Float(f)) => assert!((f - 60.5).abs() < f64::EPSILON),
            other => panic!("expected Float(60.5), got {:?}", other),
        }
    }

    #[test]
    fn test_avg() {
        let docs = vec![
            make_doc(vec![("score", Value::Int(10))]),
            make_doc(vec![("score", Value::Int(20))]),
            make_doc(vec![("score", Value::Int(30))]),
        ];
        let mock = MockOp::new(docs);
        let mut agg = AggregateOp::new(
            mock,
            vec![ResolvedAggregate {
                function: AggregateFunction::Avg,
                arg: AggregateArg::Field("score".to_string()),
                alias: "avg(score)".to_string(),
                distinct: false,
            }],
        );

        let rows = collect_all(&mut agg).unwrap();
        // Int-only input → Decimal result
        match rows[0].doc.fields.get("avg(score)") {
            Some(Value::Decimal(d)) => assert_eq!(d.as_decimal(), Decimal::from(20)),
            other => panic!("expected Decimal(20), got {:?}", other),
        }
    }

    #[test]
    fn test_min_max() {
        let docs = vec![
            make_doc(vec![("score", Value::Int(30))]),
            make_doc(vec![("score", Value::Int(10))]),
            make_doc(vec![("score", Value::Int(20))]),
        ];
        let mock = MockOp::new(docs);
        let mut agg = AggregateOp::new(
            mock,
            vec![
                ResolvedAggregate {
                    function: AggregateFunction::Min,
                    arg: AggregateArg::Field("score".to_string()),
                    alias: "min(score)".to_string(),
                    distinct: false,
                },
                ResolvedAggregate {
                    function: AggregateFunction::Max,
                    arg: AggregateArg::Field("score".to_string()),
                    alias: "max(score)".to_string(),
                    distinct: false,
                },
            ],
        );

        let rows = collect_all(&mut agg).unwrap();
        assert_eq!(rows[0].doc.fields.get("min(score)"), Some(&Value::Int(10)));
        assert_eq!(rows[0].doc.fields.get("max(score)"), Some(&Value::Int(30)));
    }

    #[test]
    fn test_empty_input() {
        let mock = MockOp::new(vec![]);
        let mut agg = AggregateOp::new(
            mock,
            vec![
                ResolvedAggregate {
                    function: AggregateFunction::Count,
                    arg: AggregateArg::Wildcard,
                    alias: "count(*)".to_string(),
                    distinct: false,
                },
                ResolvedAggregate {
                    function: AggregateFunction::Sum,
                    arg: AggregateArg::Field("x".to_string()),
                    alias: "sum(x)".to_string(),
                    distinct: false,
                },
            ],
        );

        let rows = collect_all(&mut agg).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].doc.fields.get("count(*)"), Some(&Value::Int(0)));
        assert_eq!(rows[0].doc.fields.get("sum(x)"), Some(&Value::Null));
    }

    #[test]
    fn test_sum_skips_non_numeric() {
        let docs = vec![
            make_doc(vec![("x", Value::Int(10))]),
            make_doc(vec![("x", Value::String("not a number".to_string()))]),
            make_doc(vec![("x", Value::Int(20))]),
        ];
        let mock = MockOp::new(docs);
        let mut agg = AggregateOp::new(
            mock,
            vec![ResolvedAggregate {
                function: AggregateFunction::Sum,
                arg: AggregateArg::Field("x".to_string()),
                alias: "sum(x)".to_string(),
                distinct: false,
            }],
        );

        let rows = collect_all(&mut agg).unwrap();
        // Int-only input → Decimal result
        match rows[0].doc.fields.get("sum(x)") {
            Some(Value::Decimal(d)) => assert_eq!(d.as_decimal(), Decimal::from(30)),
            other => panic!("expected Decimal(30), got {:?}", other),
        }
    }

    #[test]
    fn test_min_with_nulls() {
        let docs = vec![
            make_doc(vec![("x", Value::Null)]),
            make_doc(vec![("x", Value::Int(20))]),
            make_doc(vec![("x", Value::Int(10))]),
        ];
        let mock = MockOp::new(docs);
        let mut agg = AggregateOp::new(
            mock,
            vec![ResolvedAggregate {
                function: AggregateFunction::Min,
                arg: AggregateArg::Field("x".to_string()),
                alias: "min(x)".to_string(),
                distinct: false,
            }],
        );

        let rows = collect_all(&mut agg).unwrap();
        assert_eq!(rows[0].doc.fields.get("min(x)"), Some(&Value::Int(10)));
    }

    #[test]
    fn test_count_distinct() {
        let docs = vec![
            make_doc(vec![("city", Value::String("NYC".into()))]),
            make_doc(vec![("city", Value::String("LA".into()))]),
            make_doc(vec![("city", Value::String("NYC".into()))]),
        ];
        let mock = MockOp::new(docs);
        let mut agg = AggregateOp::new(
            mock,
            vec![ResolvedAggregate {
                function: AggregateFunction::Count,
                arg: AggregateArg::Field("city".to_string()),
                alias: "count(distinct city)".to_string(),
                distinct: true,
            }],
        );

        let rows = collect_all(&mut agg).unwrap();
        assert_eq!(
            rows[0].doc.fields.get("count(distinct city)"),
            Some(&Value::Int(2))
        );
    }
}
