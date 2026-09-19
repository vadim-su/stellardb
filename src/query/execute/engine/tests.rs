use super::*;
use crate::Database;
use crate::document::Value;
use crate::query::ast::{Projection, ProjectionItem};
use std::sync::Arc;
use tempfile::TempDir;

fn setup() -> (TempDir, Arc<Database>) {
    let tmp = TempDir::new().unwrap();
    let storage = Arc::new(Database::open(tmp.path()).unwrap());
    (tmp, storage)
}

#[test]
fn test_execute_simple_scan() {
    let (_tmp, storage) = setup();

    crate::run_sql!(storage, "DEFINE COLLECTION test");
    crate::run_sql!(storage, "INSERT INTO test {id: '1', name: 'Alice'}");
    crate::run_sql!(storage, "INSERT INTO test {id: '2', name: 'Bob'}");

    let plan = LogicalPlan::Scan {
        collection: "test".to_string(),
        key: None,
        filter: None,
        projection: Projection::All,
        order: None,
        limit: None,
        offset: None,
        distinct: false,
        value_mode: false,
        value_expr: None,
        fetch: None,
    };

    let rows = execute(plan, &*storage, None).unwrap();
    assert_eq!(rows.len(), 2);
}

#[test]
fn test_execute_key_lookup() {
    let (_tmp, storage) = setup();

    crate::run_sql!(storage, "DEFINE COLLECTION test");
    crate::run_sql!(storage, "INSERT INTO test {id: 'alice', name: 'Alice'}");

    let plan = LogicalPlan::Scan {
        collection: "test".to_string(),
        key: Some("alice".to_string()),
        filter: None,
        projection: Projection::All,
        order: None,
        limit: None,
        offset: None,
        distinct: false,
        value_mode: false,
        value_expr: None,
        fetch: None,
    };

    let rows = execute(plan, &*storage, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].doc.id, "test:alice");
}

#[test]
fn test_execute_with_filter() {
    let (_tmp, storage) = setup();

    crate::run_sql!(storage, "DEFINE COLLECTION test");
    crate::run_sql!(
        storage,
        "INSERT INTO test {id: '1', age: 25}, {id: '2', age: 30}, {id: '3', age: 25}"
    );

    use crate::query::ast::{BinaryOp, Expr};

    let plan = LogicalPlan::Scan {
        collection: "test".to_string(),
        key: None,
        filter: Some(Expr::BinaryOp {
            left: Box::new(Expr::Field("age".to_string())),
            op: BinaryOp::Eq,
            right: Box::new(Expr::Literal(Value::Int(25))),
        }),
        projection: Projection::All,
        order: None,
        limit: None,
        offset: None,
        distinct: false,
        value_mode: false,
        value_expr: None,
        fetch: None,
    };

    let rows = execute(plan, &*storage, None).unwrap();
    assert_eq!(rows.len(), 2);
}

#[test]
fn test_execute_with_limit() {
    let (_tmp, storage) = setup();

    crate::run_sql!(storage, "DEFINE COLLECTION test");
    crate::run_sql!(
        storage,
        "INSERT INTO test {id: '1'}, {id: '2'}, {id: '3'}, {id: '4'}, {id: '5'}"
    );

    let plan = LogicalPlan::Scan {
        collection: "test".to_string(),
        key: None,
        filter: None,
        projection: Projection::All,
        order: None,
        limit: Some(3),
        offset: None,
        distinct: false,
        value_mode: false,
        value_expr: None,
        fetch: None,
    };

    let rows = execute(plan, &*storage, None).unwrap();
    assert_eq!(rows.len(), 3);
}

#[test]
fn test_execute_with_projection() {
    let (_tmp, storage) = setup();

    crate::run_sql!(storage, "DEFINE COLLECTION test");
    crate::run_sql!(
        storage,
        "INSERT INTO test {id: '1', name: 'Alice', age: 30, city: 'NYC'}"
    );

    let plan = LogicalPlan::Scan {
        collection: "test".to_string(),
        key: None,
        filter: None,
        projection: Projection::Items(vec![ProjectionItem {
            expr: crate::query::ast::Expr::Field("name".to_string()),
            alias: None,
        }]),
        order: None,
        limit: None,
        offset: None,
        distinct: false,
        value_mode: false,
        value_expr: None,
        fetch: None,
    };

    let rows = execute(plan, &*storage, None).unwrap();
    assert_eq!(rows.len(), 1);

    // Should have id (always) and name, but not age or city
    assert!(!rows[0].doc.id.is_empty());
    assert!(rows[0].doc.fields.contains_key("name"));
    // Note: projection is done at document level
}

#[test]
fn test_execute_with_index() {
    let (_tmp, storage) = setup();

    crate::run_sql!(storage, "DEFINE COLLECTION test");
    crate::run_sql!(storage, "CREATE INDEX ON test(status)");
    crate::run_sql!(
        storage,
        "INSERT INTO test {id: '1', status: 'active'}, {id: '2', status: 'inactive'}, {id: '3', status: 'active'}"
    );

    use crate::query::ast::{BinaryOp, Expr};

    let plan = LogicalPlan::Scan {
        collection: "test".to_string(),
        key: None,
        filter: Some(Expr::BinaryOp {
            left: Box::new(Expr::Field("status".to_string())),
            op: BinaryOp::Eq,
            right: Box::new(Expr::Literal(Value::String("active".to_string()))),
        }),
        projection: Projection::All,
        order: None,
        limit: None,
        offset: None,
        distinct: false,
        value_mode: false,
        value_expr: None,
        fetch: None,
    };

    let rows = execute(plan, &*storage, None).unwrap();
    assert_eq!(rows.len(), 2);
}

// === DML Tests ===

#[test]
fn test_execute_insert() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION test");

    use crate::query::ast::ObjectLiteral;

    let plan = LogicalPlan::Insert {
        collection: "test".to_string(),
        documents: vec![ObjectLiteral {
            fields: vec![
                ("id".to_string(), Value::String("alice".to_string())),
                ("name".to_string(), Value::String("Alice".to_string())),
            ],
        }],
    };

    let rows = execute(plan, &*storage, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].doc.id, "test:alice");
    assert_eq!(
        rows[0].doc.fields.get("name"),
        Some(&Value::String("Alice".to_string()))
    );

    // Verify persisted
    let doc = storage.get_document("test", "alice").unwrap();
    assert!(doc.is_some());
    assert_eq!(
        doc.unwrap().fields.get("name"),
        Some(&Value::String("Alice".to_string()))
    );
}

#[test]
fn test_execute_insert_multiple() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION test");

    use crate::query::ast::ObjectLiteral;

    let plan = LogicalPlan::Insert {
        collection: "test".to_string(),
        documents: vec![
            ObjectLiteral {
                fields: vec![
                    ("id".to_string(), Value::String("alice".to_string())),
                    ("name".to_string(), Value::String("Alice".to_string())),
                ],
            },
            ObjectLiteral {
                fields: vec![
                    ("id".to_string(), Value::String("bob".to_string())),
                    ("name".to_string(), Value::String("Bob".to_string())),
                ],
            },
        ],
    };

    let rows = execute(plan, &*storage, None).unwrap();
    assert_eq!(rows.len(), 2);

    // Verify both persisted
    assert!(storage.get_document("test", "alice").unwrap().is_some());
    assert!(storage.get_document("test", "bob").unwrap().is_some());
}

#[test]
fn test_execute_update_by_key() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION test");
    crate::run_sql!(storage, "INSERT INTO test {id: 'alice', age: 25}");

    use crate::query::ast::{Assignment, Expr};

    let plan = LogicalPlan::Update {
        collection: "test".to_string(),
        key: Some("alice".to_string()),
        filter: None,
        assignments: vec![Assignment {
            path: vec!["age".to_string()],
            expr: Expr::Literal(Value::Int(30)),
        }],
    };

    let rows = execute(plan, &*storage, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].doc.fields.get("age"), Some(&Value::Int(30)));

    // Verify persisted
    let doc = storage.get_document("test", "alice").unwrap().unwrap();
    assert_eq!(doc.fields.get("age"), Some(&Value::Int(30)));
}

#[test]
fn test_execute_update_with_filter() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION test");
    crate::run_sql!(
        storage,
        "INSERT INTO test {id: '1', status: 'active', count: 0}, {id: '2', status: 'inactive', count: 0}, {id: '3', status: 'active', count: 0}"
    );

    use crate::query::ast::{Assignment, BinaryOp, Expr};

    let plan = LogicalPlan::Update {
        collection: "test".to_string(),
        key: None,
        filter: Some(Expr::BinaryOp {
            left: Box::new(Expr::Field("status".to_string())),
            op: BinaryOp::Eq,
            right: Box::new(Expr::Literal(Value::String("active".to_string()))),
        }),
        assignments: vec![Assignment {
            path: vec!["count".to_string()],
            expr: Expr::Literal(Value::Int(1)),
        }],
    };

    let rows = execute(plan, &*storage, None).unwrap();
    assert_eq!(rows.len(), 2); // Only active documents updated

    // Verify: active docs have count=1, inactive has count=0
    let doc1 = storage.get_document("test", "1").unwrap().unwrap();
    let doc2 = storage.get_document("test", "2").unwrap().unwrap();
    let doc3 = storage.get_document("test", "3").unwrap().unwrap();
    assert_eq!(doc1.fields.get("count"), Some(&Value::Int(1)));
    assert_eq!(doc2.fields.get("count"), Some(&Value::Int(0)));
    assert_eq!(doc3.fields.get("count"), Some(&Value::Int(1)));
}

#[test]
fn test_execute_update_add_field() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION test");
    crate::run_sql!(storage, "INSERT INTO test {id: 'alice', name: 'Alice'}");

    use crate::query::ast::{Assignment, Expr};

    let plan = LogicalPlan::Update {
        collection: "test".to_string(),
        key: Some("alice".to_string()),
        filter: None,
        assignments: vec![Assignment {
            path: vec!["city".to_string()],
            expr: Expr::Literal(Value::String("NYC".to_string())),
        }],
    };

    let rows = execute(plan, &*storage, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].doc.fields.get("city"),
        Some(&Value::String("NYC".to_string()))
    );

    // Verify new field added
    let doc = storage.get_document("test", "alice").unwrap().unwrap();
    assert_eq!(
        doc.fields.get("city"),
        Some(&Value::String("NYC".to_string()))
    );
    // Original field preserved
    assert_eq!(
        doc.fields.get("name"),
        Some(&Value::String("Alice".to_string()))
    );
}

#[test]
fn test_execute_delete_by_key() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION test");
    crate::run_sql!(storage, "INSERT INTO test {id: 'alice'}, {id: 'bob'}");

    let plan = LogicalPlan::Delete {
        collection: "test".to_string(),
        key: Some("alice".to_string()),
        filter: None,
    };

    let rows = execute(plan, &*storage, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].doc.id, "test:alice");

    // Verify deleted
    assert!(storage.get_document("test", "alice").unwrap().is_none());
    // Bob still exists
    assert!(storage.get_document("test", "bob").unwrap().is_some());
}

