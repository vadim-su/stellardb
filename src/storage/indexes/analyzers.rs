//! Built-in and custom analyzer definitions for FTS

use std::collections::HashMap;

use parking_lot::RwLock;
use tantivy::tokenizer::{
    AsciiFoldingFilter, Language, LowerCaser, NgramTokenizer, RawTokenizer, RemoveLongFilter,
    SimpleTokenizer, Stemmer, TextAnalyzer, Token, TokenStream, WhitespaceTokenizer,
};

use crate::schema::{AnalyzerDef, FilterConfig, TokenizerConfig};

// ---------------------------------------------------------------------------
// Regex pattern tokenizer
// ---------------------------------------------------------------------------

/// A tokenizer that splits text on a regex pattern.
/// For example, pattern `[^a-zA-Z]+` splits on non-alphabetic runs.
#[derive(Clone)]
struct RegexTokenizer {
    pattern: regex::Regex,
    token: Token,
}

impl RegexTokenizer {
    fn new(pattern: regex::Regex) -> Self {
        Self {
            pattern,
            token: Token::default(),
        }
    }
}

struct RegexTokenStream<'a> {
    /// Pre-computed (offset_from, offset_to) of non-empty tokens
    splits: Vec<(usize, usize)>,
    index: usize,
    text: &'a str,
    token: &'a mut Token,
}

impl tantivy::tokenizer::Tokenizer for RegexTokenizer {
    type TokenStream<'a> = RegexTokenStream<'a>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> RegexTokenStream<'a> {
        self.token.reset();
        // Collect gaps between regex matches (the non-matching parts are the tokens)
        let mut splits = Vec::new();
        let mut last_end = 0;
        for m in self.pattern.find_iter(text) {
            if m.start() > last_end {
                splits.push((last_end, m.start()));
            }
            last_end = m.end();
        }
        if last_end < text.len() {
            splits.push((last_end, text.len()));
        }
        RegexTokenStream {
            splits,
            index: 0,
            text,
            token: &mut self.token,
        }
    }
}

impl TokenStream for RegexTokenStream<'_> {
    fn advance(&mut self) -> bool {
        if self.index >= self.splits.len() {
            return false;
        }
        let (from, to) = self.splits[self.index];
        self.token.offset_from = from;
        self.token.offset_to = to;
        self.token.text.clear();
        self.token.text.push_str(&self.text[from..to]);
        self.token.position = self.index;
        self.token.position_length = 1;
        self.index += 1;
        true
    }

    fn token(&self) -> &Token {
        self.token
    }

    fn token_mut(&mut self) -> &mut Token {
        self.token
    }
}

/// Get a built-in analyzer by name
pub fn get_builtin_analyzer(name: &str) -> Option<TextAnalyzer> {
    match name {
        "standard" => Some(standard_analyzer()),
        "keyword" => Some(keyword_analyzer()),
        "english" => Some(lang_analyzer(Language::English)),
        "russian" => Some(lang_analyzer(Language::Russian)),
        "arabic" => Some(lang_analyzer(Language::Arabic)),
        "danish" => Some(lang_analyzer(Language::Danish)),
        "dutch" => Some(lang_analyzer(Language::Dutch)),
        "finnish" => Some(lang_analyzer(Language::Finnish)),
        "french" => Some(lang_analyzer(Language::French)),
        "german" => Some(lang_analyzer(Language::German)),
        "greek" => Some(lang_analyzer(Language::Greek)),
        "hungarian" => Some(lang_analyzer(Language::Hungarian)),
        "italian" => Some(lang_analyzer(Language::Italian)),
        "norwegian" => Some(lang_analyzer(Language::Norwegian)),
        "portuguese" => Some(lang_analyzer(Language::Portuguese)),
        "romanian" => Some(lang_analyzer(Language::Romanian)),
        "spanish" => Some(lang_analyzer(Language::Spanish)),
        "swedish" => Some(lang_analyzer(Language::Swedish)),
        "tamil" => Some(lang_analyzer(Language::Tamil)),
        "turkish" => Some(lang_analyzer(Language::Turkish)),
        _ => None,
    }
}

