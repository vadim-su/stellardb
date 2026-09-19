use pest::iterators::Pair;

use super::value::Rule;
use super::{parse_field_path, parse_ident};
use crate::document::{Decimal, DecimalValue, Value};
use crate::query::ast::{
    AggregateArg, AggregateCall, BinaryOp, Block, Expr, FtsOperator, FtsTarget, MatchField,
    MatchMode, ParentRef, TraversalDepth, TraversalDirection, TraversalExpr, TraversalMode,
    TraversalStep, TraversalTarget, UnaryOp,
};
use crate::query::error::ParseError;

/// Type alias for slice bounds to reduce complexity
type SliceBounds = (Option<Box<Expr>>, Option<Box<Expr>>);

/// Type alias for traversal start result with optional field access
type TraversalStartResult = (
    TraversalStep,
    Option<(Vec<crate::query::ast::expr::FieldSelection>, bool)>,
);

pub fn parse_where(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    let expr = pair
        .into_inner()
        .find(|p| p.as_rule() == Rule::expr)
        .expect("grammar");
    parse_expr(expr)
}

pub fn parse_expr(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    // expr = { coalesce_expr }
    let inner = pair.into_inner().next().expect("grammar");
    parse_coalesce_expr(inner)
}

fn parse_coalesce_expr(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    // coalesce_expr = { or_expr ~ (coalesce_op ~ or_expr)* }
    let mut inner = pair.into_inner();
    let mut left = parse_or_expr(inner.next().expect("grammar"))?;

    while let Some(op) = inner.next() {
        if op.as_rule() == Rule::coalesce_op {
            let right = parse_or_expr(inner.next().expect("grammar"))?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op: BinaryOp::Coalesce,
                right: Box::new(right),
            };
        }
    }
    Ok(left)
}

fn parse_or_expr(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    let mut inner = pair.into_inner();
    let mut left = parse_and_expr(inner.next().expect("grammar"))?;

    while let Some(op) = inner.next() {
        if op.as_rule() == Rule::or_op {
            let right = parse_and_expr(inner.next().expect("grammar"))?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op: BinaryOp::Or,
                right: Box::new(right),
            };
        }
    }
    Ok(left)
}

fn parse_and_expr(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    let mut inner = pair.into_inner();
    let mut left = parse_not_expr(inner.next().expect("grammar"))?;

    while let Some(op) = inner.next() {
        if op.as_rule() == Rule::and_op {
            let right = parse_not_expr(inner.next().expect("grammar"))?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op: BinaryOp::And,
                right: Box::new(right),
            };
        }
    }
    Ok(left)
}

fn parse_not_expr(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    // not_expr = { not_op? ~ comparison }
    let mut inner = pair.into_inner();
    let first = inner.next().expect("grammar");

    if first.as_rule() == Rule::not_op {
        let comparison = inner.next().expect("grammar");
        let expr = parse_comparison(comparison)?;
        Ok(Expr::UnaryOp {
            op: UnaryOp::Not,
            expr: Box::new(expr),
        })
    } else {
        parse_comparison(first)
    }
}

fn parse_comparison(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    // comparison = { in_expr ~ (comparator ~ in_expr)? }
    let mut inner = pair.into_inner();
    let left = parse_in_expr(inner.next().expect("grammar"))?;

    if let Some(comp) = inner.next() {
        if comp.as_rule() == Rule::comparator {
            let op = parse_comparator_op(comp);
            let right = parse_in_expr(inner.next().expect("grammar"))?;
            Ok(Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            })
        } else if comp.as_rule() == Rule::is_check {
            let check = comp.into_inner().next().expect("grammar");
            let op = match check.as_rule() {
                Rule::is_null => UnaryOp::IsNull,
                Rule::is_not_null => UnaryOp::IsNotNull,
                Rule::is_none => UnaryOp::IsNone,
                Rule::is_not_none => UnaryOp::IsNotNone,
                _ => unreachable!(),
            };
            Ok(Expr::UnaryOp {
                op,
                expr: Box::new(left),
            })
        } else {
            Ok(left)
        }
    } else {
        Ok(left)
    }
}

fn parse_comparator_op(pair: Pair<Rule>) -> BinaryOp {
    match pair.as_str() {
        "=" => BinaryOp::Eq,
        "!=" => BinaryOp::Ne,
        ">" => BinaryOp::Gt,
        ">=" => BinaryOp::Gte,
        "<" => BinaryOp::Lt,
        "<=" => BinaryOp::Lte,
        _ => unreachable!(),
    }
}

fn parse_in_expr(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    // in_expr = { addition ~ (not_in_op | in_op)? }
    // in_op = { kw_IN ~ (in_array | in_subquery | in_expr_target) }
    // in_array = { "[" ~ expr_list? ~ "]" }
    // in_subquery = { "(" ~ select_stmt ~ ")" }
    // in_expr_target = { variable_ref | field_ref }
    let mut inner = pair.into_inner();
    let left = parse_addition(inner.next().expect("grammar"))?;

    if let Some(in_part) = inner.next() {
        let negated = in_part.as_rule() == Rule::not_in_op;

        // Find in_array, in_subquery, or in_expr_target
        for child in in_part.into_inner() {
            match child.as_rule() {
                Rule::in_array => {
                    // Parse array literal: [expr, expr, ...]
                    let list: Result<Vec<Expr>, ParseError> = child
                        .into_inner()
                        .filter_map(|p| {
                            if p.as_rule() == Rule::expr_list {
                                Some(p.into_inner())
                            } else {
                                None
                            }
                        })
                        .flatten()
                        .filter(|p| p.as_rule() == Rule::expr)
                        .map(parse_expr)
                        .collect();
                    return Ok(Expr::InList {
                        expr: Box::new(left),
                        list: list?,
                        negated,
                    });
                }
                Rule::in_subquery => {
                    // Parse subquery: (SELECT ...)
                    let select_pair = child
                        .into_inner()
                        .find(|p| p.as_rule() == Rule::select_stmt)
                        .expect("grammar");
                    let select_ast =
                        crate::query::parse::stmt::parse_select_from_pair(select_pair)?;
                    return Ok(Expr::InSubquery {
                        expr: Box::new(left),
                        subquery: Box::new(select_ast),
                        negated,
                    });
                }
                Rule::in_expr_target => {
                    // Parse variable or field reference: $var or field
                    let inner_pair = child.into_inner().next().expect("grammar");
                    let target = match inner_pair.as_rule() {
                        Rule::variable_ref => {
                            let var_name = inner_pair.as_str()[1..].to_string(); // Skip $
                            Expr::Variable(var_name)
                        }
                        Rule::field_ref => {
                            let path =
                                parse_field_path(inner_pair.into_inner().next().expect("grammar"));
                            Expr::Field(path)
                        }
                        _ => unreachable!(),
                    };
                    return Ok(Expr::InExpr {
                        expr: Box::new(left),
                        target: Box::new(target),
                        negated,
                    });
                }
                _ => continue,
            }
        }
        // Fallback - shouldn't happen with correct grammar
        Ok(left)
    } else {
        Ok(left)
    }
}

fn parse_addition(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    // addition = { multiplication ~ ((add_op | sub_op) ~ multiplication)* }
    let mut inner = pair.into_inner();
    let mut left = parse_multiplication(inner.next().expect("grammar"))?;

    while let Some(op_pair) = inner.next() {
        let op = match op_pair.as_rule() {
            Rule::add_op => BinaryOp::Add,
            Rule::sub_op => BinaryOp::Sub,
            _ => unreachable!(),
        };
        let right = parse_multiplication(inner.next().expect("grammar"))?;
        left = Expr::BinaryOp {
            left: Box::new(left),
            op,
            right: Box::new(right),
        };
    }
    Ok(left)
}

fn parse_multiplication(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    // multiplication = { unary ~ ((mul_op | div_op) ~ unary)* }
    let mut inner = pair.into_inner();
    let mut left = parse_unary(inner.next().expect("grammar"))?;

    while let Some(op_pair) = inner.next() {
        let op = match op_pair.as_rule() {
            Rule::mul_op => BinaryOp::Mul,
            Rule::div_op => BinaryOp::Div,
            _ => unreachable!(),
        };
        let right = parse_unary(inner.next().expect("grammar"))?;
        left = Expr::BinaryOp {
            left: Box::new(left),
            op,
            right: Box::new(right),
        };
    }
    Ok(left)
}

fn parse_unary(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    // unary = { (neg_op | not_op)? ~ primary }
    let mut inner = pair.into_inner();
    let first = inner.next().expect("grammar");

    match first.as_rule() {
        Rule::neg_op => {
            let primary = inner.next().expect("grammar");
            let expr = parse_primary(primary)?;
            Ok(Expr::UnaryOp {
                op: UnaryOp::Neg,
                expr: Box::new(expr),
            })
        }
        Rule::not_op => {
            let primary = inner.next().expect("grammar");
            let expr = parse_primary(primary)?;
            Ok(Expr::UnaryOp {
                op: UnaryOp::Not,
                expr: Box::new(expr),
            })
        }
        Rule::primary => parse_primary(first),
        _ => unreachable!(
            "unary expects neg_op, not_op, or primary, got {:?}",
            first.as_rule()
        ),
    }
}

fn parse_primary(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    // primary = { primary_base ~ postfix_op* }
    let mut inner = pair.into_inner();
    let base_pair = inner.next().expect("grammar");
    let mut expr = parse_primary_base(base_pair)?;

    // Apply postfix operators left-to-right
    for postfix in inner {
        if postfix.as_rule() == Rule::postfix_op {
            expr = parse_postfix_op(expr, postfix)?;
        }
    }
    Ok(expr)
}

