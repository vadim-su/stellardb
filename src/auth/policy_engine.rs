//! ABAC policy evaluation engine.
//!
//! Evaluates Subject x Action x Resource against stored policies using
//! deny-wins semantics: if any matching policy denies, the request is denied
//! regardless of other allow policies.

use super::types::*;
use parking_lot::RwLock;

/// Attribute-based access control policy engine.
///
/// Stores a set of [`Policy`] rules and evaluates authorization requests
/// against them. The evaluation follows deny-wins semantics: an explicit
/// deny always takes precedence over an allow.
pub struct PolicyEngine {
    policies: RwLock<Vec<Policy>>,
}

impl PolicyEngine {
    /// Creates an empty policy engine with no loaded policies.
    pub fn new() -> Self {
        Self {
            policies: RwLock::new(Vec::new()),
        }
    }

    /// Replaces all policies with the given set.
    pub fn load_policies(&self, policies: Vec<Policy>) {
        *self.policies.write() = policies;
    }

    /// Appends a policy to the current set.
    pub fn add_policy(&self, policy: Policy) {
        self.policies.write().push(policy);
    }

    /// Removes a policy by name. Returns `true` if a policy was found and removed.
    pub fn remove_policy(&self, name: &str) -> bool {
        let mut policies = self.policies.write();
        let len_before = policies.len();
        policies.retain(|p| p.name != name);
        policies.len() < len_before
    }

    /// Returns a clone of all loaded policies.
    pub fn list_policies(&self) -> Vec<Policy> {
        self.policies.read().clone()
    }

    /// Evaluates an authorization request against loaded policies.
    ///
    /// Evaluation rules:
    /// 1. The `"root"` user always gets access (superuser bypass).
    /// 2. Each policy's conditions are checked with AND semantics — all must match.
    /// 3. If any matching policy has [`Effect::Deny`], access is denied (deny wins).
    /// 4. If any matching policy has [`Effect::Allow`], access is allowed.
    /// 5. If no policies match, access is denied (default deny).
    pub fn evaluate(
        &self,
        subject: &Subject,
        action: Action,
        resource: &Resource,
    ) -> AuthzDecision {
        // Root bypass
        if subject.user_id == "root" {
            return AuthzDecision::Allowed;
        }

        let policies = self.policies.read();

        let mut has_allow = false;

        for policy in policies.iter() {
            if self.policy_matches(policy, subject, action, resource) {
                match policy.effect {
                    Effect::Deny => {
                        return AuthzDecision::Denied {
                            policy: Some(policy.name.clone()),
                        };
                    }
                    Effect::Allow => {
                        has_allow = true;
                    }
                }
            }
        }

        if has_allow {
            AuthzDecision::Allowed
        } else {
            AuthzDecision::Denied { policy: None }
        }
    }

    /// Checks whether all conditions in a policy match the given request context.
    fn policy_matches(
        &self,
        policy: &Policy,
        subject: &Subject,
        action: Action,
        resource: &Resource,
    ) -> bool {
        policy
            .conditions
            .iter()
            .all(|cond| self.condition_matches(cond, subject, action, resource))
    }

    /// Resolves the field value from the request context and compares it
    /// against the condition's expected value.
    fn condition_matches(
        &self,
        condition: &Condition,
        subject: &Subject,
        action: Action,
        resource: &Resource,
    ) -> bool {
        let actual = self.resolve_field(&condition.field, subject, action, resource);

        match (&condition.op, &condition.value) {
            (ConditionOp::Eq, ConditionValue::Single(expected)) => {
                actual.as_deref() == Some(expected.as_str())
            }
            (ConditionOp::Neq, ConditionValue::Single(expected)) => {
                actual.as_deref() != Some(expected.as_str())
            }
            (ConditionOp::In, ConditionValue::List(expected)) => match &actual {
                Some(val) => expected.iter().any(|e| e == val),
                None => false,
            },
            // Mismatched op/value combinations never match
            _ => false,
        }
    }

