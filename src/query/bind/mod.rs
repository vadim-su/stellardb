//! Binding stage: validates AST against schema before planning.
//!
//! The bind stage:
//! - For INSERT/CREATE: validates types, checks required fields, applies defaults
//! - For UPDATE: validates types of assignments
//! - For SELECT/DELETE: validates expressions (function arity, aggregate args)
//! - Collections without schemas work as flexible (no validation)

use std::collections::{HashMap, HashSet};

use crate::document::Value;
use crate::query::ast::{
    AggregateArg, Assignment, CreateAst, Expr, FtsOperator, InsertAst, ObjectLiteral, Statement,
    UpdateAst,
};
use crate::query::function::Registry as FnRegistry;
use crate::schema::{
    BindError, CollectionSchema, SchemaRegistry, validate_document, validate_field_type,
};
use crate::util::suggestions::find_similar;

/// Convenience alias for `Expr::try_to_value()`.
fn try_expr_to_value(expr: &Expr) -> Option<Value> {
    expr.try_to_value()
}

/// Bind a statement against the schema registry.
///
/// Returns a validated statement with defaults applied for INSERT/CREATE.
/// Collections without schemas pass through unchanged.
pub fn bind_statement(
    stmt: Statement,
    registry: &SchemaRegistry,
    fn_registry: &FnRegistry,
) -> Result<Statement, BindError> {
    match stmt {
        Statement::Select(select) => {
            let bound = bind_select(select, fn_registry)?;
            Ok(Statement::Select(bound))
        }
        Statement::Insert(insert) => {
            let bound = bind_insert(insert, registry)?;
            Ok(Statement::Insert(bound))
        }
        Statement::Create(create) => {
            let bound = bind_create(create, registry)?;
            Ok(Statement::Create(bound))
        }
        Statement::Update(update) => {
            let bound = bind_update(update, registry)?;
            Ok(Statement::Update(bound))
        }
        Statement::Let(let_ast) => {
            // Validate the expression in LET
            validate_expr(&let_ast.expr, fn_registry, false)?;
            Ok(Statement::Let(let_ast))
        }
        Statement::Expr(expr) => {
            // Validate the expression
            validate_expr(&expr, fn_registry, false)?;
            Ok(Statement::Expr(expr))
        }
        _ => Ok(stmt),
    }
}

/// Bind SELECT statement - validate aggregate functions and expressions.
fn bind_select(
    select: crate::query::ast::SelectAst,
    fn_registry: &FnRegistry,
) -> Result<crate::query::ast::SelectAst, BindError> {
    use crate::query::ast::Projection;

    if let Projection::Items(items) = &select.projection {
        let has_agg = items.iter().any(|i| i.expr.contains_aggregate());
        let has_field = items.iter().any(|i| !i.expr.contains_aggregate());

        if has_agg && has_field && select.group.is_none() {
            return Err(BindError::MixedAggregateAndFields);
        }

        // Validate GROUP: each non-aggregate field must be in the GROUP list
        if let Some(ref group_fields) = select.group {
            for item in items {
                if !item.expr.contains_aggregate() {
                    // Must be a plain field that's in the GROUP list
                    if let crate::query::ast::Expr::Field(ref f) = item.expr {
                        if !group_fields.contains(f) {
                            return Err(BindError::InvalidGroupField(f.clone()));
                        }
                    } else {
                        // Non-field, non-aggregate expression
                        let name = format!("{:?}", item.expr);
                        return Err(BindError::InvalidGroupField(name));
                    }
                }
            }
        }

        // Validate each projection expression
        for item in items {
            validate_expr(&item.expr, fn_registry, false)?;
        }
    }

    // Validate WHERE clause if present
    if let Some(ref filter) = select.filter {
        validate_expr(filter, fn_registry, false)?;
    }

    // Validate FTS operators across the entire SELECT statement
    let mut fts_context = FtsValidationContext::new();

    if let Projection::Items(items) = &select.projection {
        for item in items {
            collect_fts_operators(&item.expr, &mut fts_context);
        }
    }
    if let Some(ref filter) = select.filter {
        collect_fts_operators(filter, &mut fts_context);
    }

    fts_context.validate()?;

    Ok(select)
}

/// Context for collecting and validating FTS operators within a query.
struct FtsValidationContext {
    /// Whether we've seen a simple @@ operator
    has_simple_fts: bool,
    /// Names used by @:name@ operators
    named_fts_operators: Vec<String>,
    /// Names referenced by score("name") calls
    score_references: Vec<String>,
}

impl FtsValidationContext {
    fn new() -> Self {
        Self {
            has_simple_fts: false,
            named_fts_operators: Vec::new(),
            score_references: Vec::new(),
        }
    }

    /// Validate FTS constraints:
    /// 1. Cannot mix @@ and @:name@ in same query
    /// 2. Named operator names must be unique
    /// 3. score(:name) must reference existing FTS operator
    fn validate(&self) -> Result<(), BindError> {
        // Check for mixed operators
        if self.has_simple_fts && !self.named_fts_operators.is_empty() {
            return Err(BindError::FtsMixedOperators);
        }

        // Check for duplicate named operators
        let mut seen_names: HashSet<&str> = HashSet::new();
        for name in &self.named_fts_operators {
            if !seen_names.insert(name.as_str()) {
                return Err(BindError::FtsDuplicateOperatorName(name.clone()));
            }
        }

        // Check that all score() references point to existing named operators
        for score_name in &self.score_references {
            if !self.named_fts_operators.contains(score_name) {
                return Err(BindError::FtsUnknownScoreName(score_name.clone()));
            }
        }

        Ok(())
    }
}