pub(super) fn parse_postfix_op(base: Expr, pair: Pair<Rule>) -> Result<Expr, ParseError> {
    let inner = pair.into_inner().next().expect("grammar");
    match inner.as_rule() {
        Rule::index_op => {
            let index_expr = inner
                .into_inner()
                .find(|p| p.as_rule() == Rule::expr)
                .expect("grammar");
            Ok(Expr::Index {
                base: Box::new(base),
                index: Box::new(parse_expr(index_expr)?),
            })
        }
        Rule::slice_op => {
            let bounds = inner
                .into_inner()
                .find(|p| p.as_rule() == Rule::slice_bounds)
                .expect("grammar");
            let (start, end) = parse_slice_bounds(bounds)?;
            Ok(Expr::Slice {
                base: Box::new(base),
                start,
                end,
            })
        }
        Rule::field_access_op => {
            let ident = inner
                .into_inner()
                .find(|p| p.as_rule() == Rule::ident)
                .expect("grammar");
            Ok(Expr::FieldAccess {
                base: Box::new(base),
                field: super::parse_ident(ident),
            })
        }
        _ => unreachable!("postfix_op got {:?}", inner.as_rule()),
    }
}

fn parse_slice_bounds(pair: Pair<Rule>) -> Result<SliceBounds, ParseError> {
    let mut start = None;
    let mut end = None;

    for child in pair.into_inner() {
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
    Ok((start, end))
}

fn parse_primary_base(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    // primary_base = { traversal_expr | subquery_expr | parent_ref | "(" ~ expr ~ ")" | const_ref | call_expr | literal | field_ref }
    let inner = pair.into_inner().next().expect("grammar");
    match inner.as_rule() {
        Rule::if_expr => parse_if_expr(inner),
        Rule::for_expr => parse_for_expr(inner),
        Rule::break_expr => Ok(Expr::Break),
        Rule::continue_expr => Ok(Expr::Continue),
        Rule::traversal_expr => parse_traversal_expr(inner),
        Rule::subquery_expr => {
            let mut inner_pairs = inner.into_inner();
            let select_pair = inner_pairs.next().expect("grammar");
            let select_ast = crate::query::parse::stmt::parse_select_from_pair(select_pair)?;
            let subquery = Box::new(Expr::Subquery(Box::new(select_ast)));

            // Check for index suffix: subquery_expr = { "(" ~ select_stmt ~ ")" ~ index_suffix? }
            if let Some(index_suffix) = inner_pairs.next() {
                // index_suffix = { "[" ~ expr ~ "]" }
                let index_expr = index_suffix
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::expr)
                    .expect("grammar");
                Ok(Expr::Index {
                    base: subquery,
                    index: Box::new(parse_expr(index_expr)?),
                })
            } else {
                Ok(*subquery)
            }
        }
        Rule::value_ref => {
            let path = inner
                .into_inner()
                .find(|p| p.as_rule() == Rule::field_path)
                .map(parse_field_path);
            Ok(Expr::ValueRef(path))
        }
        Rule::parent_ref => {
            let mut inner_pairs = inner.into_inner();

            // Count .parent occurrences in parent_chain
            let parent_chain = inner_pairs.next().expect("grammar");
            let depth = parent_chain.as_str().matches(".parent").count();

            // Last part is the field path
            let field_path_pair = inner_pairs.next().expect("grammar");
            let field = parse_field_path(field_path_pair);

            Ok(Expr::ParentRef(ParentRef { depth, field }))
        }
        Rule::variable_ref => parse_variable_ref(inner),
        Rule::expr => parse_expr(inner),
        Rule::const_ref => parse_const_ref(inner),
        Rule::call_expr => parse_call_expr(inner),
        Rule::object_expr => parse_object_expr(inner),
        Rule::array_expr => parse_array_expr(inner),
        Rule::literal => parse_literal(inner),
        Rule::fts_expr => parse_fts_expr(inner),
        Rule::knn_expr => parse_knn_expr(inner),
        Rule::reference_literal => {
            // Format: collection:key (e.g., "user:alice")
            let s = inner.as_str();
            Ok(Expr::Literal(Value::Reference(s.to_string())))
        }
        Rule::field_ref => {
            let field_path_pair = inner.into_inner().next().expect("grammar");
            Ok(Expr::Field(parse_field_path(field_path_pair)))
        }
        _ => unreachable!("primary_base got {:?}", inner.as_rule()),
    }
}

fn parse_const_ref(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    // const_ref = { const_namespace ~ "::" ~ const_name ~ !("(") }
    let mut namespace = String::new();
    let mut name = String::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::const_namespace => namespace = inner.as_str().to_string(),
            Rule::const_name => name = inner.as_str().to_string(),
            _ => {}
        }
    }

    Ok(Expr::Constant { namespace, name })
}

fn parse_variable_ref(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    // variable_ref = { "$" ~ ident }
    let ident = pair.into_inner().next().expect("grammar");
    let name = super::parse_ident(ident);
    Ok(Expr::Variable(name))
}

fn parse_block(pair: Pair<Rule>) -> Result<Block, ParseError> {
    // block = { (statement ~ (";" ~ statement?)*)? }
    let mut statements = Vec::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::statement {
            statements.push(crate::query::parse::stmt::parse_statement(inner)?);
        }
    }
    Ok(Block { statements })
}

fn parse_if_expr(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    // if_expr = { kw_IF ~ expr ~ kw_THEN ~ block ~ (kw_ELSE ~ kw_IF ~ expr ~ kw_THEN ~ block)* ~ (kw_ELSE ~ block)? ~ kw_END }
    let mut branches = Vec::new();
    let mut else_block = None;
    let mut inner = pair.into_inner().peekable();

    while let Some(p) = inner.next() {
        match p.as_rule() {
            Rule::expr => {
                let cond = parse_expr(p)?;
                // Find the next block
                let block_pair = loop {
                    let n = inner.next().expect("grammar");
                    if n.as_rule() == Rule::block {
                        break n;
                    }
                };
                branches.push((Box::new(cond), parse_block(block_pair)?));
            }
            Rule::kw_ELSE => {
                if inner.peek().map(|p| p.as_rule()) == Some(Rule::kw_IF) {
                    inner.next(); // consume kw_IF — next expr+block caught by outer loop
                } else {
                    // ELSE block
                    let block_pair = loop {
                        let n = inner.next().expect("grammar");
                        if n.as_rule() == Rule::block {
                            break n;
                        }
                    };
                    else_block = Some(parse_block(block_pair)?);
                }
            }
            _ => {} // skip kw_IF, kw_THEN, kw_END
        }
    }

    Ok(Expr::If {
        branches,
        else_block,
    })
}

fn parse_for_expr(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    // for_expr = { kw_FOR ~ variable_ref ~ kw_IN ~ expr ~ kw_DO ~ block ~ kw_END }
    let mut binding = String::new();
    let mut iterable = None;
    let mut body = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::variable_ref => {
                let ident = inner.into_inner().next().expect("grammar");
                binding = super::parse_ident(ident);
            }
            Rule::expr => {
                if iterable.is_none() {
                    iterable = Some(parse_expr(inner)?);
                }
            }
            Rule::block => {
                body = Some(parse_block(inner)?);
            }
            _ => {} // skip keywords
        }
    }

    Ok(Expr::For {
        binding,
        iterable: Box::new(iterable.ok_or(ParseError::MissingExpression)?),
        body: body.unwrap_or(Block { statements: vec![] }),
    })
}

fn parse_literal(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    // literal = { datetime_literal | duration_literal | bytes_literal | string | number | boolean | null }
    let inner = pair.into_inner().next().expect("grammar");
    match inner.as_rule() {
        Rule::datetime_literal => parse_datetime_literal(inner),
        Rule::duration_literal => parse_duration_literal(inner),
        Rule::bytes_literal => parse_bytes_literal(inner),
        Rule::string => Ok(Expr::Literal(Value::String(
            super::value::parse_string_literal(inner)?,
        ))),
        Rule::number => {
            let number_inner = inner.into_inner().next().expect("grammar");
            match number_inner.as_rule() {
                Rule::decimal_number => {
                    let s = number_inner.as_str();
                    // Strip "dec" suffix (case insensitive)
                    let num_str = s
                        .strip_suffix("dec")
                        .or_else(|| s.strip_suffix("DEC"))
                        .or_else(|| s.strip_suffix("Dec"))
                        .unwrap_or(s);
                    let d: Decimal = num_str
                        .parse()
                        .map_err(|_| ParseError::InvalidNumber(s.to_string()))?;
                    Ok(Expr::Literal(Value::Decimal(DecimalValue::new(d))))
                }
                Rule::float_number => {
                    let s = number_inner.as_str();
                    // Strip "f" suffix if present
                    let num_str = s
                        .strip_suffix('f')
                        .or_else(|| s.strip_suffix('F'))
                        .unwrap_or(s);
                    let f: f64 = num_str
                        .parse()
                        .map_err(|_| ParseError::InvalidNumber(s.to_string()))?;
                    Ok(Expr::Literal(Value::Float(f)))
                }
                Rule::int_number => {
                    let s = number_inner.as_str();
                    let i: i64 = s
                        .parse()
                        .map_err(|_| ParseError::InvalidNumber(s.to_string()))?;
                    Ok(Expr::Literal(Value::Int(i)))
                }
                _ => unreachable!("Unexpected number rule: {:?}", number_inner.as_rule()),
            }
        }
        Rule::boolean => Ok(Expr::Literal(Value::Bool(
            inner.as_str().to_lowercase() == "true",
        ))),
        Rule::null => Ok(Expr::Literal(Value::Null)),
        _ => unreachable!("Unexpected literal rule: {:?}", inner.as_rule()),
    }
}

/// Parse datetime literal: d"2024-01-15" or d"2024-01-15T10:30:00Z"
fn parse_datetime_literal(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    let s = pair.as_str();
    // Remove d" prefix and " suffix: d"2024-01-15" -> 2024-01-15
    // The string part includes quotes, so we need to handle both single and double quotes
    let datetime_str = if s.starts_with("d\"") || s.starts_with("d'") {
        &s[2..s.len() - 1]
    } else {
        return Err(ParseError::InvalidDatetime(s.to_string()));
    };

    // Try RFC3339 first (with time): 2024-01-15T10:30:00Z
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(datetime_str) {
        return Ok(Expr::Literal(Value::from_datetime(
            dt.with_timezone(&chrono::Utc),
        )));
    }

    // Try date-only format: 2024-01-15
    if let Ok(date) = chrono::NaiveDate::parse_from_str(datetime_str, "%Y-%m-%d") {
        let dt = date
            .and_hms_opt(0, 0, 0)
            .expect("00:00:00 is a valid time")
            .and_utc();
        return Ok(Expr::Literal(Value::from_datetime(dt)));
    }

    Err(ParseError::InvalidDatetime(datetime_str.to_string()))
}