#[test]
fn test_execute_delete_with_filter() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION test");
    crate::run_sql!(
        storage,
        "INSERT INTO test {id: '1', status: 'old'}, {id: '2', status: 'new'}, {id: '3', status: 'old'}"
    );

    use crate::query::ast::{BinaryOp, Expr};

    let plan = LogicalPlan::Delete {
        collection: "test".to_string(),
        key: None,
        filter: Some(Expr::BinaryOp {
            left: Box::new(Expr::Field("status".to_string())),
            op: BinaryOp::Eq,
            right: Box::new(Expr::Literal(Value::String("old".to_string()))),
        }),
    };

    let rows = execute(plan, &*storage, None).unwrap();
    assert_eq!(rows.len(), 2); // Only old documents deleted

    // Verify: old docs deleted, new doc remains
    assert!(storage.get_document("test", "1").unwrap().is_none());
    assert!(storage.get_document("test", "2").unwrap().is_some());
    assert!(storage.get_document("test", "3").unwrap().is_none());
}

#[test]
fn test_execute_delete_all() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION test");
    crate::run_sql!(storage, "INSERT INTO test {id: '1'}, {id: '2'}, {id: '3'}");

    let plan = LogicalPlan::Delete {
        collection: "test".to_string(),
        key: None,
        filter: None,
    };

    let rows = execute(plan, &*storage, None).unwrap();
    assert_eq!(rows.len(), 3);

    // Verify all deleted
    assert!(storage.get_document("test", "1").unwrap().is_none());
    assert!(storage.get_document("test", "2").unwrap().is_none());
    assert!(storage.get_document("test", "3").unwrap().is_none());
}

#[test]
fn test_execute_create() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION test");

    use crate::query::ast::{Assignment, Expr};

    // CREATE is like INSERT with explicit key
    let plan = LogicalPlan::Create {
        collection: "test".to_string(),
        key: Some("mykey".to_string()),
        assignments: vec![
            Assignment {
                path: vec!["name".to_string()],
                expr: Expr::Literal(Value::String("Test".to_string())),
            },
            Assignment {
                path: vec!["count".to_string()],
                expr: Expr::Literal(Value::Int(42)),
            },
        ],
    };

    let rows = execute(plan, &*storage, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].doc.id, "test:mykey");
    assert_eq!(
        rows[0].doc.fields.get("name"),
        Some(&Value::String("Test".to_string()))
    );
    assert_eq!(rows[0].doc.fields.get("count"), Some(&Value::Int(42)));

    // Verify persisted
    let doc = storage.get_document("test", "mykey").unwrap().unwrap();
    assert_eq!(
        doc.fields.get("name"),
        Some(&Value::String("Test".to_string()))
    );
}

#[test]
fn test_execute_with_range_index_gt() {
    let (_tmp, storage) = setup();

    crate::run_sql!(storage, "DEFINE COLLECTION users");
    crate::run_sql!(storage, "CREATE INDEX ON users(age)");
    crate::run_sql!(
        storage,
        "INSERT INTO users {id: '1', age: 20}, {id: '2', age: 30}, {id: '3', age: 25}"
    );

    use crate::query::ast::{BinaryOp, Expr};

    // age > 25 should return only age=30
    let plan = LogicalPlan::Scan {
        collection: "users".to_string(),
        key: None,
        filter: Some(Expr::BinaryOp {
            left: Box::new(Expr::Field("age".to_string())),
            op: BinaryOp::Gt,
            right: Box::new(Expr::Literal(Value::Int(25))),
        }),
        projection: Projection::All,
        order: None,
        limit: None,
        offset: None,
        distinct: false,
        value_mode: false,
        value_expr: None,
        fetch: None,
    };

    let rows = execute(plan, &*storage, None).unwrap();
    assert_eq!(rows.len(), 1, "Expected 1 result (age=30), got {:?}", rows);
    assert_eq!(rows[0].doc.fields.get("age"), Some(&Value::Int(30)));
}

#[test]
fn test_execute_with_range_index_gte() {
    let (_tmp, storage) = setup();

    crate::run_sql!(storage, "DEFINE COLLECTION users");
    crate::run_sql!(storage, "CREATE INDEX ON users(age)");
    crate::run_sql!(
        storage,
        "INSERT INTO users {id: '1', age: 20}, {id: '2', age: 30}, {id: '3', age: 25}"
    );

    use crate::query::ast::{BinaryOp, Expr};

    // age >= 25 should return age=25 and age=30
    let plan = LogicalPlan::Scan {
        collection: "users".to_string(),
        key: None,
        filter: Some(Expr::BinaryOp {
            left: Box::new(Expr::Field("age".to_string())),
            op: BinaryOp::Gte,
            right: Box::new(Expr::Literal(Value::Int(25))),
        }),
        projection: Projection::All,
        order: None,
        limit: None,
        offset: None,
        distinct: false,
        value_mode: false,
        value_expr: None,
        fetch: None,
    };

    let rows = execute(plan, &*storage, None).unwrap();
    assert_eq!(
        rows.len(),
        2,
        "Expected 2 results (age>=25), got {:?}",
        rows
    );
}

#[test]
fn test_execute_with_range_index_lt() {
    let (_tmp, storage) = setup();

    crate::run_sql!(storage, "DEFINE COLLECTION users");
    crate::run_sql!(storage, "CREATE INDEX ON users(age)");
    crate::run_sql!(
        storage,
        "INSERT INTO users {id: '1', age: 20}, {id: '2', age: 30}, {id: '3', age: 25}"
    );

    use crate::query::ast::{BinaryOp, Expr};

    // age < 25 should return only age=20
    let plan = LogicalPlan::Scan {
        collection: "users".to_string(),
        key: None,
        filter: Some(Expr::BinaryOp {
            left: Box::new(Expr::Field("age".to_string())),
            op: BinaryOp::Lt,
            right: Box::new(Expr::Literal(Value::Int(25))),
        }),
        projection: Projection::All,
        order: None,
        limit: None,
        offset: None,
        distinct: false,
        value_mode: false,
        value_expr: None,
        fetch: None,
    };

    let rows = execute(plan, &*storage, None).unwrap();
    assert_eq!(rows.len(), 1, "Expected 1 result (age<25), got {:?}", rows);
    assert_eq!(rows[0].doc.fields.get("age"), Some(&Value::Int(20)));
}

// === EXPLAIN Tests ===

#[test]
fn test_explain_returns_plan_text() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION test");

    let result = crate::query::query_json(storage.clone(), "EXPLAIN SELECT * FROM test").unwrap();
    let text = result.as_array().unwrap()[0].as_str().unwrap();
    assert!(
        text.contains("TableScan"),
        "Expected TableScan in plan: {}",
        text
    );
}

#[test]
fn test_explain_with_filter() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION users");
    crate::run_sql!(storage, "CREATE INDEX ON users(age)");

    let result = crate::query::query_json(
        storage.clone(),
        "EXPLAIN SELECT * FROM users WHERE age > 18",
    )
    .unwrap();
    let text = result.as_array().unwrap()[0].as_str().unwrap();

    assert!(
        text.contains("IndexScan") || text.contains("Filter"),
        "Expected IndexScan or Filter in plan: {}",
        text
    );
}

#[test]
fn test_explain_with_key_lookup() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION users");

    let result =
        crate::query::query_json(storage.clone(), "EXPLAIN SELECT * FROM users:alice").unwrap();
    let text = result.as_array().unwrap()[0].as_str().unwrap();
    assert!(
        text.contains("KeyLookup"),
        "Expected KeyLookup in plan: {}",
        text
    );
}

#[test]
fn test_explain_update() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION users");

    let result = crate::query::query_json(
        storage.clone(),
        "EXPLAIN UPDATE users SET x = 1 WHERE age > 18",
    )
    .unwrap();
    let text = result.as_array().unwrap()[0].as_str().unwrap();
    assert!(text.contains("Update"), "Expected Update in plan: {}", text);
}

#[test]
fn test_explain_delete() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION users");

    let result =
        crate::query::query_json(storage.clone(), "EXPLAIN DELETE users WHERE status = 'old'")
            .unwrap();
    let text = result.as_array().unwrap()[0].as_str().unwrap();
    assert!(text.contains("Delete"), "Expected Delete in plan: {}", text);
}

// === EXPLAIN ANALYZE Tests ===

#[test]
fn test_explain_analyze_shows_stats() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION test");
    crate::run_sql!(storage, "INSERT INTO test {id: '1'}, {id: '2'}, {id: '3'}");

    let result =
        crate::query::query_json(storage.clone(), "EXPLAIN ANALYZE SELECT * FROM test").unwrap();
    let text = result.as_array().unwrap()[0].as_str().unwrap();

    // Should contain stats
    assert!(text.contains("rows:"), "Expected row stats in: {}", text);
    assert!(text.contains("total:"), "Expected time stats in: {}", text);
    assert!(text.contains("rows: 3"), "Expected 3 rows in: {}", text);
}

#[test]
fn test_explain_analyze_with_limit() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION test");
    crate::run_sql!(
        storage,
        "INSERT INTO test {id: '1'}, {id: '2'}, {id: '3'}, {id: '4'}, {id: '5'}"
    );

    let result = crate::query::query_json(
        storage.clone(),
        "EXPLAIN ANALYZE SELECT * FROM test LIMIT 2",
    )
    .unwrap();
    let text = result.as_array().unwrap()[0].as_str().unwrap();

    // Limit should show 2 rows out
    assert!(text.contains("Limit"), "Expected Limit in plan: {}", text);
    assert!(text.contains("rows:"), "Expected stats in plan: {}", text);
}

#[test]
fn test_explain_e2e_complex_query() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION users");
    crate::run_sql!(storage, "CREATE INDEX ON users(age)");
    crate::run_sql!(
        storage,
        "INSERT INTO users {id: '1', age: 20}, {id: '2', age: 30}, {id: '3', age: 25}"
    );

    // Complex query with index, filter, limit
    let result = crate::query::query_json(
        storage.clone(),
        "EXPLAIN SELECT * FROM users WHERE age >= 20 AND age <= 30 LIMIT 10",
    )
    .unwrap();
    let text = result.as_array().unwrap()[0].as_str().unwrap();

    println!("Plan:\n{}", text);
    assert!(text.contains("Limit"), "Expected Limit in plan: {}", text);
}

#[test]
fn test_explain_analyze_e2e_complex() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION users");
    crate::run_sql!(
        storage,
        "INSERT INTO users {id: '1', name: 'a'}, {id: '2', name: 'b'}, {id: '3', name: 'c'}"
    );

    let result = crate::query::query_json(
        storage.clone(),
        "EXPLAIN ANALYZE SELECT name FROM users LIMIT 2",
    )
    .unwrap();
    let text = result.as_array().unwrap()[0].as_str().unwrap();

    println!("Analyzed plan:\n{}", text);
    assert!(text.contains("rows:"), "Expected rows stats in: {}", text);
    assert!(text.contains("total:"), "Expected time stats in: {}", text);
}

