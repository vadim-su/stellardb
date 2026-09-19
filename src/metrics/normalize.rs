use pest::Parser;
use pest::iterators::Pair;
use xxhash_rust::xxh3::xxh3_64;

use crate::query::parse::{Rule, StellarParser};

const MALFORMED_FINGERPRINT: &[u8] = b"stellardb:malformed-query";
const MAX_SAFE_TEMPLATE_CHARS: usize = 1_000;
pub const QUERY_SANITIZER_VERSION: u16 = 2;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SanitizedQuery {
    pub shape_hash: u64,
    pub safe_template: Option<String>,
    pub display_template: Option<String>,
    pub statement_kind: &'static str,
    pub risk: &'static str,
    pub redaction_count: usize,
    pub sanitizer_version: u16,
}

#[derive(Debug, Eq, PartialEq)]
struct Redaction {
    start: usize,
    end: usize,
    safe_replacement: Option<&'static str>,
    display_replacement: &'static str,
    shape_replacement: &'static str,
    counts_as_value: bool,
}

#[derive(Default)]
struct Classification {
    count: usize,
    first_kind: Option<&'static str>,
    risk: Option<(&'static str, u8)>,
}

impl Classification {
    fn observe(&mut self, rule: Rule, parent: Option<Rule>) {
        if parent != Some(Rule::statement) {
            return;
        }
        let Some((kind, risk, rank)) = statement_classification(rule) else {
            return;
        };
        self.count += 1;
        self.first_kind.get_or_insert(kind);
        if self
            .risk
            .is_none_or(|(_, current_rank)| rank > current_rank)
        {
            self.risk = Some((risk, rank));
        }
    }

    fn kind(&self) -> &'static str {
        match self.count {
            0 => "UNKNOWN",
            1 => self.first_kind.unwrap_or("UNKNOWN"),
            _ => "MULTI",
        }
    }

    fn risk(&self) -> &'static str {
        self.risk.map(|(risk, _)| risk).unwrap_or("unknown")
    }
}

/// Build a replayable query template without retaining literal values.
///
/// Collection and field identifiers are kept so the template remains useful.
/// Strings, data values, record keys, bytes, datetimes, durations and comments
/// are redacted. Execution-shape integers such as LIMIT, OFFSET and KNN tuning
/// parameters are preserved so the template can reproduce the original query.
/// Malformed input is never retained because its token boundaries cannot be
/// trusted.
pub fn normalize_query(sql: &str) -> (u64, Option<String>) {
    let sanitized = sanitize_query(sql);
    (sanitized.shape_hash, sanitized.safe_template)
}

/// Produce separate aggregation, replay and display representations.
pub fn sanitize_query(sql: &str) -> SanitizedQuery {
    let Ok(pairs) = StellarParser::parse(Rule::query, sql) else {
        return malformed_query();
    };

    let mut redactions = comment_redactions(sql);
    let mut classification = Classification::default();
    for pair in pairs {
        collect_redactions(pair, None, &mut redactions, &mut classification);
    }
    redactions.sort_by_key(|item| (item.start, item.end));

    let redaction_count = redactions
        .iter()
        .filter(|item| item.counts_as_value)
        .count();
    let safe = render_redactions(sql, &redactions, RenderMode::Safe);
    if safe.is_empty() {
        return malformed_query();
    }
    let display = render_redactions(sql, &redactions, RenderMode::Display);
    let shape = render_redactions(sql, &redactions, RenderMode::Shape);

    SanitizedQuery {
        shape_hash: xxh3_64(shape.as_bytes()),
        safe_template: Some(truncate_safe_template(&safe)),
        display_template: Some(truncate_safe_template(&display)),
        statement_kind: classification.kind(),
        risk: classification.risk(),
        redaction_count,
        sanitizer_version: QUERY_SANITIZER_VERSION,
    }
}

fn malformed_fingerprint() -> (u64, Option<String>) {
    (xxh3_64(MALFORMED_FINGERPRINT), None)
}