/// Parse a duration string (e.g. "5s", "1h30m") into nanoseconds.
pub fn parse_duration_nanos(s: &str) -> Result<u64, ParseError> {
    let mut total_nanos: u64 = 0;

    // Parse segments like "1h30m2s"
    let mut num_start = 0;
    let mut i = 0;
    let chars: Vec<char> = s.chars().collect();

    while i < chars.len() {
        if chars[i].is_alphabetic() {
            // Found unit start
            let num_str: String = chars[num_start..i].iter().collect();
            let num: u64 = num_str
                .parse()
                .map_err(|_| ParseError::InvalidDuration(s.to_string()))?;

            // Find unit end
            let unit_start = i;
            while i < chars.len() && chars[i].is_alphabetic() {
                i += 1;
            }
            let unit: String = chars[unit_start..i].iter().collect();

            let nanos = match unit.as_str() {
                "ns" => Some(num),
                "us" => num.checked_mul(1_000),
                "ms" => num.checked_mul(1_000_000),
                "s" => num.checked_mul(1_000_000_000),
                "m" => num.checked_mul(60 * 1_000_000_000),
                "h" => num.checked_mul(3600 * 1_000_000_000),
                "d" => num.checked_mul(86400 * 1_000_000_000),
                "w" => num.checked_mul(604800 * 1_000_000_000),
                "y" => num.checked_mul(31536000 * 1_000_000_000),
                _ => return Err(ParseError::InvalidDuration(s.to_string())),
            };

            total_nanos = total_nanos
                .checked_add(nanos.ok_or_else(|| ParseError::InvalidDuration(s.to_string()))?)
                .ok_or_else(|| ParseError::InvalidDuration(s.to_string()))?;

            num_start = i;
        } else {
            i += 1;
        }
    }

    Ok(total_nanos)
}

/// Parse duration literal: 1h30m, 2d, 500ms
fn parse_duration_literal(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    let nanos = parse_duration_nanos(pair.as_str())?;
    Ok(Expr::Literal(Value::Duration(nanos)))
}

/// Parse bytes literal: b"base64" or 0xhex
fn parse_bytes_literal(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    let s = pair.as_str();

    if s.starts_with("0x") || s.starts_with("0X") {
        // Hex encoding
        let hex = &s[2..];
        let bytes = hex::decode(hex).map_err(|_| ParseError::InvalidBytes(s.to_string()))?;
        Ok(Expr::Literal(Value::Bytes(bytes)))
    } else if s.starts_with("b\"") || s.starts_with("b'") {
        // Base64 encoding: b"..."
        let b64 = &s[2..s.len() - 1];
        use base64::{Engine, engine::general_purpose::STANDARD};
        let bytes = STANDARD
            .decode(b64)
            .map_err(|_| ParseError::InvalidBytes(s.to_string()))?;
        Ok(Expr::Literal(Value::Bytes(bytes)))
    } else {
        Err(ParseError::InvalidBytes(s.to_string()))
    }
}

/// Returns true if the given name is a known aggregate function.
fn is_aggregate_name(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "count" | "sum" | "avg" | "min" | "max"
    )
}

pub fn parse_call_expr(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    // call_expr = { (call_namespace ~ "::")? ~ func_name ~ "(" ~ call_args ~ ")" }
    let mut namespace = None;
    let mut name = String::new();
    let mut is_wildcard = false;
    let mut is_distinct = false;
    let mut args = Vec::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::call_namespace => namespace = Some(inner.as_str().to_string()),
            Rule::func_name => name = inner.as_str().to_string(),
            Rule::call_args => {
                // call_args = { "*" | distinct_arg | func_args? }
                // If it matched "*", as_str() will be "*" (possibly with whitespace)
                let trimmed = inner.as_str().trim();
                if trimmed == "*" {
                    is_wildcard = true;
                } else {
                    for child in inner.into_inner() {
                        match child.as_rule() {
                            Rule::distinct_arg => {
                                is_distinct = true;
                                // Parse the expression inside distinct_arg
                                for expr_child in child.into_inner() {
                                    if expr_child.as_rule() == Rule::expr {
                                        args.push(parse_expr(expr_child)?);
                                    }
                                }
                            }
                            Rule::func_args => {
                                for arg in child.into_inner() {
                                    if arg.as_rule() == Rule::expr {
                                        args.push(parse_expr(arg)?);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Wildcard → always aggregate
    if is_wildcard {
        return Ok(Expr::Aggregate(AggregateCall {
            namespace,
            function: name,
            arg: AggregateArg::Wildcard,
            distinct: false,
        }));
    }

    // DISTINCT aggregate with exactly one field arg
    if is_distinct
        && is_aggregate_name(&name)
        && args.len() == 1
        && let Expr::Field(ref f) = args[0]
    {
        return Ok(Expr::Aggregate(AggregateCall {
            namespace,
            function: name,
            arg: AggregateArg::Field(f.clone()),
            distinct: true,
        }));
    }

    // Known aggregate with exactly one field arg → aggregate
    if is_aggregate_name(&name)
        && args.len() == 1
        && let Expr::Field(ref f) = args[0]
    {
        return Ok(Expr::Aggregate(AggregateCall {
            namespace,
            function: name,
            arg: AggregateArg::Field(f.clone()),
            distinct: false,
        }));
    }

    // Otherwise → scalar function call
    Ok(Expr::FunctionCall {
        name,
        namespace,
        args,
    })
}

pub fn parse_array_expr(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    let items = pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::expr)
        .map(parse_expr)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Expr::Array(items))
}

fn parse_object_expr(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    let pairs = pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::object_expr_pair)
        .map(|p| {
            let mut children = p.into_inner();
            let key_pair = children.next().expect("grammar");
            let key = match key_pair.as_rule() {
                Rule::ident => super::parse_ident(key_pair),
                Rule::string => super::value::parse_string_literal(key_pair)?,
                _ => unreachable!(),
            };
            let expr_pair = children
                .find(|c| c.as_rule() == Rule::expr)
                .expect("grammar");
            let expr = parse_expr(expr_pair)?;
            Ok((key, expr))
        })
        .collect::<Result<Vec<_>, ParseError>>()?;
    Ok(Expr::Object(pairs))
}

/// Parse a full FTS expression: (match_clause | field_path) ~ fts_op ~ string
fn parse_fts_expr(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    // fts_expr = { (match_clause | field_path) ~ fts_op ~ string }
    let mut inner = pair.into_inner();

    // First child is either match_clause or field_path
    let target_pair = inner.next().expect("grammar");
    let target = match target_pair.as_rule() {
        Rule::match_clause => parse_match_clause(target_pair)?,
        Rule::field_path => FtsTarget::Field(parse_field_path(target_pair)),
        _ => unreachable!("fts_expr target got {:?}", target_pair.as_rule()),
    };

    // Second child is fts_op
    let fts_op_pair = inner.next().expect("grammar");
    let operator = parse_fts_operator(fts_op_pair)?;

    // Third child is string (the query)
    let string_pair = inner.next().expect("grammar");
    let query = super::value::parse_string_literal(string_pair)?;

    Ok(Expr::Fts {
        target,
        operator,
        query,
    })
}

/// Parse FTS operator: @@ or @:name@
fn parse_fts_operator(pair: Pair<Rule>) -> Result<FtsOperator, ParseError> {
    // fts_op = { fts_named | fts_simple }
    let inner = pair.into_inner().next().expect("grammar");
    match inner.as_rule() {
        Rule::fts_simple => Ok(FtsOperator::Simple),
        Rule::fts_named => {
            // fts_named = { "@:" ~ ident ~ "@" }
            let ident = inner
                .into_inner()
                .find(|p| p.as_rule() == Rule::ident)
                .expect("grammar");
            Ok(FtsOperator::Named(super::parse_ident(ident)))
        }
        _ => unreachable!("fts_op got {:?}", inner.as_rule()),
    }
}

/// Parse MATCH clause: MATCH(field_list) MODE?
fn parse_match_clause(pair: Pair<Rule>) -> Result<FtsTarget, ParseError> {
    // match_clause = { kw_MATCH ~ "(" ~ match_field_list ~ ")" ~ match_mode? }
    let mut fields = Vec::new();
    let mut mode = MatchMode::default();

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::match_field_list => {
                fields = parse_match_field_list(child)?;
            }
            Rule::match_mode => {
                mode = parse_match_mode(child)?;
            }
            _ => {} // Skip kw_MATCH and punctuation
        }
    }

    Ok(FtsTarget::Match { fields, mode })
}

/// Parse comma-separated field list with optional boosts
fn parse_match_field_list(pair: Pair<Rule>) -> Result<Vec<MatchField>, ParseError> {
    // match_field_list = { match_field ~ ("," ~ match_field)* }
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::match_field)
        .map(parse_match_field)
        .collect()
}

/// Parse a single field with optional boost: field^2.0
fn parse_match_field(pair: Pair<Rule>) -> Result<MatchField, ParseError> {
    // match_field = { ident ~ field_boost? }
    let mut name = String::new();
    let mut boost = None;

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::ident => {
                name = super::parse_ident(child);
            }
            Rule::field_boost => {
                boost = Some(parse_field_boost(child)?);
            }
            _ => {}
        }
    }

    Ok(MatchField { name, boost })
}

/// Parse field boost: ^number
fn parse_field_boost(pair: Pair<Rule>) -> Result<f64, ParseError> {
    // field_boost = { "^" ~ number }
    let number = pair
        .into_inner()
        .find(|p| p.as_rule() == Rule::number)
        .expect("grammar");
    number
        .as_str()
        .parse::<f64>()
        .map_err(|_| ParseError::InvalidNumber(number.as_str().to_string()))
}

