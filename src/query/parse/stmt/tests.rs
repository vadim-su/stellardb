//! Tests for statement parsing.

use crate::query::ast::{
    AggregateArg, BinaryOp, DataSource, DescribeAst, Expr, OrderDirection, PostfixOp, Projection,
    ProjectionItem, Statement,
};
use crate::query::parse::parse;

#[test]
fn test_parse_select_all() {
    let stmts = parse("SELECT * FROM user").unwrap();
    match &stmts[0] {
        Statement::Select(s) => {
            assert_eq!(s.projection, Projection::All);
            match &s.source {
                DataSource::Collection(t) => {
                    assert_eq!(t.collection, "user");
                    assert_eq!(t.key, None);
                }
                _ => panic!("expected collection source"),
            }
        }
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_select_by_id() {
    let stmts = parse("SELECT * FROM user:alice").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.source {
            DataSource::Collection(t) => {
                assert_eq!(t.collection, "user");
                assert_eq!(t.key, Some("alice".to_string()));
            }
            _ => panic!("expected collection source"),
        },
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_select_field_path() {
    // SELECT * FROM product:1.embedding
    let stmts = parse("SELECT * FROM product:1.embedding").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.source {
            DataSource::FieldPath {
                target,
                path,
                postfix,
            } => {
                assert_eq!(target.collection, "product");
                assert_eq!(target.key, Some("1".to_string()));
                assert_eq!(path, &["embedding"]);
                assert!(postfix.is_empty());
            }
            _ => panic!("expected FieldPath source, got {:?}", s.source),
        },
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_select_field_path_nested() {
    // SELECT * FROM user:alice.profile.settings
    let stmts = parse("SELECT * FROM user:alice.profile.settings").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.source {
            DataSource::FieldPath {
                target,
                path,
                postfix,
            } => {
                assert_eq!(target.collection, "user");
                assert_eq!(target.key, Some("alice".to_string()));
                assert_eq!(path, &["profile", "settings"]);
                assert!(postfix.is_empty());
            }
            _ => panic!("expected FieldPath source, got {:?}", s.source),
        },
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_select_variable_field_access() {
    // SELECT * FROM $test.value
    let stmts = parse("SELECT * FROM $test.value").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.source {
            DataSource::Expr(expr) => match expr.as_ref() {
                Expr::FieldAccess { base, field } => {
                    assert!(matches!(base.as_ref(), Expr::Variable(name) if name == "test"));
                    assert_eq!(field, "value");
                }
                _ => panic!("expected FieldAccess, got {:?}", expr),
            },
            _ => panic!("expected Expr source, got {:?}", s.source),
        },
        _ => panic!("expected select"),
    }

    // SELECT * FROM $test.value.items (nested field access on variable)
    let stmts = parse("SELECT * FROM $test.value.items").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.source {
            DataSource::Expr(expr) => match expr.as_ref() {
                Expr::FieldAccess { base, field } => {
                    assert_eq!(field, "items");
                    match base.as_ref() {
                        Expr::FieldAccess {
                            base: inner_base,
                            field: inner_field,
                        } => {
                            assert!(
                                matches!(inner_base.as_ref(), Expr::Variable(name) if name == "test")
                            );
                            assert_eq!(inner_field, "value");
                        }
                        _ => panic!("expected nested FieldAccess, got {:?}", base),
                    }
                }
                _ => panic!("expected FieldAccess, got {:?}", expr),
            },
            _ => panic!("expected Expr source, got {:?}", s.source),
        },
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_select_field_path_with_slice() {
    // SELECT * FROM product:1.embedding[0..5]
    let stmts = parse("SELECT * FROM product:1.embedding[0..5]").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.source {
            DataSource::FieldPath {
                target,
                path,
                postfix,
            } => {
                assert_eq!(target.collection, "product");
                assert_eq!(target.key, Some("1".to_string()));
                assert_eq!(path, &["embedding"]);
                assert_eq!(postfix.len(), 1);
                assert!(matches!(&postfix[0], PostfixOp::Slice { .. }));
            }
            _ => panic!("expected FieldPath source, got {:?}", s.source),
        },
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_select_with_limit() {
    let stmts = parse("SELECT * FROM user LIMIT 10").unwrap();
    match &stmts[0] {
        Statement::Select(s) => {
            assert_eq!(s.limit, Some(10));
        }
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_select_where_and() {
    let stmts = parse("SELECT * FROM user WHERE age > 18 AND status = 'active'").unwrap();
    match &stmts[0] {
        Statement::Select(s) => {
            assert!(matches!(
                s.filter,
                Some(Expr::BinaryOp {
                    op: BinaryOp::And,
                    ..
                })
            ));
        }
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_select_where_or() {
    let stmts = parse("SELECT * FROM user WHERE city = 'Moscow' OR city = 'SPb'").unwrap();
    match &stmts[0] {
        Statement::Select(s) => {
            assert!(matches!(
                s.filter,
                Some(Expr::BinaryOp {
                    op: BinaryOp::Or,
                    ..
                })
            ));
        }
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_select_where_precedence() {
    let stmts = parse("SELECT * FROM user WHERE a = 1 OR b = 2 AND c = 3").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.filter {
            Some(Expr::BinaryOp {
                left,
                op: BinaryOp::Or,
                right,
            }) => {
                assert!(matches!(
                    **left,
                    Expr::BinaryOp {
                        op: BinaryOp::Eq,
                        ..
                    }
                ));
                assert!(matches!(
                    **right,
                    Expr::BinaryOp {
                        op: BinaryOp::And,
                        ..
                    }
                ));
            }
            _ => panic!("expected OR at top level"),
        },
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_select_where_parens() {
    let stmts = parse("SELECT * FROM user WHERE (a = 1 OR b = 2) AND c = 3").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.filter {
            Some(Expr::BinaryOp {
                left,
                op: BinaryOp::And,
                right,
            }) => {
                assert!(matches!(
                    **left,
                    Expr::BinaryOp {
                        op: BinaryOp::Or,
                        ..
                    }
                ));
                assert!(matches!(
                    **right,
                    Expr::BinaryOp {
                        op: BinaryOp::Eq,
                        ..
                    }
                ));
            }
            _ => panic!("expected AND at top level"),
        },
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_insert_single() {
    let stmts = parse("INSERT INTO user {name: 'Alice', age: 30}").unwrap();
    match &stmts[0] {
        Statement::Insert(i) => {
            assert_eq!(i.collection, "user");
            assert_eq!(i.objects.len(), 1);
        }
        _ => panic!("expected insert"),
    }
}

#[test]
fn test_parse_insert_bulk() {
    let stmts = parse("INSERT INTO user {name: 'Alice'}, {name: 'Bob'}, {name: 'Carol'}").unwrap();
    match &stmts[0] {
        Statement::Insert(i) => {
            assert_eq!(i.objects.len(), 3);
        }
        _ => panic!("expected insert"),
    }
}

#[test]
fn test_parse_create() {
    let stmts = parse("CREATE user SET name = 'Alice', age = 30").unwrap();
    match &stmts[0] {
        Statement::Create(c) => {
            assert_eq!(c.target.collection, "user");
            assert_eq!(c.assignments.len(), 2);
        }
        _ => panic!("expected create"),
    }
}

#[test]
fn test_parse_create_with_array_of_objects() {
    use crate::query::ast::Expr;

    let stmts = parse(
        "CREATE orders:1 SET customer_id = 1, items = [{product_id: 1, quantity: 2, unit_price: 50.0}]",
    )
    .unwrap();
    match &stmts[0] {
        Statement::Create(c) => {
            assert_eq!(c.target.collection, "orders");
            assert_eq!(c.target.key, Some("1".to_string()));
            assert_eq!(c.assignments.len(), 2);

            // Check items assignment
            let items_assignment = &c.assignments[1];
            assert_eq!(items_assignment.path, vec!["items"]);

            // The expression should be Expr::Array([Expr::Object(...)])
            match &items_assignment.expr {
                Expr::Array(items) => {
                    assert_eq!(items.len(), 1, "Expected 1 item in array");
                    match &items[0] {
                        Expr::Object(pairs) => {
                            assert_eq!(pairs.len(), 3, "Expected 3 fields in object");
                            // Check field names
                            let field_names: Vec<&str> =
                                pairs.iter().map(|(k, _)| k.as_str()).collect();
                            assert!(field_names.contains(&"product_id"), "Missing product_id");
                            assert!(field_names.contains(&"quantity"), "Missing quantity");
                            assert!(field_names.contains(&"unit_price"), "Missing unit_price");
                        }
                        other => panic!("Expected Expr::Object inside array, got {:?}", other),
                    }
                }
                other => panic!("Expected Expr::Array for items, got {:?}", other),
            }
        }
        _ => panic!("expected create"),
    }
}

#[test]
fn test_parse_update() {
    let stmts = parse("UPDATE user:alice SET age = 31").unwrap();
    match &stmts[0] {
        Statement::Update(u) => {
            assert_eq!(u.target.collection, "user");
            assert_eq!(u.target.key, Some("alice".to_string()));
        }
        _ => panic!("expected update"),
    }
}

#[test]
fn test_parse_delete() {
    use crate::query::ast::DeleteTarget;
    let stmts = parse("DELETE user:alice").unwrap();
    match &stmts[0] {
        Statement::Delete(d) => match &d.target {
            DeleteTarget::Document(target) => {
                assert_eq!(target.collection, "user");
                assert_eq!(target.key, Some("alice".to_string()));
            }
            _ => panic!("expected document deletion"),
        },
        _ => panic!("expected delete"),
    }
}

#[test]
fn test_parse_delete_edge_specific() {
    use crate::query::ast::DeleteTarget;
    let stmts = parse("DELETE user:alice->follows->user:bob").unwrap();
    match &stmts[0] {
        Statement::Delete(d) => match &d.target {
            DeleteTarget::Edge { from, label, to } => {
                assert_eq!(from.collection, "user");
                assert_eq!(from.key, Some("alice".to_string()));
                assert_eq!(label, "follows");
                let to = to.as_ref().expect("expected 'to' target");
                assert_eq!(to.collection, "user");
                assert_eq!(to.key, Some("bob".to_string()));
            }
            _ => panic!("expected edge deletion"),
        },
        _ => panic!("expected delete"),
    }
}

#[test]
fn test_parse_delete_edge_all_by_label() {
    use crate::query::ast::DeleteTarget;
    let stmts = parse("DELETE user:alice->follows").unwrap();
    match &stmts[0] {
        Statement::Delete(d) => match &d.target {
            DeleteTarget::Edge { from, label, to } => {
                assert_eq!(from.collection, "user");
                assert_eq!(from.key, Some("alice".to_string()));
                assert_eq!(label, "follows");
                assert!(to.is_none(), "expected no 'to' target");
            }
            _ => panic!("expected edge deletion"),
        },
        _ => panic!("expected delete"),
    }
}

#[test]
fn test_parse_begin() {
    let stmts = parse("BEGIN").unwrap();
    assert!(matches!(&stmts[0], Statement::Begin(_)));
}

#[test]
fn test_parse_commit() {
    let stmts = parse("COMMIT").unwrap();
    assert!(matches!(&stmts[0], Statement::Commit(_)));
}

#[test]
fn test_parse_rollback() {
    let stmts = parse("ROLLBACK").unwrap();
    assert!(matches!(&stmts[0], Statement::Rollback(_)));
}

#[test]
fn test_parse_multi_statement() {
    let stmts = parse("SELECT * FROM user; INSERT INTO user {name: 'Alice'}").unwrap();
    assert_eq!(stmts.len(), 2);
    assert!(matches!(stmts[0], Statement::Select(_)));
    assert!(matches!(stmts[1], Statement::Insert(_)));
}

#[test]
fn test_parse_single_statement_returns_vec() {
    let stmts = parse("SELECT * FROM user").unwrap();
    assert_eq!(stmts.len(), 1);
}

#[test]
fn test_parse_trailing_semicolon() {
    let stmts = parse("SELECT * FROM user;").unwrap();
    assert_eq!(stmts.len(), 1);
}

#[test]
fn test_parse_empty_between_semicolons() {
    let stmts = parse("SELECT * FROM user;; SELECT * FROM post").unwrap();
    assert_eq!(stmts.len(), 2);
}

#[test]
fn test_parse_define_collection_strict() {
    let stmts = parse("DEFINE COLLECTION user (SCHEMA STRICT, name string REQUIRED)").unwrap();
    match &stmts[0] {
        Statement::DefineCollection(d) => {
            assert_eq!(d.name, "user");
            assert_eq!(d.mode, crate::schema::SchemaMode::Strict);
            assert_eq!(d.fields.len(), 1);
            assert_eq!(d.fields[0].name, "name");
            assert!(d.fields[0].required);
        }
        _ => panic!("expected DefineCollection"),
    }
}

#[test]
fn test_parse_define_collection_flexible() {
    let stmts = parse("DEFINE COLLECTION user (SCHEMA FLEXIBLE)").unwrap();
    match &stmts[0] {
        Statement::DefineCollection(d) => {
            assert_eq!(d.mode, crate::schema::SchemaMode::Flexible);
        }
        _ => panic!("expected DefineCollection"),
    }
}

#[test]
fn test_parse_define_collection_no_body() {
    // DEFINE COLLECTION without body defaults to FLEXIBLE
    let stmts = parse("DEFINE COLLECTION user").unwrap();
    match &stmts[0] {
        Statement::DefineCollection(d) => {
            assert_eq!(d.name, "user");
            assert_eq!(d.mode, crate::schema::SchemaMode::Flexible);
            assert!(d.fields.is_empty());
            assert!(d.indexes.is_empty());
        }
        _ => panic!("expected DefineCollection"),
    }
}

#[test]
fn test_parse_quoted_collection_name() {
    // Backtick-quoted collection name with special chars
    let stmts = parse("DEFINE COLLECTION `user-data`").unwrap();
    match &stmts[0] {
        Statement::DefineCollection(d) => {
            assert_eq!(d.name, "user-data");
        }
        _ => panic!("expected DefineCollection"),
    }
}

#[test]
fn test_parse_quoted_field_name() {
    // Backtick-quoted field name with special chars
    let stmts = parse("DEFINE COLLECTION test (`field-name` string)").unwrap();
    match &stmts[0] {
        Statement::DefineCollection(d) => {
            assert_eq!(d.fields[0].name, "field-name");
        }
        _ => panic!("expected DefineCollection"),
    }
}

#[test]
fn test_parse_define_with_default() {
    let stmts =
        parse("DEFINE COLLECTION test (name string, status string DEFAULT \"active\")").unwrap();
    match &stmts[0] {
        Statement::DefineCollection(d) => {
            assert_eq!(d.name, "test");
            assert_eq!(d.fields.len(), 2);
            assert_eq!(d.fields[0].name, "name");
            assert!(d.fields[0].default.is_none());
            assert_eq!(d.fields[1].name, "status");
            assert_eq!(
                d.fields[1].default,
                Some(crate::schema::DefaultValue::String("active".to_string()))
            );
        }
        _ => panic!("expected DefineCollection"),
    }
}

#[test]
fn test_parse_drop_quoted_collection() {
    let stmts = parse("DROP COLLECTION `user-data`").unwrap();
    match &stmts[0] {
        Statement::DropCollection(d) => {
            assert_eq!(d.name, "user-data");
        }
        _ => panic!("expected DropCollection"),
    }
}

#[test]
fn test_parse_describe_quoted_collection() {
    let stmts = parse("DESCRIBE COLLECTION `user-data`").unwrap();
    match &stmts[0] {
        Statement::Describe(DescribeAst::Collection(name)) => {
            assert_eq!(name, "user-data");
        }
        _ => panic!("expected Describe Collection"),
    }
}

#[test]
fn test_parse_update_quoted_field() {
    let stmts = parse("UPDATE user:alice SET `field-name` = 'value'").unwrap();
    match &stmts[0] {
        Statement::Update(u) => {
            assert_eq!(u.assignments[0].path, vec!["field-name".to_string()]);
        }
        _ => panic!("expected Update"),
    }
}

#[test]
fn test_parse_select_quoted_fields() {
    let stmts = parse("SELECT `field-one`, `field-two` FROM user").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 2);
                assert_eq!(
                    items[0],
                    ProjectionItem {
                        expr: Expr::Field("field-one".to_string()),
                        alias: None,
                    }
                );
                assert_eq!(
                    items[1],
                    ProjectionItem {
                        expr: Expr::Field("field-two".to_string()),
                        alias: None,
                    }
                );
            }
            _ => panic!("expected Items projection"),
        },
        _ => panic!("expected Select"),
    }
}

#[test]
fn test_parse_unicode_collection_name() {
    let stmts = parse("DEFINE COLLECTION пользователи").unwrap();
    match &stmts[0] {
        Statement::DefineCollection(d) => {
            assert_eq!(d.name, "пользователи");
        }
        _ => panic!("expected DefineCollection"),
    }
}

#[test]
fn test_parse_reference_type_any() {
    // reference without collection constraint
    let stmts = parse("DEFINE COLLECTION post (author reference)").unwrap();
    match &stmts[0] {
        Statement::DefineCollection(d) => {
            assert_eq!(d.fields.len(), 1);
            assert_eq!(d.fields[0].name, "author");
            assert!(matches!(
                d.fields[0].field_type,
                crate::schema::FieldType::Reference(None)
            ));
        }
        _ => panic!("expected DefineCollection"),
    }
}

#[test]
fn test_parse_reference_type_with_collection() {
    // reference<user> with collection constraint
    let stmts = parse("DEFINE COLLECTION post (author reference<user>)").unwrap();
    match &stmts[0] {
        Statement::DefineCollection(d) => {
            assert_eq!(d.fields.len(), 1);
            assert_eq!(d.fields[0].name, "author");
            match &d.fields[0].field_type {
                crate::schema::FieldType::Reference(Some(coll)) => {
                    assert_eq!(coll, "user");
                }
                _ => panic!("expected Reference(Some(\"user\"))"),
            }
        }
        _ => panic!("expected DefineCollection"),
    }
}

#[test]
fn test_parse_reference_type_required() {
    // reference REQUIRED
    let stmts = parse("DEFINE COLLECTION post (author reference<user> REQUIRED)").unwrap();
    match &stmts[0] {
        Statement::DefineCollection(d) => {
            assert_eq!(d.fields[0].name, "author");
            assert!(d.fields[0].required);
            match &d.fields[0].field_type {
                crate::schema::FieldType::Reference(Some(coll)) => {
                    assert_eq!(coll, "user");
                }
                _ => panic!("expected Reference(Some(\"user\"))"),
            }
        }
        _ => panic!("expected DefineCollection"),
    }
}

#[test]
fn test_parse_reference_array() {
    // Array of references: [reference<user>]
    let stmts = parse("DEFINE COLLECTION post (reviewers [reference<user>])").unwrap();
    match &stmts[0] {
        Statement::DefineCollection(d) => {
            assert_eq!(d.fields[0].name, "reviewers");
            match &d.fields[0].field_type {
                crate::schema::FieldType::Array(inner) => match inner.as_ref() {
                    crate::schema::FieldType::Reference(Some(coll)) => {
                        assert_eq!(coll, "user");
                    }
                    _ => panic!("expected Reference inside Array"),
                },
                _ => panic!("expected Array type"),
            }
        }
        _ => panic!("expected DefineCollection"),
    }
}

#[test]
fn test_parse_reference_in_where() {
    // Reference literal in WHERE clause
    let stmts = parse("SELECT * FROM users WHERE id = users:u1").unwrap();
    match &stmts[0] {
        Statement::Select(s) => {
            // Verify filter contains reference literal
            assert!(s.filter.is_some());
        }
        _ => panic!("expected Select"),
    }
}

#[test]
fn test_parse_create_index() {
    let stmts = parse("CREATE INDEX ON users(email) UNIQUE").unwrap();
    match &stmts[0] {
        Statement::CreateIndex(idx) => {
            assert_eq!(idx.collection, "users");
            assert_eq!(idx.fields, vec!["email"]);
            assert!(idx.unique);
        }
        _ => panic!("expected CreateIndex"),
    }
}

#[test]
fn test_parse_create_compound_index() {
    let stmts = parse("CREATE INDEX ON users(status, created_at)").unwrap();
    match &stmts[0] {
        Statement::CreateIndex(idx) => {
            assert_eq!(idx.fields, vec!["status", "created_at"]);
            assert!(!idx.unique);
        }
        _ => panic!("expected CreateIndex"),
    }
}

#[test]
fn test_parse_drop_index() {
    let stmts = parse("DROP INDEX idx_users_email ON users").unwrap();
    match &stmts[0] {
        Statement::DropIndex(idx) => {
            assert_eq!(idx.collection, "users");
            assert_eq!(idx.name, "idx_users_email");
        }
        _ => panic!("expected DropIndex"),
    }
}

#[test]
fn test_parse_create_hnsw_index() {
    let input = "CREATE INDEX ON products(embedding) HNSW DIMENSION 384 DIST COSINE M 16 EF_CONSTRUCTION 200";
    let result = parse(input).unwrap();
    match &result[0] {
        Statement::CreateIndex(idx) => {
            assert_eq!(idx.collection, "products");
            assert_eq!(idx.fields, vec!["embedding"]);
            assert!(!idx.unique);
            assert_eq!(idx.index_type, crate::schema::IndexType::Hnsw);
            let params = idx.hnsw_params.as_ref().expect("should have hnsw_params");
            assert_eq!(params.dimension, 384);
            assert_eq!(params.metric, crate::schema::DistanceMetric::Cosine);
            assert_eq!(params.m, 16);
            assert_eq!(params.ef_construction, 200);
        }
        _ => panic!("expected CreateIndex"),
    }
}

#[test]
fn test_parse_create_hnsw_index_minimal() {
    // Only dimension is required, others have defaults
    let input = "CREATE INDEX ON vectors(vec) HNSW DIMENSION 128";
    let result = parse(input).unwrap();
    match &result[0] {
        Statement::CreateIndex(idx) => {
            assert_eq!(idx.collection, "vectors");
            assert_eq!(idx.fields, vec!["vec"]);
            assert_eq!(idx.index_type, crate::schema::IndexType::Hnsw);
            let params = idx.hnsw_params.as_ref().expect("should have hnsw_params");
            assert_eq!(params.dimension, 128);
            // Check defaults
            assert_eq!(params.metric, crate::schema::DistanceMetric::Cosine);
            assert_eq!(params.m, 16);
            assert_eq!(params.ef_construction, 200);
        }
        _ => panic!("expected CreateIndex"),
    }
}

#[test]
fn test_parse_create_hnsw_index_euclidean() {
    let input = "CREATE INDEX ON embeddings(vector) HNSW DIMENSION 768 DIST EUCLIDEAN";
    let result = parse(input).unwrap();
    match &result[0] {
        Statement::CreateIndex(idx) => {
            assert_eq!(idx.index_type, crate::schema::IndexType::Hnsw);
            let params = idx.hnsw_params.as_ref().expect("should have hnsw_params");
            assert_eq!(params.dimension, 768);
            assert_eq!(params.metric, crate::schema::DistanceMetric::Euclidean);
        }
        _ => panic!("expected CreateIndex"),
    }
}

#[test]
fn test_parse_create_hnsw_index_dot() {
    let input = "CREATE INDEX ON embeddings(vector) HNSW DIMENSION 1024 DIST DOT M 32";
    let result = parse(input).unwrap();
    match &result[0] {
        Statement::CreateIndex(idx) => {
            assert_eq!(idx.index_type, crate::schema::IndexType::Hnsw);
            let params = idx.hnsw_params.as_ref().expect("should have hnsw_params");
            assert_eq!(params.dimension, 1024);
            assert_eq!(params.metric, crate::schema::DistanceMetric::Dot);
            assert_eq!(params.m, 32);
        }
        _ => panic!("expected CreateIndex"),
    }
}

#[test]
fn test_parse_explain_select() {
    let stmts = parse("EXPLAIN SELECT * FROM users").unwrap();
    match &stmts[0] {
        Statement::Explain(e) => {
            assert!(!e.analyze);
            assert!(matches!(*e.statement, Statement::Select(_)));
        }
        _ => panic!("expected Explain"),
    }
}

#[test]
fn test_parse_explain_analyze() {
    let stmts = parse("EXPLAIN ANALYZE SELECT * FROM users WHERE age > 18").unwrap();
    match &stmts[0] {
        Statement::Explain(e) => {
            assert!(e.analyze);
            assert!(matches!(*e.statement, Statement::Select(_)));
        }
        _ => panic!("expected Explain"),
    }
}

#[test]
fn test_parse_explain_update() {
    let stmts = parse("EXPLAIN UPDATE users SET x = 1").unwrap();
    match &stmts[0] {
        Statement::Explain(e) => {
            assert!(!e.analyze);
            assert!(matches!(*e.statement, Statement::Update(_)));
        }
        _ => panic!("expected Explain"),
    }
}

#[test]
fn test_parse_explain_delete() {
    let stmts = parse("EXPLAIN DELETE users WHERE x = 1").unwrap();
    match &stmts[0] {
        Statement::Explain(e) => {
            assert!(!e.analyze);
            assert!(matches!(*e.statement, Statement::Delete(_)));
        }
        _ => panic!("expected Explain"),
    }
}

#[test]
fn test_parse_order_single_field() {
    let stmts = parse("SELECT * FROM users ORDER name").unwrap();
    match &stmts[0] {
        Statement::Select(s) => {
            let order = s.order.as_ref().expect("should have order");
            assert_eq!(order.len(), 1);
            assert_eq!(order[0].expr, Expr::Field("name".to_string()));
            assert_eq!(order[0].direction, OrderDirection::Asc);
        }
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_order_desc() {
    let stmts = parse("SELECT * FROM users ORDER created_at DESC").unwrap();
    match &stmts[0] {
        Statement::Select(s) => {
            let order = s.order.as_ref().unwrap();
            assert_eq!(order[0].direction, OrderDirection::Desc);
        }
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_order_multiple_fields() {
    let stmts = parse("SELECT * FROM users ORDER status ASC, created_at DESC").unwrap();
    match &stmts[0] {
        Statement::Select(s) => {
            let order = s.order.as_ref().unwrap();
            assert_eq!(order.len(), 2);
            assert_eq!(order[0].expr, Expr::Field("status".to_string()));
            assert_eq!(order[0].direction, OrderDirection::Asc);
            assert_eq!(order[1].expr, Expr::Field("created_at".to_string()));
            assert_eq!(order[1].direction, OrderDirection::Desc);
        }
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_order_with_where_and_limit() {
    let stmts = parse("SELECT * FROM users WHERE active = true ORDER score DESC LIMIT 10").unwrap();
    match &stmts[0] {
        Statement::Select(s) => {
            assert!(s.filter.is_some());
            assert!(s.order.is_some());
            assert_eq!(s.limit, Some(10));
        }
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_select_no_order() {
    let stmts = parse("SELECT * FROM users").unwrap();
    match &stmts[0] {
        Statement::Select(s) => {
            assert!(s.order.is_none());
        }
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_count_star() {
    let stmts = parse("SELECT COUNT(*) FROM users").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 1);
                match &items[0].expr {
                    Expr::Aggregate(agg) => {
                        assert_eq!(agg.function, "COUNT");
                        assert!(agg.namespace.is_none());
                        assert_eq!(agg.arg, AggregateArg::Wildcard);
                    }
                    _ => panic!("expected Aggregate"),
                }
                assert!(items[0].alias.is_none());
            }
            _ => panic!("expected Items projection"),
        },
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_sum_field() {
    let stmts = parse("SELECT SUM(score) FROM users").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 1);
                match &items[0].expr {
                    Expr::Aggregate(agg) => {
                        assert_eq!(agg.function, "SUM");
                        assert_eq!(agg.arg, AggregateArg::Field("score".to_string()));
                    }
                    _ => panic!("expected Aggregate"),
                }
            }
            _ => panic!("expected Items projection"),
        },
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_multiple_aggregates() {
    let stmts = parse("SELECT COUNT(*), SUM(score), AVG(score) FROM users").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 3);
                assert!(matches!(&items[0].expr, Expr::Aggregate(_)));
                assert!(matches!(&items[1].expr, Expr::Aggregate(_)));
                assert!(matches!(&items[2].expr, Expr::Aggregate(_)));
            }
            _ => panic!("expected Items projection"),
        },
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_namespaced_aggregate() {
    let stmts = parse("SELECT builtin::count(*) FROM users").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.projection {
            Projection::Items(items) => match &items[0].expr {
                Expr::Aggregate(agg) => {
                    assert_eq!(agg.namespace, Some("builtin".to_string()));
                    assert_eq!(agg.function, "count");
                }
                _ => panic!("expected Aggregate"),
            },
            _ => panic!("expected Items projection"),
        },
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_aggregate_with_where() {
    let stmts = parse("SELECT COUNT(*) FROM users WHERE active = true").unwrap();
    match &stmts[0] {
        Statement::Select(s) => {
            assert!(s.filter.is_some());
            assert!(matches!(&s.projection, Projection::Items(_)));
        }
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_projection_with_alias() {
    let stmts = parse("SELECT name AS username FROM users").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].expr, Expr::Field("name".to_string()));
                assert_eq!(items[0].alias, Some("username".to_string()));
            }
            _ => panic!("expected Items projection"),
        },
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_aggregate_with_alias() {
    let stmts = parse("SELECT COUNT(*) AS total FROM users").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 1);
                assert!(matches!(&items[0].expr, Expr::Aggregate(_)));
                assert_eq!(items[0].alias, Some("total".to_string()));
            }
            _ => panic!("expected Items projection"),
        },
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_group_single_field() {
    let stmts = parse("SELECT status, COUNT(*) FROM orders GROUP status").unwrap();
    match &stmts[0] {
        Statement::Select(s) => {
            assert_eq!(s.group, Some(vec!["status".to_string()]));
        }
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_group_multiple_fields() {
    let stmts = parse("SELECT city, status, COUNT(*) FROM orders GROUP city, status").unwrap();
    match &stmts[0] {
        Statement::Select(s) => {
            assert_eq!(
                s.group,
                Some(vec!["city".to_string(), "status".to_string()])
            );
        }
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_group_with_where_order_limit() {
    let stmts = parse("SELECT status, COUNT(*) FROM orders WHERE active = true GROUP status ORDER status ASC LIMIT 5").unwrap();
    match &stmts[0] {
        Statement::Select(s) => {
            assert!(s.filter.is_some());
            assert_eq!(s.group, Some(vec!["status".to_string()]));
            assert!(s.order.is_some());
            assert_eq!(s.limit, Some(5));
        }
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_no_group() {
    let stmts = parse("SELECT * FROM orders").unwrap();
    match &stmts[0] {
        Statement::Select(s) => {
            assert_eq!(s.group, None);
        }
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_select_distinct() {
    let stmts = parse("SELECT DISTINCT city FROM users").unwrap();
    match &stmts[0] {
        Statement::Select(s) => {
            assert!(s.distinct);
            assert!(!s.value_mode);
        }
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_select_value() {
    let stmts = parse("SELECT VALUE name FROM users").unwrap();
    match &stmts[0] {
        Statement::Select(s) => {
            assert!(!s.distinct);
            assert!(s.value_mode);
        }
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_select_distinct_value() {
    let stmts = parse("SELECT DISTINCT VALUE city FROM users").unwrap();
    match &stmts[0] {
        Statement::Select(s) => {
            assert!(s.distinct);
            assert!(s.value_mode);
        }
        _ => panic!("expected select"),
    }
}

#[test]
fn test_parse_reindex_collection() {
    let result = parse("REINDEX idx_products_name ON products").unwrap();
    match &result[0] {
        Statement::Reindex(r) => {
            assert_eq!(r.name, "idx_products_name");
            assert_eq!(r.collection, "products");
        }
        _ => panic!("Expected Reindex statement"),
    }
}

#[test]
fn test_parse_reindex_specific_index() {
    let result = parse("REINDEX idx_products_fts_content ON products").unwrap();
    match &result[0] {
        Statement::Reindex(r) => {
            assert_eq!(r.name, "idx_products_fts_content");
            assert_eq!(r.collection, "products");
        }
        _ => panic!("Expected Reindex statement"),
    }
}

#[test]
fn test_parse_create_fulltext_index_with_analyzer() {
    let result = parse("CREATE INDEX ON articles(content) FULLTEXT ANALYZER russian").unwrap();
    match &result[0] {
        Statement::CreateIndex(idx) => {
            assert_eq!(idx.collection, "articles");
            assert_eq!(idx.fields, vec!["content"]);
            assert_eq!(idx.index_type, crate::schema::IndexType::FullText);
            assert_eq!(idx.analyzer, Some("russian".to_string()));
        }
        _ => panic!("Expected CreateIndex statement"),
    }
}

#[test]
fn test_parse_create_fulltext_index_without_analyzer() {
    let result = parse("CREATE INDEX ON articles(content) FULLTEXT").unwrap();
    match &result[0] {
        Statement::CreateIndex(idx) => {
            assert_eq!(idx.collection, "articles");
            assert_eq!(idx.fields, vec!["content"]);
            assert_eq!(idx.index_type, crate::schema::IndexType::FullText);
            assert_eq!(idx.analyzer, None);
        }
        _ => panic!("Expected CreateIndex statement"),
    }
}

#[test]
fn test_parse_create_fulltext_index_analyzer_english() {
    let result = parse("CREATE INDEX ON posts(body) FULLTEXT ANALYZER english").unwrap();
    match &result[0] {
        Statement::CreateIndex(idx) => {
            assert_eq!(idx.collection, "posts");
            assert_eq!(idx.fields, vec!["body"]);
            assert_eq!(idx.index_type, crate::schema::IndexType::FullText);
            assert_eq!(idx.analyzer, Some("english".to_string()));
        }
        _ => panic!("Expected CreateIndex statement"),
    }
}

#[test]
fn test_parse_define_analyzer_standard() {
    let result = parse("DEFINE ANALYZER my_analyzer TOKENIZER standard").unwrap();
    match &result[0] {
        Statement::DefineAnalyzer(a) => {
            assert_eq!(a.name, "my_analyzer");
            assert_eq!(a.tokenizer, crate::schema::TokenizerConfig::Standard);
            assert!(a.filters.is_empty());
        }
        _ => panic!("Expected DefineAnalyzer statement"),
    }
}

#[test]
fn test_parse_define_analyzer_whitespace() {
    let result = parse("DEFINE ANALYZER ws_analyzer TOKENIZER whitespace").unwrap();
    match &result[0] {
        Statement::DefineAnalyzer(a) => {
            assert_eq!(a.name, "ws_analyzer");
            assert_eq!(a.tokenizer, crate::schema::TokenizerConfig::Whitespace);
        }
        _ => panic!("Expected DefineAnalyzer statement"),
    }
}

#[test]
fn test_parse_define_analyzer_ngram() {
    let result = parse("DEFINE ANALYZER ngram_analyzer TOKENIZER ngram(2, 4)").unwrap();
    match &result[0] {
        Statement::DefineAnalyzer(a) => {
            assert_eq!(a.name, "ngram_analyzer");
            assert_eq!(
                a.tokenizer,
                crate::schema::TokenizerConfig::Ngram { min: 2, max: 4 }
            );
        }
        _ => panic!("Expected DefineAnalyzer statement"),
    }
}

#[test]
fn test_parse_define_analyzer_pattern() {
    let result = parse(r#"DEFINE ANALYZER pat_analyzer TOKENIZER pattern("[a-z]+")"#).unwrap();
    match &result[0] {
        Statement::DefineAnalyzer(a) => {
            assert_eq!(a.name, "pat_analyzer");
            assert_eq!(
                a.tokenizer,
                crate::schema::TokenizerConfig::Pattern {
                    regex: "[a-z]+".to_string()
                }
            );
        }
        _ => panic!("Expected DefineAnalyzer statement"),
    }
}

#[test]
fn test_parse_define_analyzer_with_filters() {
    let result = parse(
        "DEFINE ANALYZER english_analyzer TOKENIZER standard FILTERS [lowercase, stemmer(english), stopwords(english)]"
    ).unwrap();
    match &result[0] {
        Statement::DefineAnalyzer(a) => {
            assert_eq!(a.name, "english_analyzer");
            assert_eq!(a.tokenizer, crate::schema::TokenizerConfig::Standard);
            assert_eq!(a.filters.len(), 3);
            assert_eq!(a.filters[0], crate::schema::FilterConfig::Lowercase);
            assert_eq!(
                a.filters[1],
                crate::schema::FilterConfig::Stemmer {
                    lang: "english".to_string()
                }
            );
            assert_eq!(
                a.filters[2],
                crate::schema::FilterConfig::Stopwords {
                    lang: "english".to_string()
                }
            );
        }
        _ => panic!("Expected DefineAnalyzer statement"),
    }
}

#[test]
fn test_parse_define_analyzer_with_length_filter() {
    let result = parse(
        "DEFINE ANALYZER len_analyzer TOKENIZER whitespace FILTERS [lowercase, length(2, 20)]",
    )
    .unwrap();
    match &result[0] {
        Statement::DefineAnalyzer(a) => {
            assert_eq!(a.name, "len_analyzer");
            assert_eq!(a.filters.len(), 2);
            assert_eq!(a.filters[0], crate::schema::FilterConfig::Lowercase);
            assert_eq!(
                a.filters[1],
                crate::schema::FilterConfig::Length { min: 2, max: 20 }
            );
        }
        _ => panic!("Expected DefineAnalyzer statement"),
    }
}

#[test]
fn test_parse_define_analyzer_ascii_folding() {
    let result = parse(
        "DEFINE ANALYZER fold_analyzer TOKENIZER standard FILTERS [lowercase, ascii_folding]",
    )
    .unwrap();
    match &result[0] {
        Statement::DefineAnalyzer(a) => {
            assert_eq!(a.filters.len(), 2);
            assert_eq!(a.filters[0], crate::schema::FilterConfig::Lowercase);
            assert_eq!(a.filters[1], crate::schema::FilterConfig::AsciiFolding);
        }
        _ => panic!("Expected DefineAnalyzer statement"),
    }
}

#[test]
fn test_parse_drop_analyzer() {
    let result = parse("DROP ANALYZER my_analyzer").unwrap();
    match &result[0] {
        Statement::DropAnalyzer(a) => {
            assert_eq!(a.name, "my_analyzer");
        }
        _ => panic!("Expected DropAnalyzer statement"),
    }
}
#[cfg(test)]
mod expr_tests {
    use crate::query::ast::{
        AggregateArg, BinaryOp, DataSource, Expr, Projection, Statement, UnaryOp,
    };
    use crate::query::parse::parse;

    #[test]
    fn test_simple_field() {
        let stmts = parse("SELECT name FROM users").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.projection {
                Projection::Items(items) => {
                    assert_eq!(items.len(), 1);
                    assert_eq!(items[0].expr, Expr::Field("name".to_string()));
                    assert_eq!(items[0].alias, None);
                }
                _ => panic!("expected Items projection"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_alias() {
        let stmts = parse("SELECT name AS n FROM users").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.projection {
                Projection::Items(items) => {
                    assert_eq!(items.len(), 1);
                    assert_eq!(items[0].expr, Expr::Field("name".to_string()));
                    assert_eq!(items[0].alias, Some("n".to_string()));
                }
                _ => panic!("expected Items projection"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_arithmetic_in_projection_with_alias() {
        let stmts = parse("SELECT price * quantity AS total FROM orders").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.projection {
                Projection::Items(items) => {
                    assert_eq!(items.len(), 1);
                    assert_eq!(items[0].alias, Some("total".to_string()));
                    match &items[0].expr {
                        Expr::BinaryOp { left, op, right } => {
                            assert_eq!(*op, BinaryOp::Mul);
                            assert_eq!(**left, Expr::Field("price".to_string()));
                            assert_eq!(**right, Expr::Field("quantity".to_string()));
                        }
                        _ => panic!("expected BinaryOp Mul"),
                    }
                }
                _ => panic!("expected Items projection"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_precedence_add_mul() {
        // 1 + 2 * 3 should parse as Add(1, Mul(2, 3))
        let stmts = parse("SELECT 1 + 2 * 3 FROM t").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.projection {
                Projection::Items(items) => {
                    match &items[0].expr {
                        Expr::BinaryOp { left, op, right } => {
                            assert_eq!(*op, BinaryOp::Add);
                            assert_eq!(**left, Expr::Literal(crate::document::Value::Int(1)));
                            // right should be Mul(2, 3)
                            match right.as_ref() {
                                Expr::BinaryOp {
                                    left: ml,
                                    op: mop,
                                    right: mr,
                                } => {
                                    assert_eq!(*mop, BinaryOp::Mul);
                                    assert_eq!(**ml, Expr::Literal(crate::document::Value::Int(2)));
                                    assert_eq!(**mr, Expr::Literal(crate::document::Value::Int(3)));
                                }
                                _ => panic!("expected Mul as right child of Add"),
                            }
                        }
                        _ => panic!("expected BinaryOp Add at top"),
                    }
                }
                _ => panic!("expected Items projection"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_unary_neg_in_projection() {
        let stmts = parse("SELECT -score FROM t").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.projection {
                Projection::Items(items) => match &items[0].expr {
                    Expr::UnaryOp { op, expr } => {
                        assert_eq!(*op, UnaryOp::Neg);
                        assert_eq!(**expr, Expr::Field("score".to_string()));
                    }
                    _ => panic!("expected UnaryOp Neg"),
                },
                _ => panic!("expected Items projection"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_not_in_where() {
        let stmts = parse("SELECT * FROM t WHERE NOT active = true").unwrap();
        match &stmts[0] {
            Statement::Select(s) => {
                match &s.filter {
                    Some(Expr::UnaryOp { op, expr }) => {
                        assert_eq!(*op, UnaryOp::Not);
                        // The inner expression should be a comparison
                        match expr.as_ref() {
                            Expr::BinaryOp { op, .. } => {
                                assert_eq!(*op, BinaryOp::Eq);
                            }
                            _ => panic!("expected BinaryOp Eq inside NOT"),
                        }
                    }
                    _ => panic!("expected UnaryOp Not at top of filter"),
                }
            }
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_in_list() {
        let stmts = parse(r#"SELECT * FROM t WHERE status IN ["active", "pending"]"#).unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.filter {
                Some(Expr::InList {
                    expr,
                    list,
                    negated,
                }) => {
                    assert_eq!(**expr, Expr::Field("status".to_string()));
                    assert_eq!(list.len(), 2);
                    assert!(!negated);
                }
                _ => panic!("expected InList"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_not_in_list() {
        let stmts = parse(r#"SELECT * FROM t WHERE status NOT IN ["deleted"]"#).unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.filter {
                Some(Expr::InList {
                    expr,
                    list,
                    negated,
                }) => {
                    assert_eq!(**expr, Expr::Field("status".to_string()));
                    assert_eq!(list.len(), 1);
                    assert!(*negated);
                }
                _ => panic!("expected InList negated"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_in_variable() {
        let stmts = parse(r#"SELECT * FROM t WHERE status IN $statuses"#).unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.filter {
                Some(Expr::InExpr {
                    expr,
                    target,
                    negated,
                }) => {
                    assert_eq!(**expr, Expr::Field("status".to_string()));
                    assert_eq!(**target, Expr::Variable("statuses".to_string()));
                    assert!(!*negated);
                }
                _ => panic!("expected InExpr with variable"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_not_in_variable() {
        let stmts = parse(r#"SELECT * FROM t WHERE status NOT IN $excluded"#).unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.filter {
                Some(Expr::InExpr {
                    expr,
                    target,
                    negated,
                }) => {
                    assert_eq!(**expr, Expr::Field("status".to_string()));
                    assert_eq!(**target, Expr::Variable("excluded".to_string()));
                    assert!(*negated);
                }
                _ => panic!("expected InExpr negated with variable"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_function_call_upper() {
        let stmts = parse("SELECT UPPER(name) FROM users").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.projection {
                Projection::Items(items) => {
                    assert_eq!(items.len(), 1);
                    match &items[0].expr {
                        Expr::FunctionCall { name, args, .. } => {
                            assert_eq!(name, "UPPER");
                            assert_eq!(args.len(), 1);
                            assert_eq!(args[0], Expr::Field("name".to_string()));
                        }
                        _ => panic!("expected FunctionCall"),
                    }
                }
                _ => panic!("expected Items projection"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_comparison_with_expressions_both_sides() {
        let stmts = parse("SELECT * FROM t WHERE price * 2 > budget").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.filter {
                Some(Expr::BinaryOp { left, op, right }) => {
                    assert_eq!(*op, BinaryOp::Gt);
                    // left should be price * 2
                    match left.as_ref() {
                        Expr::BinaryOp {
                            left: l,
                            op: mop,
                            right: r,
                        } => {
                            assert_eq!(*mop, BinaryOp::Mul);
                            assert_eq!(**l, Expr::Field("price".to_string()));
                            assert_eq!(**r, Expr::Literal(crate::document::Value::Int(2)));
                        }
                        _ => panic!("expected Mul on left of Gt"),
                    }
                    assert_eq!(**right, Expr::Field("budget".to_string()));
                }
                _ => panic!("expected BinaryOp Gt"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_aggregate_count_star() {
        let stmts = parse("SELECT COUNT(*) FROM users").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.projection {
                Projection::Items(items) => {
                    assert_eq!(items.len(), 1);
                    match &items[0].expr {
                        Expr::Aggregate(agg) => {
                            assert_eq!(agg.function, "COUNT");
                            assert_eq!(agg.arg, AggregateArg::Wildcard);
                        }
                        _ => panic!("expected Aggregate"),
                    }
                    assert!(items[0].alias.is_none());
                }
                _ => panic!("expected Items projection"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_aggregate_count_star_with_alias() {
        let stmts = parse("SELECT COUNT(*) AS cnt FROM users").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.projection {
                Projection::Items(items) => {
                    assert_eq!(items.len(), 1);
                    assert!(matches!(&items[0].expr, Expr::Aggregate(_)));
                    assert_eq!(items[0].alias, Some("cnt".to_string()));
                }
                _ => panic!("expected Items projection"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_is_null() {
        let stmts = parse("SELECT * FROM users WHERE name IS NULL").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.filter {
                Some(Expr::UnaryOp { op, expr }) => {
                    assert_eq!(*op, UnaryOp::IsNull);
                    assert_eq!(**expr, Expr::Field("name".to_string()));
                }
                other => panic!("expected UnaryOp IsNull, got {:?}", other),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_is_not_null() {
        let stmts = parse("SELECT * FROM users WHERE name IS NOT NULL").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.filter {
                Some(Expr::UnaryOp { op, expr }) => {
                    assert_eq!(*op, UnaryOp::IsNotNull);
                    assert_eq!(**expr, Expr::Field("name".to_string()));
                }
                other => panic!("expected UnaryOp IsNotNull, got {:?}", other),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_is_none() {
        let stmts = parse("SELECT * FROM users WHERE name IS NONE").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.filter {
                Some(Expr::UnaryOp { op, expr }) => {
                    assert_eq!(*op, UnaryOp::IsNone);
                    assert_eq!(**expr, Expr::Field("name".to_string()));
                }
                other => panic!("expected UnaryOp IsNone, got {:?}", other),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_is_not_none() {
        let stmts = parse("SELECT * FROM users WHERE name IS NOT NONE").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.filter {
                Some(Expr::UnaryOp { op, expr }) => {
                    assert_eq!(*op, UnaryOp::IsNotNone);
                    assert_eq!(**expr, Expr::Field("name".to_string()));
                }
                other => panic!("expected UnaryOp IsNotNone, got {:?}", other),
            },
            _ => panic!("expected select"),
        }
    }

    // === FROM subquery tests ===

    #[test]
    fn test_parse_from_subquery() {
        let stmts = parse("SELECT * FROM (SELECT name FROM users)").unwrap();
        match &stmts[0] {
            Statement::Select(s) => {
                assert_eq!(s.projection, Projection::All);
                match &s.source {
                    DataSource::Expr(expr) => match expr.as_ref() {
                        Expr::Subquery(inner) => match &inner.source {
                            DataSource::Collection(t) => {
                                assert_eq!(t.collection, "users");
                            }
                            _ => panic!("expected collection source in inner select"),
                        },
                        _ => panic!("expected subquery expression"),
                    },
                    _ => panic!("expected expr source"),
                }
            }
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_parse_from_subquery_with_where() {
        let stmts =
            parse("SELECT name FROM (SELECT * FROM users WHERE age > 18) WHERE name = 'Alice'")
                .unwrap();
        match &stmts[0] {
            Statement::Select(s) => {
                match &s.source {
                    DataSource::Expr(expr) => match expr.as_ref() {
                        Expr::Subquery(inner) => {
                            assert!(inner.filter.is_some());
                            match &inner.source {
                                DataSource::Collection(t) => {
                                    assert_eq!(t.collection, "users");
                                }
                                _ => panic!("expected collection source in inner select"),
                            }
                        }
                        _ => panic!("expected subquery expression"),
                    },
                    _ => panic!("expected expr source"),
                }
                assert!(s.filter.is_some());
            }
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_parse_from_subquery_with_limit() {
        let stmts = parse("SELECT * FROM (SELECT * FROM users LIMIT 10) LIMIT 5").unwrap();
        match &stmts[0] {
            Statement::Select(s) => {
                assert_eq!(s.limit, Some(5));
                match &s.source {
                    DataSource::Expr(expr) => match expr.as_ref() {
                        Expr::Subquery(inner) => {
                            assert_eq!(inner.limit, Some(10));
                        }
                        _ => panic!("expected subquery expression"),
                    },
                    _ => panic!("expected expr source"),
                }
            }
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_parse_from_nested_subquery() {
        let stmts = parse("SELECT * FROM (SELECT * FROM (SELECT * FROM users))").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.source {
                DataSource::Expr(expr) => match expr.as_ref() {
                    Expr::Subquery(inner) => match &inner.source {
                        DataSource::Expr(expr2) => match expr2.as_ref() {
                            Expr::Subquery(inner2) => match &inner2.source {
                                DataSource::Collection(t) => {
                                    assert_eq!(t.collection, "users");
                                }
                                _ => panic!("expected collection source at deepest level"),
                            },
                            _ => panic!("expected subquery at middle level"),
                        },
                        _ => panic!("expected expr source at middle level"),
                    },
                    _ => panic!("expected subquery at outer level"),
                },
                _ => panic!("expected expr source at outer level"),
            },
            _ => panic!("expected select"),
        }
    }

    // Note: Field path syntax like "user:alice.tags" in FROM clause is no longer supported
    // in the simplified grammar. Use subqueries or explicit lookups instead.

    #[test]
    fn test_parse_parent_ref() {
        use crate::query::ast::ParentRef;
        let stmts = parse("SELECT * FROM (SELECT * FROM orders WHERE user = $parent.id)").unwrap();
        // The inner SELECT should have a filter with $parent.id
        match &stmts[0] {
            Statement::Select(s) => match &s.source {
                DataSource::Expr(expr) => match expr.as_ref() {
                    Expr::Subquery(inner) => {
                        // Check that the filter contains a ParentRef
                        match &inner.filter {
                            Some(Expr::BinaryOp { right, .. }) => match right.as_ref() {
                                Expr::ParentRef(ParentRef { depth, field }) => {
                                    assert_eq!(*depth, 0);
                                    assert_eq!(field, "id");
                                }
                                other => panic!("expected ParentRef, got {:?}", other),
                            },
                            other => {
                                panic!("expected BinaryOp filter with ParentRef, got {:?}", other)
                            }
                        }
                    }
                    _ => panic!("expected subquery expression"),
                },
                _ => panic!("expected expr source"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_parse_nested_parent_ref() {
        use crate::query::ast::{Expr, ParentRef};
        use crate::query::parse::parse;

        // Single level: $parent.name
        let ast = parse("SELECT $parent.name FROM users").unwrap();
        if let crate::query::ast::Statement::Select(s) = &ast[0]
            && let crate::query::ast::Projection::Items(items) = &s.projection
        {
            match &items[0].expr {
                Expr::ParentRef(ParentRef { depth, field }) => {
                    assert_eq!(*depth, 0);
                    assert_eq!(field, "name");
                }
                other => panic!("expected ParentRef, got {:?}", other),
            }
        }

        // Two levels: $parent.parent.id
        let ast = parse("SELECT $parent.parent.id FROM users").unwrap();
        if let crate::query::ast::Statement::Select(s) = &ast[0]
            && let crate::query::ast::Projection::Items(items) = &s.projection
        {
            match &items[0].expr {
                Expr::ParentRef(ParentRef { depth, field }) => {
                    assert_eq!(*depth, 1);
                    assert_eq!(field, "id");
                }
                other => panic!("expected ParentRef, got {:?}", other),
            }
        }

        // Three levels: $parent.parent.parent.email
        let ast = parse("SELECT $parent.parent.parent.email FROM users").unwrap();
        if let crate::query::ast::Statement::Select(s) = &ast[0]
            && let crate::query::ast::Projection::Items(items) = &s.projection
        {
            match &items[0].expr {
                Expr::ParentRef(ParentRef { depth, field }) => {
                    assert_eq!(*depth, 2);
                    assert_eq!(field, "email");
                }
                other => panic!("expected ParentRef, got {:?}", other),
            }
        }
    }

    #[test]
    fn test_parse_subquery_in_projection() {
        let stmts = parse("SELECT name, (SELECT COUNT(*) FROM orders WHERE user = $parent.id) AS order_count FROM users").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.projection {
                Projection::Items(items) => {
                    assert_eq!(items.len(), 2);
                    assert!(matches!(&items[0].expr, Expr::Field(f) if f == "name"));
                    assert!(matches!(&items[1].expr, Expr::Subquery(_)));
                    assert_eq!(items[1].alias, Some("order_count".to_string()));
                }
                _ => panic!("expected Items projection"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_parse_subquery_index() {
        use crate::document::Value;

        // Test: (SELECT name FROM users WHERE id = 1)[0] AS first_name
        let sql = "SELECT (SELECT name FROM users WHERE id = 1)[0] AS first_name FROM dual";
        let stmts = parse(sql).unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.projection {
                Projection::Items(items) => {
                    assert_eq!(items.len(), 1);
                    assert_eq!(items[0].alias, Some("first_name".to_string()));
                    // The expr should be Expr::Index with Subquery base
                    match &items[0].expr {
                        Expr::Index { base, index } => {
                            // Base should be a Subquery
                            match &**base {
                                Expr::Subquery(inner_select) => {
                                    // Verify the inner SELECT
                                    match &inner_select.source {
                                        DataSource::Collection(t) => {
                                            assert_eq!(t.collection, "users");
                                        }
                                        _ => panic!("expected collection source in subquery"),
                                    }
                                }
                                other => panic!("expected Subquery as base, got {:?}", other),
                            }
                            // Index should be 0
                            assert_eq!(**index, Expr::Literal(Value::Int(0)));
                        }
                        other => panic!("expected Expr::Index, got {:?}", other),
                    }
                }
                _ => panic!("expected Items projection"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_parse_subquery_index_with_expression() {
        // Test with a variable index: (SELECT * FROM items)[idx]
        let sql = "SELECT (SELECT * FROM items)[idx] FROM config";
        let stmts = parse(sql).unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.projection {
                Projection::Items(items) => match &items[0].expr {
                    Expr::Index { base, index } => {
                        assert!(matches!(&**base, Expr::Subquery(_)));
                        assert_eq!(**index, Expr::Field("idx".to_string()));
                    }
                    other => panic!("expected Expr::Index, got {:?}", other),
                },
                _ => panic!("expected Items projection"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_parse_subquery_without_index() {
        // Ensure subqueries without index still work
        let sql = "SELECT (SELECT name FROM users) AS names FROM dual";
        let stmts = parse(sql).unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.projection {
                Projection::Items(items) => {
                    assert_eq!(items[0].alias, Some("names".to_string()));
                    // Should be just Subquery, not Index
                    assert!(matches!(&items[0].expr, Expr::Subquery(_)));
                }
                _ => panic!("expected Items projection"),
            },
            _ => panic!("expected select"),
        }
    }

    // === FROM array source tests ===

    #[test]
    fn test_parse_from_array_literals() {
        let stmts = parse("SELECT * FROM [0, 1, 2]").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.source {
                DataSource::Expr(expr) => match expr.as_ref() {
                    Expr::Array(elements) => {
                        assert_eq!(elements.len(), 3);
                    }
                    _ => panic!("expected array expression"),
                },
                _ => panic!("expected expr source"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_parse_from_array_with_where() {
        let stmts = parse("SELECT value FROM [1, 2, 3] WHERE value > 1").unwrap();
        match &stmts[0] {
            Statement::Select(s) => {
                match &s.source {
                    DataSource::Expr(expr) => match expr.as_ref() {
                        Expr::Array(elements) => assert_eq!(elements.len(), 3),
                        _ => panic!("expected array expression"),
                    },
                    _ => panic!("expected expr source"),
                }
                assert!(s.filter.is_some());
            }
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_parse_from_empty_array() {
        let stmts = parse("SELECT * FROM []").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.source {
                DataSource::Expr(expr) => match expr.as_ref() {
                    Expr::Array(elements) => assert!(elements.is_empty()),
                    _ => panic!("expected array expression"),
                },
                _ => panic!("expected expr source"),
            },
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_parse_value_ref() {
        let stmts = parse("SELECT $value FROM [1, 2]").unwrap();
        match &stmts[0] {
            Statement::Select(s) => {
                match &s.projection {
                    Projection::Items(items) => {
                        assert!(matches!(&items[0].expr, Expr::ValueRef(None)));
                    }
                    _ => panic!("expected Items"),
                }
                assert!(matches!(&s.source, DataSource::Expr(_)));
            }
            _ => panic!("expected select"),
        }
    }

    #[test]
    fn test_parse_value_ref_with_path() {
        let stmts = parse("SELECT $value.name FROM [{name: 'A'}]").unwrap();
        match &stmts[0] {
            Statement::Select(s) => match &s.projection {
                Projection::Items(items) => {
                    assert!(matches!(&items[0].expr, Expr::ValueRef(Some(p)) if p == "name"));
                }
                _ => panic!("expected Items"),
            },
            _ => panic!("expected select"),
        }
    }

    // Note: "FROM [array] AS alias" syntax is no longer supported in simplified grammar.
    // Use $value instead.
}

#[cfg(test)]
mod relate_tests {
    use crate::query::ast::{RelateData, RelateSource, ReturnClause, Statement};
    use crate::query::parse::parse;

    #[test]
    fn test_parse_relate_basic() {
        let stmts = parse("RELATE user:alice->follows->user:bob").unwrap();
        assert_eq!(stmts.len(), 1);
        match &stmts[0] {
            Statement::Relate(r) => {
                assert_eq!(r.label, "follows");
                match &r.from {
                    RelateSource::Record(t) => {
                        assert_eq!(t.collection, "user");
                        assert_eq!(t.key, Some("alice".to_string()));
                    }
                    _ => panic!("expected Record source"),
                }
                match &r.to {
                    RelateSource::Record(t) => {
                        assert_eq!(t.collection, "user");
                        assert_eq!(t.key, Some("bob".to_string()));
                    }
                    _ => panic!("expected Record source"),
                }
                assert!(r.data.is_none());
                assert_eq!(r.return_clause, ReturnClause::After);
            }
            _ => panic!("expected Relate"),
        }
    }

    #[test]
    fn test_parse_relate_with_set() {
        let stmts =
            parse("RELATE user:alice->follows->user:bob SET since = 123, mutual = true").unwrap();
        match &stmts[0] {
            Statement::Relate(r) => {
                assert_eq!(r.label, "follows");
                match &r.data {
                    Some(RelateData::Set(assignments)) => {
                        assert_eq!(assignments.len(), 2);
                        assert_eq!(assignments[0].path, vec!["since".to_string()]);
                        assert_eq!(assignments[1].path, vec!["mutual".to_string()]);
                    }
                    _ => panic!("expected Set data"),
                }
            }
            _ => panic!("expected Relate"),
        }
    }

    #[test]
    fn test_parse_relate_with_content() {
        let stmts =
            parse("RELATE user:alice->follows->user:bob CONTENT {since: 123, mutual: true}")
                .unwrap();
        match &stmts[0] {
            Statement::Relate(r) => {
                assert_eq!(r.label, "follows");
                match &r.data {
                    Some(RelateData::Content(obj)) => {
                        assert_eq!(obj.fields.len(), 2);
                    }
                    _ => panic!("expected Content data"),
                }
            }
            _ => panic!("expected Relate"),
        }
    }

    #[test]
    fn test_parse_relate_return_none() {
        let stmts = parse("RELATE user:alice->follows->user:bob RETURN NONE").unwrap();
        match &stmts[0] {
            Statement::Relate(r) => {
                assert_eq!(r.return_clause, ReturnClause::None);
            }
            _ => panic!("expected Relate"),
        }
    }

    #[test]
    fn test_parse_relate_return_before() {
        let stmts = parse("RELATE user:alice->follows->user:bob RETURN BEFORE").unwrap();
        match &stmts[0] {
            Statement::Relate(r) => {
                assert_eq!(r.return_clause, ReturnClause::Before);
            }
            _ => panic!("expected Relate"),
        }
    }

    #[test]
    fn test_parse_relate_return_after() {
        let stmts = parse("RELATE user:alice->follows->user:bob RETURN AFTER").unwrap();
        match &stmts[0] {
            Statement::Relate(r) => {
                assert_eq!(r.return_clause, ReturnClause::After);
            }
            _ => panic!("expected Relate"),
        }
    }

    #[test]
    fn test_parse_relate_array_source() {
        let stmts = parse("RELATE user:alice->follows->[user:bob, user:carol]").unwrap();
        match &stmts[0] {
            Statement::Relate(r) => {
                assert_eq!(r.label, "follows");
                match &r.to {
                    RelateSource::Array(targets) => {
                        assert_eq!(targets.len(), 2);
                        assert_eq!(targets[0].collection, "user");
                        assert_eq!(targets[0].key, Some("bob".to_string()));
                        assert_eq!(targets[1].collection, "user");
                        assert_eq!(targets[1].key, Some("carol".to_string()));
                    }
                    _ => panic!("expected Array source"),
                }
            }
            _ => panic!("expected Relate"),
        }
    }

    #[test]
    fn test_parse_relate_subquery() {
        let stmts =
            parse("RELATE user:alice->follows->(SELECT * FROM user WHERE active = true)").unwrap();
        match &stmts[0] {
            Statement::Relate(r) => {
                assert_eq!(r.label, "follows");
                match &r.to {
                    RelateSource::Subquery(select) => {
                        assert!(select.filter.is_some());
                    }
                    _ => panic!("expected Subquery source"),
                }
            }
            _ => panic!("expected Relate"),
        }
    }

    #[test]
    fn test_parse_relate_collection_only() {
        // RELATE with collection only (no key) should also work
        let stmts = parse("RELATE user->follows->user").unwrap();
        match &stmts[0] {
            Statement::Relate(r) => {
                assert_eq!(r.label, "follows");
                match &r.from {
                    RelateSource::Record(t) => {
                        assert_eq!(t.collection, "user");
                        assert_eq!(t.key, None);
                    }
                    _ => panic!("expected Record source"),
                }
                match &r.to {
                    RelateSource::Record(t) => {
                        assert_eq!(t.collection, "user");
                        assert_eq!(t.key, None);
                    }
                    _ => panic!("expected Record source"),
                }
            }
            _ => panic!("expected Relate"),
        }
    }

    #[test]
    fn test_parse_relate_with_set_and_return() {
        let stmts =
            parse("RELATE user:alice->follows->user:bob SET since = 2024 RETURN NONE").unwrap();
        match &stmts[0] {
            Statement::Relate(r) => {
                assert!(matches!(&r.data, Some(RelateData::Set(_))));
                assert_eq!(r.return_clause, ReturnClause::None);
            }
            _ => panic!("expected Relate"),
        }
    }
}

// =============================================================================
// Escaped identifier tests
// =============================================================================

#[test]
fn test_quoted_ident_simple() {
    // Simple quoted identifier with unicode
    let stmts = parse("SELECT `имя` FROM users").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].expr, Expr::Field("имя".to_string()));
            }
            _ => panic!("expected items"),
        },
        _ => panic!("expected select"),
    }
}

#[test]
fn test_quoted_ident_with_dots() {
    // Dots inside quoted identifier (not a path separator)
    let stmts = parse("SELECT `field.with.dots` FROM users").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 1);
                // Should be a single field name containing dots, not a path
                assert_eq!(items[0].expr, Expr::Field("field.with.dots".to_string()));
            }
            _ => panic!("expected items"),
        },
        _ => panic!("expected select"),
    }
}

#[test]
fn test_quoted_ident_escaped_backtick() {
    // Escaped backtick: \` becomes `
    let stmts = parse(r"SELECT `field\`name` FROM users").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].expr, Expr::Field("field`name".to_string()));
            }
            _ => panic!("expected items"),
        },
        _ => panic!("expected select"),
    }
}

#[test]
fn test_quoted_ident_escaped_backslash() {
    // Escaped backslash: \\ becomes \
    let stmts = parse(r"SELECT `field\\name` FROM users").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].expr, Expr::Field("field\\name".to_string()));
            }
            _ => panic!("expected items"),
        },
        _ => panic!("expected select"),
    }
}

#[test]
fn test_quoted_ident_multiple_escapes() {
    // Multiple escapes: `a\`b\\c\`d` -> a`b\c`d
    let stmts = parse(r"SELECT `a\`b\\c\`d` FROM users").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].expr, Expr::Field("a`b\\c`d".to_string()));
            }
            _ => panic!("expected items"),
        },
        _ => panic!("expected select"),
    }
}

#[test]
fn test_quoted_ident_with_spaces_and_emoji() {
    // Spaces and emoji in field name
    let stmts = parse("SELECT `field name 🔥` FROM users").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.projection {
            Projection::Items(items) => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].expr, Expr::Field("field name 🔥".to_string()));
            }
            _ => panic!("expected items"),
        },
        _ => panic!("expected select"),
    }
}

#[test]
fn test_quoted_collection_name() {
    // Quoted collection name
    let stmts = parse("SELECT * FROM `user-data`").unwrap();
    match &stmts[0] {
        Statement::Select(s) => match &s.source {
            DataSource::Collection(t) => {
                assert_eq!(t.collection, "user-data");
            }
            _ => panic!("expected collection source"),
        },
        _ => panic!("expected select"),
    }
}

// ==========================================
// Control flow tests
// ==========================================

#[test]
fn test_parse_if_simple() {
    let stmts = parse("IF true THEN 1 END").unwrap();
    match &stmts[0] {
        Statement::Expr(Expr::If {
            branches,
            else_block,
        }) => {
            assert_eq!(branches.len(), 1);
            assert!(else_block.is_none());
        }
        other => panic!("expected If, got {:?}", other),
    }
}

#[test]
fn test_parse_if_else() {
    let stmts = parse("IF true THEN 1 ELSE 2 END").unwrap();
    match &stmts[0] {
        Statement::Expr(Expr::If {
            branches,
            else_block,
        }) => {
            assert_eq!(branches.len(), 1);
            assert!(else_block.is_some());
        }
        other => panic!("expected If, got {:?}", other),
    }
}

#[test]
fn test_parse_if_else_if() {
    let stmts =
        parse("IF $x > 10 THEN \"high\" ELSE IF $x > 5 THEN \"mid\" ELSE \"low\" END").unwrap();
    match &stmts[0] {
        Statement::Expr(Expr::If {
            branches,
            else_block,
        }) => {
            assert_eq!(branches.len(), 2);
            assert!(else_block.is_some());
        }
        other => panic!("expected If, got {:?}", other),
    }
}

#[test]
fn test_parse_if_with_statements() {
    let stmts = parse("IF true THEN LET x = 1; $x END").unwrap();
    match &stmts[0] {
        Statement::Expr(Expr::If { branches, .. }) => {
            assert_eq!(branches[0].1.statements.len(), 2);
        }
        other => panic!("expected If, got {:?}", other),
    }
}

#[test]
fn test_parse_for_simple() {
    let stmts = parse("FOR $x IN [1, 2, 3] DO $x END").unwrap();
    match &stmts[0] {
        Statement::Expr(Expr::For { binding, .. }) => {
            assert_eq!(binding, "x");
        }
        other => panic!("expected For, got {:?}", other),
    }
}

#[test]
fn test_parse_for_with_subquery() {
    let stmts = parse("FOR $u IN (SELECT * FROM user) DO $u.name END").unwrap();
    match &stmts[0] {
        Statement::Expr(Expr::For { binding, .. }) => {
            assert_eq!(binding, "u");
        }
        other => panic!("expected For, got {:?}", other),
    }
}

#[test]
fn test_parse_break() {
    let stmts = parse("BREAK").unwrap();
    assert!(matches!(&stmts[0], Statement::Expr(Expr::Break)));
}

#[test]
fn test_parse_continue() {
    let stmts = parse("CONTINUE").unwrap();
    assert!(matches!(&stmts[0], Statement::Expr(Expr::Continue)));
}

#[test]
fn test_parse_nested_if_in_for() {
    let stmts = parse("FOR $x IN [1,2,3] DO IF $x > 1 THEN $x ELSE 0 END END").unwrap();
    match &stmts[0] {
        Statement::Expr(Expr::For { body, .. }) => {
            assert_eq!(body.statements.len(), 1);
            assert!(matches!(
                &body.statements[0],
                Statement::Expr(Expr::If { .. })
            ));
        }
        other => panic!("expected For, got {:?}", other),
    }
}

#[test]
fn test_parse_if_as_let_value() {
    let stmts = parse("LET x = IF true THEN 1 ELSE 0 END").unwrap();
    match &stmts[0] {
        Statement::Let(let_ast) => {
            assert!(matches!(&let_ast.expr, Expr::If { .. }));
        }
        other => panic!("expected Let, got {:?}", other),
    }
}
