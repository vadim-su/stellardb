//! Property-based fuzz tests for the SQL parser.
//!
//! These tests verify that `parse::parse()` never panics on arbitrary input.
//! Both Ok and Err results are valid — only panics indicate bugs.
//!
//! Run: `cargo test --test fuzz_parser`

use arbitrary::{Arbitrary, Unstructured};

// ---------------------------------------------------------------------------
// Core property: the parser must not panic on any input
// ---------------------------------------------------------------------------

#[test]
fn fuzz_parse_random_bytes() {
    arbtest::arbtest(|u| {
        let bytes: &[u8] = u.arbitrary()?;
        if let Ok(input) = std::str::from_utf8(bytes) {
            let _ = stellardb::query::parse::parse(input);
        }
        Ok(())
    });
}

#[test]
fn fuzz_parse_random_utf8() {
    arbtest::arbtest(|u| {
        let input: String = u.arbitrary()?;
        let _ = stellardb::query::parse::parse(&input);
        Ok(())
    });
}

// ---------------------------------------------------------------------------
// Structured SQL generation for smarter coverage
// ---------------------------------------------------------------------------

#[derive(Debug, Arbitrary)]
enum FuzzStatement {
    Select(FuzzSelect),
    Insert(FuzzInsert),
    Create(FuzzCreate),
    Update(FuzzUpdate),
    Delete(FuzzDelete),
    Define(FuzzDefine),
    Drop(FuzzDrop),
    Describe(FuzzDescribe),
    Explain(FuzzExplain),
    Relate(FuzzRelate),
    Transaction(FuzzTransaction),
    Let(FuzzLet),
}

#[derive(Debug, Arbitrary)]
struct FuzzSelect {
    projections: FuzzProjections,
    source: FuzzSource,
    where_clause: Option<FuzzExpr>,
    order: Option<FuzzIdent>,
    limit: Option<u16>,
    offset: Option<u16>,
    group_by: Option<FuzzIdent>,
}

#[derive(Debug, Arbitrary)]
enum FuzzProjections {
    Star,
    Fields(FuzzIdent, Option<FuzzIdent>, Option<FuzzIdent>),
    Aggregate(FuzzAggFunc, FuzzIdent),
    Traversal(FuzzTraversal),
}

#[derive(Debug, Arbitrary)]
enum FuzzAggFunc {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

#[derive(Debug, Arbitrary)]
struct FuzzTraversal {
    direction: FuzzDirection,
    label: FuzzIdent,
    depth: Option<FuzzDepth>,
    target: Option<FuzzIdent>,
    field: Option<FuzzTraversalField>,
}

#[derive(Debug, Arbitrary)]
enum FuzzDirection {
    Out,
    In,
    Both,
}

#[derive(Debug, Arbitrary)]
enum FuzzDepth {
    Exact(u8),
    Range(u8, u8),
    Unbounded,
}

#[derive(Debug, Arbitrary)]
enum FuzzTraversalField {
    Star,
    Named(FuzzIdent),
}

#[derive(Debug, Arbitrary)]
enum FuzzSource {
    Collection(FuzzIdent),
    Record(FuzzIdent, FuzzIdent),
    ArraySource,
    Range(u16, u16),
}

#[derive(Debug, Arbitrary)]
struct FuzzInsert {
    collection: FuzzIdent,
    fields: Vec<(FuzzIdent, FuzzValue)>,
}

#[derive(Debug, Arbitrary)]
struct FuzzCreate {
    collection: FuzzIdent,
    key: FuzzIdent,
    fields: Vec<(FuzzIdent, FuzzValue)>,
}

#[derive(Debug, Arbitrary)]
struct FuzzUpdate {
    target: FuzzTarget,
    fields: Vec<(FuzzIdent, FuzzValue)>,
    where_clause: Option<FuzzExpr>,
}

#[derive(Debug, Arbitrary)]
struct FuzzDelete {
    target: FuzzTarget,
    where_clause: Option<FuzzExpr>,
}

#[derive(Debug, Arbitrary)]
enum FuzzTarget {
    Collection(FuzzIdent),
    Record(FuzzIdent, FuzzIdent),
}

#[derive(Debug, Arbitrary)]
struct FuzzDefine {
    collection: FuzzIdent,
    schema_mode: Option<FuzzSchemaMode>,
    fields: Vec<FuzzFieldDef>,
}

#[derive(Debug, Arbitrary)]
enum FuzzSchemaMode {
    Strict,
    Flexible,
}

#[derive(Debug, Arbitrary)]
struct FuzzFieldDef {
    name: FuzzIdent,
    ty: FuzzType,
    required: bool,
}

#[derive(Debug, Arbitrary)]
enum FuzzType {
    String,
    Int,
    Float,
    Bool,
    Datetime,
    Bytes,
}

#[derive(Debug, Arbitrary)]
struct FuzzDrop {
    collection: FuzzIdent,
    cascade: bool,
}

#[derive(Debug, Arbitrary)]
enum FuzzDescribe {
    Collection(FuzzIdent),
    Collections,
}

#[derive(Debug, Arbitrary)]
struct FuzzExplain {
    analyze: bool,
    select: FuzzSelect,
}

#[derive(Debug, Arbitrary)]
struct FuzzRelate {
    from_col: FuzzIdent,
    from_key: FuzzIdent,
    label: FuzzIdent,
    to_col: FuzzIdent,
    to_key: FuzzIdent,
    fields: Vec<(FuzzIdent, FuzzValue)>,
    return_mode: Option<FuzzReturnMode>,
}

#[derive(Debug, Arbitrary)]
enum FuzzReturnMode {
    After,
    Before,
    None,
}

#[derive(Debug, Arbitrary)]
enum FuzzTransaction {
    Begin,
    Commit,
    Rollback,
}

#[derive(Debug, Arbitrary)]
struct FuzzLet {
    name: FuzzIdent,
    value: FuzzValue,
}

#[derive(Debug, Arbitrary)]
enum FuzzExpr {
    Eq(FuzzIdent, FuzzValue),
    Neq(FuzzIdent, FuzzValue),
    Gt(FuzzIdent, FuzzValue),
    Lt(FuzzIdent, FuzzValue),
    Gte(FuzzIdent, FuzzValue),
    Lte(FuzzIdent, FuzzValue),
    IsNull(FuzzIdent),
    IsNotNull(FuzzIdent),
    And(Box<FuzzExpr>, Box<FuzzExpr>),
    Or(Box<FuzzExpr>, Box<FuzzExpr>),
    Fts(FuzzIdent, String),
}

#[derive(Debug, Arbitrary)]
enum FuzzValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<FuzzSimpleValue>),
}

