use std::collections::HashMap;

use stellardb::auth::Subject;

#[test]
fn external_subject_struct_literal_remains_source_compatible() {
    let subject = Subject {
        user_id: "alice".to_string(),
        attributes: HashMap::from([("role".to_string(), "reader".to_string())]),
    };
    assert_eq!(subject.user_id, "alice");
    assert_eq!(
        subject.attributes.get("role").map(String::as_str),
        Some("reader")
    );
}

#[test]
fn public_subject_serialization_contains_only_abac_fields() {
    let subject = Subject {
        user_id: "alice".to_string(),
        attributes: HashMap::new(),
    };
    let value = serde_json::to_value(subject).unwrap();
    assert_eq!(value["user_id"], "alice");
    assert!(value.get("attributes").is_some());
    assert!(value.get("principal_identity").is_none());
}