/// Check if analyzer name is a builtin
pub fn is_builtin_analyzer(name: &str) -> bool {
    get_builtin_analyzer(name).is_some()
}

/// List all builtin analyzer names
pub fn list_builtin_analyzers() -> Vec<&'static str> {
    vec![
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
    ]
}

fn standard_analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .build()
}

fn keyword_analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(RawTokenizer::default())
        .filter(LowerCaser)
        .build()
}

fn lang_analyzer(lang: Language) -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .filter(Stemmer::new(lang))
        .build()
}

/// Registry for custom analyzers
#[derive(Debug, Default)]
pub struct AnalyzerRegistry {
    custom: RwLock<HashMap<String, AnalyzerDef>>,
}

impl AnalyzerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get analyzer by name (checks builtin first, then custom)
    pub fn get(&self, name: &str) -> Option<TextAnalyzer> {
        // First check builtin
        if let Some(analyzer) = get_builtin_analyzer(name) {
            return Some(analyzer);
        }
        // Then check custom
        self.custom.read().get(name).map(build_analyzer)
    }

    /// Get custom analyzer definition
    pub fn get_def(&self, name: &str) -> Option<AnalyzerDef> {
        self.custom.read().get(name).cloned()
    }

    /// Register a custom analyzer
    pub fn register(&self, def: AnalyzerDef) -> Result<(), String> {
        if is_builtin_analyzer(&def.name) {
            return Err(format!("Cannot override builtin analyzer: {}", def.name));
        }
        self.custom.write().insert(def.name.clone(), def);
        Ok(())
    }

    /// Remove a custom analyzer
    pub fn remove(&self, name: &str) -> Result<(), String> {
        if is_builtin_analyzer(name) {
            return Err(format!("Cannot remove builtin analyzer: {}", name));
        }
        if self.custom.write().remove(name).is_none() {
            return Err(format!("Analyzer not found: {}", name));
        }
        Ok(())
    }

    /// Check if analyzer exists (builtin or custom)
    pub fn exists(&self, name: &str) -> bool {
        is_builtin_analyzer(name) || self.custom.read().contains_key(name)
    }

    /// Load custom analyzers (called at startup)
    pub fn load(&self, analyzers: Vec<AnalyzerDef>) {
        let mut custom = self.custom.write();
        for def in analyzers {
            custom.insert(def.name.clone(), def);
        }
    }

    /// List all custom analyzer names
    pub fn list_custom(&self) -> Vec<String> {
        self.custom.read().keys().cloned().collect()
    }
}

/// Build TextAnalyzer from AnalyzerDef
///
/// Due to Tantivy's type-changing builder pattern, we use a dynamic approach
/// where we build the analyzer by applying filters in a predefined order.
pub fn build_analyzer(def: &AnalyzerDef) -> TextAnalyzer {
    // Collect which filters we need
    let has_lowercase = def
        .filters
        .iter()
        .any(|f| matches!(f, FilterConfig::Lowercase));
    let has_ascii_folding = def
        .filters
        .iter()
        .any(|f| matches!(f, FilterConfig::AsciiFolding));
    let stemmer_lang = def.filters.iter().find_map(|f| {
        if let FilterConfig::Stemmer { lang } = f {
            parse_language(lang)
        } else {
            None
        }
    });
    let max_length = def.filters.iter().find_map(|f| {
        if let FilterConfig::Length { max, .. } = f {
            Some(*max as usize)
        } else {
            None
        }
    });

    // Build based on tokenizer type and filters
    // We use a macro-like pattern to handle the combinatorial explosion
    match &def.tokenizer {
        TokenizerConfig::Standard => build_with_filters(
            SimpleTokenizer::default(),
            has_lowercase,
            has_ascii_folding,
            stemmer_lang,
            max_length,
        ),
        TokenizerConfig::Whitespace => build_with_filters(
            WhitespaceTokenizer::default(),
            has_lowercase,
            has_ascii_folding,
            stemmer_lang,
            max_length,
        ),
        TokenizerConfig::Ngram { min, max } => {
            let tokenizer = NgramTokenizer::new(*min as usize, *max as usize, false)
                .unwrap_or_else(|_| {
                    NgramTokenizer::new(2, 3, false).expect("default ngram(2,3) must be valid")
                });
            build_with_filters(
                tokenizer,
                has_lowercase,
                has_ascii_folding,
                stemmer_lang,
                max_length,
            )
        }
        TokenizerConfig::Pattern { regex } => {
            let re = regex::Regex::new(regex).unwrap_or_else(|_| {
                // Invalid regex: fall back to splitting on non-alphanumeric (same as standard)
                regex::Regex::new(r"[^\w]+").expect("static regex must compile")
            });
            build_with_filters(
                RegexTokenizer::new(re),
                has_lowercase,
                has_ascii_folding,
                stemmer_lang,
                max_length,
            )
        }
    }
}

