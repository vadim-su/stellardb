//! Core ABAC (Attribute-Based Access Control) types for StellarDB's auth system.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A resolved authenticated user with their attributes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subject {
    /// Unique user identifier used by authorization policies.
    pub user_id: String,
    /// User attributes used for policy evaluation (e.g., role, department, team)
    pub attributes: HashMap<String, String>,
}

/// Public administrative view of a user. Internal authentication fields are
/// deliberately excluded from this boundary.
#[cfg(feature = "auth")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedUser {
    pub username: String,
    pub attributes: HashMap<String, String>,
    pub api_key_count: usize,
    pub protected: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// Public administrative view of an API key. The plaintext secret, lookup ID,
/// and all stored hashes are never represented by this type.
#[cfg(feature = "auth")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedApiKey {
    pub name: String,
    pub username: String,
    pub hint: Option<String>,
    pub created_at: Option<String>,
}

/// Server-resolved authenticated subject carrying non-forgeable session identity.
///
/// The identity is deliberately unavailable through the public `Subject` request/API
/// surface. Only authentication code can construct this carrier.
#[cfg(feature = "auth")]
#[derive(Debug, Clone)]
pub(crate) struct ResolvedSubject {
    subject: Subject,
    #[cfg(any(feature = "server", test))]
    principal_identity: String,
}

#[cfg(feature = "auth")]
impl ResolvedSubject {
    pub(crate) fn new(subject: Subject, principal_identity: String) -> Self {
        #[cfg(not(any(feature = "server", test)))]
        let _ = principal_identity;

        Self {
            subject,
            #[cfg(any(feature = "server", test))]
            principal_identity,
        }
    }

    #[cfg(feature = "server")]
    pub(crate) fn subject(&self) -> &Subject {
        &self.subject
    }

    #[cfg(any(feature = "server", test))]
    pub(crate) fn principal_identity(&self) -> &str {
        &self.principal_identity
    }

    pub(crate) fn into_subject(self) -> Subject {
        self.subject
    }
}

/// An action that can be performed on a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    Select,
    Insert,
    Update,
    Delete,
    Create,
    Drop,
    Traverse,
    Search,
    Manage,
}

impl Action {
    /// Returns the uppercase string representation of the action.
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Select => "SELECT",
            Action::Insert => "INSERT",
            Action::Update => "UPDATE",
            Action::Delete => "DELETE",
            Action::Create => "CREATE",
            Action::Drop => "DROP",
            Action::Traverse => "TRAVERSE",
            Action::Search => "SEARCH",
            Action::Manage => "MANAGE",
        }
    }
}

/// A resource being accessed, identified by database and optional collection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resource {
    /// Target database name
    pub database: String,
    /// Target collection (None means database-level resource)
    pub collection: Option<String>,
    /// Additional resource attributes for fine-grained policy matching
    pub attributes: HashMap<String, String>,
}

/// The effect a policy produces when its conditions match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effect {
    Allow,
    Deny,
}

/// A condition that must be satisfied for a policy to apply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Condition {
    /// The attribute field to evaluate (e.g., "subject.role", "resource.database")
    pub field: String,
    /// The comparison operator
    pub op: ConditionOp,
    /// The value(s) to compare against
    pub value: ConditionValue,
}

/// Comparison operator for policy conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConditionOp {
    /// Equals
    Eq,
    /// Not equals
    Neq,
    /// Value is contained in a list
    In,
}

/// Value used in a policy condition comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConditionValue {
    /// A single string value (for Eq/Neq comparisons)
    Single(String),
    /// A list of string values (for In comparisons)
    List(Vec<String>),
}

/// An access control policy with conditions and an effect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    /// Human-readable policy name
    pub name: String,
    /// Conditions that must all match for this policy to apply
    pub conditions: Vec<Condition>,
    /// The effect when all conditions match
    pub effect: Effect,
}

/// The result of an authorization decision.
#[derive(Debug, Clone, PartialEq)]
pub enum AuthzDecision {
    /// Access is granted
    Allowed,
    /// Access is denied, optionally referencing the denying policy
    Denied { policy: Option<String> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_as_str_returns_uppercase() {
        assert_eq!(Action::Select.as_str(), "SELECT");
        assert_eq!(Action::Insert.as_str(), "INSERT");
        assert_eq!(Action::Update.as_str(), "UPDATE");
        assert_eq!(Action::Delete.as_str(), "DELETE");
        assert_eq!(Action::Create.as_str(), "CREATE");
        assert_eq!(Action::Drop.as_str(), "DROP");
        assert_eq!(Action::Traverse.as_str(), "TRAVERSE");
        assert_eq!(Action::Search.as_str(), "SEARCH");
        assert_eq!(Action::Manage.as_str(), "MANAGE");
    }

    #[test]
    fn subject_with_attributes() {
        let subject = Subject {
            user_id: "user:alice".to_string(),
            attributes: HashMap::from([
                ("role".to_string(), "admin".to_string()),
                ("department".to_string(), "engineering".to_string()),
            ]),
        };
        assert_eq!(subject.user_id, "user:alice");
        assert_eq!(subject.attributes.get("role"), Some(&"admin".to_string()));
    }

    #[test]
    fn policy_serialization_roundtrip() {
        let policy = Policy {
            name: "admin-full-access".to_string(),
            conditions: vec![Condition {
                field: "subject.role".to_string(),
                op: ConditionOp::Eq,
                value: ConditionValue::Single("admin".to_string()),
            }],
            effect: Effect::Allow,
        };

        let json = serde_json::to_string(&policy).unwrap();
        let deserialized: Policy = serde_json::from_str(&json).unwrap();
        assert_eq!(policy, deserialized);
    }

    #[test]
    fn authz_decision_variants() {
        let allowed = AuthzDecision::Allowed;
        assert_eq!(allowed, AuthzDecision::Allowed);

        let denied = AuthzDecision::Denied {
            policy: Some("deny-readonly".to_string()),
        };
        assert!(matches!(denied, AuthzDecision::Denied { policy: Some(_) }));

        let denied_no_policy = AuthzDecision::Denied { policy: None };
        assert!(matches!(
            denied_no_policy,
            AuthzDecision::Denied { policy: None }
        ));
    }
}
