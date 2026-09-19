//! Schema and field type parsing for DEFINE COLLECTION.

use pest::iterators::Pair;

use super::super::value::Rule;
use crate::query::ast::{DefineCollectionAst, FieldDefAst, Statement};
use crate::query::error::ParseError;
use crate::schema::{DefaultValue, FieldDef, FieldType, IndexDef, SchemaMode};

/// Parse DEFINE COLLECTION statement.
pub fn parse_define_collection(pair: Pair<Rule>) -> Result<Statement, ParseError> {
    let mut inner = super::super::semantic_children(pair).into_iter();
    let name = super::super::parse_ident(inner.next().expect("grammar"));

    let mut mode = SchemaMode::Flexible; // default
    let mut fields = Vec::new();
    let mut indexes = Vec::new();

    if let Some(body) = inner.next() {
        // parse define_collection_body -> define_collection_items -> define_collection_item*
        // body is define_collection_body, its first child is define_collection_items
        let items = body.into_inner().next().expect("grammar"); // define_collection_items
        for item in items.into_inner() {
            // each is define_collection_item which contains one of:
            // schema_mode_def, field_def, index_def
            let inner_item = item.into_inner().next().expect("grammar");
            match inner_item.as_rule() {
                Rule::schema_mode_def => {
                    // Extract STRICT or FLEXIBLE from schema_mode_value
                    let mode_value = super::super::semantic_children(inner_item)
                        .into_iter()
                        .next()
                        .expect("grammar");
                    let mode_str = mode_value.as_str();
                    mode = if mode_str.eq_ignore_ascii_case("strict") {
                        SchemaMode::Strict
                    } else {
                        SchemaMode::Flexible
                    };
                }
                Rule::field_def => {
                    fields.push(parse_field_def(inner_item)?);
                }
                Rule::index_def => {
                    indexes.push(parse_index_def(inner_item)?);
                }
                _ => {}
            }
        }
    }

    Ok(Statement::DefineCollection(DefineCollectionAst {
        name,
        mode,
        fields,
        indexes,
    }))
}

fn parse_field_def(pair: Pair<Rule>) -> Result<FieldDefAst, ParseError> {
    let mut inner = pair.into_inner();
    let name = super::super::parse_ident(inner.next().expect("grammar"));
    let field_type = parse_field_type(inner.next().expect("grammar"))?;

    let mut required = false;
    let mut default = None;

    // Parse optional modifiers
    if let Some(modifiers) = inner.next() {
        for modifier in modifiers.into_inner() {
            match modifier.as_rule() {
                Rule::required_mod => required = true,
                Rule::default_mod => {
                    let value = super::super::semantic_children(modifier)
                        .into_iter()
                        .next()
                        .expect("grammar");
                    default = Some(parse_default_value(value)?);
                }
                _ => {}
            }
        }
    }

    Ok(FieldDefAst {
        name,
        field_type,
        required,
        default,
    })
}

fn parse_field_type(pair: Pair<Rule>) -> Result<FieldType, ParseError> {
    // field_type -> field_type_union
    let inner = pair.into_inner().next().expect("grammar");
    parse_field_type_union(inner)
}

fn parse_field_type_union(pair: Pair<Rule>) -> Result<FieldType, ParseError> {
    // field_type_union = { field_type_single ~ ("|" ~ field_type_single)* }
    let types: Vec<FieldType> = pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::field_type_single)
        .map(parse_field_type_single)
        .collect::<Result<Vec<_>, _>>()?;

    if types.len() == 1 {
        Ok(types.into_iter().next().expect("grammar"))
    } else {
        Ok(FieldType::Union(types))
    }
}

fn parse_field_type_single(pair: Pair<Rule>) -> Result<FieldType, ParseError> {
    // field_type_single = { array_type | range_type | object_type | primitive_type }
    let inner = pair.into_inner().next().expect("grammar");
    match inner.as_rule() {
        Rule::primitive_type => parse_primitive_type(inner),
        Rule::array_type => {
            let element_type = inner.into_inner().next().expect("grammar");
            let inner_type = parse_field_type_inner(element_type)?;
            Ok(FieldType::Array(Box::new(inner_type)))
        }
        Rule::range_type => {
            // range_type = { ^"range" ~ "<" ~ field_type_single ~ ">" }
            let element_type = inner
                .into_inner()
                .find(|p| p.as_rule() == Rule::field_type_single)
                .expect("grammar");
            let inner_type = parse_field_type_single(element_type)?;
            Ok(FieldType::Range(Box::new(inner_type)))
        }
        Rule::object_type => parse_object_type(inner),
        _ => Err(ParseError::UnexpectedToken(inner.as_str().to_string())),
    }
}