fn malformed_query() -> SanitizedQuery {
    let (shape_hash, _) = malformed_fingerprint();
    SanitizedQuery {
        shape_hash,
        safe_template: None,
        display_template: None,
        statement_kind: "UNPARSED",
        risk: "unknown",
        redaction_count: 0,
        sanitizer_version: QUERY_SANITIZER_VERSION,
    }
}

fn collect_redactions(
    pair: Pair<'_, Rule>,
    parent: Option<Rule>,
    redactions: &mut Vec<Redaction>,
    classification: &mut Classification,
) {
    let rule = pair.as_rule();
    classification.observe(rule, parent);

    let replacement = match rule {
        Rule::string if !matches!(parent, Some(Rule::object_pair | Rule::object_expr_pair)) => {
            Some((Some("'?'"), "‹string›", "'?'"))
        }
        Rule::number => Some((Some("0"), "‹number›", "0")),
        Rule::integer
            if matches!(
                parent,
                Some(Rule::limit_clause | Rule::offset_clause | Rule::knn_op)
            ) =>
        {
            Some((None, "", "1"))
        }
        Rule::integer => Some((Some("1"), "‹integer›", "1")),
        Rule::boolean => Some((Some("false"), "‹boolean›", "false")),
        Rule::datetime_literal => Some((
            Some("d\"1970-01-01T00:00:00Z\""),
            "‹datetime›",
            "d\"1970-01-01T00:00:00Z\"",
        )),
        Rule::duration_literal => Some((Some("0ms"), "‹duration›", "0ms")),
        Rule::bytes_literal => Some((Some("b\"\""), "‹bytes›", "b\"\"")),
        Rule::key => Some((Some("redacted"), "‹record id›", "redacted")),
        _ => None,
    };

    if let Some((safe_replacement, display_replacement, shape_replacement)) = replacement {
        let span = pair.as_span();
        redactions.push(Redaction {
            start: span.start(),
            end: span.end(),
            safe_replacement,
            display_replacement,
            shape_replacement,
            counts_as_value: safe_replacement.is_some(),
        });
        return;
    }

    for child in pair.into_inner() {
        collect_redactions(child, Some(rule), redactions, classification);
    }
}

#[derive(Clone, Copy)]
enum RenderMode {
    Safe,
    Display,
    Shape,
}

