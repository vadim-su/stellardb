//! Shared utilities for e2e benchmarks

use std::collections::HashMap;
use tempfile::TempDir;

use stellardb::query::execute::{Row, execute_ddl, plan_from_statement};
use stellardb::query::plan::LogicalPlan;
use stellardb::query::{FnRegistry, bind_statement, parse};
use stellardb::{Database, Document, Value};

/// Helper to create a test storage with data
pub struct TestDb {
    pub storage: Database,
    pub fn_registry: FnRegistry,
    _tmp: TempDir,
}

impl TestDb {
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let storage = Database::open(tmp.path()).unwrap();
        Self {
            storage,
            fn_registry: FnRegistry::default(),
            _tmp: tmp,
        }
    }

    pub fn with_users(count: usize) -> Self {
        let db = Self::new();
        for i in 0..count {
            let mut fields = HashMap::new();
            fields.insert("name".to_string(), Value::String(format!("User{}", i)));
            fields.insert("age".to_string(), Value::Int((20 + i % 50) as i64));
            fields.insert("active".to_string(), Value::Bool(i % 2 == 0));
            fields.insert("score".to_string(), Value::Float((i as f64) * 1.5 + 10.0));
            fields.insert(
                "city".to_string(),
                Value::String(["Moscow", "Berlin", "Paris", "London", "Tokyo"][i % 5].to_string()),
            );
            fields.insert(
                "email".to_string(),
                Value::String(format!("user{}@example.com", i)),
            );

            let doc = Document {
                id: format!("users:user{}", i),
                fields,
            };
            db.storage.set_document(&doc).unwrap();
        }
        db
    }

    /// Create users + orders for correlated subquery benchmarks
    pub fn with_users_and_orders(user_count: usize, orders_per_user: usize) -> Self {
        let db = Self::with_users(user_count);

        for user_i in 0..user_count {
            for order_i in 0..orders_per_user {
                let mut fields = HashMap::new();
                fields.insert(
                    "user_name".to_string(),
                    Value::String(format!("User{}", user_i)),
                );
                fields.insert(
                    "amount".to_string(),
                    Value::Float((order_i as f64 + 1.0) * 100.0),
                );
                fields.insert(
                    "status".to_string(),
                    Value::String(
                        if order_i % 3 == 0 {
                            "completed"
                        } else {
                            "pending"
                        }
                        .to_string(),
                    ),
                );
                fields.insert(
                    "product".to_string(),
                    Value::String(format!("Product{}", order_i % 10)),
                );

                let doc = Document {
                    id: format!("orders:order_{}_{}", user_i, order_i),
                    fields,
                };
                db.storage.set_document(&doc).unwrap();
            }
        }
        db
    }

    /// Create users with indexes
    pub fn with_users_indexed(count: usize) -> Self {
        let db = Self::with_users(count);
        db.execute_query("CREATE INDEX ON users(age)");
        db.execute_query("CREATE INDEX ON users(city)");
        db.execute_query("CREATE INDEX ON users(email) UNIQUE");
        db.execute_query("CREATE INDEX ON users(city, age)");
        db
    }

    pub fn execute_query(&self, sql: &str) -> Vec<Row> {
        let stmts = parse(sql).unwrap();
        let stmt = stmts.into_iter().next().unwrap();
        let bound =
            bind_statement(stmt, self.storage.schema_registry(), &self.fn_registry).unwrap();

        // Check if this is a DDL statement that needs LogicalPlan::from_ast
        // (DDL statements don't have subqueries, so from_ast is fine)
        match &bound {
            stellardb::query::ast::Statement::DefineCollection { .. }
            | stellardb::query::ast::Statement::DropCollection { .. }
            | stellardb::query::ast::Statement::Describe(_)
            | stellardb::query::ast::Statement::CreateIndex { .. }
            | stellardb::query::ast::Statement::DropIndex { .. } => {
                let plan = LogicalPlan::from_ast(bound);
                execute_ddl(plan, &self.storage).unwrap()
            }
            // For queries (SELECT, INSERT, UPDATE, DELETE), use plan_from_statement
            // to properly materialize subqueries and field paths
            _ => {
                let plan = plan_from_statement(bound, &self.storage).unwrap();
                stellardb::query::execute::execute(plan, &self.storage, None).unwrap()
            }
        }
    }
}
