//! Function and constant registry for the new typed function system.

use super::types::{Arity, FnType, ParamDef};
use crate::document::Value;
use crate::query::error::ExecuteError;
use crate::util::suggestions::find_similar;
use std::collections::HashMap;

/// Function definition
pub struct FunctionDef {
    pub namespace: &'static str,
    pub name: &'static str,
    pub params: &'static [ParamDef],
    pub return_type: FnType,
    pub variadic: bool,
    pub eval: fn(&[Value]) -> Result<Value, ExecuteError>,
}

impl FunctionDef {
    /// Get function arity from params
    pub fn arity(&self) -> Arity {
        if self.variadic {
            // Variadic functions accept 0+ arguments
            Arity::AtLeast(0)
        } else if self.params.is_empty() {
            Arity::Fixed(0)
        } else {
            Arity::Fixed(self.params.len())
        }
    }

    /// Call the function with arity validation
    pub fn call(&self, args: &[Value]) -> Result<Value, ExecuteError> {
        let arity = self.arity();
        if !arity.check(args.len()) {
            return Err(ExecuteError::ArityMismatch {
                function: format!("{}::{}", self.namespace, self.name),
                expected: arity.expected(),
                got: args.len(),
            });
        }
        (self.eval)(args)
    }
}

/// Constant definition
pub struct ConstantDef {
    pub namespace: &'static str,
    pub name: &'static str,
    pub value_type: FnType,
    pub value: Value,
}

/// Symbol kind for lookup
pub enum SymbolKind<'a> {
    Function(&'a FunctionDef),
    Constant(&'a ConstantDef),
}

/// Registry of all functions and constants
/// Uses lowercase keys for O(1) case-insensitive lookup
pub struct Registry {
    functions: HashMap<(String, String), &'static FunctionDef>,
    constants: HashMap<(String, String), &'static ConstantDef>,
}

impl Registry {
    /// Create empty registry
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
            constants: HashMap::new(),
        }
    }

    /// Register a function (stores key in lowercase for O(1) lookup)
    pub fn register_function(&mut self, def: &'static FunctionDef) {
        let key = (def.namespace.to_lowercase(), def.name.to_lowercase());
        self.functions.insert(key, def);
    }

    /// Register a constant (stores key in lowercase for O(1) lookup)
    pub fn register_constant(&mut self, def: &'static ConstantDef) {
        let key = (def.namespace.to_lowercase(), def.name.to_lowercase());
        self.constants.insert(key, def);
    }

    /// Resolve a function by namespace and name (case-insensitive, O(1))
    pub fn resolve_function(&self, namespace: &str, name: &str) -> Option<&'static FunctionDef> {
        let key = (namespace.to_lowercase(), name.to_lowercase());
        self.functions.get(&key).copied()
    }

    /// Resolve a constant by namespace and name (case-insensitive, O(1))
    pub fn resolve_constant(&self, namespace: &str, name: &str) -> Option<&'static ConstantDef> {
        let key = (namespace.to_lowercase(), name.to_lowercase());
        self.constants.get(&key).copied()
    }

    /// Lookup any symbol
    pub fn lookup(&self, namespace: &str, name: &str) -> Option<SymbolKind<'_>> {
        if let Some(f) = self.resolve_function(namespace, name) {
            return Some(SymbolKind::Function(f));
        }
        if let Some(c) = self.resolve_constant(namespace, name) {
            return Some(SymbolKind::Constant(c));
        }
        None
    }

    /// Check if name is a constant
    pub fn is_constant(&self, namespace: &str, name: &str) -> bool {
        self.resolve_constant(namespace, name).is_some()
    }

    /// Check if name is a function
    pub fn is_function(&self, namespace: &str, name: &str) -> bool {
        self.resolve_function(namespace, name).is_some()
    }

    /// Find similar function names in the same namespace (for error suggestions)
    pub fn find_similar_functions(&self, namespace: &str, name: &str) -> Vec<String> {
        let ns_lower = namespace.to_lowercase();
        let candidates: Vec<&str> = self
            .functions
            .iter()
            .filter(|((ns, _), _)| ns == &ns_lower)
            .map(|((_, fn_name), _)| fn_name.as_str())
            .collect();

        find_similar(name, candidates.into_iter(), 3)
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }

    /// Find similar constant names in the same namespace (for error suggestions)
    pub fn find_similar_constants(&self, namespace: &str, name: &str) -> Vec<String> {
        let ns_lower = namespace.to_lowercase();
        let candidates: Vec<&str> = self
            .constants
            .iter()
            .filter(|((ns, _), _)| ns == &ns_lower)
            .map(|((_, const_name), _)| const_name.as_str())
            .collect();

        find_similar(name, candidates.into_iter(), 3)
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}
