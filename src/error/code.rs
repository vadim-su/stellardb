use serde::Serialize;

/// Severity level for errors returned to clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// Structured error response sent to HTTP/gRPC clients.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub code: &'static str,
    pub message: String,
    pub severity: Severity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// Trait for error types that carry a stable error code.
pub trait ErrorCode {
    /// Stable error code in `SDB-CCNNN` format.
    fn code(&self) -> &'static str;

    /// Severity level (almost always `Error`).
    fn severity(&self) -> Severity {
        Severity::Error
    }

    /// Optional extended explanation.
    fn detail(&self) -> Option<String> {
        None
    }

    /// Optional actionable suggestion.
    fn hint(&self) -> Option<String> {
        None
    }

    /// Build the full structured error response.
    fn to_error_response(&self) -> ErrorResponse
    where
        Self: std::fmt::Display,
    {
        ErrorResponse {
            code: self.code(),
            message: self.to_string(),
            severity: self.severity(),
            detail: self.detail(),
            hint: self.hint(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_response_serialization() {
        let resp = ErrorResponse {
            code: "SDB-QE007",
            message: "Collection not found: 'users'".to_string(),
            severity: Severity::Error,
            detail: None,
            hint: Some("Did you mean: 'user'?".to_string()),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["code"], "SDB-QE007");
        assert_eq!(json["severity"], "ERROR");
        assert!(json.get("detail").is_none());
        assert_eq!(json["hint"], "Did you mean: 'user'?");
    }

    #[test]
    fn test_error_response_no_optional_fields() {
        let resp = ErrorResponse {
            code: "SDB-QP001",
            message: "Syntax error".to_string(),
            severity: Severity::Error,
            detail: None,
            hint: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json.get("detail").is_none());
        assert!(json.get("hint").is_none());
    }

    #[test]
    fn test_error_response_all_fields() {
        let resp = ErrorResponse {
            code: "SDB-QE017",
            message: "Type error".to_string(),
            severity: Severity::Error,
            detail: Some("Expected int, got string".to_string()),
            hint: Some("Cast the value first".to_string()),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["detail"], "Expected int, got string");
        assert_eq!(json["hint"], "Cast the value first");
    }
}
