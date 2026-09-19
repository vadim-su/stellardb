use crate::error::ErrorClass;

/// Classified authentication-management error.
///
/// Public `AuthService` methods keep their historical `String` boundary, while
/// server/query integrations use this type to preserve status and redaction.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    #[error("{resource} '{name}' already exists")]
    AlreadyExists {
        resource: &'static str,
        name: String,
    },
    #[error("{resource} '{name}' not found")]
    NotFound {
        resource: &'static str,
        name: String,
    },
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Internal(String),
}

impl AuthError {
    pub fn already_exists(resource: &'static str, name: impl Into<String>) -> Self {
        Self::AlreadyExists {
            resource,
            name: name.into(),
        }
    }

    pub fn not_found(resource: &'static str, name: impl Into<String>) -> Self {
        Self::NotFound {
            resource,
            name: name.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    pub fn class(&self) -> ErrorClass {
        match self {
            Self::AlreadyExists { .. } => ErrorClass::AlreadyExists,
            Self::NotFound { .. } => ErrorClass::NotFound,
            Self::Invalid(_) => ErrorClass::InvalidArgument,
            Self::Internal(_) => ErrorClass::Internal,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::AlreadyExists { .. } => "SDB-AC005",
            Self::NotFound { .. } => "SDB-AC004",
            Self::Invalid(_) => "SDB-AC003",
            Self::Internal(_) => "SDB-AC006",
        }
    }

    pub fn safe_message(&self) -> String {
        match self {
            Self::Internal(_) => "Internal error".to_string(),
            _ => self.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_and_redaction_are_stable() {
        let duplicate = AuthError::already_exists("user", "alice");
        assert_eq!(duplicate.class(), ErrorClass::AlreadyExists);
        assert_eq!(duplicate.code(), "SDB-AC005");
        assert_eq!(duplicate.safe_message(), "user 'alice' already exists");

        let marker = "/private/system/auth.db";
        let internal = AuthError::internal(marker);
        assert_eq!(internal.class(), ErrorClass::Internal);
        assert_eq!(internal.code(), "SDB-AC006");
        assert_eq!(internal.safe_message(), "Internal error");
        assert!(!internal.safe_message().contains(marker));
    }
}