fn render_redactions(sql: &str, redactions: &[Redaction], mode: RenderMode) -> String {
    let mut rendered = String::with_capacity(sql.len());
    let mut cursor = 0;
    for redaction in redactions {
        if redaction.start < cursor {
            continue;
        }
        rendered.push_str(&sql[cursor..redaction.start]);
        match mode {
            RenderMode::Safe => rendered.push_str(
                redaction
                    .safe_replacement
                    .unwrap_or(&sql[redaction.start..redaction.end]),
            ),
            RenderMode::Display => {
                if redaction.safe_replacement.is_some() {
                    rendered.push_str(redaction.display_replacement);
                } else {
                    rendered.push_str(&sql[redaction.start..redaction.end]);
                }
            }
            RenderMode::Shape => rendered.push_str(redaction.shape_replacement),
        }
        cursor = redaction.end;
    }
    rendered.push_str(&sql[cursor..]);
    rendered.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn statement_classification(rule: Rule) -> Option<(&'static str, &'static str, u8)> {
    let result = match rule {
        Rule::select_stmt => ("SELECT", "read", 1),
        Rule::explain_stmt => ("EXPLAIN", "read", 1),
        Rule::describe_stmt => ("DESCRIBE", "read", 1),
        Rule::expr_stmt => ("EXPRESSION", "read", 1),
        Rule::let_stmt => ("LET", "session", 2),
        Rule::set_stmt => ("SET", "session", 2),
        Rule::begin_stmt => ("BEGIN", "transaction", 2),
        Rule::commit_stmt => ("COMMIT", "transaction", 2),
        Rule::rollback_stmt => ("ROLLBACK", "transaction", 2),
        Rule::insert_stmt => ("INSERT", "write", 3),
        Rule::create_stmt => ("CREATE", "write", 3),
        Rule::update_stmt => ("UPDATE", "write", 3),
        Rule::upsert_stmt => ("UPSERT", "write", 3),
        Rule::delete_stmt => ("DELETE", "write", 3),
        Rule::relate_stmt => ("RELATE", "write", 3),
        Rule::define_collection_stmt => ("DEFINE COLLECTION", "schema", 4),
        Rule::drop_collection_stmt => ("DROP COLLECTION", "schema", 4),
        Rule::alter_collection_attrs_stmt => ("ALTER COLLECTION", "schema", 4),
        Rule::create_index_stmt => ("CREATE INDEX", "schema", 4),
        Rule::drop_index_stmt => ("DROP INDEX", "schema", 4),
        Rule::reindex_stmt => ("REINDEX", "schema", 4),
        Rule::define_analyzer_stmt => ("DEFINE ANALYZER", "schema", 4),
        Rule::drop_analyzer_stmt => ("DROP ANALYZER", "schema", 4),
        Rule::create_user_stmt => ("CREATE USER", "admin", 5),
        Rule::drop_user_stmt => ("DROP USER", "admin", 5),
        Rule::alter_user_stmt => ("ALTER USER", "admin", 5),
        Rule::create_api_key_stmt => ("CREATE API KEY", "admin", 5),
        Rule::drop_api_key_stmt => ("DROP API KEY", "admin", 5),
        Rule::create_policy_stmt => ("CREATE POLICY", "admin", 5),
        Rule::drop_policy_stmt => ("DROP POLICY", "admin", 5),
        _ => return None,
    };
    Some(result)
}

fn comment_redactions(sql: &str) -> Vec<Redaction> {
    let bytes = sql.as_bytes();
    let mut redactions = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'\'' | b'"' | b'`' => {
                let quote = bytes[index];
                index += 1;
                while index < bytes.len() {
                    if bytes[index] == b'\\' {
                        index = (index + 2).min(bytes.len());
                    } else if bytes[index] == quote {
                        index += 1;
                        break;
                    } else {
                        index += 1;
                    }
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                let start = index;
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                redactions.push(Redaction {
                    start,
                    end: index,
                    safe_replacement: Some(" "),
                    display_replacement: " ",
                    shape_replacement: " ",
                    counts_as_value: false,
                });
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                let start = index;
                let mut depth = 1_u32;
                index += 2;
                while index < bytes.len() && depth > 0 {
                    if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
                        depth += 1;
                        index += 2;
                    } else if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                        depth -= 1;
                        index += 2;
                    } else {
                        index += 1;
                    }
                }
                redactions.push(Redaction {
                    start,
                    end: index,
                    safe_replacement: Some(" "),
                    display_replacement: " ",
                    shape_replacement: " ",
                    counts_as_value: false,
                });
            }
            _ => index += 1,
        }
    }

    redactions
}