#[derive(Debug, Arbitrary)]
enum FuzzSimpleValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

#[derive(Debug)]
struct FuzzIdent(String);

const IDENTS: &[&str] = &[
    "user",
    "name",
    "age",
    "email",
    "id",
    "status",
    "score",
    "price",
    "title",
    "body",
    "tags",
    "vec",
    "data",
    "items",
    "orders",
    "product",
    "follows",
    "knows",
    "likes",
    "friends",
    "alice",
    "bob",
    "carol",
    "active",
    "category",
    "created_at",
    "count",
    "value",
];

impl<'a> Arbitrary<'a> for FuzzIdent {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let idx: usize = u.arbitrary()?;
        Ok(FuzzIdent(IDENTS[idx % IDENTS.len()].to_string()))
    }
}

// ---------------------------------------------------------------------------
// Rendering to SQL strings
// ---------------------------------------------------------------------------

impl FuzzStatement {
    fn to_sql(&self) -> String {
        match self {
            FuzzStatement::Select(s) => s.to_sql(),
            FuzzStatement::Insert(s) => s.to_sql(),
            FuzzStatement::Create(s) => s.to_sql(),
            FuzzStatement::Update(s) => s.to_sql(),
            FuzzStatement::Delete(s) => s.to_sql(),
            FuzzStatement::Define(s) => s.to_sql(),
            FuzzStatement::Drop(s) => s.to_sql(),
            FuzzStatement::Describe(s) => s.to_sql(),
            FuzzStatement::Explain(s) => s.to_sql(),
            FuzzStatement::Relate(s) => s.to_sql(),
            FuzzStatement::Transaction(s) => s.to_sql(),
            FuzzStatement::Let(s) => s.to_sql(),
        }
    }
}

impl FuzzSelect {
    fn to_sql(&self) -> String {
        let mut sql = String::from("SELECT ");
        sql.push_str(&self.projections.to_sql());
        sql.push_str(" FROM ");
        sql.push_str(&self.source.to_sql());
        if let Some(ref w) = self.where_clause {
            sql.push_str(" WHERE ");
            sql.push_str(&w.to_sql());
        }
        if let Some(ref g) = self.group_by {
            sql.push_str(&format!(" GROUP BY {}", g.0));
        }
        if let Some(ref o) = self.order {
            sql.push_str(&format!(" ORDER BY {}", o.0));
        }
        if let Some(l) = self.limit {
            sql.push_str(&format!(" LIMIT {}", l % 1000));
        }
        if let Some(o) = self.offset {
            sql.push_str(&format!(" OFFSET {}", o % 1000));
        }
        sql
    }
}