/// Helper to build analyzer with various filter combinations
fn build_with_filters<T: tantivy::tokenizer::Tokenizer>(
    tokenizer: T,
    lowercase: bool,
    ascii_folding: bool,
    stemmer: Option<Language>,
    max_length: Option<usize>,
) -> TextAnalyzer {
    // We need to handle all combinations due to type system constraints
    // Order: lowercase -> ascii_folding -> stemmer -> length
    match (lowercase, ascii_folding, stemmer, max_length) {
        (false, false, None, None) => TextAnalyzer::builder(tokenizer).build(),
        (true, false, None, None) => TextAnalyzer::builder(tokenizer).filter(LowerCaser).build(),
        (false, true, None, None) => TextAnalyzer::builder(tokenizer)
            .filter(AsciiFoldingFilter)
            .build(),
        (true, true, None, None) => TextAnalyzer::builder(tokenizer)
            .filter(LowerCaser)
            .filter(AsciiFoldingFilter)
            .build(),
        (false, false, Some(lang), None) => TextAnalyzer::builder(tokenizer)
            .filter(Stemmer::new(lang))
            .build(),
        (true, false, Some(lang), None) => TextAnalyzer::builder(tokenizer)
            .filter(LowerCaser)
            .filter(Stemmer::new(lang))
            .build(),
        (false, true, Some(lang), None) => TextAnalyzer::builder(tokenizer)
            .filter(AsciiFoldingFilter)
            .filter(Stemmer::new(lang))
            .build(),
        (true, true, Some(lang), None) => TextAnalyzer::builder(tokenizer)
            .filter(LowerCaser)
            .filter(AsciiFoldingFilter)
            .filter(Stemmer::new(lang))
            .build(),
        (false, false, None, Some(max)) => TextAnalyzer::builder(tokenizer)
            .filter(RemoveLongFilter::limit(max))
            .build(),
        (true, false, None, Some(max)) => TextAnalyzer::builder(tokenizer)
            .filter(LowerCaser)
            .filter(RemoveLongFilter::limit(max))
            .build(),
        (false, true, None, Some(max)) => TextAnalyzer::builder(tokenizer)
            .filter(AsciiFoldingFilter)
            .filter(RemoveLongFilter::limit(max))
            .build(),
        (true, true, None, Some(max)) => TextAnalyzer::builder(tokenizer)
            .filter(LowerCaser)
            .filter(AsciiFoldingFilter)
            .filter(RemoveLongFilter::limit(max))
            .build(),
        (false, false, Some(lang), Some(max)) => TextAnalyzer::builder(tokenizer)
            .filter(Stemmer::new(lang))
            .filter(RemoveLongFilter::limit(max))
            .build(),
        (true, false, Some(lang), Some(max)) => TextAnalyzer::builder(tokenizer)
            .filter(LowerCaser)
            .filter(Stemmer::new(lang))
            .filter(RemoveLongFilter::limit(max))
            .build(),
        (false, true, Some(lang), Some(max)) => TextAnalyzer::builder(tokenizer)
            .filter(AsciiFoldingFilter)
            .filter(Stemmer::new(lang))
            .filter(RemoveLongFilter::limit(max))
            .build(),
        (true, true, Some(lang), Some(max)) => TextAnalyzer::builder(tokenizer)
            .filter(LowerCaser)
            .filter(AsciiFoldingFilter)
            .filter(Stemmer::new(lang))
            .filter(RemoveLongFilter::limit(max))
            .build(),
    }
}