fn truncate_safe_template(value: &str) -> String {
    if value.chars().count() <= MAX_SAFE_TEMPLATE_CHARS {
        return value.to_string();
    }
    let keep = MAX_SAFE_TEMPLATE_CHARS.saturating_sub(" /* truncated */".len());
    let mut truncated: String = value.chars().take(keep).collect();
    truncated.push_str(" /* truncated */");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_and_passwords_share_safe_replay_templates() {
        let (first_hash, first_summary) = normalize_query(
            "CREATE USER 'Иван Петров' PASSWORD 'hunter2'; SELECT * FROM users WHERE email = 'иван@example.com' AND age = 42",
        );
        let (second_hash, second_summary) = normalize_query(
            "CREATE USER 'Jane Doe' PASSWORD 'different-secret'; SELECT * FROM users WHERE email = 'jane@example.com' AND age = 9001",
        );

        assert_eq!(first_hash, second_hash);
        assert_eq!(first_summary, second_summary);
        let stored = first_summary.unwrap();
        assert_eq!(
            stored,
            "CREATE USER '?' PASSWORD '?'; SELECT * FROM users WHERE email = '?' AND age = 0"
        );
        for secret in ["Иван", "hunter2", "иван@example.com", "42"] {
            assert!(!stored.contains(secret));
        }
    }

    #[test]
    fn identifiers_keep_query_templates_distinct_and_replayable() {
        let (users_hash, users) = normalize_query("SELECT name FROM users WHERE age > 25 LIMIT 10");
        let (orders_hash, orders) =
            normalize_query("SELECT total FROM orders WHERE amount > 25 LIMIT 10");

        assert_ne!(users_hash, orders_hash);
        assert_eq!(
            users.as_deref(),
            Some("SELECT name FROM users WHERE age > 0 LIMIT 10")
        );
        assert_eq!(
            orders.as_deref(),
            Some("SELECT total FROM orders WHERE amount > 0 LIMIT 10")
        );
    }

    #[test]
    fn execution_shape_numbers_are_preserved() {
        let (limit_100_hash, template) = normalize_query("SELECT *\nFROM users\nLIMIT 100;");
        assert_eq!(template.as_deref(), Some("SELECT * FROM users LIMIT 100;"));

        let (limit_1_hash, other_template) = normalize_query("SELECT * FROM users LIMIT 1;");
        assert_eq!(limit_100_hash, limit_1_hash);
        assert_eq!(
            other_template.as_deref(),
            Some("SELECT * FROM users LIMIT 1;")
        );

        let (_, template_with_redaction) =
            normalize_query("SELECT *\nFROM users\nWHERE age > 42\nLIMIT 100\nOFFSET 20;");

        assert_eq!(
            template_with_redaction.as_deref(),
            Some("SELECT * FROM users WHERE age > 0 LIMIT 100 OFFSET 20;")
        );
    }

    #[test]
    fn typed_display_template_and_risk_are_reported() {
        let sanitized = sanitize_query(
            "UPDATE users:alice SET email = 'alice@example.com', age = 42 WHERE active = true",
        );

        assert_eq!(sanitized.statement_kind, "UPDATE");
        assert_eq!(sanitized.risk, "write");
        assert_eq!(sanitized.redaction_count, 4);
        assert_eq!(sanitized.sanitizer_version, QUERY_SANITIZER_VERSION);
        assert_eq!(
            sanitized.display_template.as_deref(),
            Some(
                "UPDATE users:‹record id› SET email = ‹string›, age = ‹number› WHERE active = ‹boolean›"
            )
        );
        assert_eq!(
            sanitized.safe_template.as_deref(),
            Some("UPDATE users:redacted SET email = '?', age = 0 WHERE active = false")
        );
    }

    #[test]
    fn comments_and_all_literal_types_are_redacted() {
        let query = r#"
            /* token=super-secret */
            SELECT d"2024-01-15T10:30:00Z", 123.45dec, b"U0VDUkVU", user:Мария
            FROM `customers_private`
            WHERE email = "мария@example.com" AND active = true
        "#;
        let (_, summary) = normalize_query(query);
        let summary = summary.expect("query should be grammar-valid");

        assert!(summary.contains("FROM `customers_private`"));
        assert!(summary.contains("user:redacted"));
        for secret in [
            "super-secret",
            "2024-01-15",
            "123.45",
            "U0VDUkVU",
            "Мария",
            "мария@example.com",
            "true",
        ] {
            assert!(!summary.contains(secret));
        }
    }

    #[test]
    fn grammar_valid_ast_invalid_query_gets_safe_template() {
        let query = "SELECT 999999999999999999999999999999999999999999 AS secret_number";
        assert!(crate::query::parse::parse(query).is_err());

        let (hash, summary) = normalize_query(query);
        assert_ne!(hash, xxh3_64(MALFORMED_FINGERPRINT));
        assert_eq!(summary.as_deref(), Some("SELECT 0 AS secret_number"));
        assert!(!summary.unwrap().contains("999999"));
    }

    #[test]
    fn malformed_queries_fail_closed_without_hashing_input() {
        let first = "SELECT * FROM users WHERE password = 'пароль-секрет";
        let second = "not sql at all api_key=sk-live-secret";
        let (first_hash, first_summary) = normalize_query(first);
        let (second_hash, second_summary) = normalize_query(second);

        assert_eq!(first_hash, second_hash);
        assert!(first_summary.is_none());
        assert!(second_summary.is_none());
    }
}
