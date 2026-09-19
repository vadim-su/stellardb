use std::collections::HashMap;

use crate::document::Value;

/// Scope for typed parameters and user-defined variables (from LET statements).
///
/// Uses a frame stack so that inner blocks (loops, branches) can shadow
/// outer variables and have their locals automatically discarded on pop.
#[derive(Debug, Clone)]
pub struct VarScope {
    frames: Vec<HashMap<String, Value>>,
}

impl Default for VarScope {
    fn default() -> Self {
        Self {
            frames: vec![HashMap::new()],
        }
    }
}

impl VarScope {
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialize the root frame with parameters owned by this batch.
    pub(crate) fn with_params(params: HashMap<String, Value>) -> Self {
        Self {
            frames: vec![params],
        }
    }

    /// Push a new empty frame onto the stack (e.g. entering a block).
    pub fn push(&mut self) {
        self.frames.push(HashMap::new());
    }

    /// Pop the top frame (e.g. leaving a block).
    ///
    /// # Panics (debug)
    /// Panics if attempting to pop the root frame.
    pub fn pop(&mut self) {
        debug_assert!(
            self.frames.len() > 1,
            "VarScope::pop: cannot pop the root frame"
        );
        self.frames.pop();
    }

    /// Set a variable value in the current (top) frame.
    pub fn set(&mut self, name: String, value: Value) {
        self.frames
            .last_mut()
            .expect("VarScope must always have at least one frame")
            .insert(name, value);
    }

    /// Get a variable value, walking frames from top to bottom.
    pub fn get(&self, name: &str) -> Option<&Value> {
        for frame in self.frames.iter().rev() {
            if let Some(v) = frame.get(name) {
                return Some(v);
            }
        }
        None
    }

    /// Check if a variable exists in any frame.
    pub fn contains(&self, name: &str) -> bool {
        self.frames.iter().rev().any(|f| f.contains_key(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_set_get() {
        let mut scope = VarScope::new();
        scope.set("x".into(), Value::Int(42));
        assert_eq!(scope.get("x"), Some(&Value::Int(42)));
        assert!(scope.contains("x"));
        assert!(!scope.contains("y"));
        assert_eq!(scope.get("y"), None);
    }

    #[test]
    fn test_push_pop_shadow() {
        let mut scope = VarScope::new();
        scope.set("x".into(), Value::Int(1));

        scope.push();
        // Shadow x in inner frame
        scope.set("x".into(), Value::Int(2));
        assert_eq!(scope.get("x"), Some(&Value::Int(2)));

        scope.pop();
        // Original x restored
        assert_eq!(scope.get("x"), Some(&Value::Int(1)));
    }

    #[test]
    fn test_inner_var_not_visible_after_pop() {
        let mut scope = VarScope::new();
        scope.push();
        scope.set("tmp".into(), Value::String("gone".into()));
        assert!(scope.contains("tmp"));

        scope.pop();
        assert!(!scope.contains("tmp"));
        assert_eq!(scope.get("tmp"), None);
    }

    #[test]
    fn test_get_walks_stack() {
        let mut scope = VarScope::new();
        scope.set("a".into(), Value::Int(1));
        scope.set("b".into(), Value::Int(2));

        scope.push();
        scope.set("c".into(), Value::Int(3));

        // Can see all three
        assert_eq!(scope.get("a"), Some(&Value::Int(1)));
        assert_eq!(scope.get("b"), Some(&Value::Int(2)));
        assert_eq!(scope.get("c"), Some(&Value::Int(3)));

        scope.push();
        scope.set("d".into(), Value::Int(4));

        // Can see everything from deepest frame
        assert_eq!(scope.get("a"), Some(&Value::Int(1)));
        assert_eq!(scope.get("d"), Some(&Value::Int(4)));
    }

    #[test]
    fn test_contains_walks_stack() {
        let mut scope = VarScope::new();
        scope.set("root_var".into(), Value::Bool(true));

        scope.push();
        assert!(scope.contains("root_var"));

        scope.push();
        assert!(scope.contains("root_var"));

        scope.pop();
        scope.pop();
        assert!(scope.contains("root_var"));
    }
}