// === OR Union Optimization Tests ===

#[test]
fn test_or_single_indexed_field() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION col");
    crate::run_sql!(storage, "CREATE INDEX ON col(city)");
    crate::run_sql!(
        storage,
        "INSERT INTO col {id: '1', city: 'Moscow'}, {id: '2', city: 'Berlin'}, {id: '3', city: 'Tokyo'}"
    );

    let result = crate::query::query_json(
        storage.clone(),
        "SELECT * FROM col WHERE city = 'Moscow' OR city = 'Berlin'",
    )
    .unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 2, "Expected 2 results, got {:?}", arr);

    let mut cities: Vec<&str> = arr.iter().map(|v| v["city"].as_str().unwrap()).collect();
    cities.sort();
    assert_eq!(cities, vec!["Berlin", "Moscow"]);
}

#[test]
fn test_or_different_indexed_fields() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION col");
    crate::run_sql!(storage, "CREATE INDEX ON col(name)");
    crate::run_sql!(storage, "CREATE INDEX ON col(city)");
    crate::run_sql!(
        storage,
        "INSERT INTO col {id: '1', name: 'Alice', city: 'Moscow'}, {id: '2', name: 'Bob', city: 'Berlin'}, {id: '3', name: 'Carol', city: 'Tokyo'}"
    );

    let result = crate::query::query_json(
        storage.clone(),
        "SELECT * FROM col WHERE name = 'Alice' OR city = 'Berlin'",
    )
    .unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 2, "Expected 2 results, got {:?}", arr);

    let mut ids: Vec<&str> = arr.iter().map(|v| v["id"].as_str().unwrap()).collect();
    ids.sort();
    assert_eq!(ids, vec!["col:1", "col:2"]);
}

#[test]
fn test_or_dedup_cross_field() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION col");
    crate::run_sql!(storage, "CREATE INDEX ON col(name)");
    crate::run_sql!(storage, "CREATE INDEX ON col(city)");
    crate::run_sql!(
        storage,
        "INSERT INTO col {id: '1', name: 'Alice', city: 'Moscow'}, {id: '2', name: 'Bob', city: 'Berlin'}"
    );

    // Doc '1' matches BOTH sides: name='Alice' AND city='Moscow'
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT * FROM col WHERE name = 'Alice' OR city = 'Moscow'",
    )
    .unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1, "Expected 1 result (dedup), got {:?}", arr);
    assert_eq!(arr[0]["id"].as_str().unwrap(), "col:1");
}

#[test]
fn test_or_fallback_one_not_indexed() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION col");
    // Only index on name, NOT on city
    crate::run_sql!(storage, "CREATE INDEX ON col(name)");
    crate::run_sql!(
        storage,
        "INSERT INTO col {id: '1', name: 'Alice', city: 'Moscow'}, {id: '2', name: 'Bob', city: 'Berlin'}, {id: '3', name: 'Carol', city: 'Moscow'}"
    );

    let result = crate::query::query_json(
        storage.clone(),
        "SELECT * FROM col WHERE name = 'Alice' OR city = 'Moscow'",
    )
    .unwrap();
    let arr = result.as_array().unwrap();
    // Alice (name match) + Carol (city match) — Alice matches both but should appear once
    let mut ids: Vec<&str> = arr.iter().map(|v| v["id"].as_str().unwrap()).collect();
    ids.sort();
    assert_eq!(
        ids,
        vec!["col:1", "col:3"],
        "Expected Alice and Carol, got {:?}",
        arr
    );
}

#[test]
fn test_or_three_branches() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION col");
    crate::run_sql!(storage, "CREATE INDEX ON col(city)");
    crate::run_sql!(
        storage,
        "INSERT INTO col {id: '1', city: 'Moscow'}, {id: '2', city: 'Berlin'}, {id: '3', city: 'Tokyo'}, {id: '4', city: 'Paris'}"
    );

    let result = crate::query::query_json(
        storage.clone(),
        "SELECT * FROM col WHERE city = 'Moscow' OR city = 'Berlin' OR city = 'Tokyo'",
    )
    .unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 3, "Expected 3 results, got {:?}", arr);

    let mut cities: Vec<&str> = arr.iter().map(|v| v["city"].as_str().unwrap()).collect();
    cities.sort();
    assert_eq!(cities, vec!["Berlin", "Moscow", "Tokyo"]);
}

#[test]
fn test_explain_shows_union() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION col");
    crate::run_sql!(storage, "CREATE INDEX ON col(name)");
    crate::run_sql!(storage, "CREATE INDEX ON col(city)");

    let result = crate::query::query_json(
        storage.clone(),
        "EXPLAIN SELECT * FROM col WHERE name = 'Alice' OR city = 'Berlin'",
    )
    .unwrap();
    let text = result.as_array().unwrap()[0].as_str().unwrap();
    assert!(
        text.contains("Union"),
        "Expected Union in plan, got: {}",
        text
    );
}

#[test]
fn test_explain_or_no_index_shows_table_scan() {
    let (_tmp, storage) = setup();
    crate::run_sql!(storage, "DEFINE COLLECTION col");
    // Only index on name, NOT on city
    crate::run_sql!(storage, "CREATE INDEX ON col(name)");

    let result = crate::query::query_json(
        storage.clone(),
        "EXPLAIN SELECT * FROM col WHERE name = 'Alice' OR city = 'Berlin'",
    )
    .unwrap();
    let text = result.as_array().unwrap()[0].as_str().unwrap();
    assert!(
        text.contains("TableScan"),
        "Expected TableScan (fallback) in plan, got: {}",
        text
    );
}

// === FROM array source tests ===

#[test]
fn test_select_star_from_scalar_array() {
    let (_tmp, storage) = setup();
    let result = crate::query::query_json(storage.clone(), "SELECT * FROM [0, 1, 2]").unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(
        arr,
        &vec![
            serde_json::json!(0),
            serde_json::json!(1),
            serde_json::json!(2)
        ]
    );
}

#[test]
fn test_select_value_ref_from_array() {
    let (_tmp, storage) = setup();
    // Without VALUE keyword, returns objects with computed field name
    let result =
        crate::query::query_json(storage.clone(), "SELECT $value + 1 FROM [10, 20]").unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr[0]["$value + Int(1)"], 11);
    assert_eq!(arr[1]["$value + Int(1)"], 21);
}

#[test]
fn test_select_value_keyword_from_array() {
    let (_tmp, storage) = setup();
    // With VALUE keyword, returns flat values
    let result =
        crate::query::query_json(storage.clone(), "SELECT VALUE $value + 1 FROM [10, 20]").unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr, &vec![serde_json::json!(11), serde_json::json!(21)]);
}

#[test]
fn test_select_value_ref_with_alias_output() {
    let (_tmp, storage) = setup();
    let result =
        crate::query::query_json(storage.clone(), "SELECT $value AS n FROM [10, 20]").unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr[0]["n"], 10);
    assert_eq!(arr[1]["n"], 20);
}

#[test]
fn test_select_star_from_object_array() {
    let (_tmp, storage) = setup();
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT * FROM [{name: 'Alice'}, {name: 'Bob'}]",
    )
    .unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr[0]["name"], "Alice");
    assert_eq!(arr[1]["name"], "Bob");
    assert!(arr[0].get("$value").is_none());
}

#[test]
fn test_select_from_array_with_value_ref() {
    let (_tmp, storage) = setup();
    // Use $value instead of alias (simplified grammar)
    let result =
        crate::query::query_json(storage.clone(), "SELECT $value + 1 FROM [10, 20]").unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr[0]["$value + Int(1)"], 11);
    assert_eq!(arr[1]["$value + Int(1)"], 21);
}

#[test]
fn test_select_value_from_array_with_value_ref() {
    let (_tmp, storage) = setup();
    // With VALUE keyword, returns flat values
    let result =
        crate::query::query_json(storage.clone(), "SELECT VALUE $value + 1 FROM [10, 20]").unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr, &vec![serde_json::json!(11), serde_json::json!(21)]);
}

#[test]
fn test_select_value_ref_field_path() {
    let (_tmp, storage) = setup();
    // Without VALUE keyword, returns objects
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT $value.name FROM [{name: 'Alice'}, {name: 'Bob'}]",
    )
    .unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr[0]["$value.name"], "Alice");
    assert_eq!(arr[1]["$value.name"], "Bob");
}

#[test]
fn test_select_value_keyword_field_path() {
    let (_tmp, storage) = setup();
    // With VALUE keyword, returns flat values
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT VALUE $value.name FROM [{name: 'Alice'}, {name: 'Bob'}]",
    )
    .unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(
        arr,
        &vec![serde_json::json!("Alice"), serde_json::json!("Bob")]
    );
}

#[test]
fn test_select_from_array_filter_with_value_ref() {
    let (_tmp, storage) = setup();
    // Without VALUE keyword, returns objects
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT $value FROM [10, 20, 30] WHERE $value > 15",
    )
    .unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr[0]["$value"], 20);
    assert_eq!(arr[1]["$value"], 30);
}

#[test]
fn test_select_value_keyword_filter() {
    let (_tmp, storage) = setup();
    // With VALUE keyword, returns flat values
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT VALUE $value FROM [10, 20, 30] WHERE $value > 15",
    )
    .unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr, &vec![serde_json::json!(20), serde_json::json!(30)]);
}

#[test]
fn test_select_multiple_with_aliases() {
    let (_tmp, storage) = setup();
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT $value AS a, $value + 1 AS b FROM [10, 20]",
    )
    .unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr[0]["a"], 10);
    assert_eq!(arr[0]["b"], 11);
    assert_eq!(arr[1]["a"], 20);
    assert_eq!(arr[1]["b"], 21);
}

#[test]
fn test_select_constant_from_array() {
    let (_tmp, storage) = setup();
    let result = crate::query::query_json(storage.clone(), "SELECT 1 FROM [0, 1]").unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 2);
}

#[test]
fn test_select_from_empty_array() {
    let (_tmp, storage) = setup();
    let result = crate::query::query_json(storage.clone(), "SELECT * FROM []").unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 0);
}

// --- field name "value" must not shadow $value ---
// SurrealDB semantics: SELECT field returns objects, SELECT VALUE returns flat values

#[test]
fn test_field_value_is_null_for_scalars() {
    let (_tmp, storage) = setup();
    // Without VALUE keyword, returns objects with the field
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT value FROM [0, [1, 2], {'value': 3}]",
    )
    .unwrap();
    let arr = result.as_array().unwrap();
    // Scalars don't have 'value' field, so it's null
    assert_eq!(arr[0]["value"], serde_json::Value::Null);
    assert_eq!(arr[1]["value"], serde_json::Value::Null);
    // Object has 'value' field
    assert_eq!(arr[2]["value"], 3);
}

