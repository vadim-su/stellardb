//! Function registry for resolving aggregate and scalar functions.

use std::sync::LazyLock;

mod registry;
mod types;
#[macro_use]
mod macros;
mod definitions;

pub use definitions::register_all;
pub use macros::define_functions;
pub use registry::{ConstantDef, FunctionDef, Registry, SymbolKind};
pub use types::{AggregateFunction, Arity, FnType, ParamDef};

/// Global registry instance - initialized once on first access
static GLOBAL_REGISTRY: LazyLock<Registry> = LazyLock::new(|| {
    let mut registry = Registry::new();
    register_all(&mut registry);
    registry
});

/// Get a reference to the global registry
pub fn global_registry() -> &'static Registry {
    &GLOBAL_REGISTRY
}

/// Get the default registry with all built-in functions and constants
pub fn default_registry() -> Registry {
    let mut registry = Registry::new();
    register_all(&mut registry);
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_registry_is_singleton() {
        let r1 = global_registry();
        let r2 = global_registry();
        // Both calls should return the exact same reference (same pointer)
        assert!(
            std::ptr::eq(r1, r2),
            "global_registry() should return the same reference on every call"
        );
    }

    #[test]
    fn test_global_registry_has_functions() {
        let registry = global_registry();
        // Verify some built-in functions are registered
        assert!(
            registry.lookup("math", "abs").is_some(),
            "global registry should contain math::abs"
        );
        assert!(
            registry.lookup("string", "upper").is_some(),
            "global registry should contain string::upper"
        );
    }
}
