pub mod expr;
pub mod stmt;
pub mod value;

use pest::Parser;
use pest::iterators::Pair;
use pest_derive::Parser;

use crate::query::ast::Statement;
use crate::query::error::ParseError;

#[derive(Parser)]
#[grammar = "query/grammar.pest"]
pub struct StellarParser;

/// Returns true if the rule is a `kw_*` keyword boundary rule.
/// These are atomic rules that exist only to enforce word boundaries
/// and should be skipped when extracting semantic children.
fn is_keyword_rule(rule: Rule) -> bool {
    matches!(
        rule,
        Rule::kw_SELECT
            | Rule::kw_FROM
            | Rule::kw_WHERE
            | Rule::kw_ORDER
            | Rule::kw_LIMIT
            | Rule::kw_OFFSET
            | Rule::kw_GROUP
            | Rule::kw_AS
            | Rule::kw_INSERT
            | Rule::kw_INTO
            | Rule::kw_CREATE
            | Rule::kw_SET
            | Rule::kw_UPDATE
            | Rule::kw_DELETE
            | Rule::kw_BEGIN
            | Rule::kw_COMMIT
            | Rule::kw_ROLLBACK
            | Rule::kw_EXPLAIN
            | Rule::kw_ANALYZE
            | Rule::kw_DEFINE
            | Rule::kw_COLLECTION
            | Rule::kw_COLLECTIONS
            | Rule::kw_SCHEMA
            | Rule::kw_STRICT
            | Rule::kw_FLEXIBLE
            | Rule::kw_REQUIRED
            | Rule::kw_DEFAULT
            | Rule::kw_DROP
            | Rule::kw_CASCADE
            | Rule::kw_DESCRIBE
            | Rule::kw_INDEX
            | Rule::kw_ON
            | Rule::kw_UNIQUE
            | Rule::kw_REINDEX
            | Rule::kw_ASC
            | Rule::kw_DESC
            | Rule::kw_IS
            | Rule::kw_NULL
            | Rule::kw_NONE
            | Rule::kw_IN
            | Rule::kw_AND
            | Rule::kw_OR
            | Rule::kw_NOT
            | Rule::kw_RELATE
            | Rule::kw_CONTENT
            | Rule::kw_RETURN
            | Rule::kw_AFTER
            | Rule::kw_BEFORE
            | Rule::kw_ALL
            | Rule::kw_TIMEOUT
            | Rule::kw_ALTER
            | Rule::kw_USER
            | Rule::kw_PASSWORD
            | Rule::kw_API
            | Rule::kw_KEY
            | Rule::kw_POLICY
            | Rule::kw_WHEN
            | Rule::kw_ALLOW
            | Rule::kw_DENY
            | Rule::kw_REMOVE
            | Rule::kw_ACTION
            | Rule::kw_FOR
    )
}

/// Filter out keyword boundary rules from a pair's children,
/// returning only semantically meaningful children.
pub fn semantic_children(pair: Pair<Rule>) -> Vec<Pair<Rule>> {
    pair.into_inner()
        .filter(|p| !is_keyword_rule(p.as_rule()))
        .collect()
}

/// Parse an identifier, handling quoted (backtick) vs unquoted forms.
/// Quoted identifiers support backslash escaping: \` for backtick, \\ for backslash.
pub fn parse_ident(pair: Pair<Rule>) -> String {
    // ident = { quoted_ident | unquoted_ident }
    let inner = pair
        .into_inner()
        .next()
        .expect("ident rule must have inner");
    match inner.as_rule() {
        Rule::quoted_ident => {
            // Strip surrounding backticks and process escape sequences
            let s = inner.as_str();
            let content = &s[1..s.len() - 1];
            unescape_ident(content)
        }
        Rule::unquoted_ident => inner.as_str().to_string(),
        _ => inner.as_str().to_string(),
    }
}

/// Parse a collection name (wrapper around ident).
pub fn parse_collection(pair: Pair<Rule>) -> String {
    // collection = { ident }
    let ident_pair = pair
        .into_inner()
        .next()
        .expect("collection rule must have ident");
    parse_ident(ident_pair)
}