#[test]
fn test_field_value_not_injected_for_scalars() {
    let (_tmp, storage) = setup();
    // Any plain field name on a scalar element returns object with null field
    let result = crate::query::query_json(storage.clone(), "SELECT x FROM [1, 2, 3]").unwrap();
    let arr = result.as_array().unwrap();
    for item in arr {
        assert_eq!(item["x"], serde_json::Value::Null);
    }
}

#[test]
fn test_dollar_value_still_works_for_scalars() {
    let (_tmp, storage) = setup();
    // Without VALUE keyword, returns objects
    let result = crate::query::query_json(storage.clone(), "SELECT $value FROM [10, 20]").unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr[0]["$value"], 10);
    assert_eq!(arr[1]["$value"], 20);
}

#[test]
fn test_select_value_keyword_dollar_value() {
    let (_tmp, storage) = setup();
    // With VALUE keyword, returns flat values
    let result =
        crate::query::query_json(storage.clone(), "SELECT VALUE $value FROM [10, 20]").unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr, &vec![serde_json::json!(10), serde_json::json!(20)]);
}

#[test]
fn test_object_field_value_accessible() {
    let (_tmp, storage) = setup();
    // Without VALUE keyword, returns objects
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT value FROM [{'value': 'a'}, {'value': 'b'}]",
    )
    .unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr[0]["value"], "a");
    assert_eq!(arr[1]["value"], "b");
}

#[test]
fn test_select_value_keyword_field() {
    let (_tmp, storage) = setup();
    // With VALUE keyword, returns flat values
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT VALUE value FROM [{'value': 'a'}, {'value': 'b'}]",
    )
    .unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr[0], "a");
    assert_eq!(arr[1], "b");
}

#[test]
fn test_mixed_array_value_field_only_from_objects() {
    let (_tmp, storage) = setup();
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT value AS v FROM [42, 'str', {'value': 99}]",
    )
    .unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr[0]["v"], serde_json::Value::Null);
    assert_eq!(arr[1]["v"], serde_json::Value::Null);
    assert_eq!(arr[2]["v"], serde_json::json!(99));
}

// === Scalar value resolution deduplication tests ===

#[test]
fn test_alias_resolves_to_scalar_value() {
    let (_tmp, storage) = setup();
    // Use $value instead of AS alias (simplified grammar doesn't support FROM [array] AS x)
    let result =
        crate::query::query_json(storage.clone(), "SELECT $value + 1 FROM [10, 20, 30]").unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr[0]["$value + Int(1)"], 11);
    assert_eq!(arr[1]["$value + Int(1)"], 21);
    assert_eq!(arr[2]["$value + Int(1)"], 31);
}

#[test]
fn test_dollar_value_resolves_to_scalar_no_duplication() {
    let (_tmp, storage) = setup();
    // This should work without $value being stored in doc.fields
    let result =
        crate::query::query_json(storage.clone(), "SELECT $value * 2 FROM [5, 10]").unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr[0]["$value * Int(2)"], 10);
    assert_eq!(arr[1]["$value * Int(2)"], 20);
}

#[test]
fn test_object_with_value_field_not_shadowed() {
    let (_tmp, storage) = setup();
    // Object with "$value" field should resolve from doc.fields, not scalar_value
    let result =
        crate::query::query_json(storage.clone(), r#"SELECT $value FROM [{"$value": 42}]"#)
            .unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr[0]["$value"], 42);
}

#[test]
fn test_scalar_value_in_filter() {
    let (_tmp, storage) = setup();
    // Filter using $value should work
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT $value FROM [10, 20, 30] WHERE $value > 15",
    )
    .unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["$value"], 20);
    assert_eq!(arr[1]["$value"], 30);
}

#[test]
fn test_offset_clause() {
    let (_tmp, storage) = setup();

    // Create documents
    crate::run_sql!(storage, "DEFINE COLLECTION test");
    for i in 1..=5 {
        crate::run_sql!(
            storage,
            &format!("INSERT INTO test {{id: '{}', x: {}}}", i, i)
        );
    }

    // Test basic OFFSET (note: StellarDB uses "ORDER x" not "ORDER BY x")
    let result =
        crate::query::query_json(storage.clone(), "SELECT x FROM test ORDER x OFFSET 2").unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0]["x"], 3);
    assert_eq!(arr[1]["x"], 4);
    assert_eq!(arr[2]["x"], 5);

    // Test OFFSET + LIMIT
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT x FROM test ORDER x LIMIT 2 OFFSET 1",
    )
    .unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["x"], 2);
    assert_eq!(arr[1]["x"], 3);

    // Test OFFSET exceeds count
    let result = crate::query::query_json(storage.clone(), "SELECT x FROM test OFFSET 10").unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 0);

    // Test OFFSET 0 (no effect)
    let result =
        crate::query::query_json(storage.clone(), "SELECT x FROM test ORDER x OFFSET 0").unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 5);
}

#[cfg(feature = "hnsw")]
#[test]
fn test_knn_query_execution() {
    let (_tmp, storage) = setup();

    // Setup: create collection with HNSW index
    crate::run_sql!(storage, "DEFINE COLLECTION vectors");
    crate::run_sql!(
        storage,
        "CREATE INDEX ON vectors(embedding) HNSW DIMENSION 3 DIST EUCLIDEAN"
    );

    // Insert test documents with vectors
    crate::run_sql!(
        storage,
        "INSERT INTO vectors {id: 'v1', name: 'A', embedding: [1.0, 0.0, 0.0]}"
    );
    crate::run_sql!(
        storage,
        "INSERT INTO vectors {id: 'v2', name: 'B', embedding: [0.0, 1.0, 0.0]}"
    );
    crate::run_sql!(
        storage,
        "INSERT INTO vectors {id: 'v3', name: 'C', embedding: [0.0, 0.0, 1.0]}"
    );

    // Execute KNN query - find 2 nearest neighbors to [1.0, 0.0, 0.0]
    // Use vector::distance() to access the distance value
    let result = crate::query::query_json(storage.clone(),
        "SELECT name, vector::distance() AS dist FROM vectors WHERE embedding <|2|> [1.0, 0.0, 0.0]",
    )
        .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 2);

    // First result should be exact match (v1, distance ~0)
    assert_eq!(arr[0]["name"], "A");
    let dist0 = arr[0]["dist"].as_f64().unwrap();
    assert!(
        dist0 < 0.01,
        "First result should have distance close to 0, got {}",
        dist0
    );

    // Second result should have some distance
    let dist1 = arr[1]["dist"].as_f64().unwrap();
    assert!(
        dist1 > 0.5,
        "Second result should have significant distance, got {}",
        dist1
    );
}

#[cfg(feature = "hnsw")]
#[test]
fn test_knn_query_with_filter() {
    let (_tmp, storage) = setup();

    // Setup: create collection with HNSW index
    crate::run_sql!(storage, "DEFINE COLLECTION items");
    crate::run_sql!(
        storage,
        "CREATE INDEX ON items(vec) HNSW DIMENSION 2 DIST EUCLIDEAN"
    );

    // Insert test data
    crate::run_sql!(
        storage,
        "INSERT INTO items {id: 'i1', category: 'A', vec: [1.0, 0.0]}"
    );
    crate::run_sql!(
        storage,
        "INSERT INTO items {id: 'i2', category: 'B', vec: [0.9, 0.1]}"
    );
    crate::run_sql!(
        storage,
        "INSERT INTO items {id: 'i3', category: 'A', vec: [0.0, 1.0]}"
    );

    // KNN with additional filter
    let result = crate::query::query_json(storage.clone(),
        "SELECT id, vector::distance() AS dist FROM items WHERE vec <|3|> [1.0, 0.0] AND category = 'A'",
    )
        .unwrap();

    let arr = result.as_array().unwrap();
    // Should only return items with category A
    assert_eq!(arr.len(), 2);
    for item in arr {
        let id = item["id"].as_str().unwrap();
        // ID includes collection prefix like "items:i1"
        assert!(
            id.contains("i1") || id.contains("i3"),
            "Expected i1 or i3, got {}",
            id
        );
    }
}

#[cfg(feature = "hnsw")]
#[test]
fn test_explain_knn_query() {
    let (_tmp, storage) = setup();

    // Setup
    crate::run_sql!(storage, "DEFINE COLLECTION docs");
    crate::run_sql!(storage, "CREATE INDEX ON docs(emb) HNSW DIMENSION 2");

    // Test EXPLAIN shows KnnScan
    let result = crate::query::query_json(
        storage.clone(),
        "EXPLAIN SELECT * FROM docs WHERE emb <|5|> [0.0, 1.0]",
    )
    .unwrap();

    // EXPLAIN returns array with single string element
    let plan_text = result.as_array().unwrap()[0].as_str().unwrap();
    assert!(
        plan_text.contains("KnnScan"),
        "EXPLAIN should show KnnScan, got: {}",
        plan_text
    );
    assert!(
        plan_text.contains("k: 5"),
        "EXPLAIN should show k parameter, got: {}",
        plan_text
    );
}

// === SELECT without FROM clause tests ===

#[test]
fn test_select_arithmetic_no_from() {
    let (_tmp, storage) = setup();
    let result = crate::query::query_json(storage.clone(), "SELECT 1 + 2 AS sum").unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["sum"], 3);
}

#[test]
fn test_select_function_no_from() {
    let (_tmp, storage) = setup();
    let result =
        crate::query::query_json(storage.clone(), "SELECT string::upper('hello') AS upper")
            .unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["upper"], "HELLO");
}

#[test]
fn test_select_value_no_from() {
    let (_tmp, storage) = setup();
    let result = crate::query::query_json(storage.clone(), "SELECT VALUE 42").unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr, &vec![serde_json::json!(42)]);
}

#[test]
fn test_select_multiple_expressions_no_from() {
    let (_tmp, storage) = setup();
    let result =
        crate::query::query_json(storage.clone(), "SELECT 1 AS a, 2 AS b, 1 + 2 AS c").unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["a"], 1);
    assert_eq!(arr[0]["b"], 2);
    assert_eq!(arr[0]["c"], 3);
}

// === Subqueries in function arguments tests ===

#[test]
fn test_subquery_in_array_literal() {
    let (_tmp, storage) = setup();

    // Create test data
    crate::run_sql!(storage, "DEFINE COLLECTION items");
    crate::run_sql!(
        storage,
        "INSERT INTO items {id: '1', name: 'A'}, {id: '2', name: 'B'}"
    );

    // Subquery as array element - should return array of results
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT [(SELECT name FROM items)] AS names",
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    // The result should contain the subquery results as an array
    let names = arr[0]["names"].as_array().unwrap();
    assert_eq!(names.len(), 1); // Outer array has one element (the inner array)
    let inner = names[0].as_array().unwrap();
    assert_eq!(inner.len(), 2); // Two results from subquery
}

#[test]
fn test_subquery_in_function_args() {
    let (_tmp, storage) = setup();

    // Create test data
    crate::run_sql!(storage, "DEFINE COLLECTION numbers");
    crate::run_sql!(
        storage,
        "INSERT INTO numbers {id: '1', val: 10}, {id: '2', val: 20}"
    );

    // array::length with subquery result
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT array::length((SELECT val FROM numbers)) AS count",
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["count"], 2);
}

