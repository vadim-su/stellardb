//! SELECT statement parsing.

use pest::iterators::Pair;

use super::super::expr::{parse_expr, parse_where};
use super::super::value::{Rule, parse_limit};
use super::super::{parse_field_path, parse_field_path_segments};
use super::common::parse_target;
use crate::query::ast::{
    DataSource, FetchItem, LetAst, OrderDirection, OrderItem, PostfixOp, Projection,
    ProjectionItem, SelectAst,
};
use crate::query::error::ParseError;

/// Parse LET statement.
pub(super) fn parse_let(pair: Pair<Rule>) -> Result<LetAst, ParseError> {
    // let_stmt = { kw_LET ~ ident ~ "=" ~ expr }
    let mut name = String::new();
    let mut expr = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::ident => name = super::super::parse_ident(inner),
            Rule::expr => expr = Some(parse_expr(inner)?),
            _ => {}
        }
    }

    Ok(LetAst {
        name,
        expr: expr.ok_or(ParseError::MissingExpression)?,
    })
}

/// Parse SELECT statement.
pub(super) fn parse_select(pair: Pair<Rule>) -> Result<SelectAst, ParseError> {
    let mut projection = Projection::All;
    let mut source = DataSource::None;
    let mut filter = None;
    let mut order = None;
    let mut limit = None;
    let mut offset = None;
    let mut group = None;
    let mut distinct = false;
    let mut value_mode = false;
    let mut fetch = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::distinct_mod => distinct = true,
            Rule::value_mod => value_mode = true,
            Rule::projection => projection = parse_projection(inner)?,
            Rule::data_source => source = parse_data_source(inner)?,
            Rule::where_clause => filter = Some(parse_where(inner)?),
            Rule::group_clause => group = Some(parse_group(inner)?),
            Rule::order_clause => order = Some(parse_order(inner)?),
            Rule::limit_clause => limit = Some(parse_limit(inner)?),
            Rule::offset_clause => {
                let int_pair = inner
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::integer)
                    .expect("grammar");
                offset = Some(int_pair.as_str().parse::<usize>().expect("grammar"));
            }
            Rule::fetch_clause => fetch = Some(parse_fetch_clause(inner)?),
            _ => {}
        }
    }

    Ok(SelectAst {
        projection,
        distinct,
        value_mode,
        source,
        filter,
        group,
        order,
        limit,
        offset,
        fetch,
    })
}

/// Parse SELECT from a pair (public wrapper).
pub fn parse_select_from_pair(pair: Pair<Rule>) -> Result<SelectAst, ParseError> {
    parse_select(pair)
}

fn parse_fetch_clause(pair: Pair<Rule>) -> Result<Vec<FetchItem>, ParseError> {
    let mut items = Vec::new();

    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::fetch_list {
            for item in inner.into_inner() {
                if item.as_rule() == Rule::fetch_item {
                    items.push(parse_fetch_item(item)?);
                }
            }
        }
    }

    Ok(items)
}

fn parse_fetch_item(pair: Pair<Rule>) -> Result<FetchItem, ParseError> {
    let mut path = Vec::new();
    let mut alias = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::field_path => {
                path = parse_field_path_segments(inner);
            }
            Rule::ident => {
                // This is the AS alias
                alias = Some(super::super::parse_ident(inner));
            }
            _ => {}
        }
    }

    Ok(FetchItem { path, alias })
}

fn parse_data_source(pair: Pair<Rule>) -> Result<DataSource, ParseError> {
    let mut inner = pair.into_inner();
    let first = inner.next().expect("grammar");

    match first.as_rule() {
        Rule::target => {
            let target = parse_target(first);
            let mut path = Vec::new();
            let mut postfix = Vec::new();

            // Collect remaining pairs: optional field_path then postfix_ops
            for p in inner {
                match p.as_rule() {
                    Rule::field_path => path = parse_field_path_segments(p),
                    Rule::postfix_op => postfix.push(parse_postfix_to_op(p)?),
                    _ => {}
                }
            }

            if !path.is_empty() || !postfix.is_empty() {
                Ok(DataSource::FieldPath {
                    target,
                    path,
                    postfix,
                })
            } else {
                Ok(DataSource::Collection(target))
            }
        }
        Rule::from_expr => {
            let expr = parse_from_expr(first)?;
            Ok(DataSource::Expr(Box::new(expr)))
        }
        _ => unreachable!("unexpected rule in data_source: {:?}", first.as_rule()),
    }
}

