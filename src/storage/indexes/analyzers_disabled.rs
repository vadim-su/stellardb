//! Analyzer registry stub used when the `fts` feature is disabled.

use std::collections::HashMap;

use parking_lot::RwLock;

use crate::schema::AnalyzerDef;

const BUILTIN_ANALYZERS: &[&str] = &[
    "standard",
    "keyword",
    "arabic",
    "danish",
    "dutch",
    "english",
    "finnish",
    "french",
    "german",
    "greek",
    "hungarian",
    "italian",
    "norwegian",
    "portuguese",
    "romanian",
    "russian",
    "spanish",
    "swedish",
    "tamil",
    "turkish",
];

/// Check if analyzer name is a builtin.
pub fn is_builtin_analyzer(name: &str) -> bool {
    BUILTIN_ANALYZERS.contains(&name)
}

/// List all builtin analyzer names.
pub fn list_builtin_analyzers() -> Vec<&'static str> {
    BUILTIN_ANALYZERS.to_vec()
}

/// Registry for custom analyzer definitions.
#[derive(Debug, Default)]
pub struct AnalyzerRegistry {
    custom: RwLock<HashMap<String, AnalyzerDef>>,
}

impl AnalyzerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if analyzer exists (builtin or custom).
    pub fn exists(&self, name: &str) -> bool {
        is_builtin_analyzer(name) || self.custom.read().contains_key(name)
    }

    /// Get custom analyzer definition.
    pub fn get_def(&self, name: &str) -> Option<AnalyzerDef> {
        self.custom.read().get(name).cloned()
    }

    /// Register a custom analyzer definition.
    pub fn register(&self, def: AnalyzerDef) -> Result<(), String> {
        if is_builtin_analyzer(&def.name) {
            return Err(format!("Cannot override builtin analyzer: {}", def.name));
        }
        self.custom.write().insert(def.name.clone(), def);
        Ok(())
    }

    /// Remove a custom analyzer definition.
    pub fn remove(&self, name: &str) -> Result<(), String> {
        if is_builtin_analyzer(name) {
            return Err(format!("Cannot remove builtin analyzer: {}", name));
        }
        if self.custom.write().remove(name).is_none() {
            return Err(format!("Analyzer not found: {}", name));
        }
        Ok(())
    }

    /// Load custom analyzers.
    pub fn load(&self, analyzers: Vec<AnalyzerDef>) {
        let mut custom = self.custom.write();
        for def in analyzers {
            custom.insert(def.name.clone(), def);
        }
    }

    /// List all custom analyzer names.
    pub fn list_custom(&self) -> Vec<String> {
        self.custom.read().keys().cloned().collect()
    }
}