/// Parse match mode: MODE SUM or MODE MAX(tie_breaker?)
fn parse_match_mode(pair: Pair<Rule>) -> Result<MatchMode, ParseError> {
    // match_mode = { kw_MODE ~ (mode_max | mode_sum) }
    let inner = pair.into_inner();

    for child in inner {
        match child.as_rule() {
            Rule::mode_sum => return Ok(MatchMode::Sum),
            Rule::mode_max => {
                // mode_max = { ^"MAX" ~ ("(" ~ number ~ ")")? }
                let tie_breaker = child
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::number)
                    .map(|n| {
                        n.as_str()
                            .parse::<f64>()
                            .map_err(|_| ParseError::InvalidNumber(n.as_str().to_string()))
                    })
                    .transpose()?;
                return Ok(MatchMode::Max { tie_breaker });
            }
            _ => {} // Skip kw_MODE
        }
    }

    // Default if somehow nothing matched
    Ok(MatchMode::Sum)
}

/// Parse KNN expression: field <|K|> vector or field <|K, effort|> vector
fn parse_knn_expr(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    // knn_expr = { field_path ~ knn_op ~ primary }
    let mut inner = pair.into_inner();

    let field_pair = inner.next().expect("grammar");
    let field = parse_field_path(field_pair);

    let knn_op = inner.next().expect("grammar");
    let vector_pair = inner.next().expect("grammar");

    // Parse knn_op: <|K|> or <|K, effort|>
    // knn_op = { "<|" ~ integer ~ ("," ~ integer)? ~ "|>" }
    let mut knn_inner = knn_op.into_inner();
    let k: usize = knn_inner
        .next()
        .expect("grammar: knn_op requires k value")
        .as_str()
        .parse()
        .unwrap_or(10);
    let effort: Option<usize> = knn_inner.next().map(|p| p.as_str().parse().unwrap_or(100));

    let vector = parse_primary(vector_pair)?;

    Ok(Expr::KnnSearch {
        field,
        vector: Box::new(vector),
        k,
        effort,
    })
}

/// Parse a graph traversal expression: ->follows->user, <-likes{1..3}<-*
fn parse_traversal_expr(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    let mut steps = Vec::new();
    let mut target = TraversalTarget::EdgeAll; // default: return all edge fields (id, from, to + user fields)
    let mut target_direction = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::traversal_start => {
                let (step, field_access) = parse_traversal_start(inner)?;
                target_direction = Some(step.direction);

                // If there's field access on the start edge (e.g., ->follows.{since, to}),
                // this is edge field selection, not node field selection
                if let Some((fields_or_path, is_multi_field)) = field_access {
                    if is_multi_field {
                        target = TraversalTarget::EdgeFields(fields_or_path);
                    } else if let Some(first) = fields_or_path.into_iter().next() {
                        target = TraversalTarget::EdgeField(first.path);
                    }
                }
                steps.push(step);
            }
            Rule::traversal_step => {
                steps.push(parse_traversal_step(inner)?);
            }
            Rule::traversal_end => {
                let (final_step, final_target, final_direction) = parse_traversal_end(inner)?;
                if let Some(step) = final_step {
                    steps.push(step);
                }
                target = final_target;
                target_direction = final_direction;
            }
            _ => {}
        }
    }

    if steps.is_empty() {
        return Err(ParseError::UnexpectedToken("Empty traversal".into()));
    }

    Ok(Expr::Traversal(TraversalExpr {
        steps,
        target,
        target_direction,
    }))
}

/// Parse traversal_start with optional field access: ->edge.{fields} or ->edge.field
fn parse_traversal_start(pair: Pair<Rule>) -> Result<TraversalStartResult, ParseError> {
    let mut direction = TraversalDirection::Outgoing;
    let mut label = None;
    let mut edge_filter = None;
    let mut depth = TraversalDepth::Single;
    let mut mode = TraversalMode::Deduplicate;
    let mut field_access = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::traversal_direction => {
                direction = match inner.as_str() {
                    "->" => TraversalDirection::Outgoing,
                    "<-" => TraversalDirection::Incoming,
                    "<->" => TraversalDirection::Bidirectional,
                    _ => unreachable!(),
                };
            }
            Rule::traversal_label_part => {
                let label_inner = inner.into_inner().next().expect("grammar");
                match label_inner.as_rule() {
                    Rule::traversal_simple_label => {
                        label = Some(super::parse_ident(
                            label_inner.into_inner().next().expect("grammar"),
                        ));
                    }
                    Rule::traversal_wildcard => {
                        label = None; // wildcard
                    }
                    Rule::traversal_filtered_label => {
                        let mut parts = label_inner.into_inner();
                        label = Some(super::parse_ident(parts.next().expect("grammar")));
                        let expr_pair = parts.find(|p| p.as_rule() == Rule::expr).expect("grammar");
                        edge_filter = Some(Box::new(parse_expr(expr_pair)?));
                    }
                    _ => {}
                }
            }
            Rule::traversal_depth => {
                let (d, m) = parse_traversal_depth(inner)?;
                depth = d;
                mode = m;
            }
            Rule::traversal_field_access => {
                // Parse .{fields} or .field
                for child in inner.into_inner() {
                    match child.as_rule() {
                        Rule::traversal_field_selection => {
                            let fields = parse_traversal_field_selection(child)?;
                            field_access = Some((fields, true)); // true = multi-field
                        }
                        Rule::field_path => {
                            let path = parse_field_path(child);
                            field_access = Some((
                                vec![crate::query::ast::expr::FieldSelection { alias: None, path }],
                                false,
                            )); // false = single field
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let step = TraversalStep {
        direction,
        label,
        edge_filter,
        depth,
        mode,
        node_filter: None,
    };

    Ok((step, field_access))
}

/// Parse a single traversal step: ->follows{1..5}, <-likes, etc.
fn parse_traversal_step(pair: Pair<Rule>) -> Result<TraversalStep, ParseError> {
    let mut direction = TraversalDirection::Outgoing;
    let mut label = None;
    let mut edge_filter = None;
    let mut depth = TraversalDepth::Single;
    let mut mode = TraversalMode::Deduplicate;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::traversal_direction => {
                direction = match inner.as_str() {
                    "->" => TraversalDirection::Outgoing,
                    "<-" => TraversalDirection::Incoming,
                    "<->" => TraversalDirection::Bidirectional,
                    _ => unreachable!(),
                };
            }
            Rule::traversal_label_part => {
                let label_inner = inner.into_inner().next().expect("grammar");
                match label_inner.as_rule() {
                    Rule::traversal_simple_label => {
                        label = Some(super::parse_ident(
                            label_inner.into_inner().next().expect("grammar"),
                        ));
                    }
                    Rule::traversal_wildcard => {
                        label = None; // wildcard
                    }
                    Rule::traversal_filtered_label => {
                        let mut parts = label_inner.into_inner();
                        label = Some(super::parse_ident(parts.next().expect("grammar")));
                        // Find the expr after WHERE keyword
                        let expr_pair = parts.find(|p| p.as_rule() == Rule::expr).expect("grammar");
                        edge_filter = Some(Box::new(parse_expr(expr_pair)?));
                    }
                    _ => {}
                }
            }
            Rule::traversal_depth => {
                let (d, m) = parse_traversal_depth(inner)?;
                depth = d;
                mode = m;
            }
            _ => {}
        }
    }

    Ok(TraversalStep {
        direction,
        label,
        edge_filter,
        depth,
        mode,
        node_filter: None,
    })
}

/// Parse depth specification: {3}, {1..5}, {..}, etc.
fn parse_traversal_depth(pair: Pair<Rule>) -> Result<(TraversalDepth, TraversalMode), ParseError> {
    let mut depth = TraversalDepth::Single;
    let mut mode = TraversalMode::Deduplicate;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::depth_spec => {
                let spec_str = inner.as_str().trim();
                depth = if spec_str == ".." {
                    TraversalDepth::Range {
                        min: None,
                        max: None,
                    }
                } else if let Some(stripped) = spec_str.strip_prefix("..") {
                    let max: usize = stripped
                        .trim()
                        .parse()
                        .map_err(|_| ParseError::InvalidNumber("Invalid depth".into()))?;
                    TraversalDepth::Range {
                        min: None,
                        max: Some(max),
                    }
                } else if let Some(stripped) = spec_str.strip_suffix("..") {
                    let min: usize = stripped
                        .trim()
                        .parse()
                        .map_err(|_| ParseError::InvalidNumber("Invalid depth".into()))?;
                    TraversalDepth::Range {
                        min: Some(min),
                        max: None,
                    }
                } else if spec_str.contains("..") {
                    let parts: Vec<&str> = spec_str.split("..").collect();
                    let min: usize = parts[0]
                        .trim()
                        .parse()
                        .map_err(|_| ParseError::InvalidNumber("Invalid depth".into()))?;
                    let max: usize = parts[1]
                        .trim()
                        .parse()
                        .map_err(|_| ParseError::InvalidNumber("Invalid depth".into()))?;
                    TraversalDepth::Range {
                        min: Some(min),
                        max: Some(max),
                    }
                } else {
                    let n: usize = spec_str
                        .parse()
                        .map_err(|_| ParseError::InvalidNumber("Invalid depth".into()))?;
                    TraversalDepth::Exact(n)
                };
            }
            Rule::traversal_mode => {
                mode = TraversalMode::All;
            }
            _ => {}
        }
    }

    Ok((depth, mode))
}

/// Parse multi-field selection: {field1, alias: field2, ...}
fn parse_traversal_field_selection(
    pair: Pair<Rule>,
) -> Result<Vec<crate::query::ast::expr::FieldSelection>, ParseError> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::traversal_field_item)
        .map(|item| {
            let mut alias: Option<String> = None;
            let mut path = String::new();

            for child in item.into_inner() {
                match child.as_rule() {
                    Rule::ident => {
                        // First ident could be alias (if followed by field_path) or the path itself
                        if alias.is_none() && path.is_empty() {
                            alias = Some(parse_ident(child));
                        }
                    }
                    Rule::field_path => {
                        path = parse_field_path(child);
                    }
                    _ => {}
                }
            }

            // If we only got alias but no path, the alias IS the path
            if path.is_empty()
                && let Some(a) = alias.take()
            {
                path = a;
            }

            Ok(crate::query::ast::expr::FieldSelection { alias, path })
        })
        .collect()
}