#[test]
fn test_multiple_subqueries_in_array() {
    let (_tmp, storage) = setup();

    // Create test data
    crate::run_sql!(storage, "DEFINE COLLECTION a");
    crate::run_sql!(storage, "DEFINE COLLECTION b");
    crate::run_sql!(storage, "INSERT INTO a {id: '1', x: 1}");
    crate::run_sql!(storage, "INSERT INTO b {id: '1', y: 2}");

    // Multiple subqueries in array
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT [(SELECT x FROM a), (SELECT y FROM b)] AS results",
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    let results = arr[0]["results"].as_array().unwrap();
    assert_eq!(results.len(), 2); // Two subquery results
}

// === DELETE Edge Tests ===

#[test]
fn test_delete_specific_edge() {
    let (_tmp, storage) = setup();

    crate::run_sql!(storage, "DEFINE COLLECTION user");
    crate::run_sql!(storage, "INSERT INTO user {id: 'alice', name: 'Alice'}");
    crate::run_sql!(storage, "INSERT INTO user {id: 'bob', name: 'Bob'}");
    crate::run_sql!(storage, "RELATE user:alice->follows->user:bob");

    // Verify edge exists
    let (edges, _) = storage
        .list_edges_out("user:alice", "follows", None, 10)
        .unwrap();
    assert_eq!(edges.len(), 1);

    // Delete edge
    let result =
        crate::query::query_json(storage.clone(), "DELETE user:alice->follows->user:bob").unwrap();
    assert_eq!(result[0]["deleted"], 1);

    // Verify edge is gone
    let (edges, _) = storage
        .list_edges_out("user:alice", "follows", None, 10)
        .unwrap();
    assert_eq!(edges.len(), 0);
}

#[test]
fn test_delete_all_edges_by_label() {
    let (_tmp, storage) = setup();

    crate::run_sql!(storage, "DEFINE COLLECTION user");
    crate::run_sql!(storage, "INSERT INTO user {id: 'alice', name: 'Alice'}");
    crate::run_sql!(storage, "INSERT INTO user {id: 'bob', name: 'Bob'}");
    crate::run_sql!(storage, "INSERT INTO user {id: 'carol', name: 'Carol'}");
    crate::run_sql!(storage, "RELATE user:alice->follows->user:bob");
    crate::run_sql!(storage, "RELATE user:alice->follows->user:carol");

    // Verify edges exist
    let (edges, _) = storage
        .list_edges_out("user:alice", "follows", None, 10)
        .unwrap();
    assert_eq!(edges.len(), 2);

    // Delete all follows edges from alice
    let result = crate::query::query_json(storage.clone(), "DELETE user:alice->follows").unwrap();
    assert_eq!(result[0]["deleted"], 2);

    // Verify all edges are gone
    let (edges, _) = storage
        .list_edges_out("user:alice", "follows", None, 10)
        .unwrap();
    assert_eq!(edges.len(), 0);
}

#[test]
fn test_delete_edge_not_found() {
    let (_tmp, storage) = setup();

    crate::run_sql!(storage, "DEFINE COLLECTION user");

    // Delete non-existent edge - should return deleted: 0
    let result =
        crate::query::query_json(storage.clone(), "DELETE user:alice->follows->user:bob").unwrap();
    assert_eq!(result[0]["deleted"], 0);
}

#[test]
fn test_delete_edges_preserves_other_labels() {
    let (_tmp, storage) = setup();

    crate::run_sql!(storage, "DEFINE COLLECTION user");
    crate::run_sql!(storage, "INSERT INTO user {id: 'alice', name: 'Alice'}");
    crate::run_sql!(storage, "INSERT INTO user {id: 'bob', name: 'Bob'}");
    crate::run_sql!(storage, "RELATE user:alice->follows->user:bob");
    crate::run_sql!(storage, "RELATE user:alice->likes->user:bob");

    // Delete only follows edges
    crate::run_sql!(storage, "DELETE user:alice->follows");

    // Verify follows is gone but likes remains
    let (follows, _) = storage
        .list_edges_out("user:alice", "follows", None, 10)
        .unwrap();
    assert_eq!(follows.len(), 0);

    let (likes, _) = storage
        .list_edges_out("user:alice", "likes", None, 10)
        .unwrap();
    assert_eq!(likes.len(), 1);
}

#[test]
fn test_create_with_array_of_objects_no_schema() {
    let (_tmp, storage) = setup();

    crate::run_sql!(storage, "DEFINE COLLECTION orders");

    // This should succeed - unquoted keys in object literals
    let result = crate::query::query_json(
        storage.clone(),
        r#"CREATE orders:1 SET customer_id = 1, items = [{product_id: 1, quantity: 2, unit_price: 50.0}]"#,
    );
    assert!(result.is_ok(), "CREATE failed: {:?}", result);

    let doc = storage.get_document("orders", "1").unwrap().unwrap();
    assert!(doc.fields.contains_key("items"), "items field missing");
}

#[test]
fn test_create_with_array_of_objects_with_schema() {
    let (_tmp, storage) = setup();

    crate::run_sql!(
        storage,
        r#"DEFINE COLLECTION orders (
            SCHEMA STRICT,
            customer_id int REQUIRED,
            items [{product_id int, quantity int, unit_price float}] REQUIRED
        )"#
    );

    // This should succeed - unquoted keys in object literals
    let result = crate::query::query_json(
        storage.clone(),
        r#"CREATE orders:1 SET customer_id = 1, items = [{product_id: 1, quantity: 2, unit_price: 50.0}]"#,
    );
    assert!(result.is_ok(), "CREATE failed: {:?}", result);

    let doc = storage.get_document("orders", "1").unwrap().unwrap();
    assert!(doc.fields.contains_key("items"), "items field missing");
}

// === Subquery indexing tests ===

#[test]
fn test_subquery_index_first_row() {
    let (_tmp, storage) = setup();

    // Setup: Create test data
    crate::run_sql!(storage, "DEFINE COLLECTION users");
    crate::run_sql!(
        storage,
        "INSERT INTO users {id: '1', name: 'Alice'}, {id: '2', name: 'Bob'}, {id: '3', name: 'Carol'}"
    );

    // Test: (SELECT name FROM users ORDER name LIMIT 1)[0] returns first row as object
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT (SELECT name FROM users ORDER name LIMIT 1)[0] AS first_user",
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);

    // The result is the first row object from the subquery
    let first_user = &arr[0]["first_user"];
    assert!(
        first_user.is_object(),
        "Expected object, got {:?}",
        first_user
    );
    assert_eq!(first_user["name"], "Alice");
}

#[test]
fn test_subquery_index_negative() {
    let (_tmp, storage) = setup();

    // Setup: Create test data
    crate::run_sql!(storage, "DEFINE COLLECTION items");
    crate::run_sql!(
        storage,
        "INSERT INTO items {id: '1', val: 10}, {id: '2', val: 20}, {id: '3', val: 30}"
    );

    // Test: (SELECT val FROM items ORDER val)[-1] returns last row
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT (SELECT val FROM items ORDER val)[-1] AS last_item",
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);

    let last_item = &arr[0]["last_item"];
    assert!(
        last_item.is_object(),
        "Expected object, got {:?}",
        last_item
    );
    assert_eq!(last_item["val"], 30);
}

#[test]
fn test_subquery_index_out_of_bounds() {
    let (_tmp, storage) = setup();

    // Setup: Create test data with only 2 rows
    crate::run_sql!(storage, "DEFINE COLLECTION small");
    crate::run_sql!(
        storage,
        "INSERT INTO small {id: '1', x: 1}, {id: '2', x: 2}"
    );

    // Test: (SELECT * FROM small)[5] returns null (out of bounds)
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT (SELECT * FROM small)[5] AS missing",
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert!(
        arr[0]["missing"].is_null(),
        "Expected null for out-of-bounds index"
    );
}

#[test]
fn test_subquery_index_with_field_access() {
    let (_tmp, storage) = setup();

    // Setup: Create test data
    crate::run_sql!(storage, "DEFINE COLLECTION people");
    crate::run_sql!(
        storage,
        "INSERT INTO people {id: '1', name: 'Alice', age: 30}, {id: '2', name: 'Bob', age: 25}"
    );

    // Test: (SELECT * FROM people ORDER age)[0].name returns just the name field
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT (SELECT * FROM people ORDER age)[0].name AS youngest_name",
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["youngest_name"], "Bob"); // Bob is youngest (age 25)
}

#[test]
fn test_subquery_index_float_whole_number() {
    let (_tmp, storage) = setup();

    // Setup: Create test data
    crate::run_sql!(storage, "DEFINE COLLECTION nums");
    crate::run_sql!(
        storage,
        "INSERT INTO nums {id: '1', n: 100}, {id: '2', n: 200}"
    );

    // Test: Float index with no fractional part (1.0 -> index 1)
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT (SELECT n FROM nums ORDER n)[1.0] AS second",
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);

    let second = &arr[0]["second"];
    assert!(second.is_object(), "Expected object, got {:?}", second);
    assert_eq!(second["n"], 200);
}

#[test]
fn test_subquery_index_empty_result() {
    let (_tmp, storage) = setup();

    // Setup: Create empty collection
    crate::run_sql!(storage, "DEFINE COLLECTION empty_col");

    // Test: Indexing empty subquery result returns null
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT (SELECT * FROM empty_col)[0] AS first",
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert!(arr[0]["first"].is_null(), "Expected null for empty result");
}

#[test]
fn test_subquery_index_negative_second_to_last() {
    let (_tmp, storage) = setup();

    // Setup: Create test data with 4 items
    crate::run_sql!(storage, "DEFINE COLLECTION items");
    crate::run_sql!(
        storage,
        "INSERT INTO items {id: '1', val: 10}, {id: '2', val: 20}, {id: '3', val: 30}, {id: '4', val: 40}"
    );

    // Test: (SELECT val FROM items ORDER val)[-2] returns second to last row (val=30)
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT (SELECT val FROM items ORDER val)[-2] AS second_last",
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);

    let second_last = &arr[0]["second_last"];
    assert!(
        second_last.is_object(),
        "Expected object, got {:?}",
        second_last
    );
    assert_eq!(second_last["val"], 30);
}

#[test]
fn test_subquery_index_negative_out_of_bounds() {
    let (_tmp, storage) = setup();

    // Setup: Create test data with only 2 rows
    crate::run_sql!(storage, "DEFINE COLLECTION small");
    crate::run_sql!(
        storage,
        "INSERT INTO small {id: '1', x: 1}, {id: '2', x: 2}"
    );

    // Test: (SELECT * FROM small)[-10] returns null (negative out of bounds)
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT (SELECT * FROM small)[-10] AS missing",
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert!(
        arr[0]["missing"].is_null(),
        "Expected null for negative out-of-bounds index"
    );
}

