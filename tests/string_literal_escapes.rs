use stellardb::{EmbeddedDatabase, Value};

fn string_field(db: &EmbeddedDatabase, collection: &str, id: &str, field: &str) -> String {
    let document = db
        .get_document(collection, id)
        .unwrap()
        .unwrap_or_else(|| panic!("missing document {collection}:{id}"));

    match document.fields.get(field) {
        Some(Value::String(value)) => value.clone(),
        other => panic!("expected string field {field}, got {other:?}"),
    }
}

#[test]
fn insert_accepts_json_escaped_strings_and_keys() {
    let dir = tempfile::tempdir().unwrap();
    let db = EmbeddedDatabase::open(dir.path()).unwrap();
    db.execute("DEFINE COLLECTION quote_test").unwrap();

    // This shape is intentionally produced exactly as a JSON serializer would
    // produce it for the SciFact corpus loader.
    let text = "This leaves behind a firmly adherent \"stroma.\" At days 4–6...";
    let json_text = serde_json::to_string(text).unwrap();
    let query = format!(r#"INSERT INTO quote_test {{"id": "scifact", "text": {json_text}}}"#);
    db.execute(&query).unwrap();
    assert_eq!(string_field(&db, "quote_test", "scifact", "text"), text);

    db.execute(
        r#"INSERT INTO quote_test {
            "id": "escapes",
            "escaped\"key": "quote=\" slash=\/ backslash=\\ backspace=\b formfeed=\f newline=\n carriage=\r tab=\t",
            "unicode": "Unicode: \u041f\u0440\u0438\u0432\u0435\u0442 \uD83D\uDE80"
        }"#,
    )
    .unwrap();

    assert_eq!(
        string_field(&db, "quote_test", "escapes", "escaped\"key"),
        "quote=\" slash=/ backslash=\\ backspace=\u{8} formfeed=\u{c} newline=\n carriage=\r tab=\t"
    );
    assert_eq!(
        string_field(&db, "quote_test", "escapes", "unicode"),
        "Unicode: Привет 🚀"
    );
}

#[test]
fn single_quoted_strings_allow_escaped_quote_and_backslash() {
    let dir = tempfile::tempdir().unwrap();
    let db = EmbeddedDatabase::open(dir.path()).unwrap();
    db.execute("DEFINE COLLECTION quote_test").unwrap();

    db.execute(r#"INSERT INTO quote_test {'id': 'single', 'text': 'It\'s working: C:\\tmp'}"#)
        .unwrap();

    assert_eq!(
        string_field(&db, "quote_test", "single", "text"),
        "It's working: C:\\tmp"
    );
}

#[test]
fn create_and_update_decode_escaped_expression_literals() {
    let dir = tempfile::tempdir().unwrap();
    let db = EmbeddedDatabase::open(dir.path()).unwrap();
    db.execute("DEFINE COLLECTION quote_test").unwrap();

    db.execute(r#"CREATE quote_test:expressions SET text = "He said: \"hello\"", path = "C:\\Users\\alice""#)
        .unwrap();
    assert_eq!(
        string_field(&db, "quote_test", "expressions", "text"),
        "He said: \"hello\""
    );
    assert_eq!(
        string_field(&db, "quote_test", "expressions", "path"),
        "C:\\Users\\alice"
    );

    db.execute(
        r#"UPDATE quote_test:expressions SET text = "first\nsecond: \u041f\u0440\u0438\u0432\u0435\u0442""#,
    )
    .unwrap();
    assert_eq!(
        string_field(&db, "quote_test", "expressions", "text"),
        "first\nsecond: Привет"
    );
}

#[test]
fn relate_content_decodes_escaped_object_strings() {
    let dir = tempfile::tempdir().unwrap();
    let db = EmbeddedDatabase::open(dir.path()).unwrap();
    db.execute("DEFINE COLLECTION user").unwrap();
    db.execute("CREATE user:alice SET name = 'Alice'").unwrap();
    db.execute("CREATE user:bob SET name = 'Bob'").unwrap();

    let rows = db
        .execute(
            r#"RELATE user:alice->follows->user:bob CONTENT {note: "Alice said \"hello\"", localized: "\u041f\u0440\u0438\u0432\u0435\u0442"}"#,
        )
        .unwrap();
    let edge = rows
        .results
        .first()
        .and_then(|result| result.rows.first())
        .expect("RELATE must return its edge");

    assert_eq!(
        edge.get_field("note"),
        Some(Value::String("Alice said \"hello\"".to_string()))
    );
    assert_eq!(
        edge.get_field("localized"),
        Some(Value::String("Привет".to_string()))
    );
}

#[test]
fn malformed_unicode_surrogates_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let db = EmbeddedDatabase::open(dir.path()).unwrap();
    db.execute("DEFINE COLLECTION quote_test").unwrap();

    let lone_high =
        db.execute(r#"INSERT INTO quote_test {"id": "high", "text": "\uD83Dnot-a-low-surrogate"}"#);
    assert!(lone_high.is_err(), "a lone high surrogate must be rejected");

    let lone_low = db.execute(r#"INSERT INTO quote_test {"id": "low", "text": "\uDE80"}"#);
    assert!(lone_low.is_err(), "a lone low surrogate must be rejected");
}
