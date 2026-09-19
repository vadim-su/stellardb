// Edge case integration tests for StellarDB
// These tests cover boundary conditions, error handling, and unexpected inputs

mod common;

// =============================================================================
// SQL Operations
// =============================================================================

#[path = "edge_cases/sql/query.rs"]
mod query;

#[path = "edge_cases/sql/create.rs"]
mod create;

#[path = "edge_cases/sql/update.rs"]
mod update;

#[path = "edge_cases/sql/upsert.rs"]
mod upsert;

#[path = "edge_cases/sql/limit.rs"]
mod limit;

#[path = "edge_cases/sql/order.rs"]
mod order;

#[path = "edge_cases/sql/aggregate.rs"]
mod aggregate;

#[path = "edge_cases/sql/multi_statement.rs"]
mod multi_statement;

#[path = "edge_cases/sql/expression.rs"]
mod expression;

#[path = "edge_cases/sql/alias.rs"]
mod alias;

#[path = "edge_cases/sql/in_expr.rs"]
mod in_expr;

#[path = "edge_cases/sql/scalar_functions.rs"]
mod scalar_functions;

#[path = "edge_cases/sql/expression_errors.rs"]
mod expression_errors;

#[path = "edge_cases/sql/is_null.rs"]
mod is_null;

#[path = "edge_cases/sql/group.rs"]
mod group;

#[path = "edge_cases/sql/subquery.rs"]
mod subquery;

#[path = "edge_cases/sql/expr_source.rs"]
mod expr_source;

#[path = "edge_cases/sql/coalesce.rs"]
mod coalesce;

#[path = "edge_cases/sql/timeout.rs"]
mod timeout;

// =============================================================================
// Data & Validation
// =============================================================================

#[path = "edge_cases/data/document_id.rs"]
mod document_id;

#[path = "edge_cases/data/field_validation.rs"]
mod field_validation;

#[path = "edge_cases/data/boundary_values.rs"]
mod boundary_values;

#[path = "edge_cases/data/datetime.rs"]
mod datetime;

// =============================================================================
// Schema & Collections
// =============================================================================

#[path = "edge_cases/schema/collection.rs"]
mod collection;

// =============================================================================
// Transactions
// =============================================================================

#[path = "edge_cases/transaction/transaction.rs"]
mod transaction;

// =============================================================================
// HTTP REST API
// =============================================================================

#[path = "edge_cases/http/http_api.rs"]
mod http_api;

#[path = "edge_cases/http/security.rs"]
mod security;

#[path = "edge_cases/http/body_limits.rs"]
mod body_limits;

#[path = "edge_cases/http/edge_limit_clamp.rs"]
mod edge_limit_clamp;
