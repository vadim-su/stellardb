//! Group aggregate operator — computes aggregate functions per group

use std::collections::HashMap;

use crate::document::{Document, Value};
use crate::query::ast::AggregateArg;
use crate::query::error::ExecuteError;
use crate::query::execute::operators::accumulator::{Accumulator, serialize_value};
use crate::query::execute::operators::operator::{Operator, Row};
use crate::query::plan::logical::ResolvedAggregate;

pub struct GroupAggregateOp<I: Operator> {
    input: I,
    group_fields: Vec<String>,
    aggregates: Vec<ResolvedAggregate>,
    results: Vec<Row>,
    position: usize,
}

impl<I: Operator> GroupAggregateOp<I> {
    pub fn new(input: I, group_fields: Vec<String>, aggregates: Vec<ResolvedAggregate>) -> Self {
        Self {
            input,
            group_fields,
            aggregates,
            results: Vec::new(),
            position: 0,
        }
    }
}

/// Serializes group key values to a string for HashMap lookup
fn group_key(values: &[Value]) -> String {
    values
        .iter()
        .map(serialize_value)
        .collect::<Vec<_>>()
        .join("|")
}

impl<I: Operator> Operator for GroupAggregateOp<I> {
    fn open(&mut self) -> Result<(), ExecuteError> {
        self.input.open()?;

        // group_key_str -> (group_values, accumulators)
        let mut groups: HashMap<String, (Vec<Value>, Vec<Accumulator>)> = HashMap::new();
        // Maintain insertion order
        let mut order: Vec<String> = Vec::new();

        while let Some(row) = self.input.next()? {
            let key_values: Vec<Value> = self
                .group_fields
                .iter()
                .map(|f| row.get_field(f).unwrap_or(Value::Null))
                .collect();
            let key_str = group_key(&key_values);

            let entry = groups.entry(key_str.clone()).or_insert_with(|| {
                order.push(key_str.clone());
                let accs = self
                    .aggregates
                    .iter()
                    .map(|a| Accumulator::new(a.function, a.distinct))
                    .collect();
                (key_values, accs)
            });

            for (acc, agg) in entry.1.iter_mut().zip(&self.aggregates) {
                match &agg.arg {
                    AggregateArg::Wildcard => acc.feed_wildcard(),
                    AggregateArg::Field(f) => acc.feed(row.get_field(f).as_ref()),
                }
            }
        }

        self.results = order
            .iter()
            .map(|key_str| {
                let (key_values, accs) = groups
                    .remove(key_str)
                    .expect("group key from order must exist in groups map");
                let mut fields = HashMap::new();

                for (field, value) in self.group_fields.iter().zip(key_values) {
                    fields.insert(field.clone(), value);
                }

                for (acc, agg) in accs.iter().zip(&self.aggregates) {
                    fields.insert(agg.alias.clone(), acc.result());
                }

                Row::from_doc(Document {
                    id: String::new(),
                    fields,
                })
            })
            .collect();

        self.position = 0;
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Row>, ExecuteError> {
        if self.position >= self.results.len() {
            return Ok(None);
        }
        let row = self.results[self.position].clone();
        self.position += 1;
        Ok(Some(row))
    }

    fn close(&mut self) -> Result<(), ExecuteError> {
        self.results.clear();
        self.position = 0;
        self.input.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DecimalValue;
    use crate::query::execute::operators::operator::collect_all;
    use crate::query::execute::operators::operator::test_util::{MockOp, make_doc};
    use crate::query::function::AggregateFunction;
    use rust_decimal::Decimal;

    #[test]
    fn test_group_count() {
        let docs = vec![
            make_doc(vec![
                ("status", Value::String("active".into())),
                ("x", Value::Int(1)),
            ]),
            make_doc(vec![
                ("status", Value::String("inactive".into())),
                ("x", Value::Int(2)),
            ]),
            make_doc(vec![
                ("status", Value::String("active".into())),
                ("x", Value::Int(3)),
            ]),
        ];
        let mock = MockOp::new(docs);
        let mut op = GroupAggregateOp::new(
            mock,
            vec!["status".to_string()],
            vec![ResolvedAggregate {
                function: AggregateFunction::Count,
                arg: AggregateArg::Wildcard,
                alias: "count(*)".to_string(),
                distinct: false,
            }],
        );

        let rows = collect_all(&mut op).unwrap();
        assert_eq!(rows.len(), 2);

        let active = rows
            .iter()
            .find(|r| r.doc.fields.get("status") == Some(&Value::String("active".into())))
            .unwrap();
        assert_eq!(active.doc.fields.get("count(*)"), Some(&Value::Int(2)));

        let inactive = rows
            .iter()
            .find(|r| r.doc.fields.get("status") == Some(&Value::String("inactive".into())))
            .unwrap();
        assert_eq!(inactive.doc.fields.get("count(*)"), Some(&Value::Int(1)));
    }

    #[test]
    fn test_group_sum() {
        let docs = vec![
            make_doc(vec![
                ("cat", Value::String("a".into())),
                ("val", Value::Int(10)),
            ]),
            make_doc(vec![
                ("cat", Value::String("b".into())),
                ("val", Value::Int(20)),
            ]),
            make_doc(vec![
                ("cat", Value::String("a".into())),
                ("val", Value::Int(30)),
            ]),
        ];
        let mock = MockOp::new(docs);
        let mut op = GroupAggregateOp::new(
            mock,
            vec!["cat".to_string()],
            vec![ResolvedAggregate {
                function: AggregateFunction::Sum,
                arg: AggregateArg::Field("val".to_string()),
                alias: "sum(val)".to_string(),
                distinct: false,
            }],
        );

        let rows = collect_all(&mut op).unwrap();
        assert_eq!(rows.len(), 2);

        let a = rows
            .iter()
            .find(|r| r.doc.fields.get("cat") == Some(&Value::String("a".into())))
            .unwrap();
        // Int-only input → Decimal result
        assert_eq!(
            a.doc.fields.get("sum(val)"),
            Some(&Value::Decimal(DecimalValue::new(Decimal::from(40))))
        );
    }

    #[test]
    fn test_group_multiple_fields() {
        let docs = vec![
            make_doc(vec![
                ("city", Value::String("NYC".into())),
                ("status", Value::String("active".into())),
            ]),
            make_doc(vec![
                ("city", Value::String("NYC".into())),
                ("status", Value::String("inactive".into())),
            ]),
            make_doc(vec![
                ("city", Value::String("NYC".into())),
                ("status", Value::String("active".into())),
            ]),
            make_doc(vec![
                ("city", Value::String("LA".into())),
                ("status", Value::String("active".into())),
            ]),
        ];
        let mock = MockOp::new(docs);
        let mut op = GroupAggregateOp::new(
            mock,
            vec!["city".to_string(), "status".to_string()],
            vec![ResolvedAggregate {
                function: AggregateFunction::Count,
                arg: AggregateArg::Wildcard,
                alias: "count(*)".to_string(),
                distinct: false,
            }],
        );

        let rows = collect_all(&mut op).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn test_group_empty_input() {
        let mock = MockOp::new(vec![]);
        let mut op = GroupAggregateOp::new(
            mock,
            vec!["status".to_string()],
            vec![ResolvedAggregate {
                function: AggregateFunction::Count,
                arg: AggregateArg::Wildcard,
                alias: "count(*)".to_string(),
                distinct: false,
            }],
        );

        let rows = collect_all(&mut op).unwrap();
        assert_eq!(rows.len(), 0);
    }

    #[test]
    fn test_group_count_distinct() {
        // Group by category, count distinct tags per category
        let docs = vec![
            make_doc(vec![
                ("category", Value::String("A".into())),
                ("tag", Value::String("x".into())),
            ]),
            make_doc(vec![
                ("category", Value::String("A".into())),
                ("tag", Value::String("y".into())),
            ]),
            make_doc(vec![
                ("category", Value::String("A".into())),
                ("tag", Value::String("x".into())), // duplicate tag in A
            ]),
            make_doc(vec![
                ("category", Value::String("B".into())),
                ("tag", Value::String("z".into())),
            ]),
        ];
        let mock = MockOp::new(docs);
        let mut op = GroupAggregateOp::new(
            mock,
            vec!["category".to_string()],
            vec![ResolvedAggregate {
                function: AggregateFunction::Count,
                arg: AggregateArg::Field("tag".to_string()),
                alias: "count(distinct tag)".to_string(),
                distinct: true,
            }],
        );

        let rows = collect_all(&mut op).unwrap();
        assert_eq!(rows.len(), 2);

        let a = rows
            .iter()
            .find(|r| r.doc.fields.get("category") == Some(&Value::String("A".into())))
            .unwrap();
        // Category A has 2 distinct tags: x and y
        assert_eq!(
            a.doc.fields.get("count(distinct tag)"),
            Some(&Value::Int(2))
        );

        let b = rows
            .iter()
            .find(|r| r.doc.fields.get("category") == Some(&Value::String("B".into())))
            .unwrap();
        // Category B has 1 distinct tag: z
        assert_eq!(
            b.doc.fields.get("count(distinct tag)"),
            Some(&Value::Int(1))
        );
    }
}