/// Parse traversal end: ->user, ->user.*, ->user.name
///
/// The traversal end specifies what to return at the end of traversal.
/// When a node target like `->user` is specified, it means "return nodes
/// in the user collection" - it does NOT create an additional traversal step.
fn parse_traversal_end(
    pair: Pair<Rule>,
) -> Result<
    (
        Option<TraversalStep>,
        TraversalTarget,
        Option<TraversalDirection>,
    ),
    ParseError,
> {
    let mut target = TraversalTarget::EdgeAll; // default: return all edge fields
    let mut direction = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::traversal_direction => {
                // Capture direction for the target (used when target is edge label, not collection)
                direction = Some(match inner.as_str() {
                    "->" => TraversalDirection::Outgoing,
                    "<-" => TraversalDirection::Incoming,
                    "<->" => TraversalDirection::Bidirectional,
                    _ => TraversalDirection::Outgoing,
                });
            }
            Rule::traversal_node_target => {
                // Check children to determine target type
                let mut node_name = String::new();
                let mut is_all = false;
                let mut single_field: Option<String> = None;
                let mut field_selections: Vec<crate::query::ast::expr::FieldSelection> = Vec::new();

                // Check the raw string first for ".*" pattern (literal doesn't produce child)
                let raw_target = inner.as_str();
                if raw_target.ends_with(".*") {
                    is_all = true;
                }

                for child in inner.into_inner() {
                    match child.as_rule() {
                        Rule::ident => {
                            node_name = parse_ident(child);
                        }
                        Rule::field_path => {
                            single_field = Some(parse_field_path(child));
                        }
                        Rule::traversal_field_selection => {
                            field_selections = parse_traversal_field_selection(child)?;
                        }
                        _ => {}
                    }
                }

                target = if !field_selections.is_empty() {
                    TraversalTarget::NodeFields(node_name, field_selections)
                } else if is_all {
                    TraversalTarget::NodeAll(node_name)
                } else if let Some(field) = single_field {
                    TraversalTarget::NodeField(node_name, field)
                } else {
                    TraversalTarget::Node(node_name)
                };
            }
            _ => {}
        }
    }

    // No step is created - the target just specifies what to return
    Ok((None, target, direction))
}

#[cfg(test)]
mod tests {
    use crate::document::Value;
    use crate::query::ast::{AggregateArg, BinaryOp, Expr, Projection, Statement, UnaryOp};
    use crate::query::parse::parse;

    fn select_projection(sql: &str) -> Projection {
        let stmts = parse(sql).unwrap();
        match stmts.into_iter().next().unwrap() {
            Statement::Select(s) => s.projection,
            _ => panic!("expected select"),
        }
    }

    fn first_expr(sql: &str) -> Expr {
        match select_projection(sql) {
            Projection::Items(items) => items.into_iter().next().unwrap().expr,
            _ => panic!("expected items"),
        }
    }