/// Parse a postfix_op pair into a PostfixOp AST node.
fn parse_postfix_to_op(pair: Pair<Rule>) -> Result<PostfixOp, ParseError> {
    let inner = pair.into_inner().next().expect("grammar");
    match inner.as_rule() {
        Rule::field_access_op => {
            let ident = inner
                .into_inner()
                .find(|p| p.as_rule() == Rule::ident)
                .expect("grammar");
            Ok(PostfixOp::FieldAccess(super::super::parse_ident(ident)))
        }
        Rule::index_op => {
            let index_expr = inner
                .into_inner()
                .find(|p| p.as_rule() == Rule::expr)
                .expect("grammar");
            Ok(PostfixOp::Index(Box::new(parse_expr(index_expr)?)))
        }
        Rule::slice_op => {
            let bounds = inner
                .into_inner()
                .find(|p| p.as_rule() == Rule::slice_bounds)
                .expect("grammar");
            let mut start = None;
            let mut end = None;
            for child in bounds.into_inner() {
                match child.as_rule() {
                    Rule::slice_start => {
                        let expr = child
                            .into_inner()
                            .find(|p| p.as_rule() == Rule::expr)
                            .expect("grammar");
                        start = Some(Box::new(parse_expr(expr)?));
                    }
                    Rule::slice_end => {
                        let expr = child
                            .into_inner()
                            .find(|p| p.as_rule() == Rule::expr)
                            .expect("grammar");
                        end = Some(Box::new(parse_expr(expr)?));
                    }
                    _ => {}
                }
            }
            Ok(PostfixOp::Slice { start, end })
        }
        _ => unreachable!("postfix_op got {:?}", inner.as_rule()),
    }
}

fn parse_from_expr(pair: Pair<Rule>) -> Result<crate::query::ast::Expr, ParseError> {
    use crate::query::ast::Expr;

    let mut pairs = pair.into_inner();
    let inner = pairs.next().expect("grammar");

    match inner.as_rule() {
        Rule::select_stmt => {
            let select = parse_select(inner)?;
            Ok(Expr::Subquery(Box::new(select)))
        }
        Rule::range_literal => parse_range_literal(inner),
        Rule::call_expr => super::super::expr::parse_call_expr(inner),
        Rule::parent_ref => {
            use crate::query::ast::{Expr as E, ParentRef};
            let mut inner_pairs = inner.into_inner();
            let parent_chain = inner_pairs.next().expect("grammar");
            let depth = parent_chain.as_str().matches(".parent").count();
            let field_path_pair = inner_pairs.next().expect("grammar");
            let field = parse_field_path(field_path_pair);
            let mut expr = E::ParentRef(ParentRef { depth, field });
            for postfix in pairs {
                if postfix.as_rule() == Rule::postfix_op {
                    expr = super::super::expr::parse_postfix_op(expr, postfix)?;
                }
            }
            Ok(expr)
        }
        Rule::variable_ref => {
            let name = inner
                .into_inner()
                .find(|p| p.as_rule() == Rule::ident)
                .map(|p| super::super::parse_ident(p))
                .unwrap_or_default();
            let mut expr = Expr::Variable(name);
            // Apply any postfix operators (field access, indexing, slicing)
            for postfix in pairs {
                if postfix.as_rule() == Rule::postfix_op {
                    expr = super::super::expr::parse_postfix_op(expr, postfix)?;
                }
            }
            Ok(expr)
        }
        Rule::array_expr => super::super::expr::parse_array_expr(inner),
        Rule::expr => super::super::expr::parse_expr(inner),
        _ => unreachable!("unexpected rule in from_expr: {:?}", inner.as_rule()),
    }
}

fn parse_range_literal(pair: Pair<Rule>) -> Result<crate::query::ast::Expr, ParseError> {
    use crate::query::ast::Expr;

    let mut bounds = pair.into_inner();
    let start = parse_range_bound(bounds.next().expect("grammar"))?;
    let end = parse_range_bound(bounds.next().expect("grammar"))?;

    Ok(Expr::Range {
        start: Box::new(start),
        end: Box::new(end),
    })
}

fn parse_range_bound(pair: Pair<Rule>) -> Result<crate::query::ast::Expr, ParseError> {
    use crate::document::Value;
    use crate::query::ast::Expr;

    let inner = pair.into_inner().next().expect("grammar");
    match inner.as_rule() {
        Rule::integer => {
            let n: i64 = inner.as_str().parse().expect("grammar");
            Ok(Expr::Literal(Value::Int(n)))
        }
        Rule::variable_ref => {
            let name = inner
                .into_inner()
                .find(|p| p.as_rule() == Rule::ident)
                .map(|p| super::super::parse_ident(p))
                .unwrap_or_default();
            Ok(Expr::Variable(name))
        }
        Rule::expr => super::super::expr::parse_expr(inner),
        _ => unreachable!(),
    }
}