fn parse_primitive_type(pair: Pair<Rule>) -> Result<FieldType, ParseError> {
    // Check if there's a reference_type child inside primitive_type
    let mut inner_iter = pair.clone().into_inner();
    if let Some(ref_type) = inner_iter.next()
        && ref_type.as_rule() == Rule::reference_type
    {
        // Parse reference type - may have a collection type parameter
        let collection = ref_type
            .into_inner()
            .find(|p| p.as_rule() == Rule::ident)
            .map(|p| super::super::parse_ident(p));
        return Ok(FieldType::Reference(collection));
    }

    // For non-reference types, match on the string
    let type_str = pair.as_str().to_lowercase();
    Ok(match type_str.as_str() {
        "any" => FieldType::Any,
        "string" => FieldType::String,
        "int" => FieldType::Int,
        "float" => FieldType::Float,
        "decimal" => FieldType::Decimal,
        "bool" => FieldType::Bool,
        "datetime" => FieldType::Datetime,
        "duration" => FieldType::Duration,
        "bytes" => FieldType::Bytes,
        "array" => FieldType::AnyArray,
        "object" => FieldType::Object {
            fields: vec![],
            mode: SchemaMode::Flexible,
        },
        _ => return Err(ParseError::InvalidType(type_str)),
    })
}

fn parse_field_type_inner(pair: Pair<Rule>) -> Result<FieldType, ParseError> {
    match pair.as_rule() {
        Rule::primitive_type => parse_primitive_type(pair),
        Rule::object_type => parse_object_type(pair),
        Rule::field_type_single => parse_field_type_single(pair),
        _ => Err(ParseError::UnexpectedToken(pair.as_str().to_string())),
    }
}

fn parse_object_type(pair: Pair<Rule>) -> Result<FieldType, ParseError> {
    let mut mode = SchemaMode::Strict; // Default for nested objects
    let mut fields = Vec::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::nested_schema_mode => {
                // Extract schema mode value
                for mode_inner in inner.into_inner() {
                    if mode_inner.as_rule() == Rule::schema_mode_value {
                        mode = if mode_inner.as_str().eq_ignore_ascii_case("FLEXIBLE") {
                            SchemaMode::Flexible
                        } else {
                            SchemaMode::Strict
                        };
                    }
                }
            }
            Rule::nested_field => {
                fields.push(parse_nested_field(inner)?);
            }
            _ => {}
        }
    }

    Ok(FieldType::Object { fields, mode })
}

fn parse_nested_field(pair: Pair<Rule>) -> Result<FieldDef, ParseError> {
    let mut inner = pair.into_inner();
    let name = super::super::parse_ident(inner.next().expect("grammar"));
    let field_type = parse_field_type(inner.next().expect("grammar"))?;

    let mut required = false;
    let mut default = None;

    // Parse optional modifiers
    if let Some(modifiers) = inner.next() {
        for modifier in modifiers.into_inner() {
            match modifier.as_rule() {
                Rule::required_mod => required = true,
                Rule::default_mod => {
                    let value = super::super::semantic_children(modifier)
                        .into_iter()
                        .next()
                        .expect("grammar");
                    default = Some(parse_default_value(value)?);
                }
                _ => {}
            }
        }
    }

    Ok(FieldDef {
        name,
        field_type,
        required,
        default,
    })
}

fn parse_default_value(pair: Pair<Rule>) -> Result<DefaultValue, ParseError> {
    let inner = pair.into_inner().next().expect("grammar");
    match inner.as_rule() {
        Rule::now_fn => Ok(DefaultValue::Now),
        Rule::string => Ok(DefaultValue::String(
            super::super::value::parse_string_literal(inner)?,
        )),
        Rule::number => {
            let s = inner.as_str();
            if s.contains('.') {
                Ok(DefaultValue::Float(
                    s.parse().expect("grammar guarantees valid float"),
                ))
            } else {
                Ok(DefaultValue::Int(
                    s.parse().expect("grammar guarantees valid int"),
                ))
            }
        }
        Rule::boolean => Ok(DefaultValue::Bool(
            inner.as_str().eq_ignore_ascii_case("true"),
        )),
        _ => Err(ParseError::UnexpectedToken(inner.as_str().to_string())),
    }
}