impl FuzzProjections {
    fn to_sql(&self) -> String {
        match self {
            FuzzProjections::Star => "*".to_string(),
            FuzzProjections::Fields(a, b, c) => {
                let mut fields = vec![a.0.clone()];
                if let Some(b) = b {
                    fields.push(b.0.clone());
                }
                if let Some(c) = c {
                    fields.push(c.0.clone());
                }
                fields.join(", ")
            }
            FuzzProjections::Aggregate(func, field) => {
                let f = match func {
                    FuzzAggFunc::Count => "COUNT",
                    FuzzAggFunc::Sum => "SUM",
                    FuzzAggFunc::Avg => "AVG",
                    FuzzAggFunc::Min => "MIN",
                    FuzzAggFunc::Max => "MAX",
                };
                format!("{}({})", f, field.0)
            }
            FuzzProjections::Traversal(t) => t.to_sql(),
        }
    }
}

impl FuzzTraversal {
    fn to_sql(&self) -> String {
        let arrow = match self.direction {
            FuzzDirection::Out => "->",
            FuzzDirection::In => "<-",
            FuzzDirection::Both => "<->",
        };
        let mut sql = format!("{}{}", arrow, self.label.0);
        if let Some(ref depth) = self.depth {
            match depth {
                FuzzDepth::Exact(n) => sql.push_str(&format!("{{{}}}", n % 10)),
                FuzzDepth::Range(a, b) => sql.push_str(&format!("{{{}..{}}}", a % 10, b % 10)),
                FuzzDepth::Unbounded => sql.push_str("{..}"),
            }
        }
        if let Some(ref target) = self.target {
            sql.push_str(&format!("->{}", target.0));
            if let Some(ref field) = self.field {
                match field {
                    FuzzTraversalField::Star => sql.push_str(".*"),
                    FuzzTraversalField::Named(f) => sql.push_str(&format!(".{}", f.0)),
                }
            }
        }
        sql
    }
}

impl FuzzSource {
    fn to_sql(&self) -> String {
        match self {
            FuzzSource::Collection(c) => c.0.clone(),
            FuzzSource::Record(c, k) => format!("{}:{}", c.0, k.0),
            FuzzSource::ArraySource => "[1, 2, 3]".to_string(),
            FuzzSource::Range(a, b) => format!("{}..{}", a, b),
        }
    }
}