fn parse_group(pair: Pair<Rule>) -> Result<Vec<String>, ParseError> {
    Ok(pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::field_path)
        .map(parse_field_path)
        .collect())
}

fn parse_projection(pair: Pair<Rule>) -> Result<Projection, ParseError> {
    let inner = pair.into_inner().next();

    match inner {
        Some(p) if p.as_rule() == Rule::projection_item_list => {
            // Use flat_map because field_expansion can produce multiple items
            let items: Result<Vec<ProjectionItem>, ParseError> = p
                .into_inner()
                .filter(|p| p.as_rule() == Rule::projection_item)
                .map(parse_projection_item)
                .collect::<Result<Vec<Vec<_>>, _>>()
                .map(|v| v.into_iter().flatten().collect());
            Ok(Projection::Items(items?))
        }
        _ => Ok(Projection::All),
    }
}

/// Parse a projection item list into a vector of ProjectionItem.
pub(super) fn parse_projection_item_list(
    pair: Pair<Rule>,
) -> Result<Vec<ProjectionItem>, ParseError> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::projection_item)
        .map(parse_projection_item)
        .collect::<Result<Vec<Vec<_>>, _>>()
        .map(|v| v.into_iter().flatten().collect())
}

fn parse_projection_item(pair: Pair<Rule>) -> Result<Vec<ProjectionItem>, ParseError> {
    use crate::query::ast::Expr;

    let mut items = Vec::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::field_expansion => {
                // field_expansion = { field_expansion_prefix ~ "{" ~ field_expansion_item ~ ("," ~ field_expansion_item)* ~ "}" }
                // Expands field.{a, b} into [field.a, field.b]
                // Supports aliasing: field.{alias: source} -> field.source AS alias
                let mut prefix = String::new();
                let mut expansion_items: Vec<(Option<String>, String)> = Vec::new();

                for child in inner.into_inner() {
                    match child.as_rule() {
                        Rule::field_expansion_prefix => {
                            // Prefix ends with ".", e.g., "address." or "user.profile."
                            prefix = child.as_str().to_string();
                        }
                        Rule::field_expansion_item => {
                            // field_expansion_item = { (ident ~ ":")? ~ ident }
                            let idents: Vec<_> = child
                                .into_inner()
                                .filter(|p| p.as_rule() == Rule::ident)
                                .map(|p| super::super::parse_ident(p))
                                .collect();

                            match idents.len() {
                                1 => {
                                    // Just field name, no alias
                                    expansion_items.push((None, idents[0].clone()));
                                }
                                2 => {
                                    // alias: field
                                    expansion_items
                                        .push((Some(idents[0].clone()), idents[1].clone()));
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }

                // Expand each field with the prefix
                for (alias, field) in expansion_items {
                    let full_path = format!("{}{}", prefix, field);
                    items.push(ProjectionItem {
                        expr: Expr::Field(full_path),
                        alias,
                    });
                }
            }
            Rule::expr => {
                items.push(ProjectionItem {
                    expr: parse_expr(inner)?,
                    alias: None,
                });
            }
            Rule::alias_clause => {
                // alias_clause = { kw_AS ~ ident }
                // Apply alias to the last item
                if let Some(last) = items.last_mut()
                    && let Some(ident) = inner.into_inner().find(|p| p.as_rule() == Rule::ident)
                {
                    last.alias = Some(super::super::parse_ident(ident));
                }
            }
            _ => {}
        }
    }

    if items.is_empty() {
        return Err(ParseError::MissingExpression);
    }

    Ok(items)
}

/// Parse ORDER BY clause.
pub(super) fn parse_order(pair: Pair<Rule>) -> Result<Vec<OrderItem>, ParseError> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::order_item)
        .map(parse_order_item)
        .collect()
}

fn parse_order_item(pair: Pair<Rule>) -> Result<OrderItem, ParseError> {
    let mut expr = None;
    let mut direction = OrderDirection::Asc;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::expr => expr = Some(parse_expr(inner)?),
            Rule::order_dir => {
                direction = if inner.as_str().eq_ignore_ascii_case("DESC") {
                    OrderDirection::Desc
                } else {
                    OrderDirection::Asc
                };
            }
            _ => {}
        }
    }

    Ok(OrderItem {
        expr: expr.expect("order_item must contain an expression"),
        direction,
    })
}