    /// Resolves a dotted field path to the corresponding string value from
    /// the request context.
    ///
    /// Supported field paths:
    /// - `"action"` — the action being performed (e.g., "SELECT")
    /// - `"subject.<attr>"` — a subject attribute (e.g., "subject.role")
    /// - `"resource.database"` — the target database name
    /// - `"resource.collection"` — the target collection name (if any)
    /// - `"resource.<attr>"` — a resource attribute
    fn resolve_field(
        &self,
        field: &str,
        subject: &Subject,
        action: Action,
        resource: &Resource,
    ) -> Option<String> {
        if field == "action" {
            return Some(action.as_str().to_string());
        }

        if let Some(attr) = field.strip_prefix("subject.") {
            return subject.attributes.get(attr).cloned();
        }

        if let Some(attr) = field.strip_prefix("resource.") {
            return match attr {
                "database" => Some(resource.database.clone()),
                "collection" => resource.collection.clone(),
                _ => resource.attributes.get(attr).cloned(),
            };
        }

        None
    }
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a subject with a given user_id and optional attributes.
    fn make_subject(user_id: &str, attrs: &[(&str, &str)]) -> Subject {
        Subject {
            user_id: user_id.to_string(),
            attributes: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    /// Helper: create a resource with database, optional collection, and optional attributes.
    fn make_resource(database: &str, collection: Option<&str>, attrs: &[(&str, &str)]) -> Resource {
        Resource {
            database: database.to_string(),
            collection: collection.map(|s| s.to_string()),
            attributes: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    /// Helper: create a condition.
    fn cond(field: &str, op: ConditionOp, value: ConditionValue) -> Condition {
        Condition {
            field: field.to_string(),
            op,
            value,
        }
    }

    /// Helper: create a policy.
    fn policy(name: &str, conditions: Vec<Condition>, effect: Effect) -> Policy {
        Policy {
            name: name.to_string(),
            conditions,
            effect,
        }
    }

    #[test]
    fn test_root_bypasses_all() {
        let engine = PolicyEngine::new();
        // No policies at all — root should still be allowed
        let subject = make_subject("root", &[]);
        let resource = make_resource("mydb", Some("users"), &[]);

        let decision = engine.evaluate(&subject, Action::Select, &resource);
        assert_eq!(decision, AuthzDecision::Allowed);
    }

    #[test]
    fn test_default_deny() {
        let engine = PolicyEngine::new();
        let subject = make_subject("user:alice", &[("role", "viewer")]);
        let resource = make_resource("mydb", Some("users"), &[]);

        let decision = engine.evaluate(&subject, Action::Select, &resource);
        assert_eq!(decision, AuthzDecision::Denied { policy: None });
    }

    #[test]
    fn test_allow_policy() {
        let engine = PolicyEngine::new();
        engine.add_policy(policy(
            "admin-all",
            vec![cond(
                "subject.role",
                ConditionOp::Eq,
                ConditionValue::Single("admin".to_string()),
            )],
            Effect::Allow,
        ));

        let subject = make_subject("user:alice", &[("role", "admin")]);
        let resource = make_resource("mydb", Some("users"), &[]);

        let decision = engine.evaluate(&subject, Action::Select, &resource);
        assert_eq!(decision, AuthzDecision::Allowed);

        // Non-admin should be denied
        let other = make_subject("user:bob", &[("role", "viewer")]);
        let decision = engine.evaluate(&other, Action::Select, &resource);
        assert_eq!(decision, AuthzDecision::Denied { policy: None });
    }

    #[test]
    fn test_deny_wins_over_allow() {
        let engine = PolicyEngine::new();

        // Allow admins everything
        engine.add_policy(policy(
            "admin-allow",
            vec![cond(
                "subject.role",
                ConditionOp::Eq,
                ConditionValue::Single("admin".to_string()),
            )],
            Effect::Allow,
        ));

        // Deny deletes on production database
        engine.add_policy(policy(
            "deny-prod-delete",
            vec![
                cond(
                    "action",
                    ConditionOp::Eq,
                    ConditionValue::Single("DELETE".to_string()),
                ),
                cond(
                    "resource.database",
                    ConditionOp::Eq,
                    ConditionValue::Single("production".to_string()),
                ),
            ],
            Effect::Deny,
        ));

        let subject = make_subject("user:alice", &[("role", "admin")]);
        let resource = make_resource("production", Some("users"), &[]);

        // Admin can SELECT on production
        let decision = engine.evaluate(&subject, Action::Select, &resource);
        assert_eq!(decision, AuthzDecision::Allowed);

        // Admin CANNOT delete on production (deny wins)
        let decision = engine.evaluate(&subject, Action::Delete, &resource);
        assert_eq!(
            decision,
            AuthzDecision::Denied {
                policy: Some("deny-prod-delete".to_string())
            }
        );
    }

    #[test]
    fn test_neq_condition() {
        let engine = PolicyEngine::new();

        // Allow anyone whose role is NOT "guest"
        engine.add_policy(policy(
            "allow-non-guest",
            vec![cond(
                "subject.role",
                ConditionOp::Neq,
                ConditionValue::Single("guest".to_string()),
            )],
            Effect::Allow,
        ));

        let admin = make_subject("user:alice", &[("role", "admin")]);
        let guest = make_subject("user:bob", &[("role", "guest")]);
        let resource = make_resource("mydb", Some("data"), &[]);

        // Admin is allowed (role != guest)
        assert_eq!(
            engine.evaluate(&admin, Action::Select, &resource),
            AuthzDecision::Allowed
        );

        // Guest is denied (role == guest, so Neq does not match)
        assert_eq!(
            engine.evaluate(&guest, Action::Select, &resource),
            AuthzDecision::Denied { policy: None }
        );
    }

    #[test]
    fn test_in_condition() {
        let engine = PolicyEngine::new();

        // Allow SELECT and SEARCH for analysts
        engine.add_policy(policy(
            "analyst-read",
            vec![
                cond(
                    "subject.role",
                    ConditionOp::Eq,
                    ConditionValue::Single("analyst".to_string()),
                ),
                cond(
                    "action",
                    ConditionOp::In,
                    ConditionValue::List(vec!["SELECT".to_string(), "SEARCH".to_string()]),
                ),
            ],
            Effect::Allow,
        ));

        let analyst = make_subject("user:carol", &[("role", "analyst")]);
        let resource = make_resource("analytics", Some("events"), &[]);

        // SELECT allowed
        assert_eq!(
            engine.evaluate(&analyst, Action::Select, &resource),
            AuthzDecision::Allowed
        );

        // SEARCH allowed
        assert_eq!(
            engine.evaluate(&analyst, Action::Search, &resource),
            AuthzDecision::Allowed
        );

        // INSERT denied (not in list)
        assert_eq!(
            engine.evaluate(&analyst, Action::Insert, &resource),
            AuthzDecision::Denied { policy: None }
        );
    }

    #[test]
    fn test_remove_policy() {
        let engine = PolicyEngine::new();

        engine.add_policy(policy(
            "temp-allow",
            vec![cond(
                "subject.role",
                ConditionOp::Eq,
                ConditionValue::Single("temp".to_string()),
            )],
            Effect::Allow,
        ));

        assert_eq!(engine.list_policies().len(), 1);

        // Remove existing
        assert!(engine.remove_policy("temp-allow"));
        assert_eq!(engine.list_policies().len(), 0);

        // Remove non-existent
        assert!(!engine.remove_policy("no-such-policy"));
    }

    #[test]
    fn test_multiple_conditions_and() {
        let engine = PolicyEngine::new();

        // Allow only if role=editor AND action=UPDATE AND resource.database=content
        engine.add_policy(policy(
            "editor-update-content",
            vec![
                cond(
                    "subject.role",
                    ConditionOp::Eq,
                    ConditionValue::Single("editor".to_string()),
                ),
                cond(
                    "action",
                    ConditionOp::Eq,
                    ConditionValue::Single("UPDATE".to_string()),
                ),
                cond(
                    "resource.database",
                    ConditionOp::Eq,
                    ConditionValue::Single("content".to_string()),
                ),
            ],
            Effect::Allow,
        ));

        let editor = make_subject("user:dan", &[("role", "editor")]);
        let content = make_resource("content", Some("articles"), &[]);
        let other_db = make_resource("finance", Some("reports"), &[]);

        // All conditions match
        assert_eq!(
            engine.evaluate(&editor, Action::Update, &content),
            AuthzDecision::Allowed
        );

        // Wrong action — one condition fails, no match
        assert_eq!(
            engine.evaluate(&editor, Action::Delete, &content),
            AuthzDecision::Denied { policy: None }
        );

        // Wrong database
        assert_eq!(
            engine.evaluate(&editor, Action::Update, &other_db),
            AuthzDecision::Denied { policy: None }
        );

        // Wrong role
        let viewer = make_subject("user:eve", &[("role", "viewer")]);
        assert_eq!(
            engine.evaluate(&viewer, Action::Update, &content),
            AuthzDecision::Denied { policy: None }
        );
    }

    #[test]
    fn test_resource_database_attribute() {
        let engine = PolicyEngine::new();

        // Deny all access to the "secrets" database
        engine.add_policy(policy(
            "deny-secrets-db",
            vec![cond(
                "resource.database",
                ConditionOp::Eq,
                ConditionValue::Single("secrets".to_string()),
            )],
            Effect::Deny,
        ));

        // Allow everything for admins
        engine.add_policy(policy(
            "admin-all",
            vec![cond(
                "subject.role",
                ConditionOp::Eq,
                ConditionValue::Single("admin".to_string()),
            )],
            Effect::Allow,
        ));

        let admin = make_subject("user:alice", &[("role", "admin")]);
        let secrets = make_resource("secrets", Some("keys"), &[]);
        let normal = make_resource("mydb", Some("users"), &[]);

        // Admin on secrets DB — denied (deny wins)
        assert_eq!(
            engine.evaluate(&admin, Action::Select, &secrets),
            AuthzDecision::Denied {
                policy: Some("deny-secrets-db".to_string())
            }
        );

        // Admin on normal DB — allowed
        assert_eq!(
            engine.evaluate(&admin, Action::Select, &normal),
            AuthzDecision::Allowed
        );
    }

    #[test]
    fn test_load_policies_replaces_all() {
        let engine = PolicyEngine::new();

        engine.add_policy(policy("old-policy", vec![], Effect::Allow));
        assert_eq!(engine.list_policies().len(), 1);

        engine.load_policies(vec![
            policy("new-1", vec![], Effect::Allow),
            policy("new-2", vec![], Effect::Deny),
        ]);

        let policies = engine.list_policies();
        assert_eq!(policies.len(), 2);
        assert_eq!(policies[0].name, "new-1");
        assert_eq!(policies[1].name, "new-2");
    }

    #[test]
    fn test_resource_collection_condition() {
        let engine = PolicyEngine::new();

        // Deny access to the "audit_log" collection
        engine.add_policy(policy(
            "deny-audit-log",
            vec![cond(
                "resource.collection",
                ConditionOp::Eq,
                ConditionValue::Single("audit_log".to_string()),
            )],
            Effect::Deny,
        ));

        let subject = make_subject("user:alice", &[("role", "admin")]);
        let audit = make_resource("mydb", Some("audit_log"), &[]);
        let users = make_resource("mydb", Some("users"), &[]);

        assert_eq!(
            engine.evaluate(&subject, Action::Select, &audit),
            AuthzDecision::Denied {
                policy: Some("deny-audit-log".to_string())
            }
        );

        // Different collection — deny policy doesn't match, no allow policy either
        assert_eq!(
            engine.evaluate(&subject, Action::Select, &users),
            AuthzDecision::Denied { policy: None }
        );
    }

    #[test]
    fn test_resource_custom_attribute() {
        let engine = PolicyEngine::new();

        // Allow access only to resources with sensitivity=public
        engine.add_policy(policy(
            "public-only",
            vec![cond(
                "resource.sensitivity",
                ConditionOp::Eq,
                ConditionValue::Single("public".to_string()),
            )],
            Effect::Allow,
        ));

        let subject = make_subject("user:alice", &[("role", "viewer")]);
        let public_res = make_resource("mydb", Some("docs"), &[("sensitivity", "public")]);
        let private_res = make_resource("mydb", Some("docs"), &[("sensitivity", "private")]);

        assert_eq!(
            engine.evaluate(&subject, Action::Select, &public_res),
            AuthzDecision::Allowed
        );

        assert_eq!(
            engine.evaluate(&subject, Action::Select, &private_res),
            AuthzDecision::Denied { policy: None }
        );
    }
}