impl FuzzInsert {
    fn to_sql(&self) -> String {
        let fields = if self.fields.is_empty() {
            "name: 'test'".to_string()
        } else {
            self.fields
                .iter()
                .take(5)
                .map(|(k, v)| format!("{}: {}", k.0, v.to_sql()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!("INSERT INTO {} {{{}}}", self.collection.0, fields)
    }
}

impl FuzzCreate {
    fn to_sql(&self) -> String {
        let fields = if self.fields.is_empty() {
            "name = 'test'".to_string()
        } else {
            self.fields
                .iter()
                .take(5)
                .map(|(k, v)| format!("{} = {}", k.0, v.to_sql()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!("CREATE {}:{} SET {}", self.collection.0, self.key.0, fields)
    }
}

impl FuzzUpdate {
    fn to_sql(&self) -> String {
        let fields = if self.fields.is_empty() {
            "name = 'test'".to_string()
        } else {
            self.fields
                .iter()
                .take(5)
                .map(|(k, v)| format!("{} = {}", k.0, v.to_sql()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut sql = format!("UPDATE {} SET {}", self.target.to_sql(), fields);
        if let Some(ref w) = self.where_clause {
            sql.push_str(" WHERE ");
            sql.push_str(&w.to_sql());
        }
        sql
    }
}

impl FuzzDelete {
    fn to_sql(&self) -> String {
        let mut sql = format!("DELETE {}", self.target.to_sql());
        if let Some(ref w) = self.where_clause {
            sql.push_str(" WHERE ");
            sql.push_str(&w.to_sql());
        }
        sql
    }
}

impl FuzzTarget {
    fn to_sql(&self) -> String {
        match self {
            FuzzTarget::Collection(c) => c.0.clone(),
            FuzzTarget::Record(c, k) => format!("{}:{}", c.0, k.0),
        }
    }
}

impl FuzzDefine {
    fn to_sql(&self) -> String {
        let mut parts = Vec::new();
        if let Some(ref mode) = self.schema_mode {
            match mode {
                FuzzSchemaMode::Strict => parts.push("SCHEMA STRICT".to_string()),
                FuzzSchemaMode::Flexible => parts.push("SCHEMA FLEXIBLE".to_string()),
            }
        }
        for f in self.fields.iter().take(5) {
            let ty = match f.ty {
                FuzzType::String => "string",
                FuzzType::Int => "int",
                FuzzType::Float => "float",
                FuzzType::Bool => "bool",
                FuzzType::Datetime => "datetime",
                FuzzType::Bytes => "bytes",
            };
            let mut def = format!("{} {}", f.name.0, ty);
            if f.required {
                def.push_str(" REQUIRED");
            }
            parts.push(def);
        }
        if parts.is_empty() {
            format!("DEFINE COLLECTION {}", self.collection.0)
        } else {
            format!(
                "DEFINE COLLECTION {} ({})",
                self.collection.0,
                parts.join(", ")
            )
        }
    }
}

impl FuzzDrop {
    fn to_sql(&self) -> String {
        let mut sql = format!("DROP COLLECTION {}", self.collection.0);
        if self.cascade {
            sql.push_str(" CASCADE");
        }
        sql
    }
}

impl FuzzDescribe {
    fn to_sql(&self) -> String {
        match self {
            FuzzDescribe::Collection(c) => format!("DESCRIBE COLLECTION {}", c.0),
            FuzzDescribe::Collections => "DESCRIBE COLLECTIONS".to_string(),
        }
    }
}

impl FuzzExplain {
    fn to_sql(&self) -> String {
        let prefix = if self.analyze {
            "EXPLAIN ANALYZE "
        } else {
            "EXPLAIN "
        };
        format!("{}{}", prefix, self.select.to_sql())
    }
}

impl FuzzRelate {
    fn to_sql(&self) -> String {
        let mut sql = format!(
            "RELATE {}:{}->{}->{}:{}",
            self.from_col.0, self.from_key.0, self.label.0, self.to_col.0, self.to_key.0
        );
        if !self.fields.is_empty() {
            let fields = self
                .fields
                .iter()
                .take(5)
                .map(|(k, v)| format!("{} = {}", k.0, v.to_sql()))
                .collect::<Vec<_>>()
                .join(", ");
            sql.push_str(&format!(" SET {}", fields));
        }
        if let Some(ref mode) = self.return_mode {
            let m = match mode {
                FuzzReturnMode::After => "AFTER",
                FuzzReturnMode::Before => "BEFORE",
                FuzzReturnMode::None => "NONE",
            };
            sql.push_str(&format!(" RETURN {}", m));
        }
        sql
    }
}

impl FuzzTransaction {
    fn to_sql(&self) -> String {
        match self {
            FuzzTransaction::Begin => "BEGIN".to_string(),
            FuzzTransaction::Commit => "COMMIT".to_string(),
            FuzzTransaction::Rollback => "ROLLBACK".to_string(),
        }
    }
}

impl FuzzLet {
    fn to_sql(&self) -> String {
        format!("LET {} = {}", self.name.0, self.value.to_sql())
    }
}

impl FuzzExpr {
    fn to_sql(&self) -> String {
        match self {
            FuzzExpr::Eq(f, v) => format!("{} = {}", f.0, v.to_sql()),
            FuzzExpr::Neq(f, v) => format!("{} != {}", f.0, v.to_sql()),
            FuzzExpr::Gt(f, v) => format!("{} > {}", f.0, v.to_sql()),
            FuzzExpr::Lt(f, v) => format!("{} < {}", f.0, v.to_sql()),
            FuzzExpr::Gte(f, v) => format!("{} >= {}", f.0, v.to_sql()),
            FuzzExpr::Lte(f, v) => format!("{} <= {}", f.0, v.to_sql()),
            FuzzExpr::IsNull(f) => format!("{} IS NULL", f.0),
            FuzzExpr::IsNotNull(f) => format!("{} IS NOT NULL", f.0),
            FuzzExpr::And(a, b) => format!("({} AND {})", a.to_sql(), b.to_sql()),
            FuzzExpr::Or(a, b) => format!("({} OR {})", a.to_sql(), b.to_sql()),
            FuzzExpr::Fts(f, q) => {
                let escaped = q.replace('\\', "\\\\").replace('"', "\\\"");
                format!("{} @@ \"{}\"", f.0, escaped)
            }
        }
    }
}

impl FuzzValue {
    fn to_sql(&self) -> String {
        match self {
            FuzzValue::Null => "null".to_string(),
            FuzzValue::Bool(b) => b.to_string(),
            FuzzValue::Int(i) => i.to_string(),
            FuzzValue::Float(f) => {
                if f.is_nan() || f.is_infinite() {
                    "0.0".to_string()
                } else {
                    format!("{:.6}", f)
                }
            }
            FuzzValue::Str(s) => {
                let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
                format!("'{}'", escaped)
            }
            FuzzValue::Array(items) => {
                let elems: Vec<String> = items.iter().take(5).map(|v| v.to_sql()).collect();
                format!("[{}]", elems.join(", "))
            }
        }
    }
}

impl FuzzSimpleValue {
    fn to_sql(&self) -> String {
        match self {
            FuzzSimpleValue::Null => "null".to_string(),
            FuzzSimpleValue::Bool(b) => b.to_string(),
            FuzzSimpleValue::Int(i) => i.to_string(),
            FuzzSimpleValue::Float(f) => {
                if f.is_nan() || f.is_infinite() {
                    "0.0".to_string()
                } else {
                    format!("{:.6}", f)
                }
            }
            FuzzSimpleValue::Str(s) => {
                let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
                format!("'{}'", escaped)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Test: structured SQL generation — must not panic
// ---------------------------------------------------------------------------

#[test]
fn fuzz_parse_structured_sql() {
    arbtest::arbtest(|u| {
        let stmt: FuzzStatement = u.arbitrary()?;
        let sql = stmt.to_sql();
        let _ = stellardb::query::parse::parse(&sql);
        Ok(())
    });
}

#[test]
fn fuzz_parse_multi_statement() {
    arbtest::arbtest(|u| {
        let count: u8 = u.arbitrary()?;
        let count = (count % 5) + 1;
        let stmts: Vec<String> = (0..count)
            .map(|_| -> arbitrary::Result<String> {
                let stmt: FuzzStatement = u.arbitrary()?;
                Ok(stmt.to_sql())
            })
            .collect::<Result<_, _>>()?;
        let sql = stmts.join("; ");
        let _ = stellardb::query::parse::parse(&sql);
        Ok(())
    });
}

// ---------------------------------------------------------------------------
// Targeted edge cases
// ---------------------------------------------------------------------------

#[test]
fn fuzz_parse_sql_keywords_as_input() {
    let keywords = [
        "SELECT",
        "FROM",
        "WHERE",
        "INSERT",
        "INTO",
        "UPDATE",
        "DELETE",
        "CREATE",
        "DROP",
        "DEFINE",
        "COLLECTION",
        "SET",
        "ORDER",
        "BY",
        "LIMIT",
        "OFFSET",
        "GROUP",
        "AS",
        "AND",
        "OR",
        "NOT",
        "NULL",
        "NONE",
        "IN",
        "IS",
        "BEGIN",
        "COMMIT",
        "ROLLBACK",
        "EXPLAIN",
        "ANALYZE",
        "RELATE",
        "CONTENT",
        "RETURN",
        "AFTER",
        "BEFORE",
        "DESCRIBE",
        "CASCADE",
        "SCHEMA",
        "STRICT",
        "FLEXIBLE",
        "REQUIRED",
        "DEFAULT",
        "INDEX",
        "ON",
        "UNIQUE",
        "REINDEX",
        "ASC",
        "DESC",
        "ALL",
        "TIMEOUT",
        "LET",
    ];
    for kw in &keywords {
        let _ = stellardb::query::parse::parse(kw);
        let _ = stellardb::query::parse::parse(&kw.to_lowercase());
        let _ = stellardb::query::parse::parse(&format!("{0} {0}", kw));
    }
}

#[test]
fn fuzz_parse_boundary_strings() {
    let cases = [
        "",
        " ",
        "\t\n\r",
        ";",
        ";;;",
        "SELECT",
        "SELECT *",
        "SELECT * FROM",
        "SELECT FROM WHERE",
        "'unterminated string",
        "`unterminated backtick",
        "SELECT * FROM user WHERE age > ",
        "(((",
        ")))",
        "SELECT 999999999999999999999999999999999999 FROM t",
        "SELECT -999999999999999999999999999999999999 FROM t",
        "SELECT 1.7976931348623157e+308 FROM t",
        "SELECT -1.7976931348623157e+308 FROM t",
        "SELECT 0.0 FROM t",
        "SELECT '' FROM t",
        "SELECT '''' FROM t",
        "RELATE a:b->c->d:e->f->g:h",
        "SELECT ->->-> FROM x",
        "SELECT <-<-<- FROM x",
        "SELECT arr[0][1][2][3][4][5] FROM t",
        "SELECT arr[0..1][2..3][4..5] FROM t",
        "SELECT * FROM user; SELECT * FROM user; SELECT * FROM user",
    ];
    for case in &cases {
        let _ = stellardb::query::parse::parse(case);
    }
}