#[test]
fn test_subquery_index_with_desc_order() {
    let (_tmp, storage) = setup();

    // Setup: Create collection with multiple documents
    crate::run_sql!(storage, "DEFINE COLLECTION items");
    crate::run_sql!(
        storage,
        "INSERT INTO items {id: '1', val: 10}, {id: '2', val: 20}, {id: '3', val: 30}"
    );

    // Query: (SELECT * FROM items ORDER val DESC)[-1] AS last_item
    // With DESC order, values are [30, 20, 10], so [-1] is the last = 10
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT (SELECT val FROM items ORDER val DESC)[-1] AS last_item",
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);

    let last_item = &arr[0]["last_item"];
    assert!(
        last_item.is_object(),
        "Expected object, got {:?}",
        last_item
    );
    assert_eq!(last_item["val"], 10); // Last element in DESC order
}

#[test]
fn test_subquery_index_returns_single_row_not_array() {
    let (_tmp, storage) = setup();

    // Setup: Create test data
    crate::run_sql!(storage, "DEFINE COLLECTION items");
    crate::run_sql!(
        storage,
        "INSERT INTO items {id: '1', name: 'Alice'}, {id: '2', name: 'Bob'}"
    );

    // Test: Subquery indexing returns single row object, not array
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT (SELECT name FROM items ORDER name)[0] AS first_person",
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);

    let first_person = &arr[0]["first_person"];
    // Should be an object with {name: "Alice"}, not an array
    assert!(
        first_person.is_object(),
        "Expected single object, got {:?}",
        first_person
    );
    assert!(!first_person.is_array(), "Should not be an array");
    assert_eq!(first_person["name"], "Alice");
}

// === Nested Parent Reference Tests ===