/// Recursively collect FTS operators and score() references from an expression tree.
fn collect_fts_operators(expr: &Expr, ctx: &mut FtsValidationContext) {
    match expr {
        Expr::Fts { operator, .. } => match operator {
            FtsOperator::Simple => ctx.has_simple_fts = true,
            FtsOperator::Named(name) => ctx.named_fts_operators.push(name.clone()),
        },
        Expr::FunctionCall {
            name,
            namespace,
            args,
        } => {
            // Check if this is fts::score() with a name argument
            if name.eq_ignore_ascii_case("score")
                && namespace.as_deref() == Some("fts")
                && args.len() == 1
                && let Expr::Literal(Value::String(score_name)) = &args[0]
            {
                ctx.score_references.push(score_name.clone());
            }
            // Recurse into arguments
            for arg in args {
                collect_fts_operators(arg, ctx);
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            collect_fts_operators(left, ctx);
            collect_fts_operators(right, ctx);
        }
        Expr::UnaryOp { expr, .. } => collect_fts_operators(expr, ctx),
        Expr::Aggregate(_) => {} // Aggregates don't contain FTS operators
        Expr::InList { expr, list, .. } => {
            collect_fts_operators(expr, ctx);
            for item in list {
                collect_fts_operators(item, ctx);
            }
        }
        Expr::InSubquery { expr, .. } => {
            collect_fts_operators(expr, ctx);
            // Note: We don't recurse into subqueries - each subquery has its own FTS context
        }
        Expr::InExpr { expr, target, .. } => {
            collect_fts_operators(expr, ctx);
            collect_fts_operators(target, ctx);
        }
        Expr::Subquery(_) => {
            // Subqueries have their own FTS validation context
        }
        Expr::Array(items) => {
            for item in items {
                collect_fts_operators(item, ctx);
            }
        }
        Expr::Object(pairs) => {
            for (_, value_expr) in pairs {
                collect_fts_operators(value_expr, ctx);
            }
        }
        Expr::Index { base, index } => {
            collect_fts_operators(base, ctx);
            collect_fts_operators(index, ctx);
        }
        Expr::Slice { base, start, end } => {
            collect_fts_operators(base, ctx);
            if let Some(s) = start {
                collect_fts_operators(s, ctx);
            }
            if let Some(e) = end {
                collect_fts_operators(e, ctx);
            }
        }
        Expr::FieldAccess { base, .. } => collect_fts_operators(base, ctx),
        // Leaf nodes without FTS operators
        Expr::Literal(_)
        | Expr::Field(_)
        | Expr::ParentRef(_)
        | Expr::ValueRef(_)
        | Expr::Constant { .. }
        | Expr::Variable(_) => {}
        Expr::KnnSearch { vector, .. } => collect_fts_operators(vector, ctx),
        // Traversal expressions may contain edge filters that could have FTS operators
        Expr::Traversal(t) => {
            for step in &t.steps {
                if let Some(filter) = &step.edge_filter {
                    collect_fts_operators(filter, ctx);
                }
            }
        }
        // Range expression - recurse into start and end
        Expr::Range { start, end } => {
            collect_fts_operators(start, ctx);
            collect_fts_operators(end, ctx);
        }
        // Control flow - recurse into sub-expressions
        Expr::If {
            branches,
            else_block,
        } => {
            for (cond, block) in branches {
                collect_fts_operators(cond, ctx);
                for stmt in &block.statements {
                    if let crate::query::ast::Statement::Expr(e) = stmt {
                        collect_fts_operators(e, ctx);
                    }
                }
            }
            if let Some(b) = else_block {
                for stmt in &b.statements {
                    if let crate::query::ast::Statement::Expr(e) = stmt {
                        collect_fts_operators(e, ctx);
                    }
                }
            }
        }
        Expr::For { iterable, body, .. } => {
            collect_fts_operators(iterable, ctx);
            for stmt in &body.statements {
                if let crate::query::ast::Statement::Expr(e) = stmt {
                    collect_fts_operators(e, ctx);
                }
            }
        }
        Expr::Break | Expr::Continue => {}
    }
}

/// Recursively validate an expression tree.
/// `in_subquery` tracks whether we are inside a correlated subquery expression,
/// which is the only context where `$parent` references are valid.
fn validate_expr(
    expr: &Expr,
    fn_registry: &FnRegistry,
    in_subquery: bool,
) -> Result<(), BindError> {
    match expr {
        Expr::Literal(_) | Expr::Field(_) => Ok(()),
        Expr::BinaryOp { left, right, .. } => {
            validate_expr(left, fn_registry, in_subquery)?;
            validate_expr(right, fn_registry, in_subquery)?;
            Ok(())
        }
        Expr::UnaryOp { expr, .. } => validate_expr(expr, fn_registry, in_subquery),
        Expr::FunctionCall {
            name,
            namespace,
            args,
        } => {
            // Check if this is actually a constant being called as function
            if let Some(ns) = namespace
                && fn_registry.is_constant(ns, name)
            {
                return Err(BindError::ConstantCalledAsFunction {
                    namespace: ns.clone(),
                    name: name.clone(),
                });
            }
            // Validate arity for known scalar functions
            validate_scalar_arity(name, args.len())?;
            for arg in args {
                validate_expr(arg, fn_registry, in_subquery)?;
            }
            Ok(())
        }
        Expr::Aggregate(call) => {
            // Validate aggregate function exists
            let func = crate::query::function::AggregateFunction::from_name(&call.function)
                .ok_or_else(|| BindError::UnknownFunction {
                    namespace: call.namespace.clone().unwrap_or_else(|| "builtin".into()),
                    name: call.function.clone(),
                })?;
            // Only COUNT allows *
            match (&func, &call.arg) {
                (crate::query::function::AggregateFunction::Count, _) => {}
                (_, AggregateArg::Wildcard) => {
                    return Err(BindError::InvalidAggregateArg {
                        function: call.function.clone(),
                        reason: "only COUNT allows * argument".into(),
                    });
                }
                _ => {}
            }
            Ok(())
        }
        Expr::InList { expr, list, .. } => {
            validate_expr(expr, fn_registry, in_subquery)?;
            for item in list {
                validate_expr(item, fn_registry, in_subquery)?;
            }
            Ok(())
        }
        Expr::InSubquery { expr, .. } => {
            validate_expr(expr, fn_registry, in_subquery)?;
            Ok(())
        }
        Expr::InExpr { expr, target, .. } => {
            validate_expr(expr, fn_registry, in_subquery)?;
            validate_expr(target, fn_registry, in_subquery)?;
            Ok(())
        }
        Expr::Subquery(select) => {
            // Validate inner select with in_subquery=true so $parent refs are allowed
            if let crate::query::ast::Projection::Items(items) = &select.projection {
                for item in items {
                    validate_expr(&item.expr, fn_registry, true)?;
                }
            }
            if let Some(ref filter) = select.filter {
                validate_expr(filter, fn_registry, true)?;
            }
            Ok(())
        }
        Expr::ParentRef(_) => {
            if in_subquery {
                Ok(())
            } else {
                Err(BindError::ParentRefOutsideSubquery)
            }
        }
        Expr::Array(items) => {
            for item in items {
                validate_expr(item, fn_registry, in_subquery)?;
            }
            Ok(())
        }
        Expr::Object(pairs) => {
            for (_, expr) in pairs {
                validate_expr(expr, fn_registry, in_subquery)?;
            }
            Ok(())
        }
        Expr::ValueRef(_) => Ok(()),
        Expr::Index { base, index } => {
            validate_expr(base, fn_registry, in_subquery)?;
            validate_expr(index, fn_registry, in_subquery)?;
            Ok(())
        }
        Expr::Slice { base, start, end } => {
            validate_expr(base, fn_registry, in_subquery)?;
            if let Some(s) = start {
                validate_expr(s, fn_registry, in_subquery)?;
            }
            if let Some(e) = end {
                validate_expr(e, fn_registry, in_subquery)?;
            }
            Ok(())
        }
        Expr::FieldAccess { base, .. } => {
            validate_expr(base, fn_registry, in_subquery)?;
            Ok(())
        }
        Expr::Constant { namespace, name } => {
            // Validate constant exists in registry
            if !fn_registry.is_constant(namespace, name) {
                // Check if it's actually a function being used without parens
                if fn_registry.is_function(namespace, name) {
                    return Err(BindError::FunctionUsedAsConstant {
                        namespace: namespace.clone(),
                        name: name.clone(),
                    });
                }
                return Err(BindError::UnknownConstant {
                    namespace: namespace.clone(),
                    name: name.clone(),
                    suggestions: fn_registry.find_similar_constants(namespace, name),
                });
            }
            Ok(())
        }
        // Variables are checked at runtime (VarScope may change)
        Expr::Variable(_) => Ok(()),
        // FTS expressions are validated at runtime
        Expr::Fts { .. } => Ok(()),
        // KNN search expressions are validated at runtime
        Expr::KnnSearch { vector, .. } => validate_expr(vector, fn_registry, in_subquery),
        // Traversal expressions - validate any edge filters
        Expr::Traversal(t) => {
            for step in &t.steps {
                if let Some(filter) = &step.edge_filter {
                    validate_expr(filter, fn_registry, in_subquery)?;
                }
            }
            Ok(())
        }
        // Range expression - validate start and end
        Expr::Range { start, end } => {
            validate_expr(start, fn_registry, in_subquery)?;
            validate_expr(end, fn_registry, in_subquery)?;
            Ok(())
        }
        // Control flow - validate sub-expressions
        Expr::If {
            branches,
            else_block,
        } => {
            for (cond, block) in branches {
                validate_expr(cond, fn_registry, in_subquery)?;
                for stmt in &block.statements {
                    if let crate::query::ast::Statement::Expr(e) = stmt {
                        validate_expr(e, fn_registry, in_subquery)?;
                    }
                }
            }
            if let Some(b) = else_block {
                for stmt in &b.statements {
                    if let crate::query::ast::Statement::Expr(e) = stmt {
                        validate_expr(e, fn_registry, in_subquery)?;
                    }
                }
            }
            Ok(())
        }
        Expr::For { iterable, body, .. } => {
            validate_expr(iterable, fn_registry, in_subquery)?;
            for stmt in &body.statements {
                if let crate::query::ast::Statement::Expr(e) = stmt {
                    validate_expr(e, fn_registry, in_subquery)?;
                }
            }
            Ok(())
        }
        Expr::Break | Expr::Continue => Ok(()),
    }
}

/// Validate scalar function arity for known builtins.
fn validate_scalar_arity(name: &str, got: usize) -> Result<(), BindError> {
    match name.to_lowercase().as_str() {
        // 1 argument
        "upper" | "lower" | "length" | "trim" => {
            if got != 1 {
                return Err(BindError::ArityMismatch {
                    function: name.to_string(),
                    expected: 1,
                    got,
                });
            }
        }
        // 2 arguments
        "split" | "starts_with" | "ends_with" => {
            if got != 2 {
                return Err(BindError::ArityMismatch {
                    function: name.to_string(),
                    expected: 2,
                    got,
                });
            }
        }
        // 3 arguments
        "replace" => {
            if got != 3 {
                return Err(BindError::ArityMismatch {
                    function: name.to_string(),
                    expected: 3,
                    got,
                });
            }
        }
        // 2-3 arguments
        "substring" => {
            if !(2..=3).contains(&got) {
                return Err(BindError::ArityMismatch {
                    function: name.to_string(),
                    expected: 2,
                    got,
                });
            }
        }
        // variadic (1+)
        "concat" => {
            if got == 0 {
                return Err(BindError::ArityMismatch {
                    function: name.to_string(),
                    expected: 1,
                    got: 0,
                });
            }
        }
        // Array functions - 1 argument
        "array_length" | "array_reverse" | "array_flatten" => {
            if got != 1 {
                return Err(BindError::ArityMismatch {
                    function: name.to_string(),
                    expected: 1,
                    got,
                });
            }
        }
        // Array functions - 2 arguments
        "array_contains" | "array_append" => {
            if got != 2 {
                return Err(BindError::ArityMismatch {
                    function: name.to_string(),
                    expected: 2,
                    got,
                });
            }
        }
        // Math functions - 0 arguments (constants)
        "math_pi" | "math_e" => {
            if got != 0 {
                return Err(BindError::ArityMismatch {
                    function: name.to_string(),
                    expected: 0,
                    got,
                });
            }
        }
        // Math functions - 1 argument
        "math_abs" | "math_round" | "math_floor" | "math_ceil" | "math_trunc" | "math_sign"
        | "math_sqrt" | "math_exp" | "math_sin" | "math_cos" | "math_tan" => {
            if got != 1 {
                return Err(BindError::ArityMismatch {
                    function: name.to_string(),
                    expected: 1,
                    got,
                });
            }
        }
        // Math functions - 2 arguments
        "math_pow" | "math_log" | "math_mod" | "math_min" | "math_max" => {
            if got != 2 {
                return Err(BindError::ArityMismatch {
                    function: name.to_string(),
                    expected: 2,
                    got,
                });
            }
        }
        // Type functions - 1 argument
        "type_to_int" | "type_to_float" | "type_to_string" | "type_to_bool" | "type_is_null"
        | "type_is_int" | "type_is_float" | "type_is_number" | "type_is_string"
        | "type_is_bool" | "type_is_array" | "type_is_object" | "type_of" => {
            if got != 1 {
                return Err(BindError::ArityMismatch {
                    function: name.to_string(),
                    expected: 1,
                    got,
                });
            }
        }
        // Type functions - 2 arguments
        "type_default" => {
            if got != 2 {
                return Err(BindError::ArityMismatch {
                    function: name.to_string(),
                    expected: 2,
                    got,
                });
            }
        }
        // Type functions - variadic (2+)
        "type_coalesce" if got < 2 => {
            return Err(BindError::ArityMismatch {
                function: name.to_string(),
                expected: 2,
                got,
            });
        }
        _ => {} // unknown function -- will fail at eval time
    }
    Ok(())
}

/// Bind INSERT statement - validate each document against schema.
fn bind_insert(insert: InsertAst, registry: &SchemaRegistry) -> Result<InsertAst, BindError> {
    // Check if collection exists
    if !registry.collection_exists(&insert.collection) {
        return Err(BindError::UnknownCollection {
            name: insert.collection.clone(),
            suggestions: registry.find_similar_collections(&insert.collection),
        });
    }

    let schema = registry.get(&insert.collection);

    // Collection exists but no schema = flexible, pass through
    let Some(schema) = schema else {
        return Ok(insert);
    };

    let mut bound_objects = Vec::with_capacity(insert.objects.len());

    for obj in insert.objects {
        let bound_fields = validate_object_literal(&schema, &obj)?;
        bound_objects.push(ObjectLiteral {
            fields: bound_fields,
        });
    }

    Ok(InsertAst {
        collection: insert.collection,
        objects: bound_objects,
    })
}

/// Bind CREATE statement - validate assignments against schema.
fn bind_create(create: CreateAst, registry: &SchemaRegistry) -> Result<CreateAst, BindError> {
    let collection = &create.target.collection;

    // Check if collection exists
    if !registry.collection_exists(collection) {
        return Err(BindError::UnknownCollection {
            name: collection.clone(),
            suggestions: registry.find_similar_collections(collection),
        });
    }

    let schema = registry.get(collection);

    // Collection exists but no schema = flexible, pass through
    let Some(schema) = schema else {
        return Ok(create);
    };

    // Convert assignments with constant expressions (Literal, Object, Array) to HashMap for validation
    // Non-constant expressions (fields, functions) will be validated at runtime
    let mut fields: HashMap<String, Value> = HashMap::new();
    for a in &create.assignments {
        let field_name = a.path.join(".");
        // Try to extract constant value from expression (Literal, Object, Array)
        if let Some(value) = try_expr_to_value(&a.expr) {
            fields.insert(field_name, value);
        }
    }

    // Validate literal fields and get defaults applied
    let validated = validate_document(&schema, &fields)?;

    // Build assignments: keep original non-constant expressions,
    // update constant ones with validated/defaulted values
    let mut assignments: Vec<Assignment> = Vec::new();

    // First, add validated/defaulted fields as literal expressions
    for (field, value) in validated {
        // Check if this field was in the original assignments as non-constant expression
        // (e.g., function call, field reference - things that need runtime evaluation)
        let is_non_constant = create
            .assignments
            .iter()
            .any(|a| a.path.join(".") == field && try_expr_to_value(&a.expr).is_none());

        if !is_non_constant {
            assignments.push(Assignment {
                path: field.split('.').map(String::from).collect(),
                expr: Expr::Literal(value),
            });
        }
    }

    // Add back non-constant expressions (they weren't validated)
    for a in &create.assignments {
        if try_expr_to_value(&a.expr).is_none() {
            assignments.push(a.clone());
        }
    }

    Ok(CreateAst {
        target: create.target,
        assignments,
    })
}

/// Bind UPDATE statement - validate assignment types only.
fn bind_update(update: UpdateAst, registry: &SchemaRegistry) -> Result<UpdateAst, BindError> {
    let collection = &update.target.collection;

    // Check if collection exists
    if !registry.collection_exists(collection) {
        return Err(BindError::UnknownCollection {
            name: collection.clone(),
            suggestions: registry.find_similar_collections(collection),
        });
    }

    let schema = registry.get(collection);

    // Collection exists but no schema = flexible, pass through
    let Some(schema) = schema else {
        return Ok(update);
    };

    // Validate each assignment type
    for assignment in &update.assignments {
        validate_assignment(&schema, assignment)?;
    }

    // UPDATE doesn't apply defaults - just validates types
    Ok(update)
}

/// Validate an ObjectLiteral against schema, return fields with defaults.
fn validate_object_literal(
    schema: &CollectionSchema,
    obj: &ObjectLiteral,
) -> Result<Vec<(String, Value)>, BindError> {
    // Convert ObjectLiteral fields to HashMap for validation
    let fields: HashMap<String, Value> = obj.fields.iter().cloned().collect();
    let validated = validate_document(schema, &fields)?;
    // Convert back to Vec<(String, Value)>
    Ok(validated.into_iter().collect())
}

/// Validate a single assignment against schema field definition.
fn validate_assignment(
    schema: &CollectionSchema,
    assignment: &Assignment,
) -> Result<(), BindError> {
    // Get the top-level field name from path
    let field_name = assignment.path.first().map(|s| s.as_str()).unwrap_or("");

    // Skip "id" field - it's special
    if field_name == "id" {
        return Ok(());
    }

    // Find field in schema
    let field_def = schema.get_field(field_name);

    match field_def {
        Some(def) => {
            // For literal expressions, we can validate at bind time
            if let Expr::Literal(value) = &assignment.expr {
                // Check if trying to set required field to null
                if def.required && matches!(value, Value::Null) {
                    return Err(BindError::RequiredFieldNull {
                        collection: schema.name.clone(),
                        field: field_name.to_string(),
                    });
                }

                // Validate type matches field definition
                validate_field_type(field_name, value, &def.field_type)?;
            }
            // Non-literal expressions: type validation deferred to runtime

            Ok(())
        }
        None => {
            // Unknown field in strict mode is an error
            if schema.is_strict() {
                let field_names: Vec<&str> =
                    schema.fields.iter().map(|f| f.name.as_str()).collect();
                return Err(BindError::UnknownField {
                    collection: schema.name.clone(),
                    field: field_name.to_string(),
                    suggestions: find_similar(field_name, field_names.into_iter(), 3)
                        .into_iter()
                        .map(|s| s.to_string())
                        .collect(),
                });
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::ast::{DataSource, Expr, ProjectionItem, Target, UnaryOp};
    use crate::query::function::default_registry;
    use crate::schema::{CollectionSchema, DefaultValue, FieldDef, FieldType, SchemaMode};

    fn make_registry_with_schema(schema: CollectionSchema) -> SchemaRegistry {
        let registry = SchemaRegistry::new();
        registry.register(schema);
        registry
    }

    #[test]
    fn test_insert_unknown_collection_fails() {
        let registry = SchemaRegistry::new();

        let insert = InsertAst {
            collection: "unknown".to_string(),
            objects: vec![ObjectLiteral {
                fields: vec![("name".to_string(), Value::String("Alice".to_string()))],
            }],
        };

        let result = bind_statement(Statement::Insert(insert), &registry, &default_registry());
        assert!(
            matches!(result, Err(BindError::UnknownCollection { ref name, .. }) if name == "unknown")
        );
    }

    #[test]
    fn test_insert_schemaless_collection_passes_through() {
        let registry = SchemaRegistry::new();
        registry.register_schemaless("test");

        let insert = InsertAst {
            collection: "test".to_string(),
            objects: vec![ObjectLiteral {
                fields: vec![("name".to_string(), Value::String("Alice".to_string()))],
            }],
        };

        let result = bind_statement(
            Statement::Insert(insert.clone()),
            &registry,
            &default_registry(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_insert_validates_required_field() {
        let schema = CollectionSchema {
            name: "user".to_string(),
            mode: SchemaMode::Strict,
            fields: vec![FieldDef {
                name: "name".to_string(),
                field_type: FieldType::String,
                required: true,
                default: None,
            }],
            indexes: vec![],
        };
        let registry = make_registry_with_schema(schema);

        // Missing required field
        let insert = InsertAst {
            collection: "user".to_string(),
            objects: vec![ObjectLiteral { fields: vec![] }],
        };

        let result = bind_statement(Statement::Insert(insert), &registry, &default_registry());
        assert!(matches!(
            result,
            Err(BindError::RequiredFieldMissing { .. })
        ));
    }

    #[test]
    fn test_insert_applies_default() {
        let schema = CollectionSchema {
            name: "user".to_string(),
            mode: SchemaMode::Flexible,
            fields: vec![FieldDef {
                name: "status".to_string(),
                field_type: FieldType::String,
                required: false,
                default: Some(DefaultValue::String("active".to_string())),
            }],
            indexes: vec![],
        };
        let registry = make_registry_with_schema(schema);

        let insert = InsertAst {
            collection: "user".to_string(),
            objects: vec![ObjectLiteral { fields: vec![] }],
        };

        let result =
            bind_statement(Statement::Insert(insert), &registry, &default_registry()).unwrap();

        if let Statement::Insert(bound) = result {
            let fields = &bound.objects[0].fields;
            let has_status = fields
                .iter()
                .any(|(k, v)| k == "status" && *v == Value::String("active".to_string()));
            assert!(has_status, "Default 'status' field should be applied");
        } else {
            panic!("Expected Insert statement");
        }
    }

    #[test]
    fn test_create_validates_and_applies_defaults() {
        let schema = CollectionSchema {
            name: "user".to_string(),
            mode: SchemaMode::Flexible,
            fields: vec![
                FieldDef {
                    name: "name".to_string(),
                    field_type: FieldType::String,
                    required: true,
                    default: None,
                },
                FieldDef {
                    name: "age".to_string(),
                    field_type: FieldType::Int,
                    required: false,
                    default: Some(DefaultValue::Int(0)),
                },
            ],
            indexes: vec![],
        };
        let registry = make_registry_with_schema(schema);

        let create = CreateAst {
            target: Target {
                collection: "user".to_string(),
                key: Some("alice".to_string()),
            },
            assignments: vec![Assignment {
                path: vec!["name".to_string()],
                expr: Expr::Literal(Value::String("Alice".to_string())),
            }],
        };

        let result =
            bind_statement(Statement::Create(create), &registry, &default_registry()).unwrap();

        if let Statement::Create(bound) = result {
            let has_age = bound.assignments.iter().any(|a| a.path == vec!["age"]);
            assert!(has_age, "Default 'age' field should be applied");
        } else {
            panic!("Expected Create statement");
        }
    }

    #[test]
    fn test_update_rejects_null_on_required() {
        let schema = CollectionSchema {
            name: "user".to_string(),
            mode: SchemaMode::Flexible,
            fields: vec![FieldDef {
                name: "name".to_string(),
                field_type: FieldType::String,
                required: true,
                default: None,
            }],
            indexes: vec![],
        };
        let registry = make_registry_with_schema(schema);

        let update = UpdateAst {
            target: Target {
                collection: "user".to_string(),
                key: Some("alice".to_string()),
            },
            filter: None,
            assignments: vec![Assignment {
                path: vec!["name".to_string()],
                expr: Expr::Literal(Value::Null),
            }],
        };

        let result = bind_statement(Statement::Update(update), &registry, &default_registry());
        assert!(matches!(result, Err(BindError::RequiredFieldNull { .. })));
    }

    #[test]
    fn test_update_rejects_unknown_field_strict() {
        let schema = CollectionSchema {
            name: "user".to_string(),
            mode: SchemaMode::Strict,
            fields: vec![FieldDef {
                name: "name".to_string(),
                field_type: FieldType::String,
                required: true,
                default: None,
            }],
            indexes: vec![],
        };
        let registry = make_registry_with_schema(schema);

        let update = UpdateAst {
            target: Target {
                collection: "user".to_string(),
                key: Some("alice".to_string()),
            },
            filter: None,
            assignments: vec![Assignment {
                path: vec!["unknown".to_string()],
                expr: Expr::Literal(Value::String("value".to_string())),
            }],
        };

        let result = bind_statement(Statement::Update(update), &registry, &default_registry());
        assert!(matches!(result, Err(BindError::UnknownField { .. })));
    }

    #[test]
    fn test_select_passes_through() {
        let registry = SchemaRegistry::new();

        let select = crate::query::ast::SelectAst {
            projection: crate::query::ast::Projection::All,
            distinct: false,
            value_mode: false,
            source: DataSource::Collection(Target {
                collection: "user".to_string(),
                key: None,
            }),
            filter: None,
            group: None,
            order: None,
            limit: None,
            offset: None,
            fetch: None,
        };

        let result = bind_statement(
            Statement::Select(select.clone()),
            &registry,
            &default_registry(),
        );
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), Statement::Select(_)));
    }

    #[test]
    fn test_select_aggregate_validates() {
        let registry = SchemaRegistry::new();

        let select = crate::query::ast::SelectAst {
            projection: crate::query::ast::Projection::Items(vec![ProjectionItem {
                expr: Expr::Aggregate(crate::query::ast::AggregateCall {
                    namespace: None,
                    function: "count".to_string(),
                    arg: crate::query::ast::AggregateArg::Wildcard,
                    distinct: false,
                }),
                alias: None,
            }]),
            distinct: false,
            value_mode: false,
            source: DataSource::Collection(Target {
                collection: "user".to_string(),
                key: None,
            }),
            filter: None,
            group: None,
            order: None,
            limit: None,
            offset: None,
            fetch: None,
        };

        let result = bind_statement(Statement::Select(select), &registry, &default_registry());
        assert!(result.is_ok());
    }

    #[test]
    fn test_select_mixed_agg_and_field_fails() {
        let registry = SchemaRegistry::new();

        let select = crate::query::ast::SelectAst {
            projection: crate::query::ast::Projection::Items(vec![
                ProjectionItem {
                    expr: Expr::Field("name".to_string()),
                    alias: None,
                },
                ProjectionItem {
                    expr: Expr::Aggregate(crate::query::ast::AggregateCall {
                        namespace: None,
                        function: "count".to_string(),
                        arg: crate::query::ast::AggregateArg::Wildcard,
                        distinct: false,
                    }),
                    alias: None,
                },
            ]),
            distinct: false,
            value_mode: false,
            source: DataSource::Collection(Target {
                collection: "user".to_string(),
                key: None,
            }),
            filter: None,
            group: None,
            order: None,
            limit: None,
            offset: None,
            fetch: None,
        };

        let result = bind_statement(Statement::Select(select), &registry, &default_registry());
        assert!(matches!(result, Err(BindError::MixedAggregateAndFields)));
    }

    #[test]
    fn test_select_sum_star_fails() {
        let registry = SchemaRegistry::new();

        let select = crate::query::ast::SelectAst {
            projection: crate::query::ast::Projection::Items(vec![ProjectionItem {
                expr: Expr::Aggregate(crate::query::ast::AggregateCall {
                    namespace: None,
                    function: "sum".to_string(),
                    arg: crate::query::ast::AggregateArg::Wildcard,
                    distinct: false,
                }),
                alias: None,
            }]),
            distinct: false,
            value_mode: false,
            source: DataSource::Collection(Target {
                collection: "user".to_string(),
                key: None,
            }),
            filter: None,
            group: None,
            order: None,
            limit: None,
            offset: None,
            fetch: None,
        };

        let result = bind_statement(Statement::Select(select), &registry, &default_registry());
        assert!(matches!(result, Err(BindError::InvalidAggregateArg { .. })));
    }

    #[test]
    fn test_select_unknown_function_fails() {
        let registry = SchemaRegistry::new();

        let select = crate::query::ast::SelectAst {
            projection: crate::query::ast::Projection::Items(vec![ProjectionItem {
                expr: Expr::Aggregate(crate::query::ast::AggregateCall {
                    namespace: None,
                    function: "foobar".to_string(),
                    arg: crate::query::ast::AggregateArg::Wildcard,
                    distinct: false,
                }),
                alias: None,
            }]),
            distinct: false,
            value_mode: false,
            source: DataSource::Collection(Target {
                collection: "user".to_string(),
                key: None,
            }),
            filter: None,
            group: None,
            order: None,
            limit: None,
            offset: None,
            fetch: None,
        };

        let result = bind_statement(Statement::Select(select), &registry, &default_registry());
        assert!(matches!(result, Err(BindError::UnknownFunction { .. })));
    }

    #[test]
    fn test_scalar_arity_upper_requires_one_arg() {
        let result = validate_scalar_arity("upper", 0);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));

        let result = validate_scalar_arity("upper", 2);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));

        let result = validate_scalar_arity("upper", 1);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scalar_arity_lower_requires_one_arg() {
        let result = validate_scalar_arity("lower", 0);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));

        let result = validate_scalar_arity("lower", 1);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scalar_arity_concat_requires_at_least_one() {
        let result = validate_scalar_arity("concat", 0);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));

        let result = validate_scalar_arity("concat", 1);
        assert!(result.is_ok());

        let result = validate_scalar_arity("concat", 3);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_expr_function_call_arity() {
        // UPPER with no args should fail
        let expr = Expr::FunctionCall {
            name: "upper".to_string(),
            namespace: None,
            args: vec![],
        };
        assert!(validate_expr(&expr, &default_registry(), false).is_err());

        // UPPER with one arg should succeed
        let expr = Expr::FunctionCall {
            name: "upper".to_string(),
            namespace: None,
            args: vec![Expr::Field("name".to_string())],
        };
        assert!(validate_expr(&expr, &default_registry(), false).is_ok());
    }

    #[test]
    fn test_parent_ref_outside_subquery_fails() {
        let expr = Expr::ParentRef(crate::query::ast::ParentRef {
            depth: 0,
            field: "id".to_string(),
        });
        let result = validate_expr(&expr, &default_registry(), false);
        assert!(matches!(result, Err(BindError::ParentRefOutsideSubquery)));
    }

    #[test]
    fn test_parent_ref_inside_subquery_ok() {
        let expr = Expr::ParentRef(crate::query::ast::ParentRef {
            depth: 0,
            field: "id".to_string(),
        });
        let result = validate_expr(&expr, &default_registry(), true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_subquery_with_parent_ref_in_filter_ok() {
        use crate::query::ast::{BinaryOp, Projection, SelectAst};
        let subquery_expr = Expr::Subquery(Box::new(SelectAst {
            projection: Projection::All,
            distinct: false,
            value_mode: false,
            source: DataSource::Collection(Target {
                collection: "orders".to_string(),
                key: None,
            }),
            filter: Some(Expr::BinaryOp {
                left: Box::new(Expr::Field("user_id".to_string())),
                op: BinaryOp::Eq,
                right: Box::new(Expr::ParentRef(crate::query::ast::ParentRef {
                    depth: 0,
                    field: "id".to_string(),
                })),
            }),
            group: None,
            order: None,
            limit: None,
            offset: None,
            fetch: None,
        }));
        // Subquery at top level (not in_subquery) should be fine — the inner $parent is valid
        let result = validate_expr(&subquery_expr, &default_registry(), false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scalar_arity_length_requires_one() {
        let result = validate_scalar_arity("length", 0);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("length", 1);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scalar_arity_substring_requires_two_or_three() {
        let result = validate_scalar_arity("substring", 1);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("substring", 2);
        assert!(result.is_ok());
        let result = validate_scalar_arity("substring", 3);
        assert!(result.is_ok());
        let result = validate_scalar_arity("substring", 4);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
    }

    #[test]
    fn test_scalar_arity_replace_requires_three() {
        let result = validate_scalar_arity("replace", 2);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("replace", 3);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scalar_arity_trim_requires_one() {
        let result = validate_scalar_arity("trim", 0);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("trim", 1);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scalar_arity_split_requires_two() {
        let result = validate_scalar_arity("split", 1);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("split", 2);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scalar_arity_starts_ends_with_requires_two() {
        let result = validate_scalar_arity("starts_with", 1);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("starts_with", 2);
        assert!(result.is_ok());

        let result = validate_scalar_arity("ends_with", 1);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("ends_with", 2);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scalar_arity_array_length_requires_one() {
        let result = validate_scalar_arity("array_length", 0);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("array_length", 1);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scalar_arity_array_contains_requires_two() {
        let result = validate_scalar_arity("array_contains", 1);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("array_contains", 2);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scalar_arity_array_append_requires_two() {
        let result = validate_scalar_arity("array_append", 1);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("array_append", 2);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scalar_arity_array_reverse_requires_one() {
        let result = validate_scalar_arity("array_reverse", 0);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("array_reverse", 1);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scalar_arity_array_flatten_requires_one() {
        let result = validate_scalar_arity("array_flatten", 0);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("array_flatten", 1);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scalar_arity_math_pi_requires_zero() {
        let result = validate_scalar_arity("math_pi", 1);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("math_pi", 0);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scalar_arity_math_e_requires_zero() {
        let result = validate_scalar_arity("math_e", 1);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("math_e", 0);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scalar_arity_math_abs_requires_one() {
        let result = validate_scalar_arity("math_abs", 0);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("math_abs", 2);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("math_abs", 1);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scalar_arity_math_sqrt_requires_one() {
        let result = validate_scalar_arity("math_sqrt", 0);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("math_sqrt", 1);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scalar_arity_math_pow_requires_two() {
        let result = validate_scalar_arity("math_pow", 1);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("math_pow", 3);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("math_pow", 2);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scalar_arity_math_log_requires_two() {
        let result = validate_scalar_arity("math_log", 1);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("math_log", 2);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scalar_arity_math_min_max_requires_two() {
        let result = validate_scalar_arity("math_min", 1);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("math_min", 2);
        assert!(result.is_ok());

        let result = validate_scalar_arity("math_max", 1);
        assert!(matches!(result, Err(BindError::ArityMismatch { .. })));
        let result = validate_scalar_arity("math_max", 2);
        assert!(result.is_ok());
    }

    // ========================
    // FTS Validation Tests
    // ========================

    use crate::query::ast::{FtsTarget, Projection, SelectAst};

    fn make_fts_select_with_filter(filter: Expr) -> SelectAst {
        SelectAst {
            projection: Projection::All,
            distinct: false,
            value_mode: false,
            source: DataSource::Collection(Target {
                collection: "articles".to_string(),
                key: None,
            }),
            filter: Some(filter),
            group: None,
            order: None,
            limit: None,
            offset: None,
            fetch: None,
        }
    }

    fn make_fts_simple(field: &str, query: &str) -> Expr {
        Expr::Fts {
            target: FtsTarget::Field(field.to_string()),
            operator: FtsOperator::Simple,
            query: query.to_string(),
        }
    }

    fn make_fts_named(field: &str, name: &str, query: &str) -> Expr {
        Expr::Fts {
            target: FtsTarget::Field(field.to_string()),
            operator: FtsOperator::Named(name.to_string()),
            query: query.to_string(),
        }
    }

    #[test]
    fn test_fts_simple_operator_ok() {
        let registry = SchemaRegistry::new();
        let select = make_fts_select_with_filter(make_fts_simple("content", "hello world"));

        let result = bind_statement(Statement::Select(select), &registry, &default_registry());
        assert!(result.is_ok());
    }

    #[test]
    fn test_fts_named_operator_ok() {
        let registry = SchemaRegistry::new();
        let select = make_fts_select_with_filter(make_fts_named("content", "main", "hello world"));

        let result = bind_statement(Statement::Select(select), &registry, &default_registry());
        assert!(result.is_ok());
    }

    #[test]
    fn test_fts_multiple_named_operators_ok() {
        use crate::query::ast::BinaryOp;

        let registry = SchemaRegistry::new();
        // content @:title@ "rust" AND body @:body@ "programming"
        let filter = Expr::BinaryOp {
            left: Box::new(make_fts_named("content", "title", "rust")),
            op: BinaryOp::And,
            right: Box::new(make_fts_named("body", "body", "programming")),
        };
        let select = make_fts_select_with_filter(filter);

        let result = bind_statement(Statement::Select(select), &registry, &default_registry());
        assert!(result.is_ok());
    }

    #[test]
    fn test_fts_mixed_operators_fails() {
        use crate::query::ast::BinaryOp;

        let registry = SchemaRegistry::new();
        // content @@ "rust" AND body @:body@ "programming" -- MIXED!
        let filter = Expr::BinaryOp {
            left: Box::new(make_fts_simple("content", "rust")),
            op: BinaryOp::And,
            right: Box::new(make_fts_named("body", "body", "programming")),
        };
        let select = make_fts_select_with_filter(filter);

        let result = bind_statement(Statement::Select(select), &registry, &default_registry());
        assert!(matches!(result, Err(BindError::FtsMixedOperators)));
    }

    #[test]
    fn test_fts_duplicate_named_operators_fails() {
        use crate::query::ast::BinaryOp;

        let registry = SchemaRegistry::new();
        // content @:main@ "rust" AND body @:main@ "programming" -- DUPLICATE!
        let filter = Expr::BinaryOp {
            left: Box::new(make_fts_named("content", "main", "rust")),
            op: BinaryOp::And,
            right: Box::new(make_fts_named("body", "main", "programming")),
        };
        let select = make_fts_select_with_filter(filter);

        let result = bind_statement(Statement::Select(select), &registry, &default_registry());
        assert!(matches!(result, Err(BindError::FtsDuplicateOperatorName(ref n)) if n == "main"));
    }

    #[test]
    fn test_fts_score_reference_ok() {
        let registry = SchemaRegistry::new();
        // SELECT fts::score("main") WHERE content @:main@ "rust"
        let score_call = Expr::FunctionCall {
            name: "score".to_string(),
            namespace: Some("fts".to_string()),
            args: vec![Expr::Literal(Value::String("main".to_string()))],
        };
        let select = SelectAst {
            projection: Projection::Items(vec![ProjectionItem {
                expr: score_call,
                alias: Some("score".to_string()),
            }]),
            distinct: false,
            value_mode: false,
            source: DataSource::Collection(Target {
                collection: "articles".to_string(),
                key: None,
            }),
            filter: Some(make_fts_named("content", "main", "rust")),
            group: None,
            order: None,
            limit: None,
            offset: None,
            fetch: None,
        };

        let result = bind_statement(Statement::Select(select), &registry, &default_registry());
        assert!(result.is_ok());
    }

    #[test]
    fn test_fts_score_unknown_reference_fails() {
        let registry = SchemaRegistry::new();
        // SELECT fts::score("nonexistent") WHERE content @:main@ "rust"
        let score_call = Expr::FunctionCall {
            name: "score".to_string(),
            namespace: Some("fts".to_string()),
            args: vec![Expr::Literal(Value::String("nonexistent".to_string()))],
        };
        let select = SelectAst {
            projection: Projection::Items(vec![ProjectionItem {
                expr: score_call,
                alias: Some("score".to_string()),
            }]),
            distinct: false,
            value_mode: false,
            source: DataSource::Collection(Target {
                collection: "articles".to_string(),
                key: None,
            }),
            filter: Some(make_fts_named("content", "main", "rust")),
            group: None,
            order: None,
            limit: None,
            offset: None,
            fetch: None,
        };

        let result = bind_statement(Statement::Select(select), &registry, &default_registry());
        assert!(matches!(result, Err(BindError::FtsUnknownScoreName(ref n)) if n == "nonexistent"));
    }

    #[test]
    fn test_fts_score_no_arg_with_simple_operator_ok() {
        let registry = SchemaRegistry::new();
        // SELECT fts::score() WHERE content @@ "rust" -- OK: unnamed score with simple operator
        let score_call = Expr::FunctionCall {
            name: "score".to_string(),
            namespace: Some("fts".to_string()),
            args: vec![],
        };
        let select = SelectAst {
            projection: Projection::Items(vec![ProjectionItem {
                expr: score_call,
                alias: Some("score".to_string()),
            }]),
            distinct: false,
            value_mode: false,
            source: DataSource::Collection(Target {
                collection: "articles".to_string(),
                key: None,
            }),
            filter: Some(make_fts_simple("content", "rust")),
            group: None,
            order: None,
            limit: None,
            offset: None,
            fetch: None,
        };

        let result = bind_statement(Statement::Select(select), &registry, &default_registry());
        assert!(result.is_ok());
    }

    #[test]
    fn test_fts_validation_context_empty_ok() {
        let ctx = FtsValidationContext::new();
        assert!(ctx.validate().is_ok());
    }

    #[test]
    fn test_fts_in_projection_and_filter() {
        let registry = SchemaRegistry::new();
        // FTS in both projection and filter should be collected correctly
        let fts_expr = make_fts_named("content", "main", "rust");
        let select = SelectAst {
            projection: Projection::Items(vec![ProjectionItem {
                expr: fts_expr.clone(),
                alias: Some("match".to_string()),
            }]),
            distinct: false,
            value_mode: false,
            source: DataSource::Collection(Target {
                collection: "articles".to_string(),
                key: None,
            }),
            filter: Some(fts_expr),
            group: None,
            order: None,
            limit: None,
            offset: None,
            fetch: None,
        };

        // Two uses of @:main@ should fail (duplicate)
        let result = bind_statement(Statement::Select(select), &registry, &default_registry());
        assert!(matches!(result, Err(BindError::FtsDuplicateOperatorName(ref n)) if n == "main"));
    }

    // =========================================================================
    // try_expr_to_value tests
    // =========================================================================

    #[test]
    fn test_try_expr_to_value_literal() {
        let expr = Expr::Literal(Value::Int(42));
        assert_eq!(try_expr_to_value(&expr), Some(Value::Int(42)));

        let expr = Expr::Literal(Value::Float(3.5));
        assert_eq!(try_expr_to_value(&expr), Some(Value::Float(3.5)));

        let expr = Expr::Literal(Value::String("hello".to_string()));
        assert_eq!(
            try_expr_to_value(&expr),
            Some(Value::String("hello".to_string()))
        );
    }

    #[test]
    fn test_try_expr_to_value_array_positive() {
        let expr = Expr::Array(vec![
            Expr::Literal(Value::Float(0.1)),
            Expr::Literal(Value::Float(0.2)),
            Expr::Literal(Value::Float(0.3)),
        ]);
        let result = try_expr_to_value(&expr);
        assert_eq!(
            result,
            Some(Value::Array(vec![
                Value::Float(0.1),
                Value::Float(0.2),
                Value::Float(0.3),
            ]))
        );
    }

    #[test]
    fn test_try_expr_to_value_array_with_negative_numbers() {
        // This is the bug fix: negative numbers in arrays should work
        let expr = Expr::Array(vec![
            Expr::UnaryOp {
                op: UnaryOp::Neg,
                expr: Box::new(Expr::Literal(Value::Float(0.1))),
            },
            Expr::Literal(Value::Float(0.2)),
            Expr::UnaryOp {
                op: UnaryOp::Neg,
                expr: Box::new(Expr::Literal(Value::Float(0.3))),
            },
        ]);
        let result = try_expr_to_value(&expr);
        assert_eq!(
            result,
            Some(Value::Array(vec![
                Value::Float(-0.1),
                Value::Float(0.2),
                Value::Float(-0.3),
            ]))
        );
    }

    #[test]
    fn test_try_expr_to_value_negative_int() {
        let expr = Expr::UnaryOp {
            op: UnaryOp::Neg,
            expr: Box::new(Expr::Literal(Value::Int(42))),
        };
        assert_eq!(try_expr_to_value(&expr), Some(Value::Int(-42)));
    }

    #[test]
    fn test_try_expr_to_value_negative_float() {
        let expr = Expr::UnaryOp {
            op: UnaryOp::Neg,
            expr: Box::new(Expr::Literal(Value::Float(3.5))),
        };
        assert_eq!(try_expr_to_value(&expr), Some(Value::Float(-3.5)));
    }

    #[test]
    fn test_try_expr_to_value_non_constant_returns_none() {
        // Field reference is not a constant
        let expr = Expr::Field("name".to_string());
        assert_eq!(try_expr_to_value(&expr), None);

        // Function call is not a constant
        let expr = Expr::FunctionCall {
            name: "upper".to_string(),
            namespace: None,
            args: vec![Expr::Literal(Value::String("test".to_string()))],
        };
        assert_eq!(try_expr_to_value(&expr), None);
    }

    #[test]
    fn test_try_expr_to_value_array_with_non_constant_returns_none() {
        // Array containing a field reference should return None
        let expr = Expr::Array(vec![
            Expr::Literal(Value::Float(0.1)),
            Expr::Field("x".to_string()), // non-constant
        ]);
        assert_eq!(try_expr_to_value(&expr), None);
    }
}