/// Parse a field path (e.g., "address.city" or "`field.name`.foo").
/// Returns the path as a dot-separated string with escape sequences processed.
pub fn parse_field_path(pair: Pair<Rule>) -> String {
    // field_path = { ident ~ ("." ~ ident)* }
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::ident)
        .map(parse_ident)
        .collect::<Vec<_>>()
        .join(".")
}

/// Parse a field path into a Vec of individual segments.
pub fn parse_field_path_segments(pair: Pair<Rule>) -> Vec<String> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::ident)
        .map(parse_ident)
        .collect()
}

/// Process escape sequences in a quoted identifier.
/// Supported: \` -> `, \\ -> \
fn unescape_ident(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('`') => result.push('`'),
                Some('\\') => result.push('\\'),
                Some(other) => {
                    // Unknown escape - keep as-is (backslash + char)
                    result.push('\\');
                    result.push(other);
                }
                None => {
                    // Trailing backslash - keep it
                    result.push('\\');
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

pub fn parse(input: &str) -> Result<Vec<Statement>, ParseError> {
    let pairs = StellarParser::parse(Rule::query, input)?;

    for pair in pairs {
        if pair.as_rule() == Rule::statement_list {
            return stmt::parse_statement_list(pair);
        }
    }

    unreachable!("grammar guarantees statement_list")
}

/// Check if input is valid according to grammar (without full AST conversion)
pub fn grammar_check(input: &str) -> Result<(), ParseError> {
    StellarParser::parse(Rule::query, input)?;
    Ok(())
}

#[cfg(test)]
mod grammar_tests {
    use super::*;

    #[test]
    fn test_grammar_define_collection_with_schema() {
        let sql = r#"
            DEFINE COLLECTION user (
                SCHEMA STRICT,
                name string REQUIRED,
                email string,
                age int DEFAULT 0,
                address {
                    street string REQUIRED,
                    city string REQUIRED
                },
                tags [string],
                INDEX ON email UNIQUE,
                INDEX ON (name, age)
            )
        "#;
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_define_collection_flexible() {
        let sql = "DEFINE COLLECTION event (SCHEMA FLEXIBLE, timestamp datetime REQUIRED)";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_define_collection_minimal() {
        let sql = "DEFINE COLLECTION log (SCHEMA FLEXIBLE)";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_define_collection_no_body() {
        let sql = "DEFINE COLLECTION temp";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_drop_collection() {
        let sql = "DROP COLLECTION temp";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_drop_collection_cascade() {
        let sql = "DROP COLLECTION temp CASCADE";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_describe_collection() {
        let sql = "DESCRIBE COLLECTION user";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_describe_collections() {
        let sql = "DESCRIBE COLLECTIONS";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_ddl_case_insensitive() {
        let sql = "define collection Test (schema flexible)";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_define_with_now_default() {
        let sql = "DEFINE COLLECTION event (timestamp datetime DEFAULT now())";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_define_with_nested_array() {
        let sql = "DEFINE COLLECTION post (comments [{author string, text string REQUIRED}])";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_explain() {
        let sql = "EXPLAIN SELECT * FROM users";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_explain_analyze() {
        let sql = "EXPLAIN ANALYZE SELECT * FROM users WHERE age > 18";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_explain_update() {
        let sql = "EXPLAIN UPDATE users SET status = 'active' WHERE age > 18";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_explain_requires_space() {
        // "explainSELECT" must not parse as EXPLAIN + SELECT
        assert!(grammar_check("explainSELECT * FROM users").is_err());
        assert!(grammar_check("EXPLAINSELECT * FROM users").is_err());
        // "ANALYZESELECT" must not parse as ANALYZE + SELECT
        assert!(grammar_check("EXPLAIN ANALYZESELECT * FROM users").is_err());
    }

    #[test]
    fn test_grammar_keyword_boundaries() {
        // All keyword fusions must fail to parse
        let bad_queries = [
            "SELECTFROM users",
            "SELECT * FROMUSERS",
            "INSERTINTO users {name: 'a'}",
            "INSERT INTOusers {name: 'a'}",
            "DELETEFROM users",
            "DELETE users WHEREname = 'a'",
            "UPDATEusers SET x = 1",
            "UPDATE users SETx = 1",
            "DEFINECOLLECTION foo",
            "DEFINE COLLECTIONfoo",
            "DROPCOLLECTION foo",
            "DROP COLLECTIONfoo",
            "DESCRIBECOLLECTION foo",
            "DESCRIBE COLLECTIONfoo",
            "CREATEINDEX ON foo(bar)",
            "CREATE INDEXON foo(bar)",
            "CREATE INDEX ONfoo(bar)",
            "DROPINDEX ON foo(bar)",
            "SELECT * FROM users WHEREx = 1",
            "SELECT * FROM users ORDERname",
            "SELECT * FROM users LIMIT10",
            "SELECT * FROM users WHERE x = 1 ANDy = 2",
            "SELECT * FROM users WHERE x = 1 ORy = 2",
            // Note: "WHERE NOTx = 1" is valid because NOTx is a valid identifier
            // and NOT is optional in not_expr, so it parses as field NOTx = 1.
        ];
        for q in &bad_queries {
            assert!(
                grammar_check(q).is_err(),
                "Expected parse failure for fused keywords: {:?}",
                q
            );
        }
    }

    #[test]
    fn test_grammar_error_shows_clean_keyword_names() {
        // Verify error messages show "FROM" not "kw_FROM"
        let err = grammar_check("SELECT * FROMUSERS").unwrap_err();
        let msg = err.to_string();
        assert!(
            !msg.contains("kw_"),
            "Error message should not contain 'kw_' prefix: {}",
            msg
        );
        assert!(
            msg.contains("FROM"),
            "Error message should mention FROM: {}",
            msg
        );
    }

    #[test]
    fn test_grammar_explain_delete() {
        let sql = "EXPLAIN DELETE users WHERE status = 'inactive'";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_object_expr_in_select() {
        let sql = "SELECT {'name': name, 'age': age} as info FROM users";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_array_expr_in_select() {
        let sql = "SELECT [name, age, 'literal'] as arr FROM users";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_empty_object_expr() {
        let sql = "SELECT {} as empty FROM users";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_empty_array_expr() {
        let sql = "SELECT [] as empty FROM users";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_nested_object_array_expr() {
        let sql = "SELECT {'items': [1, 2, name], 'meta': {'x': age}} as data FROM users";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_object_with_expression_values() {
        let sql = "SELECT {'total': price * qty, 'half': price / 2} as calc FROM orders";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_array_source() {
        let sql = "SELECT 1 FROM [0, 1]";
        assert!(
            grammar_check(sql).is_ok(),
            "Failed to parse: {:?}",
            grammar_check(sql)
        );
    }

    #[test]
    fn test_grammar_value_ref() {
        grammar_check("SELECT $value FROM [1, 2]").unwrap();
        grammar_check("SELECT $value.name FROM [{name: 'A'}]").unwrap();
        // Note: "AS alias" syntax after array is no longer supported in simplified grammar
    }

    #[test]
    fn test_grammar_array_index() {
        grammar_check("SELECT arr[0] FROM t").unwrap();
        grammar_check("SELECT arr[-1] FROM t").unwrap();
        grammar_check("SELECT [1,2,3][0] FROM t").unwrap();
    }

    #[test]
    fn test_grammar_array_slice() {
        grammar_check("SELECT arr[0..2] FROM t").unwrap();
        grammar_check("SELECT arr[1..] FROM t").unwrap();
        grammar_check("SELECT arr[..2] FROM t").unwrap();
        grammar_check("SELECT arr[..] FROM t").unwrap();
        grammar_check("SELECT arr[-2..-1] FROM t").unwrap();
    }

    #[test]
    fn test_grammar_postfix_chain() {
        grammar_check("SELECT matrix[0][1] FROM t").unwrap();
        grammar_check("SELECT users[0].name FROM t").unwrap();
        grammar_check("SELECT data[0..2][0] FROM t").unwrap();
        grammar_check("SELECT (SELECT * FROM users)[0] FROM t").unwrap();
        grammar_check("SELECT (SELECT * FROM users)[0].name FROM t").unwrap();
    }

    #[test]
    fn test_grammar_offset() {
        grammar_check("SELECT * FROM t OFFSET 2").unwrap();
        grammar_check("SELECT * FROM t LIMIT 5 OFFSET 2").unwrap();
    }

    #[test]
    fn test_grammar_relate_basic() {
        grammar_check("RELATE user:alice->follows->user:bob").unwrap();
    }

    #[test]
    fn test_grammar_relate_with_set() {
        grammar_check("RELATE user:alice->follows->user:bob SET since = 123").unwrap();
    }

    #[test]
    fn test_grammar_relate_with_content() {
        grammar_check("RELATE user:alice->follows->user:bob CONTENT {since: 123}").unwrap();
    }

    #[test]
    fn test_grammar_relate_with_return() {
        grammar_check("RELATE user:alice->follows->user:bob RETURN AFTER").unwrap();
        grammar_check("RELATE user:alice->follows->user:bob RETURN BEFORE").unwrap();
        grammar_check("RELATE user:alice->follows->user:bob RETURN NONE").unwrap();
    }

    #[test]
    fn test_grammar_relate_array() {
        grammar_check("RELATE user:alice->follows->[user:bob, user:carol]").unwrap();
    }

    #[test]
    fn test_grammar_relate_subquery() {
        grammar_check("RELATE user:alice->follows->(SELECT id FROM user WHERE active = true)")
            .unwrap();
    }

    #[test]
    fn test_grammar_traversal_basic() {
        grammar_check("SELECT ->follows FROM user:alice").unwrap();
        grammar_check("SELECT <-follows FROM user:bob").unwrap();
        grammar_check("SELECT <->friends FROM user:alice").unwrap();
    }

    #[test]
    fn test_grammar_traversal_with_target() {
        grammar_check("SELECT ->follows->user FROM user:alice").unwrap();
        grammar_check("SELECT ->follows->user.* FROM user:alice").unwrap();
        grammar_check("SELECT ->follows->user.name FROM user:alice").unwrap();
    }

    #[test]
    fn test_grammar_traversal_depth() {
        grammar_check("SELECT ->follows{2}->user FROM user:alice").unwrap();
        grammar_check("SELECT ->follows{1..5}->user FROM user:alice").unwrap();
        grammar_check("SELECT ->follows{..}->user FROM user:alice").unwrap();
        grammar_check("SELECT ->follows{1..}->user FROM user:alice").unwrap();
        grammar_check("SELECT ->follows{..5}->user FROM user:alice").unwrap();
    }

    #[test]
    fn test_grammar_traversal_filter() {
        grammar_check("SELECT ->(follows WHERE since > 2024)->user FROM user:alice").unwrap();
    }

    #[test]
    fn test_grammar_traversal_wildcard() {
        grammar_check("SELECT ->*->user FROM user:alice").unwrap();
        grammar_check("SELECT ->*{1..3}->user FROM user:alice").unwrap();
    }

    #[test]
    fn test_grammar_traversal_all_mode() {
        grammar_check("SELECT ->follows{1..5 ALL}->user FROM user:alice").unwrap();
    }

    #[test]
    fn test_grammar_datetime_literal() {
        grammar_check(r#"SELECT d"2024-01-15" as dt"#).unwrap();
        grammar_check(r#"SELECT d"2024-01-15T10:30:00Z" as dt"#).unwrap();
        grammar_check(r#"SELECT d'2024-01-15' as dt"#).unwrap();
    }

    #[test]
    fn test_grammar_duration_literal() {
        grammar_check("SELECT 1h as dur").unwrap();
        grammar_check("SELECT 30m as dur").unwrap();
        grammar_check("SELECT 1h30m as dur").unwrap();
        grammar_check("SELECT 500ms as dur").unwrap();
        grammar_check("SELECT 1d2h3m4s as dur").unwrap();
        grammar_check("SELECT 1ns as dur").unwrap();
        grammar_check("SELECT 1us as dur").unwrap();
        grammar_check("SELECT 1w as dur").unwrap();
        grammar_check("SELECT 1y as dur").unwrap();
    }

    #[test]
    fn test_grammar_bytes_literal() {
        grammar_check(r#"SELECT b"SGVsbG8=" as bytes"#).unwrap();
        grammar_check(r#"SELECT b"" as bytes"#).unwrap();
        grammar_check("SELECT 0x48656c6c6f as bytes").unwrap();
        grammar_check("SELECT 0xFF as bytes").unwrap();
        grammar_check("SELECT 0xDEADBEEF as bytes").unwrap();
    }

    #[test]
    fn test_grammar_range_literal() {
        grammar_check("SELECT * FROM 1..10").unwrap();
        grammar_check("SELECT * FROM 0..100").unwrap();
        grammar_check("SELECT * FROM $start..$end").unwrap();
    }
}