pub(super) fn parse_hnsw_params(pair: Pair<Rule>) -> crate::schema::HnswParams {
    let mut params = crate::schema::HnswParams::default();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::hnsw_dimension => {
                // hnsw_dimension = { kw_DIMENSION ~ integer }
                // kw_DIMENSION is atomic and won't appear as a child
                // We need to find the integer child
                if let Some(int_pair) = inner.into_inner().find(|p| p.as_rule() == Rule::integer) {
                    params.dimension = int_pair.as_str().parse().unwrap_or(0);
                }
            }
            Rule::hnsw_dist => {
                // hnsw_dist = { kw_DIST ~ distance_metric }
                // distance_metric = { kw_COSINE | kw_EUCLIDEAN | kw_DOT }
                if let Some(metric_pair) = inner
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::distance_metric)
                {
                    // The distance_metric rule directly matches one of the keywords
                    // Since they're atomic, we need to check the first child or the string content
                    if let Some(kw) = metric_pair.into_inner().next() {
                        params.metric = match kw.as_rule() {
                            Rule::kw_COSINE => crate::schema::DistanceMetric::Cosine,
                            Rule::kw_EUCLIDEAN => crate::schema::DistanceMetric::Euclidean,
                            Rule::kw_DOT => crate::schema::DistanceMetric::Dot,
                            _ => crate::schema::DistanceMetric::Cosine,
                        };
                    }
                }
            }
            Rule::hnsw_m => {
                // hnsw_m = { kw_M ~ integer }
                if let Some(int_pair) = inner.into_inner().find(|p| p.as_rule() == Rule::integer) {
                    params.m = int_pair.as_str().parse().unwrap_or(16);
                }
            }
            Rule::hnsw_ef => {
                // hnsw_ef = { kw_EF_CONSTRUCTION ~ integer }
                if let Some(int_pair) = inner.into_inner().find(|p| p.as_rule() == Rule::integer) {
                    params.ef_construction = int_pair.as_str().parse().unwrap_or(200);
                }
            }
            _ => {}
        }
    }
    params
}

pub(super) fn parse_index_def(pair: Pair<Rule>) -> Result<IndexDef, ParseError> {
    let mut inner = super::super::semantic_children(pair).into_iter();
    let fields_pair = inner.next().expect("grammar");

    let fields: Vec<String> = fields_pair
        .into_inner()
        .map(super::super::parse_field_path)
        .collect();

    // Check for index options (unique_mod, fulltext_mod, or hnsw_mod)
    let mut unique = false;
    let mut index_type = crate::schema::IndexType::BTree;
    let mut hnsw_params: Option<crate::schema::HnswParams> = None;

    if let Some(opt_pair) = inner.next() {
        match opt_pair.as_rule() {
            Rule::index_options => {
                // index_options contains unique_mod, fulltext_mod, or hnsw_mod
                if let Some(opt) = opt_pair.into_inner().next() {
                    match opt.as_rule() {
                        Rule::unique_mod => unique = true,
                        Rule::fulltext_mod => index_type = crate::schema::IndexType::FullText,
                        Rule::hnsw_mod => {
                            index_type = crate::schema::IndexType::Hnsw;
                            // Find the hnsw_params child within hnsw_mod
                            if let Some(params_pair) =
                                opt.into_inner().find(|p| p.as_rule() == Rule::hnsw_params)
                            {
                                hnsw_params = Some(parse_hnsw_params(params_pair));
                            }
                        }
                        _ => {}
                    }
                }
            }
            Rule::unique_mod => unique = true,
            Rule::fulltext_mod => index_type = crate::schema::IndexType::FullText,
            Rule::hnsw_mod => {
                index_type = crate::schema::IndexType::Hnsw;
                // Find the hnsw_params child within hnsw_mod
                if let Some(params_pair) = opt_pair
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::hnsw_params)
                {
                    hnsw_params = Some(parse_hnsw_params(params_pair));
                }
            }
            _ => {}
        }
    }

    // Auto-generate name for inline index definitions
    Ok(IndexDef {
        name: String::new(), // Will be generated later with collection context
        fields,
        unique,
        index_type,
        hnsw_params,
        analyzer: None,
    })
}
