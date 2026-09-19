use std::collections::HashMap;
use std::fmt;

use super::{Action, AuthzDecision, Resource, Subject};

/// Narrow authorization contract used by the core query engine.
///
/// Authentication, identity management, and policy persistence live behind the
/// `auth` feature. Embedded applications can provide their own authorizer
/// without compiling the full `AuthService`.
pub trait AuthorizationProvider: Send + Sync {
    fn authorize(&self, subject: &Subject, action: Action, resource: &Resource) -> AuthzDecision;

    fn get_collection_attributes(
        &self,
        database: &str,
        collection: &str,
    ) -> HashMap<String, String> {
        let _ = (database, collection);
        HashMap::new()
    }
}

/// Transport-independent authorization failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationError {
    MissingSubject,
    SystemDatabase,
    Denied { policy: Option<String> },
}

impl fmt::Display for AuthorizationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSubject => write!(f, "Access denied: authenticated subject required"),
            Self::SystemDatabase => write!(f, "Access denied: system database is private"),
            Self::Denied { policy: Some(name) } => write!(f, "Access denied by policy '{name}'"),
            Self::Denied { policy: None } => write!(f, "Access denied: no matching policy"),
        }
    }
}

impl std::error::Error for AuthorizationError {}

/// Authorize an action consistently across query, HTTP, and gRPC boundaries.
///
/// When auth is not configured, access remains unrestricted for embedded/test
/// deployments. Once auth is configured, a missing subject is rejected rather
/// than silently bypassing policy evaluation.
pub fn authorize(
    authorization: Option<&dyn AuthorizationProvider>,
    subject: Option<&Subject>,
    action: Action,
    database: &str,
    collection: Option<&str>,
) -> Result<(), AuthorizationError> {
    if database == "_system" {
        return Err(AuthorizationError::SystemDatabase);
    }

    let Some(authorization) = authorization else {
        return Ok(());
    };
    let subject = subject.ok_or(AuthorizationError::MissingSubject)?;

    let attributes = collection
        .map(|name| authorization.get_collection_attributes(database, name))
        .unwrap_or_default();
    let resource = Resource {
        database: database.to_string(),
        collection: collection.map(str::to_string),
        attributes,
    };

    match authorization.authorize(subject, action, &resource) {
        AuthzDecision::Allowed => Ok(()),
        AuthzDecision::Denied { policy } => Err(AuthorizationError::Denied { policy }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_database_is_never_available_through_the_query_boundary() {
        assert_eq!(
            authorize(None, None, Action::Select, "_system", Some("config")),
            Err(AuthorizationError::SystemDatabase)
        );
    }
}