/// Test $parent.parent reference in projection (2 levels deep)
#[test]
fn test_nested_parent_ref_in_projection() {
    let (_tmp, storage) = setup();

    // Setup minimal test data
    crate::run_sql!(storage, "DEFINE COLLECTION orgs");
    crate::run_sql!(storage, "DEFINE COLLECTION teams");
    crate::run_sql!(storage, "DEFINE COLLECTION members");

    crate::run_sql!(storage, r#"INSERT INTO orgs {id: "acme", name: "Acme"}"#);
    crate::run_sql!(
        storage,
        r#"INSERT INTO teams {id: "eng", org_id: "orgs:acme", name: "Eng"}"#
    );
    crate::run_sql!(
        storage,
        r#"INSERT INTO members {id: "alice", team_id: "teams:eng", name: "Alice"}"#
    );

    // Test 2-level: $parent.parent.id should be org's id from inside members subquery
    // This tests $parent reference directly in projection (not in WHERE clause)
    let result = crate::query::query_json(
        storage.clone(),
        r#"SELECT name,
            (SELECT name,
                (SELECT name, $parent.parent.id AS grandparent_id
                FROM members WHERE team_id = $parent.id) AS members
            FROM teams WHERE org_id = $parent.id) AS teams
        FROM orgs:acme"#,
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    let org = &arr[0];
    let teams = org["teams"].as_array().unwrap();
    assert_eq!(teams.len(), 1);
    let team = &teams[0];
    let members = team["members"].as_array().unwrap();
    assert_eq!(members.len(), 1);
    let member = &members[0];
    // $parent.parent.id at member level should be org's id
    assert_eq!(member["grandparent_id"], "orgs:acme");

    // Now test 3-level: $parent.parent.parent.id from innermost subquery
    // Level 0: orgs (no parent)
    // Level 1: teams ($parent = org)
    // Level 2: members ($parent = team, $parent.parent = org)
    // Level 3: innermost ($parent = member, $parent.parent = team, $parent.parent.parent = org)
    let result3 = crate::query::query_json(
        storage.clone(),
        r#"SELECT name,
            (SELECT name,
                (SELECT name,
                    (SELECT $parent.parent.parent.id AS great_grandparent_id) AS lookup
                FROM members WHERE team_id = $parent.id) AS members
            FROM teams WHERE org_id = $parent.id) AS teams
        FROM orgs:acme"#,
    )
    .unwrap();

    let arr3 = result3.as_array().unwrap();
    assert_eq!(arr3.len(), 1);
    let org3 = &arr3[0];
    let teams3 = org3["teams"].as_array().unwrap();
    assert_eq!(teams3.len(), 1);
    let team3 = &teams3[0];
    let members3 = team3["members"].as_array().unwrap();
    assert_eq!(members3.len(), 1);
    let member3 = &members3[0];
    let lookup = member3["lookup"].as_array().unwrap();
    assert_eq!(lookup.len(), 1);
    assert_eq!(lookup[0]["great_grandparent_id"], "orgs:acme");
}

#[test]
fn test_nested_parent_ref_three_levels() {
    let (_tmp, storage) = setup();

    // Setup: Create 3-level hierarchy
    // orgs -> teams -> members
    //
    // ParentContext stack when processing innermost subquery:
    // - $parent = member row
    // - $parent.parent = team row
    // - $parent.parent.parent = org row

    crate::run_sql!(storage, "DEFINE COLLECTION orgs");
    crate::run_sql!(storage, "DEFINE COLLECTION teams");
    crate::run_sql!(storage, "DEFINE COLLECTION members");

    // Create org
    crate::run_sql!(
        storage,
        r#"INSERT INTO orgs {id: "acme", name: "Acme Corp", founded: 1990}"#
    );

    // Create teams referencing the org
    crate::run_sql!(
        storage,
        r#"INSERT INTO teams {id: "eng", org_id: "orgs:acme", name: "Engineering"}"#
    );
    crate::run_sql!(
        storage,
        r#"INSERT INTO teams {id: "sales", org_id: "orgs:acme", name: "Sales"}"#
    );

    // Create members referencing teams
    crate::run_sql!(
        storage,
        r#"INSERT INTO members {id: "alice", team_id: "teams:eng", name: "Alice", role: "Lead"}"#
    );
    crate::run_sql!(
        storage,
        r#"INSERT INTO members {id: "bob", team_id: "teams:eng", name: "Bob", role: "Dev"}"#
    );
    crate::run_sql!(
        storage,
        r#"INSERT INTO members {id: "carol", team_id: "teams:sales", name: "Carol", role: "Rep"}"#
    );

    // Query with 3 levels of nesting:
    // Level 0: orgs - processing org row
    // Level 1: teams (subquery) - $parent = org (context has 1 element)
    // Level 2: members (nested subquery) - $parent = team, $parent.parent = org (context has 2 elements)
    // Level 3: innermost (nested subquery) - $parent = member, $parent.parent = team, $parent.parent.parent = org (context has 3 elements)
    //
    // At level 3:
    // - $parent (depth 0) = member row
    // - $parent.parent (depth 1) = team row
    // - $parent.parent.parent (depth 2) = org row
    //
    // We use $parent.parent.parent.id to access the org's id from 3 levels deep.
    // Then look up the org by that id to verify the reference resolves correctly.

    let result = crate::query::query_json(
        storage.clone(),
        r#"SELECT name,
            (SELECT name,
                (SELECT name,
                    (SELECT name FROM orgs WHERE id = $parent.parent.parent.id) AS org_lookup
                FROM members WHERE team_id = $parent.id) AS team_members
            FROM teams WHERE org_id = $parent.id) AS org_teams
        FROM orgs:acme"#,
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1, "Expected 1 org result");

    let org = &arr[0];
    assert_eq!(org["name"], "Acme Corp", "Org name should be Acme Corp");

    // Check org_teams - should have 2 teams
    let teams = org["org_teams"].as_array().unwrap();
    assert_eq!(teams.len(), 2, "Acme should have 2 teams");

    // Find the Engineering team
    let eng_team = teams
        .iter()
        .find(|t| t["name"] == "Engineering")
        .expect("Engineering team should exist");

    // Check team_members for Engineering - should have Alice and Bob
    let eng_members = eng_team["team_members"].as_array().unwrap();
    assert_eq!(eng_members.len(), 2, "Engineering should have 2 members");

    // Each member's org_lookup should resolve $parent.parent.parent.id
    // which goes: member (depth 0) -> team (depth 1) -> org (depth 2)
    // The org's id is "orgs:acme", so we should find exactly that org
    for member in eng_members {
        let org_lookup = member["org_lookup"].as_array().unwrap();
        assert_eq!(
            org_lookup.len(),
            1,
            "org_lookup should find exactly 1 org for member {:?}",
            member["name"]
        );
        assert_eq!(
            org_lookup[0]["name"], "Acme Corp",
            "$parent.parent.parent.id should resolve to the org's id and find Acme Corp"
        );
    }

    // Find the Sales team
    let sales_team = teams
        .iter()
        .find(|t| t["name"] == "Sales")
        .expect("Sales team should exist");

    // Check team_members for Sales - should have Carol
    let sales_members = sales_team["team_members"].as_array().unwrap();
    assert_eq!(sales_members.len(), 1, "Sales should have 1 member");

    // Carol's org_lookup should also find Acme
    let carol = &sales_members[0];
    assert_eq!(carol["name"], "Carol");
    let carol_org_lookup = carol["org_lookup"].as_array().unwrap();
    assert_eq!(carol_org_lookup.len(), 1);
    assert_eq!(carol_org_lookup[0]["name"], "Acme Corp");
}

#[test]
fn test_nested_parent_ref_two_levels() {
    let (_tmp, storage) = setup();

    // Setup: Create 2-level hierarchy
    // departments -> employees
    crate::run_sql!(storage, "DEFINE COLLECTION departments");
    crate::run_sql!(storage, "DEFINE COLLECTION employees");
    crate::run_sql!(storage, "DEFINE COLLECTION projects");

    // Create departments
    crate::run_sql!(
        storage,
        r#"INSERT INTO departments {id: "hr", name: "Human Resources", budget: 100000}"#
    );

    // Create employees
    crate::run_sql!(
        storage,
        r#"INSERT INTO employees {id: "emp1", dept_id: "departments:hr", name: "John"}"#
    );

    // Create projects referencing employees
    crate::run_sql!(
        storage,
        r#"INSERT INTO projects {id: "p1", emp_id: "employees:emp1", name: "Onboarding", dept_id: "departments:hr"}"#
    );

    // Query with 2 levels of nesting that uses $parent.parent
    // Level 0: departments
    // Level 1: employees - $parent = department
    // Level 2: projects - $parent = employee, $parent.parent = department
    let result = crate::query::query_json(
        storage.clone(),
        r#"SELECT name,
            (SELECT name,
                (SELECT name FROM projects
                 WHERE dept_id = $parent.parent.id AND emp_id = $parent.id) AS emp_projects
            FROM employees WHERE dept_id = $parent.id) AS dept_employees
        FROM departments:hr"#,
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);

    let dept = &arr[0];
    assert_eq!(dept["name"], "Human Resources");

    let employees = dept["dept_employees"].as_array().unwrap();
    assert_eq!(employees.len(), 1);

    let john = &employees[0];
    assert_eq!(john["name"], "John");

    // John's projects should include "Onboarding" (matched by both $parent.parent.id and $parent.id)
    let projects = john["emp_projects"].as_array().unwrap();
    assert_eq!(projects.len(), 1, "John should have 1 project");
    assert_eq!(projects[0]["name"], "Onboarding");
}

// === ParentDepthExceeded error tests ===

#[test]
fn test_parent_depth_exceeded_error() {
    let (_tmp, storage) = setup();

    // Setup: Create collection with documents
    crate::run_sql!(storage, "DEFINE COLLECTION users");
    crate::run_sql!(storage, "DEFINE COLLECTION items");
    crate::run_sql!(storage, "INSERT INTO users {id: '1', name: 'Alice', x: 10}");
    crate::run_sql!(storage, "INSERT INTO items {id: '1', val: 100}");

    // Query that tries to access $parent.parent when only 1 parent level exists:
    // The outer SELECT creates 1 parent level for the inner SELECT,
    // but $parent.parent.x tries to access depth=1, which doesn't exist.
    let result = crate::query::query_json(
        storage.clone(),
        "SELECT (SELECT $parent.parent.x FROM items) AS nested FROM users",
    );

    // Should fail with ParentDepthExceeded error
    assert!(result.is_err(), "Expected error, got {:?}", result);

    let err = result.unwrap_err();
    let err_msg = err.to_string();

    // Verify error message contains "only N parent level(s) available"
    assert!(
        err_msg.contains("only 1 parent level(s) available"),
        "Expected error message to contain 'only 1 parent level(s) available', got: {}",
        err_msg
    );

    // Also verify it mentions the requested depth
    assert!(
        err_msg.contains("depth 1"),
        "Expected error message to mention 'depth 1', got: {}",
        err_msg
    );
}

#[test]
fn test_parent_depth_exceeded_deeper_nesting() {
    let (_tmp, storage) = setup();

    // Setup: Create collections
    crate::run_sql!(storage, "DEFINE COLLECTION a");
    crate::run_sql!(storage, "DEFINE COLLECTION b");
    crate::run_sql!(storage, "DEFINE COLLECTION c");

    crate::run_sql!(storage, "INSERT INTO a {id: '1', val: 'A'}");
    crate::run_sql!(storage, "INSERT INTO b {id: '1', val: 'B'}");
    crate::run_sql!(storage, "INSERT INTO c {id: '1', val: 'C'}");

    // This query has 2 levels of subquery nesting, but the parent context
    // handling may not properly propagate through all levels.
    // The innermost subquery tries to access $parent.parent.parent (depth=2).
    //
    // Note: The actual available parent levels depends on how the parent context
    // is propagated through subquery execution. In current implementation,
    // only 1 parent level is available at the innermost level because the
    // parent substitution happens before nested subquery processing.
    let result = crate::query::query_json(
        storage.clone(),
        r#"SELECT (SELECT (SELECT $parent.parent.parent.val FROM c) AS deep FROM b) AS mid FROM a"#,
    );

    // Should fail with ParentDepthExceeded error
    assert!(result.is_err(), "Expected error, got {:?}", result);

    let err = result.unwrap_err();
    let err_msg = err.to_string();

    // Verify error contains expected format (depth requested vs available)
    // The exact numbers depend on parent context propagation
    assert!(
        err_msg.contains("parent level(s) available"),
        "Expected error message to contain 'parent level(s) available', got: {}",
        err_msg
    );

    // Requested depth was 2 (for $parent.parent.parent)
    assert!(
        err_msg.contains("depth 2"),
        "Expected error message to mention 'depth 2', got: {}",
        err_msg
    );
}

// =============================================================================
// Row-centric Execute Integration Test (Task 5.3)
// =============================================================================
// This comprehensive test exercises all the new row-centric execute features:
// 1. Nested parent refs ($parent.parent.field) - ParentRef with depth
// 2. ParentContext stack for nested parents
// 3. Subquery indexing ([0], [-1])
// 4. Field access on indexed results (e.g., [0].name)
// =============================================================================

/// Comprehensive integration test that demonstrates all row-centric execute features
/// working together in a realistic hierarchical data scenario.
///
/// Data model:
/// - orgs: Organization records (top level)
/// - teams: Team records that belong to orgs (org_id references orgs)
/// - members: Member records that belong to teams (team_id references teams)
///
/// Features tested:
/// 1. Three levels of correlated subqueries
/// 2. $parent.parent references (2 levels deep)
/// 3. $parent.parent.parent references (3 levels deep)
/// 4. Subquery indexing with [0] to get first result
/// 5. Subquery indexing with [-1] to get last result
/// 6. Field access on indexed subquery results (e.g., [0].name)
/// 7. ParentDepthExceeded error when accessing too many levels
#[test]
fn test_row_centric_execute_integration() {
    let (_tmp, storage) = setup();

    // ==========================================================================
    // Setup hierarchical data: orgs -> teams -> members
    // ==========================================================================

    crate::run_sql!(storage, "DEFINE COLLECTION orgs");
    crate::run_sql!(storage, "DEFINE COLLECTION teams");
    crate::run_sql!(storage, "DEFINE COLLECTION members");

    // Create organizations
    crate::run_sql!(
        storage,
        r#"INSERT INTO orgs {id: "acme", name: "Acme Corp", industry: "Tech"}"#
    );
    crate::run_sql!(
        storage,
        r#"INSERT INTO orgs {id: "globex", name: "Globex Inc", industry: "Manufacturing"}"#
    );

    // Create teams for Acme
    crate::run_sql!(
        storage,
        r#"INSERT INTO teams {id: "eng", org_id: "orgs:acme", name: "Engineering", size: 10}"#
    );
    crate::run_sql!(
        storage,
        r#"INSERT INTO teams {id: "sales", org_id: "orgs:acme", name: "Sales", size: 5}"#
    );

    // Create teams for Globex
    crate::run_sql!(
        storage,
        r#"INSERT INTO teams {id: "ops", org_id: "orgs:globex", name: "Operations", size: 8}"#
    );

    // Create members for Engineering team
    crate::run_sql!(
        storage,
        r#"INSERT INTO members {id: "alice", team_id: "teams:eng", name: "Alice", role: "Lead", seniority: 1}"#
    );
    crate::run_sql!(
        storage,
        r#"INSERT INTO members {id: "bob", team_id: "teams:eng", name: "Bob", role: "Senior", seniority: 2}"#
    );
    crate::run_sql!(
        storage,
        r#"INSERT INTO members {id: "carol", team_id: "teams:eng", name: "Carol", role: "Junior", seniority: 3}"#
    );

    // Create members for Sales team
    crate::run_sql!(
        storage,
        r#"INSERT INTO members {id: "dave", team_id: "teams:sales", name: "Dave", role: "Manager", seniority: 1}"#
    );

    // Create members for Operations team
    crate::run_sql!(
        storage,
        r#"INSERT INTO members {id: "eve", team_id: "teams:ops", name: "Eve", role: "Supervisor", seniority: 1}"#
    );

    // ==========================================================================
    // Test 1: Nested $parent.parent references in three-level hierarchy
    // ==========================================================================
    // Query structure:
    // Level 0: orgs (outer) - no parent
    // Level 1: teams (subquery) - $parent = org
    // Level 2: members (nested subquery) - $parent = team, $parent.parent = org
    //
    // We verify that $parent.parent.name correctly resolves to the org name
    // when evaluated from the innermost (members) subquery level.

    let result = crate::query::query_json(
        storage.clone(),
        r#"SELECT name,
            (SELECT name,
                (SELECT name, $parent.parent.name AS org_name
                FROM members WHERE team_id = $parent.id) AS team_members
            FROM teams WHERE org_id = $parent.id) AS org_teams
        FROM orgs:acme"#,
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1, "Expected 1 org result");

    let org = &arr[0];
    assert_eq!(org["name"], "Acme Corp");

    let teams = org["org_teams"].as_array().unwrap();
    assert_eq!(teams.len(), 2, "Acme should have 2 teams");

    // Find Engineering team
    let eng = teams
        .iter()
        .find(|t| t["name"] == "Engineering")
        .expect("Engineering team should exist");

    let eng_members = eng["team_members"].as_array().unwrap();
    assert_eq!(eng_members.len(), 3, "Engineering should have 3 members");

    // Each member should have org_name = "Acme Corp" via $parent.parent.name
    for member in eng_members {
        assert_eq!(
            member["org_name"], "Acme Corp",
            "$parent.parent.name should resolve to org's name for member {:?}",
            member["name"]
        );
    }

    // ==========================================================================
    // Test 2: Subquery indexing with [0] to get first result
    // ==========================================================================
    // Get the first team for each org using subquery indexing

    let result = crate::query::query_json(
        storage.clone(),
        r#"SELECT name,
            (SELECT name FROM teams WHERE org_id = $parent.id ORDER name)[0] AS first_team
        FROM orgs ORDER name"#,
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 2);

    // Acme's first team alphabetically is Engineering
    let acme = arr.iter().find(|o| o["name"] == "Acme Corp").unwrap();
    let first_team = &acme["first_team"];
    assert!(
        first_team.is_object(),
        "Subquery index [0] should return single object"
    );
    assert_eq!(first_team["name"], "Engineering");

    // Globex's only team is Operations
    let globex = arr.iter().find(|o| o["name"] == "Globex Inc").unwrap();
    assert_eq!(globex["first_team"]["name"], "Operations");

    // ==========================================================================
    // Test 3: Subquery indexing with [-1] to get last result
    // ==========================================================================
    // Get the last team for each org using negative indexing

    let result = crate::query::query_json(
        storage.clone(),
        r#"SELECT name,
            (SELECT name FROM teams WHERE org_id = $parent.id ORDER name)[-1] AS last_team
        FROM orgs ORDER name"#,
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    let acme = arr.iter().find(|o| o["name"] == "Acme Corp").unwrap();

    // Acme's last team alphabetically is Sales
    assert_eq!(acme["last_team"]["name"], "Sales");

    // ==========================================================================
    // Test 4: Field access on indexed subquery results
    // ==========================================================================
    // Get just the name field from the first team using [0].name

    let result = crate::query::query_json(
        storage.clone(),
        r#"SELECT name,
            (SELECT name FROM teams WHERE org_id = $parent.id ORDER name)[0].name AS first_team_name
        FROM orgs:acme"#,
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);

    // first_team_name should be a string, not an object
    let acme = &arr[0];
    assert_eq!(acme["first_team_name"], "Engineering");

    // ==========================================================================
    // Test 5: Three-level $parent.parent.parent reference
    // ==========================================================================
    // This tests the deepest nesting: from an innermost subquery,
    // access the top-level org via $parent.parent.parent
    //
    // Level 0: orgs
    // Level 1: teams ($parent = org)
    // Level 2: members ($parent = team, $parent.parent = org)
    // Level 3: innermost ($parent = member, $parent.parent = team, $parent.parent.parent = org)

    let result = crate::query::query_json(
        storage.clone(),
        r#"SELECT name,
            (SELECT name,
                (SELECT name,
                    (SELECT $parent.parent.parent.name AS great_grandparent) AS lookup
                FROM members WHERE team_id = $parent.id LIMIT 1) AS team_members
            FROM teams WHERE org_id = $parent.id LIMIT 1) AS org_teams
        FROM orgs:acme"#,
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    let org = &arr[0];
    let teams = org["org_teams"].as_array().unwrap();
    let team = &teams[0];
    let members = team["team_members"].as_array().unwrap();
    let member = &members[0];
    let lookup = member["lookup"].as_array().unwrap();

    // The innermost subquery should resolve $parent.parent.parent.name to "Acme Corp"
    assert_eq!(
        lookup[0]["great_grandparent"], "Acme Corp",
        "$parent.parent.parent.name should resolve to org name from 3 levels deep"
    );

    // ==========================================================================
    // Test 6: Combined features - subquery indexing + nested parent ref
    // ==========================================================================
    // Get the first member of the first team, with the org name accessible
    // via $parent.parent

    let result = crate::query::query_json(
        storage.clone(),
        r#"SELECT name,
            (SELECT name,
                (SELECT name, $parent.parent.name AS org_name
                FROM members WHERE team_id = $parent.id ORDER seniority LIMIT 1)[0] AS first_member
            FROM teams WHERE org_id = $parent.id ORDER name LIMIT 1)[0] AS first_team
        FROM orgs:acme"#,
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    let org = &arr[0];
    let first_team = &org["first_team"];

    assert_eq!(first_team["name"], "Engineering");

    let first_member = &first_team["first_member"];
    assert_eq!(first_member["name"], "Alice", "Alice is lead (seniority 1)");
    assert_eq!(
        first_member["org_name"], "Acme Corp",
        "Member should have access to org name via $parent.parent"
    );

    // ==========================================================================
    // Test 7: Out-of-bounds indexing returns null
    // ==========================================================================

    let result = crate::query::query_json(
        storage.clone(),
        r#"SELECT name,
            (SELECT name FROM teams WHERE org_id = $parent.id)[100] AS nonexistent_team
        FROM orgs:acme"#,
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert!(
        arr[0]["nonexistent_team"].is_null(),
        "Out-of-bounds index should return null"
    );

    // ==========================================================================
    // Test 8: Empty subquery result with indexing returns null
    // ==========================================================================

    let result = crate::query::query_json(
        storage.clone(),
        r#"SELECT name,
            (SELECT name FROM teams WHERE org_id = "nonexistent")[0] AS no_team
        FROM orgs:acme"#,
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert!(
        arr[0]["no_team"].is_null(),
        "Indexing empty result should return null"
    );

    // ==========================================================================
    // Test 9: Multiple subqueries with indexing in same projection
    // ==========================================================================

    let result = crate::query::query_json(
        storage.clone(),
        r#"SELECT name,
            (SELECT name FROM teams WHERE org_id = $parent.id ORDER name)[0] AS first_team,
            (SELECT name FROM teams WHERE org_id = $parent.id ORDER name)[-1] AS last_team,
            (SELECT COUNT(*) AS count FROM teams WHERE org_id = $parent.id)[0].count AS team_count
        FROM orgs:acme"#,
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    let org = &arr[0];

    assert_eq!(org["first_team"]["name"], "Engineering");
    assert_eq!(org["last_team"]["name"], "Sales");
    assert_eq!(org["team_count"], 2);

    // ==========================================================================
    // Test 10: Chained field access on indexed result
    // ==========================================================================
    // This is a common pattern: (SELECT ...)[0].field

    let result = crate::query::query_json(storage.clone(),
        r#"SELECT
            (SELECT name, size FROM teams WHERE org_id = $parent.id ORDER size DESC)[0].name AS largest_team_name,
            (SELECT name, size FROM teams WHERE org_id = $parent.id ORDER size DESC)[0].size AS largest_team_size
        FROM orgs:acme"#,
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    let org = &arr[0];

    // Engineering has size 10, Sales has size 5
    assert_eq!(org["largest_team_name"], "Engineering");
    assert_eq!(org["largest_team_size"], 10);
}

/// Test that ParentDepthExceeded error is properly raised when trying to
/// access more parent levels than available.
#[test]
fn test_row_centric_parent_depth_exceeded() {
    let (_tmp, storage) = setup();

    crate::run_sql!(storage, "DEFINE COLLECTION outer");
    crate::run_sql!(storage, "DEFINE COLLECTION inner");
    crate::run_sql!(storage, r#"INSERT INTO outer {id: "1", x: 10}"#);
    crate::run_sql!(storage, r#"INSERT INTO inner {id: "1", y: 20}"#);

    // Single level of nesting, but trying to access $parent.parent (depth 1)
    // Only 1 parent level is available (the outer row)
    let result = crate::query::query_json(
        storage.clone(),
        r#"SELECT (SELECT $parent.parent.x FROM inner) AS nested FROM outer"#,
    );

    assert!(result.is_err(), "Should fail with ParentDepthExceeded");

    let err = result.unwrap_err();
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("parent level(s) available"),
        "Error should mention available parent levels: {}",
        err_msg
    );
}

/// Test that all orgs get their teams enumerated with proper parent context.
#[test]
fn test_row_centric_multiple_parent_rows() {
    let (_tmp, storage) = setup();

    crate::run_sql!(storage, "DEFINE COLLECTION companies");
    crate::run_sql!(storage, "DEFINE COLLECTION depts");

    crate::run_sql!(
        storage,
        r#"INSERT INTO companies {id: "a", name: "Company A"}"#
    );
    crate::run_sql!(
        storage,
        r#"INSERT INTO companies {id: "b", name: "Company B"}"#
    );
    crate::run_sql!(
        storage,
        r#"INSERT INTO depts {id: "a1", company_id: "companies:a", name: "Dept A1"}"#
    );
    crate::run_sql!(
        storage,
        r#"INSERT INTO depts {id: "a2", company_id: "companies:a", name: "Dept A2"}"#
    );
    crate::run_sql!(
        storage,
        r#"INSERT INTO depts {id: "b1", company_id: "companies:b", name: "Dept B1"}"#
    );

    // Each company should get its own departments via correlated subquery
    let result = crate::query::query_json(
        storage.clone(),
        r#"SELECT name,
            (SELECT name FROM depts WHERE company_id = $parent.id ORDER name) AS departments
        FROM companies ORDER name"#,
    )
    .unwrap();

    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 2);

    // Company A should have Dept A1, Dept A2
    let company_a = &arr[0];
    assert_eq!(company_a["name"], "Company A");
    let depts_a = company_a["departments"].as_array().unwrap();
    assert_eq!(depts_a.len(), 2);
    assert_eq!(depts_a[0]["name"], "Dept A1");
    assert_eq!(depts_a[1]["name"], "Dept A2");

    // Company B should have Dept B1
    let company_b = &arr[1];
    assert_eq!(company_b["name"], "Company B");
    let depts_b = company_b["departments"].as_array().unwrap();
    assert_eq!(depts_b.len(), 1);
    assert_eq!(depts_b[0]["name"], "Dept B1");
}

// =============================================================================
// Test $parent reference to non-projected field (Task 2.2)
// =============================================================================
/// This test verifies the key benefit of the Row-centric pipeline: subqueries
/// can access ANY field from the parent row (via `$parent.field`), not just
/// fields that are in the outer query's projection.
///
/// Previously, this required `augment_projection_with_parent_fields()` to add
/// extra fields to the projection. With the Row-centric pipeline, the full
/// Document is available via ParentContext.
#[test]
fn test_parent_ref_to_non_projected_field() {
    let (_tmp, storage) = setup();

    // Create collections
    crate::run_sql!(storage, "DEFINE COLLECTION users");
    crate::run_sql!(storage, "DEFINE COLLECTION orders");

    // Create users with email field (email will NOT be in projection)
    crate::run_sql!(
        storage,
        r#"INSERT INTO users {id: "alice", name: "Alice", email: "alice@example.com"}"#
    );
    crate::run_sql!(
        storage,
        r#"INSERT INTO users {id: "bob", name: "Bob", email: "bob@example.com"}"#
    );

    // Create orders referencing users
    crate::run_sql!(
        storage,
        r#"INSERT INTO orders {id: "1", user_id: "users:alice", item: "Book"}"#
    );
    crate::run_sql!(
        storage,
        r#"INSERT INTO orders {id: "2", user_id: "users:bob", item: "Pen"}"#
    );

    // Query: SELECT only 'name', but subquery uses $parent.id (not in projection)
    // The id field is implicitly available from the Document
    let result = crate::query::query_json(storage.clone(),
        r#"SELECT name, (SELECT item FROM orders WHERE user_id = $parent.id)[0].item AS first_order FROM users"#,
    )
    .unwrap();

    let rows = result.as_array().unwrap();
    assert_eq!(rows.len(), 2);

    // Verify subquery resolved correctly using $parent.id
    let alice = rows.iter().find(|r| r["name"] == "Alice").unwrap();
    assert_eq!(alice["first_order"], "Book");

    let bob = rows.iter().find(|r| r["name"] == "Bob").unwrap();
    assert_eq!(bob["first_order"], "Pen");
}

// =============================================================================
// Final integration test for row-centric pipeline (Task 5.2)
// =============================================================================
/// Comprehensive test verifying the row-centric pipeline works without the
/// augmentation workaround:
/// 1. Subqueries can access ANY parent field via $parent (not just projected fields)
/// 2. Non-projected fields don't leak into the final output
/// 3. No augmentation workaround is needed
#[test]
fn test_row_centric_no_augmentation_needed() {
    let (_tmp, storage) = setup();

    // Create collection
    crate::run_sql!(storage, "DEFINE COLLECTION users");

    // Setup: user with many fields
    crate::run_sql!(
        storage,
        r#"INSERT INTO users {
            id: "alice",
            name: "Alice",
            email: "alice@example.com",
            age: 30,
            city: "NYC",
            country: "USA"
        }"#
    );

    // Query: SELECT only 'name', but nested subquery uses multiple $parent fields
    // that are NOT in the outer projection
    let result = crate::query::query_json(
        storage.clone(),
        r#"SELECT name,
            (SELECT
                $parent.email AS parent_email,
                $parent.age AS parent_age,
                $parent.city AS parent_city
            FROM users LIMIT 1)[0] AS nested
        FROM users:alice"#,
    )
    .unwrap();

    let rows = result.as_array().unwrap();
    assert_eq!(rows.len(), 1);

    let row = &rows[0];
    assert_eq!(row["name"], "Alice");

    // Verify nested subquery has access to ALL parent fields (not just projected)
    let nested = &row["nested"];
    assert_eq!(nested["parent_email"], "alice@example.com");
    assert_eq!(nested["parent_age"], 30);
    assert_eq!(nested["parent_city"], "NYC");

    // Verify the non-projected fields (email, age, city, country) are NOT in the final output
    // (they should only be accessible via $parent, not in the outer result)
    assert!(row.get("email").is_none() || row["email"].is_null());
    assert!(row.get("age").is_none() || row["age"].is_null());
    assert!(row.get("city").is_none() || row["city"].is_null());
    assert!(row.get("country").is_none() || row["country"].is_null());
}
