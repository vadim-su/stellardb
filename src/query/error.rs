use crate::error::StorageError;
use crate::schema::BindError;
use pest::error::Error as PestError;

use crate::query::parse::Rule;

#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("Parse error: {0}")]
    Parse(#[from] ParseError),

    #[error("Execute error: {0}")]
    Execute(#[from] ExecuteError),

    #[error("Bind error: {0}")]
    Bind(#[from] BindError),
}

/// Rename `kw_*` rules to their clean keyword names in error messages.
fn clean_rule_name(rule: &Rule) -> String {
    let s = format!("{:?}", rule);
    if let Some(keyword) = s.strip_prefix("kw_") {
        keyword.to_string()
    } else {
        s
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("\n{0}")]
    Pest(Box<PestError<Rule>>),

    #[error("Invalid number: {0}")]
    InvalidNumber(String),

    #[error("Empty input: no statements found")]
    EmptyInput,

    #[error("Invalid type: {0}")]
    InvalidType(String),

    #[error("Unexpected token: {0}")]
    UnexpectedToken(String),

    #[error("Missing expression")]
    MissingExpression,

    #[error("Invalid datetime: {0}")]
    InvalidDatetime(String),

    #[error("Invalid duration: {0}")]
    InvalidDuration(String),

    #[error("Invalid bytes: {0}")]
    InvalidBytes(String),

    #[error("Invalid string escape: {0}")]
    InvalidStringEscape(String),
}

impl From<PestError<Rule>> for ParseError {
    fn from(err: PestError<Rule>) -> Self {
        ParseError::Pest(Box::new(err.renamed_rules(clean_rule_name)))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExecuteError {
    #[error("Document not found: {0}")]
    NotFound(String),

    #[error("Document already exists: {0}")]
    AlreadyExists(String),

    #[error("Invalid document ID: {0}")]
    InvalidId(String),

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("Transaction statement {0} must be handled at session layer")]
    TransactionNotSupported(&'static str),

    #[error("Collection not empty: {0} (use CASCADE to force drop)")]
    CollectionNotEmpty(String),

    #[error("Collection not found: {0}")]
    CollectionNotFound(String),

    #[error("DDL statements are not allowed inside transactions")]
    DdlNotAllowedInTransaction,

    #[error("Session required for transaction operations")]
    SessionRequired,

    #[error("session access denied")]
    SessionAccessDenied,

    #[error("session belongs to a different database")]
    SessionDatabaseMismatch,

    #[error("session is unknown, expired, or no longer active")]
    SessionExpired,

    #[error("transaction conflict")]
    TransactionConflict,

    #[error("Schema validation error: {0}")]
    SchemaValidation(#[from] BindError),

    #[error("Index already exists on {0}({1})")]
    IndexAlreadyExists(String, String),

    #[error("Index not found on {0}({1})")]
    IndexNotFound(String, String),

    #[error("Unique constraint violation: {field} = {value}")]
    UniqueViolation { field: String, value: String },

    #[error("Query timeout: exceeded {0}")]
    QueryTimeout(String),

    #[error("operation exceeded the server DB deadline")]
    ServerDeadlineExceeded,

    #[error("Not yet implemented: {0}")]
    NotYetImplemented(&'static str),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Operator not initialized: {0}")]
    OperatorNotInitialized(&'static str),

    #[error("Undefined variable: ${0}")]
    UndefinedVariable(String),

    #[error("Arithmetic overflow in {0}")]
    Overflow(&'static str),

    #[error("Unexpected expression in eval: {0}")]
    UnexpectedExpression(&'static str),

    #[error("Edge type not defined: {0}")]
    EdgeTypeUndefined(String),

    #[error("Invalid encoding: {0}")]
    InvalidEncoding(String),

    #[error("Type error: expected {expected}, got {got} in {op}")]
    TypeError {
        expected: &'static str,
        got: String,
        op: &'static str,
    },

    #[error("Division by zero")]
    DivisionByZero,

    #[error("Function {function} expects {expected} argument(s), got {got}")]
    ArityMismatch {
        function: String,
        expected: String,
        got: usize,
    },

    #[error("Subquery must return exactly {expected} column(s), got {got}")]
    SubqueryColumnCount { expected: usize, got: usize },

    #[error("SELECT with GROUP: field '{0}' must be a GROUP field or an aggregate")]
    InvalidGroupSelect(String),

    #[error("Analyzer not found: {0}")]
    AnalyzerNotFound(String),

    #[error(
        "Cannot access parent at depth {requested}: only {available} parent level(s) available"
    )]
    ParentDepthExceeded { requested: usize, available: usize },

    #[error("Unknown function: {0}")]
    UnknownFunction(String),
}

use crate::error::ErrorCode;

impl ErrorCode for ParseError {
    fn code(&self) -> &'static str {
        match self {
            Self::Pest(_) => "SDB-QP001",
            Self::InvalidNumber(_) => "SDB-QP002",
            Self::EmptyInput => "SDB-QP003",
            Self::InvalidType(_) => "SDB-QP004",
            Self::UnexpectedToken(_) => "SDB-QP005",
            Self::MissingExpression => "SDB-QP006",
            Self::InvalidDatetime(_) => "SDB-QP007",
            Self::InvalidDuration(_) => "SDB-QP008",
            Self::InvalidBytes(_) => "SDB-QP009",
            Self::InvalidStringEscape(_) => "SDB-QP010",
        }
    }
}

impl ErrorCode for ExecuteError {
    fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "SDB-QE001",
            Self::AlreadyExists(_) => "SDB-QE002",
            Self::InvalidId(_) => "SDB-QE003",
            Self::InvalidOperation(_) => "SDB-QE035",
            Self::Storage(e) => e.code(),
            Self::TransactionNotSupported(_) => "SDB-QE005",
            Self::CollectionNotEmpty(_) => "SDB-QE006",
            Self::CollectionNotFound(_) => "SDB-QE007",
            Self::DdlNotAllowedInTransaction => "SDB-QE008",
            Self::SessionRequired => "SDB-QE009",
            Self::SessionAccessDenied => "SDB-QE031",
            Self::SessionDatabaseMismatch => "SDB-QE032",
            Self::TransactionConflict => "SDB-QE033",
            Self::SessionExpired => "SDB-QE034",
            Self::SchemaValidation(e) => e.code(),
            Self::IndexAlreadyExists(..) => "SDB-QE011",
            Self::IndexNotFound(..) => "SDB-QE012",
            Self::UniqueViolation { .. } => "SDB-QE013",
            Self::QueryTimeout(_) => "SDB-QE014",
            Self::ServerDeadlineExceeded => "SDB-SV003",
            Self::NotYetImplemented(_) => "SDB-QE015",
            Self::Internal(_) => "SDB-QE016",
            Self::OperatorNotInitialized(_) => "SDB-QE025",
            Self::UndefinedVariable(_) => "SDB-QE026",
            Self::Overflow(_) => "SDB-QE027",
            Self::UnexpectedExpression(_) => "SDB-QE028",
            Self::EdgeTypeUndefined(_) => "SDB-QE029",
            Self::InvalidEncoding(_) => "SDB-QE030",
            Self::TypeError { .. } => "SDB-QE017",
            Self::DivisionByZero => "SDB-QE018",
            Self::ArityMismatch { .. } => "SDB-QE019",
            Self::SubqueryColumnCount { .. } => "SDB-QE020",
            Self::InvalidGroupSelect(_) => "SDB-QE021",
            Self::AnalyzerNotFound(_) => "SDB-QE022",
            Self::ParentDepthExceeded { .. } => "SDB-QE023",
            Self::UnknownFunction(_) => "SDB-QE024",
        }
    }
}

impl ErrorCode for QueryError {
    fn code(&self) -> &'static str {
        match self {
            Self::Parse(e) => e.code(),
            Self::Execute(e) => e.code(),
            Self::Bind(e) => e.code(),
        }
    }

    fn hint(&self) -> Option<String> {
        match self {
            Self::Parse(e) => e.hint(),
            Self::Execute(e) => e.hint(),
            Self::Bind(e) => e.hint(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    #[test]
    fn test_execute_error_codes() {
        assert_eq!(ExecuteError::DivisionByZero.code(), "SDB-QE018");
        assert_eq!(
            ExecuteError::CollectionNotFound("x".into()).code(),
            "SDB-QE007"
        );
        assert_eq!(
            ExecuteError::OperatorNotInitialized("TableScanOp").code(),
            "SDB-QE025"
        );
        assert_eq!(
            ExecuteError::UndefinedVariable("x".into()).code(),
            "SDB-QE026"
        );
        assert_eq!(ExecuteError::Overflow("add").code(), "SDB-QE027");
        assert_eq!(
            ExecuteError::UnexpectedExpression("Aggregate").code(),
            "SDB-QE028"
        );
        assert_eq!(
            ExecuteError::EdgeTypeUndefined("knows".into()).code(),
            "SDB-QE029"
        );
        assert_eq!(
            ExecuteError::InvalidEncoding("base64".into()).code(),
            "SDB-QE030"
        );
        assert_eq!(ExecuteError::Internal("test".into()).code(), "SDB-QE016");
        assert_eq!(
            ExecuteError::InvalidOperation("test".into()).code(),
            "SDB-QE035"
        );
    }

    #[test]
    fn test_parse_error_codes() {
        assert_eq!(ParseError::EmptyInput.code(), "SDB-QP003");
        assert_eq!(ParseError::InvalidNumber("x".into()).code(), "SDB-QP002");
        assert_eq!(ParseError::MissingExpression.code(), "SDB-QP006");
        assert_eq!(
            ParseError::InvalidStringEscape("x".into()).code(),
            "SDB-QP010"
        );
    }

    #[test]
    fn test_query_error_delegates_code() {
        let qe = QueryError::Execute(ExecuteError::DivisionByZero);
        assert_eq!(qe.code(), "SDB-QE018");

        let qe = QueryError::Parse(ParseError::EmptyInput);
        assert_eq!(qe.code(), "SDB-QP003");
    }

    #[test]
    fn test_execute_error_storage_delegates() {
        use crate::error::StorageError;
        let se = StorageError::NotFound("test".into());
        let ee = ExecuteError::Storage(se);
        assert_eq!(ee.code(), "SDB-ST006");
    }
}