    #[test]
    fn test_parse_array_expr_literals() {
        let expr = first_expr("SELECT [1, 'two', true] as a FROM t");
        match expr {
            Expr::Array(items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0], Expr::Literal(Value::Int(1)));
                assert_eq!(items[1], Expr::Literal(Value::String("two".into())));
                assert_eq!(items[2], Expr::Literal(Value::Bool(true)));
            }
            _ => panic!("expected Expr::Array, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_array_expr_with_fields() {
        let expr = first_expr("SELECT [name, age] as a FROM t");
        match expr {
            Expr::Array(items) => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0], Expr::Field("name".into()));
                assert_eq!(items[1], Expr::Field("age".into()));
            }
            _ => panic!("expected Expr::Array"),
        }
    }

    #[test]
    fn test_parse_empty_array() {
        let expr = first_expr("SELECT [] as a FROM t");
        match expr {
            Expr::Array(items) => assert!(items.is_empty()),
            _ => panic!("expected Expr::Array"),
        }
    }

    #[test]
    fn test_parse_object_expr() {
        let expr = first_expr("SELECT {'name': name, 'val': 42} as o FROM t");
        match expr {
            Expr::Object(pairs) => {
                assert_eq!(pairs.len(), 2);
                assert_eq!(pairs[0].0, "name");
                assert_eq!(pairs[0].1, Expr::Field("name".into()));
                assert_eq!(pairs[1].0, "val");
                assert_eq!(pairs[1].1, Expr::Literal(Value::Int(42)));
            }
            _ => panic!("expected Expr::Object, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_object_with_ident_keys() {
        let expr = first_expr("SELECT {name: name, age: 10} as o FROM t");
        match expr {
            Expr::Object(pairs) => {
                assert_eq!(pairs.len(), 2);
                assert_eq!(pairs[0].0, "name");
                assert_eq!(pairs[1].0, "age");
            }
            _ => panic!("expected Expr::Object"),
        }
    }

    #[test]
    fn test_parse_empty_object() {
        let expr = first_expr("SELECT {} as o FROM t");
        match expr {
            Expr::Object(pairs) => assert!(pairs.is_empty()),
            _ => panic!("expected Expr::Object"),
        }
    }

    #[test]
    fn test_parse_object_with_expression_value() {
        let expr = first_expr("SELECT {'total': price * 2} as o FROM t");
        match expr {
            Expr::Object(pairs) => {
                assert_eq!(pairs.len(), 1);
                assert_eq!(pairs[0].0, "total");
                assert!(matches!(
                    pairs[0].1,
                    Expr::BinaryOp {
                        op: BinaryOp::Mul,
                        ..
                    }
                ));
            }
            _ => panic!("expected Expr::Object"),
        }
    }

    #[test]
    fn test_parse_nested_array_in_object() {
        let expr = first_expr("SELECT {'items': [1, 2]} as o FROM t");
        match expr {
            Expr::Object(pairs) => {
                assert_eq!(pairs[0].0, "items");
                assert!(matches!(pairs[0].1, Expr::Array(_)));
            }
            _ => panic!("expected Expr::Object"),
        }
    }

    #[test]
    fn test_parse_nested_object_in_array() {
        let expr = first_expr("SELECT [{'x': 1}, {'y': 2}] as a FROM t");
        match expr {
            Expr::Array(items) => {
                assert_eq!(items.len(), 2);
                assert!(matches!(items[0], Expr::Object(_)));
                assert!(matches!(items[1], Expr::Object(_)));
            }
            _ => panic!("expected Expr::Array"),
        }
    }

    #[test]
    fn test_parse_array_index() {
        let expr = first_expr("SELECT arr[0] as x FROM t");
        match expr {
            Expr::Index { base, index } => {
                assert_eq!(*base, Expr::Field("arr".into()));
                assert_eq!(*index, Expr::Literal(Value::Int(0)));
            }
            _ => panic!("expected Expr::Index, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_array_index_negative() {
        let expr = first_expr("SELECT arr[-1] as x FROM t");
        match expr {
            Expr::Index { base, index } => {
                assert_eq!(*base, Expr::Field("arr".into()));
                assert!(matches!(
                    *index,
                    Expr::UnaryOp {
                        op: UnaryOp::Neg,
                        ..
                    }
                ));
            }
            _ => panic!("expected Expr::Index"),
        }
    }

    #[test]
    fn test_parse_array_slice() {
        let expr = first_expr("SELECT arr[0..2] as x FROM t");
        match expr {
            Expr::Slice { base, start, end } => {
                assert_eq!(*base, Expr::Field("arr".into()));
                assert_eq!(*start.unwrap(), Expr::Literal(Value::Int(0)));
                assert_eq!(*end.unwrap(), Expr::Literal(Value::Int(2)));
            }
            _ => panic!("expected Expr::Slice, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_array_slice_open_end() {
        let expr = first_expr("SELECT arr[1..] as x FROM t");
        match expr {
            Expr::Slice { start, end, .. } => {
                assert!(start.is_some());
                assert!(end.is_none());
            }
            _ => panic!("expected Expr::Slice"),
        }
    }

    #[test]
    fn test_parse_array_slice_open_start() {
        let expr = first_expr("SELECT arr[..2] as x FROM t");
        match expr {
            Expr::Slice { start, end, .. } => {
                assert!(start.is_none());
                assert!(end.is_some());
            }
            _ => panic!("expected Expr::Slice"),
        }
    }

    #[test]
    fn test_parse_index_chain() {
        let expr = first_expr("SELECT matrix[0][1] as x FROM t");
        match expr {
            Expr::Index { base, index } => {
                assert_eq!(*index, Expr::Literal(Value::Int(1)));
                match *base {
                    Expr::Index {
                        base: inner_base,
                        index: inner_idx,
                    } => {
                        assert_eq!(*inner_base, Expr::Field("matrix".into()));
                        assert_eq!(*inner_idx, Expr::Literal(Value::Int(0)));
                    }
                    _ => panic!("expected inner Index"),
                }
            }
            _ => panic!("expected Expr::Index"),
        }
    }

    #[test]
    fn test_parse_index_then_field() {
        let expr = first_expr("SELECT users[0].name as x FROM t");
        match expr {
            Expr::FieldAccess { base, field } => {
                assert_eq!(field, "name");
                assert!(matches!(*base, Expr::Index { .. }));
            }
            _ => panic!("expected Expr::FieldAccess, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_subquery_index() {
        let expr = first_expr("SELECT (SELECT * FROM users)[0] as x FROM t");
        match expr {
            Expr::Index { base, index } => {
                assert!(matches!(*base, Expr::Subquery(_)));
                assert_eq!(*index, Expr::Literal(Value::Int(0)));
            }
            _ => panic!("expected Expr::Index"),
        }
    }

    #[test]
    fn test_parse_const_ref_pi() {
        // Test math::pi without parentheses -> Expr::Constant
        let expr = first_expr("SELECT math::pi AS pi FROM t");
        match expr {
            Expr::Constant { namespace, name } => {
                assert_eq!(name, "pi");
                assert_eq!(namespace, "math");
            }
            _ => panic!("expected Expr::Constant, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_const_ref_e() {
        // Test math::e without parentheses -> Expr::Constant
        let expr = first_expr("SELECT math::e AS e FROM t");
        match expr {
            Expr::Constant { namespace, name } => {
                assert_eq!(name, "e");
                assert_eq!(namespace, "math");
            }
            _ => panic!("expected Expr::Constant, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_const_ref_with_parens_is_function() {
        // Test math::pi() with parentheses -> Expr::FunctionCall (not Constant!)
        let expr = first_expr("SELECT math::pi() AS pi FROM t");
        match expr {
            Expr::FunctionCall {
                name,
                namespace,
                args,
            } => {
                assert_eq!(name, "pi");
                assert_eq!(namespace, Some("math".to_string()));
                assert!(args.is_empty());
            }
            _ => panic!("expected Expr::FunctionCall, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_const_ref_in_expression() {
        // Test math::pi in multiplication expression -> Expr::Constant on left
        let expr = first_expr("SELECT math::pi * 2 AS tau FROM t");
        match expr {
            Expr::BinaryOp {
                left,
                op: BinaryOp::Mul,
                right,
            } => {
                match *left {
                    Expr::Constant { namespace, name } => {
                        assert_eq!(name, "pi");
                        assert_eq!(namespace, "math");
                    }
                    _ => panic!("expected Expr::Constant on left"),
                }
                assert_eq!(*right, Expr::Literal(Value::Int(2)));
            }
            _ => panic!("expected Expr::BinaryOp, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_const_ref_as_function_arg() {
        // Test math::pi as argument to another function: math::sin(math::pi)
        // The inner math::pi is Expr::Constant
        let expr = first_expr("SELECT math::sin(math::pi) AS s FROM t");
        match expr {
            Expr::FunctionCall {
                name,
                namespace,
                args,
            } => {
                assert_eq!(name, "sin");
                assert_eq!(namespace, Some("math".to_string()));
                assert_eq!(args.len(), 1);
                match &args[0] {
                    Expr::Constant { namespace, name } => {
                        assert_eq!(name, "pi");
                        assert_eq!(namespace, "math");
                    }
                    _ => panic!("expected Expr::Constant as arg, got {:?}", args[0]),
                }
            }
            _ => panic!("expected Expr::FunctionCall, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_variable_ref() {
        // Test $var -> Expr::Variable
        let expr = first_expr("SELECT $tax_rate AS rate FROM t");
        match expr {
            Expr::Variable(name) => {
                assert_eq!(name, "tax_rate");
            }
            _ => panic!("expected Expr::Variable, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_let_statement() {
        // Test LET statement parsing
        let stmts = parse("LET x = 42").unwrap();
        match &stmts[0] {
            Statement::Let(let_ast) => {
                assert_eq!(let_ast.name, "x");
                assert_eq!(let_ast.expr, Expr::Literal(Value::Int(42)));
            }
            _ => panic!("expected Statement::Let, got {:?}", stmts[0]),
        }
    }

    #[test]
    fn test_parse_let_with_expression() {
        // Test LET with expression
        let stmts = parse("LET total = price * 1.2").unwrap();
        match &stmts[0] {
            Statement::Let(let_ast) => {
                assert_eq!(let_ast.name, "total");
                assert!(matches!(let_ast.expr, Expr::BinaryOp { .. }));
            }
            _ => panic!("expected Statement::Let"),
        }
    }

    #[test]
    fn test_parse_count_distinct() {
        let stmts = parse("SELECT COUNT(DISTINCT city) FROM users").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.projection {
                Projection::Items(items) => match &items[0].expr {
                    Expr::Aggregate(agg) => {
                        assert_eq!(agg.function, "COUNT");
                        assert!(agg.distinct);
                        assert_eq!(agg.arg, AggregateArg::Field("city".to_string()));
                    }
                    _ => panic!("expected Aggregate"),
                },
                _ => panic!("expected Items projection"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_parse_sum_distinct() {
        let stmts = parse("SELECT SUM(DISTINCT score) FROM users").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.projection {
                Projection::Items(items) => match &items[0].expr {
                    Expr::Aggregate(agg) => {
                        assert_eq!(agg.function, "SUM");
                        assert!(agg.distinct);
                    }
                    _ => panic!("expected Aggregate"),
                },
                _ => panic!("expected Items projection"),
            },
            _ => panic!("expected select"),
        }
    }

    // ================================
    // FTS Expression Parsing Tests
    // ================================

    use crate::query::ast::{FtsOperator, FtsTarget, MatchMode};

    #[test]
    fn test_parse_fts_simple_field() {
        // Test: title @@ "search query"
        let expr = first_expr("SELECT title @@ 'hello world' FROM docs");
        match expr {
            Expr::Fts {
                target,
                operator,
                query,
            } => {
                assert_eq!(target, FtsTarget::Field("title".to_string()));
                assert_eq!(operator, FtsOperator::Simple);
                assert_eq!(query, "hello world");
            }
            _ => panic!("expected Expr::Fts, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_fts_named_operator() {
        // Test: content @:search@ "query"
        let expr = first_expr("SELECT content @:search@ \"rust programming\" FROM docs");
        match expr {
            Expr::Fts {
                target,
                operator,
                query,
            } => {
                assert_eq!(target, FtsTarget::Field("content".to_string()));
                assert_eq!(operator, FtsOperator::Named("search".to_string()));
                assert_eq!(query, "rust programming");
            }
            _ => panic!("expected Expr::Fts, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_fts_match_clause_simple() {
        // Test: MATCH(title, body) @@ "query"
        let expr = first_expr("SELECT MATCH(title, body) @@ 'hello' FROM docs");
        match expr {
            Expr::Fts {
                target,
                operator,
                query,
            } => {
                match target {
                    FtsTarget::Match { fields, mode } => {
                        assert_eq!(fields.len(), 2);
                        assert_eq!(fields[0].name, "title");
                        assert_eq!(fields[0].boost, None);
                        assert_eq!(fields[1].name, "body");
                        assert_eq!(fields[1].boost, None);
                        assert_eq!(mode, MatchMode::Sum);
                    }
                    _ => panic!("expected FtsTarget::Match"),
                }
                assert_eq!(operator, FtsOperator::Simple);
                assert_eq!(query, "hello");
            }
            _ => panic!("expected Expr::Fts, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_fts_match_with_boosts() {
        // Test: MATCH(title^2.0, body^0.5) @@ "query"
        let expr = first_expr("SELECT MATCH(title^2.0, body^0.5) @@ 'search' FROM docs");
        match expr {
            Expr::Fts { target, .. } => match target {
                FtsTarget::Match { fields, .. } => {
                    assert_eq!(fields.len(), 2);
                    assert_eq!(fields[0].name, "title");
                    assert_eq!(fields[0].boost, Some(2.0));
                    assert_eq!(fields[1].name, "body");
                    assert_eq!(fields[1].boost, Some(0.5));
                }
                _ => panic!("expected FtsTarget::Match"),
            },
            _ => panic!("expected Expr::Fts, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_fts_match_mode_sum() {
        // Test: MATCH(title, body) MODE SUM @@ "query"
        let expr = first_expr("SELECT MATCH(title, body) MODE SUM @@ 'test' FROM docs");
        match expr {
            Expr::Fts { target, .. } => match target {
                FtsTarget::Match { mode, .. } => {
                    assert_eq!(mode, MatchMode::Sum);
                }
                _ => panic!("expected FtsTarget::Match"),
            },
            _ => panic!("expected Expr::Fts, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_fts_match_mode_max() {
        // Test: MATCH(title, body) MODE MAX @@ "query"
        let expr = first_expr("SELECT MATCH(title, body) MODE MAX @@ 'test' FROM docs");
        match expr {
            Expr::Fts { target, .. } => match target {
                FtsTarget::Match { mode, .. } => {
                    assert_eq!(mode, MatchMode::Max { tie_breaker: None });
                }
                _ => panic!("expected FtsTarget::Match"),
            },
            _ => panic!("expected Expr::Fts, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_fts_match_mode_max_with_tie_breaker() {
        // Test: MATCH(title, body) MODE MAX(0.3) @@ "query"
        let expr = first_expr("SELECT MATCH(title, body) MODE MAX(0.3) @@ 'test' FROM docs");
        match expr {
            Expr::Fts { target, .. } => match target {
                FtsTarget::Match { mode, .. } => {
                    assert_eq!(
                        mode,
                        MatchMode::Max {
                            tie_breaker: Some(0.3)
                        }
                    );
                }
                _ => panic!("expected FtsTarget::Match"),
            },
            _ => panic!("expected Expr::Fts, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_fts_in_where_clause() {
        // Test FTS in WHERE clause
        let stmts = parse("SELECT * FROM docs WHERE title @@ 'rust'").unwrap();
        match &stmts[0] {
            Statement::Select(s) => {
                let filter = s.filter.as_ref().expect("expected filter");
                match filter {
                    Expr::Fts {
                        target,
                        operator,
                        query,
                    } => {
                        assert_eq!(*target, FtsTarget::Field("title".to_string()));
                        assert_eq!(*operator, FtsOperator::Simple);
                        assert_eq!(query, "rust");
                    }
                    _ => panic!("expected Expr::Fts in filter, got {:?}", filter),
                }
            }
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_parse_fts_dotted_field() {
        // Test: metadata.title @@ "query"
        let expr = first_expr("SELECT metadata.title @@ 'test' FROM docs");
        match expr {
            Expr::Fts { target, .. } => {
                assert_eq!(target, FtsTarget::Field("metadata.title".to_string()));
            }
            _ => panic!("expected Expr::Fts, got {:?}", expr),
        }
    }

    // ================================
    // KNN Expression Parsing Tests
    // ================================

    #[test]
    fn test_parse_knn_simple() {
        let input = "SELECT * FROM products WHERE embedding <|10|> [0.1, 0.2, 0.3]";
        let result = parse(input).unwrap();
        match &result[0] {
            Statement::Select(s) => {
                let filter = s.filter.as_ref().expect("expected filter");
                match filter {
                    Expr::KnnSearch {
                        field,
                        k,
                        effort,
                        vector,
                    } => {
                        assert_eq!(field, "embedding");
                        assert_eq!(*k, 10);
                        assert!(effort.is_none());
                        assert!(matches!(**vector, Expr::Array(_)));
                    }
                    _ => panic!("expected Expr::KnnSearch, got {:?}", filter),
                }
            }
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_parse_knn_with_effort() {
        let input = "SELECT * FROM products WHERE embedding <|10, 100|> [0.1, 0.2]";
        let result = parse(input).unwrap();
        match &result[0] {
            Statement::Select(s) => {
                let filter = s.filter.as_ref().expect("expected filter");
                match filter {
                    Expr::KnnSearch {
                        field,
                        k,
                        effort,
                        vector,
                    } => {
                        assert_eq!(field, "embedding");
                        assert_eq!(*k, 10);
                        assert_eq!(*effort, Some(100));
                        assert!(matches!(**vector, Expr::Array(_)));
                    }
                    _ => panic!("expected Expr::KnnSearch, got {:?}", filter),
                }
            }
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_parse_knn_with_variable() {
        let input = "SELECT * FROM products WHERE embedding <|5|> $query_vec";
        let result = parse(input).unwrap();
        match &result[0] {
            Statement::Select(s) => {
                let filter = s.filter.as_ref().expect("expected filter");
                match filter {
                    Expr::KnnSearch {
                        field,
                        k,
                        effort,
                        vector,
                    } => {
                        assert_eq!(field, "embedding");
                        assert_eq!(*k, 5);
                        assert!(effort.is_none());
                        match &**vector {
                            Expr::Variable(name) => assert_eq!(name, "query_vec"),
                            _ => panic!("expected Expr::Variable, got {:?}", vector),
                        }
                    }
                    _ => panic!("expected Expr::KnnSearch, got {:?}", filter),
                }
            }
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_parse_knn_in_projection() {
        // KNN can also appear in projection
        let input = "SELECT embedding <|3|> [1.0, 2.0] FROM products";
        let result = parse(input).unwrap();
        match &result[0] {
            Statement::Select(s) => match &s.projection {
                Projection::Items(items) => match &items[0].expr {
                    Expr::KnnSearch { field, k, .. } => {
                        assert_eq!(field, "embedding");
                        assert_eq!(*k, 3);
                    }
                    _ => panic!("expected Expr::KnnSearch"),
                },
                _ => panic!("expected Items projection"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_parse_knn_dotted_field() {
        // Test KNN with dotted field path
        let input = "SELECT * FROM products WHERE data.embedding <|10|> [0.5, 0.5]";
        let result = parse(input).unwrap();
        match &result[0] {
            Statement::Select(s) => {
                let filter = s.filter.as_ref().expect("expected filter");
                match filter {
                    Expr::KnnSearch { field, .. } => {
                        assert_eq!(field, "data.embedding");
                    }
                    _ => panic!("expected Expr::KnnSearch, got {:?}", filter),
                }
            }
            _ => panic!("expected select"),
        }
    }

    // ================================
    // Graph Traversal Parsing Tests
    // ================================

    use crate::query::ast::{TraversalDepth, TraversalDirection, TraversalMode, TraversalTarget};

    #[test]
    fn test_parse_traversal_basic() {
        // Test: ->follows (simple outgoing traversal)
        let expr = first_expr("SELECT ->follows FROM user:alice");
        match expr {
            Expr::Traversal(t) => {
                assert_eq!(t.steps.len(), 1);
                assert_eq!(t.steps[0].direction, TraversalDirection::Outgoing);
                assert_eq!(t.steps[0].label, Some("follows".to_string()));
                assert_eq!(t.steps[0].depth, TraversalDepth::Single);
                assert_eq!(t.target, TraversalTarget::EdgeAll); // default: all edge fields
            }
            _ => panic!("expected Expr::Traversal, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_traversal_incoming() {
        // Test: <-follows (incoming traversal)
        let expr = first_expr("SELECT <-follows FROM user:bob");
        match expr {
            Expr::Traversal(t) => {
                assert_eq!(t.steps.len(), 1);
                assert_eq!(t.steps[0].direction, TraversalDirection::Incoming);
                assert_eq!(t.steps[0].label, Some("follows".to_string()));
            }
            _ => panic!("expected Expr::Traversal, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_traversal_bidirectional() {
        // Test: <->friends (bidirectional traversal)
        let expr = first_expr("SELECT <->friends FROM user:alice");
        match expr {
            Expr::Traversal(t) => {
                assert_eq!(t.steps.len(), 1);
                assert_eq!(t.steps[0].direction, TraversalDirection::Bidirectional);
                assert_eq!(t.steps[0].label, Some("friends".to_string()));
            }
            _ => panic!("expected Expr::Traversal, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_traversal_with_node_target() {
        // The final ->user is parsed as a node target (not an edge step)
        // because no further traversal direction follows it.
        // So ->follows->user parses as: 1 step (follows) + target Node("user")
        let expr = first_expr("SELECT ->follows->user FROM user:alice");
        match expr {
            Expr::Traversal(t) => {
                // One step: ->follows
                assert_eq!(t.steps.len(), 1);
                assert_eq!(t.steps[0].label, Some("follows".to_string()));
                // Target is Node("user")
                assert_eq!(t.target, TraversalTarget::Node("user".to_string()));
            }
            _ => panic!("expected Expr::Traversal, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_traversal_with_node_all() {
        // Test: ->follows->user.* (all fields of target node)
        let expr = first_expr("SELECT ->follows->user.* FROM user:alice");
        match expr {
            Expr::Traversal(t) => {
                assert_eq!(t.target, TraversalTarget::NodeAll("user".to_string()));
            }
            _ => panic!("expected Expr::Traversal, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_traversal_with_node_field() {
        // Test: ->follows->user.name (specific field of target node)
        let expr = first_expr("SELECT ->follows->user.name FROM user:alice");
        match expr {
            Expr::Traversal(t) => {
                assert_eq!(
                    t.target,
                    TraversalTarget::NodeField("user".to_string(), "name".to_string())
                );
            }
            _ => panic!("expected Expr::Traversal, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_traversal_exact_depth() {
        // Test: ->follows{2}->user.* (exact 2 hops, return all user fields)
        let expr = first_expr("SELECT ->follows{2}->user.* FROM user:alice");
        match expr {
            Expr::Traversal(t) => {
                assert_eq!(t.steps[0].depth, TraversalDepth::Exact(2));
                assert_eq!(t.target, TraversalTarget::NodeAll("user".to_string()));
            }
            _ => panic!("expected Expr::Traversal, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_traversal_range_depth() {
        // Test: ->follows{1..5}->user.* (1 to 5 hops)
        let expr = first_expr("SELECT ->follows{1..5}->user.* FROM user:alice");
        match expr {
            Expr::Traversal(t) => {
                assert_eq!(
                    t.steps[0].depth,
                    TraversalDepth::Range {
                        min: Some(1),
                        max: Some(5)
                    }
                );
            }
            _ => panic!("expected Expr::Traversal, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_traversal_open_range_depth() {
        // Test: ->follows{..}->user.* (unlimited hops)
        let expr = first_expr("SELECT ->follows{..}->user.* FROM user:alice");
        match expr {
            Expr::Traversal(t) => {
                assert_eq!(
                    t.steps[0].depth,
                    TraversalDepth::Range {
                        min: None,
                        max: None
                    }
                );
            }
            _ => panic!("expected Expr::Traversal, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_traversal_min_only_depth() {
        // Test: ->follows{2..}->user.* (at least 2 hops)
        let expr = first_expr("SELECT ->follows{2..}->user.* FROM user:alice");
        match expr {
            Expr::Traversal(t) => {
                assert_eq!(
                    t.steps[0].depth,
                    TraversalDepth::Range {
                        min: Some(2),
                        max: None
                    }
                );
            }
            _ => panic!("expected Expr::Traversal, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_traversal_max_only_depth() {
        // Test: ->follows{..5}->user.* (at most 5 hops)
        let expr = first_expr("SELECT ->follows{..5}->user.* FROM user:alice");
        match expr {
            Expr::Traversal(t) => {
                assert_eq!(
                    t.steps[0].depth,
                    TraversalDepth::Range {
                        min: None,
                        max: Some(5)
                    }
                );
            }
            _ => panic!("expected Expr::Traversal, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_traversal_wildcard() {
        // Test: ->*->user.* (any edge type)
        let expr = first_expr("SELECT ->*->user.* FROM user:alice");
        match expr {
            Expr::Traversal(t) => {
                assert_eq!(t.steps[0].label, None); // wildcard
            }
            _ => panic!("expected Expr::Traversal, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_traversal_all_mode() {
        // Test: ->follows{1..5 ALL}->user.* (return all paths)
        let expr = first_expr("SELECT ->follows{1..5 ALL}->user.* FROM user:alice");
        match expr {
            Expr::Traversal(t) => {
                assert_eq!(t.steps[0].mode, TraversalMode::All);
            }
            _ => panic!("expected Expr::Traversal, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_traversal_with_filter() {
        // Test: ->(follows WHERE since > 2024)->user.* (filtered traversal)
        let expr = first_expr("SELECT ->(follows WHERE since > 2024)->user.* FROM user:alice");
        match expr {
            Expr::Traversal(t) => {
                assert_eq!(t.steps[0].label, Some("follows".to_string()));
                assert!(t.steps[0].edge_filter.is_some());
                // Verify it's a comparison expression
                match t.steps[0].edge_filter.as_ref().unwrap().as_ref() {
                    Expr::BinaryOp {
                        op: BinaryOp::Gt, ..
                    } => {}
                    other => panic!("expected BinaryOp Gt, got {:?}", other),
                }
            }
            _ => panic!("expected Expr::Traversal, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_traversal_target_direction_outgoing() {
        // When the final target has -> before it, target_direction should be Outgoing
        // <-follows->wishlisted: "wishlisted" is the target, -> is before it
        let expr = first_expr("SELECT <-follows->wishlisted FROM customer:1");
        match expr {
            Expr::Traversal(t) => {
                assert_eq!(t.steps.len(), 1);
                assert_eq!(t.steps[0].direction, TraversalDirection::Incoming);
                assert_eq!(t.steps[0].label, Some("follows".to_string()));
                assert_eq!(t.target, TraversalTarget::Node("wishlisted".to_string()));
                assert_eq!(t.target_direction, Some(TraversalDirection::Outgoing));
            }
            _ => panic!("expected Expr::Traversal, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_traversal_target_direction_incoming() {
        // When the final target has <- before it, target_direction should be Incoming
        // ->follows<-subscribers: "subscribers" is the target, <- is before it
        let expr = first_expr("SELECT ->follows<-subscribers FROM user:1");
        match expr {
            Expr::Traversal(t) => {
                assert_eq!(t.steps.len(), 1);
                assert_eq!(t.steps[0].direction, TraversalDirection::Outgoing);
                assert_eq!(t.steps[0].label, Some("follows".to_string()));
                assert_eq!(t.target, TraversalTarget::Node("subscribers".to_string()));
                assert_eq!(t.target_direction, Some(TraversalDirection::Incoming));
            }
            _ => panic!("expected Expr::Traversal, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_traversal_target_direction_bidirectional() {
        // When the final target has <-> before it, target_direction should be Bidirectional
        let expr = first_expr("SELECT ->follows<->connections FROM user:1");
        match expr {
            Expr::Traversal(t) => {
                assert_eq!(t.target, TraversalTarget::Node("connections".to_string()));
                assert_eq!(t.target_direction, Some(TraversalDirection::Bidirectional));
            }
            _ => panic!("expected Expr::Traversal, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_traversal_target_direction_none_for_collection() {
        // When target is specified with .* (NodeAll), direction is still captured
        let expr = first_expr("SELECT ->follows->user.* FROM user:alice");
        match expr {
            Expr::Traversal(t) => {
                assert_eq!(t.target, TraversalTarget::NodeAll("user".to_string()));
                // Direction is captured even for real collection targets
                assert_eq!(t.target_direction, Some(TraversalDirection::Outgoing));
            }
            _ => panic!("expected Expr::Traversal, got {:?}", expr),
        }
    }

    // ================================
    // Multi-field Selection (Brace Expansion) Tests
    // ================================

    #[test]
    fn test_parse_field_expansion_simple() {
        // Test: address.{city, country} expands to address.city, address.country
        let projection = select_projection("SELECT address.{city, country} FROM users");
        match projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].expr, Expr::Field("address.city".to_string()));
                assert_eq!(items[1].expr, Expr::Field("address.country".to_string()));
                assert!(items[0].alias.is_none());
                assert!(items[1].alias.is_none());
            }
            _ => panic!("expected Projection::Items"),
        }
    }

    #[test]
    fn test_parse_field_expansion_deep_nesting() {
        // Test: user.profile.address.{street, zip} expands correctly
        let projection = select_projection("SELECT user.profile.address.{street, zip} FROM data");
        match projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 2);
                assert_eq!(
                    items[0].expr,
                    Expr::Field("user.profile.address.street".to_string())
                );
                assert_eq!(
                    items[1].expr,
                    Expr::Field("user.profile.address.zip".to_string())
                );
            }
            _ => panic!("expected Projection::Items"),
        }
    }

    #[test]
    fn test_parse_field_expansion_multiple_fields() {
        // Test: data.{a, b, c, d} expands to four fields
        let projection = select_projection("SELECT data.{a, b, c, d} FROM t");
        match projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 4);
                assert_eq!(items[0].expr, Expr::Field("data.a".to_string()));
                assert_eq!(items[1].expr, Expr::Field("data.b".to_string()));
                assert_eq!(items[2].expr, Expr::Field("data.c".to_string()));
                assert_eq!(items[3].expr, Expr::Field("data.d".to_string()));
            }
            _ => panic!("expected Projection::Items"),
        }
    }

    #[test]
    fn test_parse_field_expansion_with_other_fields() {
        // Test: mixing field expansion with regular fields
        let projection = select_projection("SELECT id, address.{city, zip}, name FROM users");
        match projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 4);
                assert_eq!(items[0].expr, Expr::Field("id".to_string()));
                assert_eq!(items[1].expr, Expr::Field("address.city".to_string()));
                assert_eq!(items[2].expr, Expr::Field("address.zip".to_string()));
                assert_eq!(items[3].expr, Expr::Field("name".to_string()));
            }
            _ => panic!("expected Projection::Items"),
        }
    }

    #[test]
    fn test_parse_field_expansion_single_field() {
        // Test: address.{city} works with single field (edge case)
        let projection = select_projection("SELECT address.{city} FROM users");
        match projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].expr, Expr::Field("address.city".to_string()));
            }
            _ => panic!("expected Projection::Items"),
        }
    }

    #[test]
    fn test_parse_multiple_field_expansions() {
        // Test: multiple brace expansions in same query
        let projection =
            select_projection("SELECT user.{name, email}, address.{city, country} FROM data");
        match projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 4);
                assert_eq!(items[0].expr, Expr::Field("user.name".to_string()));
                assert_eq!(items[1].expr, Expr::Field("user.email".to_string()));
                assert_eq!(items[2].expr, Expr::Field("address.city".to_string()));
                assert_eq!(items[3].expr, Expr::Field("address.country".to_string()));
            }
            _ => panic!("expected Projection::Items"),
        }
    }

    #[test]
    fn test_parse_field_expansion_with_alias() {
        // Test: address.{zip: postal_code, city} -> address.postal_code AS zip, address.city
        let projection = select_projection("SELECT address.{zip: postal_code, city} FROM users");
        match projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 2);
                // First item: address.postal_code AS zip
                assert_eq!(
                    items[0].expr,
                    Expr::Field("address.postal_code".to_string())
                );
                assert_eq!(items[0].alias, Some("zip".to_string()));
                // Second item: address.city (no alias)
                assert_eq!(items[1].expr, Expr::Field("address.city".to_string()));
                assert!(items[1].alias.is_none());
            }
            _ => panic!("expected Projection::Items"),
        }
    }

    #[test]
    fn test_parse_field_expansion_all_aliased() {
        // Test: all fields have aliases
        let projection =
            select_projection("SELECT data.{x: field_a, y: field_b, z: field_c} FROM t");
        match projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0].expr, Expr::Field("data.field_a".to_string()));
                assert_eq!(items[0].alias, Some("x".to_string()));
                assert_eq!(items[1].expr, Expr::Field("data.field_b".to_string()));
                assert_eq!(items[1].alias, Some("y".to_string()));
                assert_eq!(items[2].expr, Expr::Field("data.field_c".to_string()));
                assert_eq!(items[2].alias, Some("z".to_string()));
            }
            _ => panic!("expected Projection::Items"),
        }
    }

    #[test]
    fn test_parse_field_expansion_mixed_alias_and_plain() {
        // Test: mixing aliased and non-aliased fields
        let projection = select_projection("SELECT user.{id, full_name: name, age} FROM users");
        match projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 3);
                // id - no alias
                assert_eq!(items[0].expr, Expr::Field("user.id".to_string()));
                assert!(items[0].alias.is_none());
                // name AS full_name
                assert_eq!(items[1].expr, Expr::Field("user.name".to_string()));
                assert_eq!(items[1].alias, Some("full_name".to_string()));
                // age - no alias
                assert_eq!(items[2].expr, Expr::Field("user.age".to_string()));
                assert!(items[2].alias.is_none());
            }
            _ => panic!("expected Projection::Items"),
        }
    }

    #[test]
    fn test_parse_traversal_edge_fields() {
        // ->contains.{quantity, name} - multi-field on edge
        let expr = first_expr("SELECT ->contains.{quantity, name} FROM recipe:1");
        match expr {
            Expr::Traversal(t) => {
                // Edge field access parses directly to EdgeFields
                match &t.target {
                    TraversalTarget::EdgeFields(fields) => {
                        assert_eq!(fields.len(), 2);
                        assert_eq!(fields[0].path, "quantity");
                        assert!(fields[0].alias.is_none());
                        assert_eq!(fields[1].path, "name");
                    }
                    _ => panic!("expected EdgeFields, got {:?}", t.target),
                }
                // The edge label should be in the step
                assert_eq!(t.steps.len(), 1);
                assert_eq!(t.steps[0].label, Some("contains".to_string()));
            }
            _ => panic!("expected Traversal, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_traversal_fields_with_alias() {
        // ->contains.{grams: quantity_grams, ingredient: to.name} - edge field with aliases
        let expr = first_expr(
            "SELECT ->contains.{grams: quantity_grams, ingredient: to.name} FROM recipe:1",
        );
        match expr {
            Expr::Traversal(t) => {
                match &t.target {
                    TraversalTarget::EdgeFields(fields) => {
                        assert_eq!(fields.len(), 2);
                        assert_eq!(fields[0].alias, Some("grams".to_string()));
                        assert_eq!(fields[0].path, "quantity_grams");
                        assert_eq!(fields[1].alias, Some("ingredient".to_string()));
                        assert_eq!(fields[1].path, "to.name");
                    }
                    _ => panic!("expected EdgeFields, got {:?}", t.target),
                }
                // The edge label should be in the step
                assert_eq!(t.steps.len(), 1);
                assert_eq!(t.steps[0].label, Some("contains".to_string()));
            }
            _ => panic!("expected Traversal, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_traversal_node_fields() {
        let expr = first_expr("SELECT ->follows->user.{name, email} FROM user:alice");
        match expr {
            Expr::Traversal(t) => {
                assert_eq!(t.steps.len(), 1);
                assert_eq!(t.steps[0].label, Some("follows".to_string()));
                match &t.target {
                    TraversalTarget::NodeFields(name, fields) => {
                        assert_eq!(name, "user");
                        assert_eq!(fields.len(), 2);
                        assert_eq!(fields[0].path, "name");
                        assert_eq!(fields[1].path, "email");
                    }
                    _ => panic!("expected NodeFields, got {:?}", t.target),
                }
            }
            _ => panic!("expected Traversal, got {:?}", expr),
        }
    }
}