fn parse_language(lang: &str) -> Option<Language> {
    match lang.to_lowercase().as_str() {
        "english" => Some(Language::English),
        "russian" => Some(Language::Russian),
        "french" => Some(Language::French),
        "german" => Some(Language::German),
        "spanish" => Some(Language::Spanish),
        "italian" => Some(Language::Italian),
        "portuguese" => Some(Language::Portuguese),
        "dutch" => Some(Language::Dutch),
        "swedish" => Some(Language::Swedish),
        "norwegian" => Some(Language::Norwegian),
        "danish" => Some(Language::Danish),
        "finnish" => Some(Language::Finnish),
        "hungarian" => Some(Language::Hungarian),
        "romanian" => Some(Language::Romanian),
        "turkish" => Some(Language::Turkish),
        "arabic" => Some(Language::Arabic),
        "greek" => Some(Language::Greek),
        "tamil" => Some(Language::Tamil),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_analyzers() {
        assert!(get_builtin_analyzer("standard").is_some());
        assert!(get_builtin_analyzer("keyword").is_some());
        assert!(get_builtin_analyzer("russian").is_some());
        assert!(get_builtin_analyzer("english").is_some());
        assert!(get_builtin_analyzer("nonexistent").is_none());
    }

    #[test]
    fn test_is_builtin() {
        assert!(is_builtin_analyzer("standard"));
        assert!(is_builtin_analyzer("russian"));
        assert!(!is_builtin_analyzer("custom"));
    }

    #[test]
    fn test_list_builtins() {
        let list = list_builtin_analyzers();
        assert!(list.contains(&"standard"));
        assert!(list.contains(&"russian"));
        assert_eq!(list.len(), 20); // standard + keyword + 18 languages
    }

    #[test]
    fn test_registry_new() {
        let registry = AnalyzerRegistry::new();
        assert!(registry.list_custom().is_empty());
    }

    #[test]
    fn test_registry_get_builtin() {
        let registry = AnalyzerRegistry::new();
        // Should return builtin analyzers
        assert!(registry.get("standard").is_some());
        assert!(registry.get("english").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_registry_register_custom() {
        let registry = AnalyzerRegistry::new();

        let def = AnalyzerDef {
            name: "my_analyzer".to_string(),
            tokenizer: TokenizerConfig::Standard,
            filters: vec![FilterConfig::Lowercase],
        };

        // Register should succeed
        assert!(registry.register(def.clone()).is_ok());

        // Should exist now
        assert!(registry.exists("my_analyzer"));

        // Should be able to get the analyzer
        assert!(registry.get("my_analyzer").is_some());

        // Should be able to get the definition
        let retrieved = registry.get_def("my_analyzer");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "my_analyzer");

        // Should be in custom list
        assert!(registry.list_custom().contains(&"my_analyzer".to_string()));
    }

    #[test]
    fn test_registry_cannot_override_builtin() {
        let registry = AnalyzerRegistry::new();

        let def = AnalyzerDef {
            name: "standard".to_string(), // builtin name
            tokenizer: TokenizerConfig::Standard,
            filters: vec![],
        };

        // Should fail
        let result = registry.register(def);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cannot override builtin"));
    }

    #[test]
    fn test_registry_remove_custom() {
        let registry = AnalyzerRegistry::new();

        let def = AnalyzerDef {
            name: "removable".to_string(),
            tokenizer: TokenizerConfig::Whitespace,
            filters: vec![],
        };

        registry.register(def).unwrap();
        assert!(registry.exists("removable"));

        // Remove should succeed
        assert!(registry.remove("removable").is_ok());
        assert!(!registry.exists("removable"));
    }

    #[test]
    fn test_registry_cannot_remove_builtin() {
        let registry = AnalyzerRegistry::new();

        let result = registry.remove("standard");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cannot remove builtin"));
    }

    #[test]
    fn test_registry_remove_nonexistent() {
        let registry = AnalyzerRegistry::new();

        let result = registry.remove("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Analyzer not found"));
    }

    #[test]
    fn test_registry_load() {
        let registry = AnalyzerRegistry::new();

        let analyzers = vec![
            AnalyzerDef {
                name: "analyzer1".to_string(),
                tokenizer: TokenizerConfig::Standard,
                filters: vec![FilterConfig::Lowercase],
            },
            AnalyzerDef {
                name: "analyzer2".to_string(),
                tokenizer: TokenizerConfig::Whitespace,
                filters: vec![FilterConfig::AsciiFolding],
            },
        ];

        registry.load(analyzers);

        assert!(registry.exists("analyzer1"));
        assert!(registry.exists("analyzer2"));
        assert_eq!(registry.list_custom().len(), 2);
    }

    #[test]
    fn test_registry_exists() {
        let registry = AnalyzerRegistry::new();

        // Builtins should exist
        assert!(registry.exists("standard"));
        assert!(registry.exists("english"));

        // Non-existent should not exist
        assert!(!registry.exists("nonexistent"));

        // Register custom
        let def = AnalyzerDef {
            name: "custom".to_string(),
            tokenizer: TokenizerConfig::Standard,
            filters: vec![],
        };
        registry.register(def).unwrap();

        // Custom should exist
        assert!(registry.exists("custom"));
    }

    #[test]
    fn test_build_analyzer_standard() {
        let def = AnalyzerDef {
            name: "test".to_string(),
            tokenizer: TokenizerConfig::Standard,
            filters: vec![FilterConfig::Lowercase],
        };

        let analyzer = build_analyzer(&def);
        // Just verify it builds without panic
        let _ = analyzer;
    }

    #[test]
    fn test_build_analyzer_whitespace() {
        let def = AnalyzerDef {
            name: "test".to_string(),
            tokenizer: TokenizerConfig::Whitespace,
            filters: vec![FilterConfig::Lowercase, FilterConfig::AsciiFolding],
        };

        let analyzer = build_analyzer(&def);
        let _ = analyzer;
    }

    #[test]
    fn test_build_analyzer_ngram() {
        let def = AnalyzerDef {
            name: "test".to_string(),
            tokenizer: TokenizerConfig::Ngram { min: 2, max: 4 },
            filters: vec![FilterConfig::Lowercase],
        };

        let analyzer = build_analyzer(&def);
        let _ = analyzer;
    }

    #[test]
    fn test_build_analyzer_with_stemmer() {
        let def = AnalyzerDef {
            name: "test".to_string(),
            tokenizer: TokenizerConfig::Standard,
            filters: vec![
                FilterConfig::Lowercase,
                FilterConfig::Stemmer {
                    lang: "english".to_string(),
                },
            ],
        };

        let analyzer = build_analyzer(&def);
        let _ = analyzer;
    }

    #[test]
    fn test_build_analyzer_with_length_filter() {
        let def = AnalyzerDef {
            name: "test".to_string(),
            tokenizer: TokenizerConfig::Standard,
            filters: vec![
                FilterConfig::Lowercase,
                FilterConfig::Length { min: 2, max: 50 },
            ],
        };

        let analyzer = build_analyzer(&def);
        let _ = analyzer;
    }

    #[test]
    fn test_parse_language() {
        assert_eq!(parse_language("english"), Some(Language::English));
        assert_eq!(parse_language("ENGLISH"), Some(Language::English));
        assert_eq!(parse_language("Russian"), Some(Language::Russian));
        assert_eq!(parse_language("french"), Some(Language::French));
        assert_eq!(parse_language("german"), Some(Language::German));
        assert_eq!(parse_language("spanish"), Some(Language::Spanish));
        assert_eq!(parse_language("unknown"), None);
    }

    #[test]
    fn test_build_analyzer_pattern() {
        let def = AnalyzerDef {
            name: "test".to_string(),
            tokenizer: TokenizerConfig::Pattern {
                regex: r"[^a-zA-Z]+".to_string(),
            },
            filters: vec![FilterConfig::Lowercase],
        };

        let analyzer = build_analyzer(&def);
        let _ = analyzer;
    }

    #[test]
    fn test_pattern_tokenizer_splits_correctly() {
        use tantivy::tokenizer::Tokenizer;

        let re = regex::Regex::new(r"[^a-zA-Z]+").unwrap();
        let mut tokenizer = RegexTokenizer::new(re);
        let mut stream = tokenizer.token_stream("hello-world 123 foo");

        let mut tokens = Vec::new();
        while stream.advance() {
            tokens.push(stream.token().text.clone());
        }
        assert_eq!(tokens, vec!["hello", "world", "foo"]);
    }

    #[test]
    fn test_pattern_tokenizer_comma_split() {
        use tantivy::tokenizer::Tokenizer;

        let re = regex::Regex::new(r"\s*,\s*").unwrap();
        let mut tokenizer = RegexTokenizer::new(re);
        let mut stream = tokenizer.token_stream("one, two , three");

        let mut tokens = Vec::new();
        while stream.advance() {
            tokens.push(stream.token().text.clone());
        }
        assert_eq!(tokens, vec!["one", "two", "three"]);
    }

    #[test]
    fn test_pattern_tokenizer_empty_string() {
        use tantivy::tokenizer::Tokenizer;

        let re = regex::Regex::new(r"\s+").unwrap();
        let mut tokenizer = RegexTokenizer::new(re);
        let mut stream = tokenizer.token_stream("");

        assert!(!stream.advance());
    }

    #[test]
    fn test_pattern_tokenizer_no_match() {
        use tantivy::tokenizer::Tokenizer;

        let re = regex::Regex::new(r"@").unwrap();
        let mut tokenizer = RegexTokenizer::new(re);
        let mut stream = tokenizer.token_stream("hello world");

        let mut tokens = Vec::new();
        while stream.advance() {
            tokens.push(stream.token().text.clone());
        }
        // No @ in input, so the entire text is one token
        assert_eq!(tokens, vec!["hello world"]);
    }

    #[test]
    fn test_pattern_tokenizer_offsets() {
        use tantivy::tokenizer::Tokenizer;

        let re = regex::Regex::new(r"\s+").unwrap();
        let mut tokenizer = RegexTokenizer::new(re);
        let text = "hello world foo";
        let mut stream = tokenizer.token_stream(text);

        assert!(stream.advance());
        assert_eq!(stream.token().offset_from, 0);
        assert_eq!(stream.token().offset_to, 5);
        assert_eq!(stream.token().position, 0);

        assert!(stream.advance());
        assert_eq!(stream.token().offset_from, 6);
        assert_eq!(stream.token().offset_to, 11);
        assert_eq!(stream.token().position, 1);

        assert!(stream.advance());
        assert_eq!(stream.token().offset_from, 12);
        assert_eq!(stream.token().offset_to, 15);
        assert_eq!(stream.token().position, 2);

        assert!(!stream.advance());
    }

    #[test]
    fn test_pattern_tokenizer_invalid_regex_fallback() {
        // Invalid regex should fall back to \W+ splitting
        let def = AnalyzerDef {
            name: "test".to_string(),
            tokenizer: TokenizerConfig::Pattern {
                regex: r"[invalid".to_string(),
            },
            filters: vec![],
        };

        // Should not panic
        let _analyzer = build_analyzer(&def);
    }
}
