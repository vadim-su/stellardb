//! Analyzer statement parsing: DEFINE ANALYZER, DROP ANALYZER.

use pest::iterators::Pair;

use super::super::value::Rule;
use crate::query::ast::{DefineAnalyzerAst, DropAnalyzerAst};
use crate::query::error::ParseError;
use crate::schema::{FilterConfig, TokenizerConfig};

/// Parse DEFINE ANALYZER statement.
pub(super) fn parse_define_analyzer(pair: Pair<Rule>) -> Result<DefineAnalyzerAst, ParseError> {
    let mut name = String::new();
    let mut tokenizer = TokenizerConfig::Standard;
    let mut filters = Vec::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::ident => name = super::super::parse_ident(inner),
            Rule::tokenizer_clause => {
                if let Some(spec) = inner
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::tokenizer_spec)
                {
                    tokenizer = parse_tokenizer_spec(spec)?;
                }
            }
            Rule::filters_clause => {
                for filter in inner.into_inner() {
                    if filter.as_rule() == Rule::filter_spec {
                        filters.push(parse_filter_spec(filter)?);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(DefineAnalyzerAst {
        name,
        tokenizer,
        filters,
    })
}

fn parse_tokenizer_spec(pair: Pair<Rule>) -> Result<TokenizerConfig, ParseError> {
    let inner = pair.into_inner().next().expect("grammar");
    match inner.as_rule() {
        Rule::kw_standard => Ok(TokenizerConfig::Standard),
        Rule::kw_whitespace => Ok(TokenizerConfig::Whitespace),
        Rule::ngram_tokenizer => {
            let mut nums = inner.into_inner().filter(|p| p.as_rule() == Rule::integer);
            let min = nums
                .next()
                .expect("grammar")
                .as_str()
                .parse()
                .expect("grammar");
            let max = nums
                .next()
                .expect("grammar")
                .as_str()
                .parse()
                .expect("grammar");
            Ok(TokenizerConfig::Ngram { min, max })
        }
        Rule::pattern_tokenizer => {
            let regex = inner
                .into_inner()
                .find(|p| p.as_rule() == Rule::string)
                .map(super::super::value::parse_string_literal)
                .transpose()?
                .unwrap_or_default();
            Ok(TokenizerConfig::Pattern { regex })
        }
        _ => Ok(TokenizerConfig::Standard),
    }
}

fn parse_filter_spec(pair: Pair<Rule>) -> Result<FilterConfig, ParseError> {
    let inner = pair.into_inner().next().expect("grammar");
    match inner.as_rule() {
        Rule::kw_lowercase => Ok(FilterConfig::Lowercase),
        Rule::kw_ascii_folding => Ok(FilterConfig::AsciiFolding),
        Rule::stemmer_filter => {
            let lang = inner
                .into_inner()
                .find(|p| p.as_rule() == Rule::ident)
                .map(|p| super::super::parse_ident(p))
                .unwrap_or_default();
            Ok(FilterConfig::Stemmer { lang })
        }
        Rule::stopwords_filter => {
            let lang = inner
                .into_inner()
                .find(|p| p.as_rule() == Rule::ident)
                .map(|p| super::super::parse_ident(p))
                .unwrap_or_default();
            Ok(FilterConfig::Stopwords { lang })
        }
        Rule::length_filter => {
            let mut nums = inner.into_inner().filter(|p| p.as_rule() == Rule::integer);
            let min = nums
                .next()
                .expect("grammar")
                .as_str()
                .parse()
                .expect("grammar");
            let max = nums
                .next()
                .expect("grammar")
                .as_str()
                .parse()
                .expect("grammar");
            Ok(FilterConfig::Length { min, max })
        }
        _ => Ok(FilterConfig::Lowercase),
    }
}

/// Parse DROP ANALYZER statement.
pub(super) fn parse_drop_analyzer(pair: Pair<Rule>) -> Result<DropAnalyzerAst, ParseError> {
    let name = pair
        .into_inner()
        .find(|p| p.as_rule() == Rule::ident)
        .map(|p| super::super::parse_ident(p))
        .unwrap_or_default();
    Ok(DropAnalyzerAst { name })
}
