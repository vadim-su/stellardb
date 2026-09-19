//! Utilities for generating "did you mean?" suggestions in error messages.

/// Find similar strings using Levenshtein distance and prefix matching.
///
/// Returns up to `max_results` suggestions, prioritizing:
/// 1. Prefix matches (needle starts with candidate or vice versa)
/// 2. Lowest edit distance
///
/// Only returns suggestions with edit distance <= 2 and < needle length.
pub fn find_similar<'a>(
    needle: &str,
    candidates: impl Iterator<Item = &'a str>,
    max_results: usize,
) -> Vec<&'a str> {
    let needle_lower = needle.to_lowercase();

    let mut scored: Vec<(&str, usize)> = candidates
        .filter_map(|candidate| {
            let candidate_lower = candidate.to_lowercase();

            // Prioritize prefix matches (score 0)
            if candidate_lower.starts_with(&needle_lower)
                || needle_lower.starts_with(&candidate_lower)
            {
                return Some((candidate, 0));
            }

            let distance = levenshtein(&needle_lower, &candidate_lower);
            // Only suggest if edit distance is small relative to name length
            if distance <= 2 && distance < needle_lower.len() {
                Some((candidate, distance))
            } else {
                None
            }
        })
        .collect();

    // Sort by score (prefix matches first, then edit distance)
    scored.sort_by_key(|(_, d)| *d);

    // Return up to max_results suggestions
    scored
        .into_iter()
        .take(max_results)
        .map(|(name, _)| name)
        .collect()
}

/// Format suggestions for error message.
///
/// Returns empty string if no suggestions, otherwise " Did you mean: x, y, z?"
pub fn format_suggestions(suggestions: &[&str]) -> String {
    if suggestions.is_empty() {
        String::new()
    } else {
        format!(". Did you mean: {}?", suggestions.join(", "))
    }
}

/// Format suggestions with a prefix (e.g., namespace).
///
/// Returns empty string if no suggestions, otherwise " Did you mean: ns::x, ns::y?"
pub fn format_suggestions_with_prefix(suggestions: &[&str], prefix: &str) -> String {
    if suggestions.is_empty() {
        String::new()
    } else {
        let formatted: Vec<String> = suggestions
            .iter()
            .map(|s| format!("{}::{}", prefix, s))
            .collect();
        format!(". Did you mean: {}?", formatted.join(", "))
    }
}

/// Simple Levenshtein distance calculation.
fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }

    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_identical() {
        assert_eq!(levenshtein("hello", "hello"), 0);
    }

    #[test]
    fn test_levenshtein_one_char_diff() {
        assert_eq!(levenshtein("hello", "hallo"), 1);
        assert_eq!(levenshtein("cat", "cut"), 1);
    }

    #[test]
    fn test_levenshtein_insertion() {
        assert_eq!(levenshtein("hello", "helloo"), 1);
        assert_eq!(levenshtein("cat", "cats"), 1);
    }

    #[test]
    fn test_levenshtein_deletion() {
        assert_eq!(levenshtein("hello", "helo"), 1);
    }

    #[test]
    fn test_levenshtein_empty() {
        assert_eq!(levenshtein("", "hello"), 5);
        assert_eq!(levenshtein("hello", ""), 5);
        assert_eq!(levenshtein("", ""), 0);
    }

    #[test]
    fn test_find_similar_prefix_match() {
        let candidates = ["modulo", "multiply", "min", "max"];
        let result = find_similar("mod", candidates.iter().copied(), 3);
        // "modulo" should be first (prefix match)
        assert_eq!(result.first(), Some(&"modulo"));
    }

    #[test]
    fn test_find_similar_edit_distance() {
        let candidates = ["upper", "lower", "trim", "split"];
        let result = find_similar("uper", candidates.iter().copied(), 3);
        assert!(result.contains(&"upper"));
    }

    #[test]
    fn test_find_similar_case_insensitive() {
        let candidates = ["Upper", "Lower", "TRIM"];
        let result = find_similar("upper", candidates.iter().copied(), 3);
        assert!(result.contains(&"Upper"));
    }

    #[test]
    fn test_find_similar_no_match() {
        let candidates = ["function", "parameter", "variable"];
        let result = find_similar("xyz", candidates.iter().copied(), 3);
        assert!(result.is_empty());
    }

    #[test]
    fn test_find_similar_max_results() {
        let candidates = ["aa", "ab", "ac", "ad", "ae"];
        let result = find_similar("a", candidates.iter().copied(), 2);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_format_suggestions_empty() {
        assert_eq!(format_suggestions(&[]), "");
    }

    #[test]
    fn test_format_suggestions_single() {
        assert_eq!(format_suggestions(&["foo"]), ". Did you mean: foo?");
    }

    #[test]
    fn test_format_suggestions_multiple() {
        assert_eq!(
            format_suggestions(&["foo", "bar"]),
            ". Did you mean: foo, bar?"
        );
    }

    #[test]
    fn test_format_suggestions_with_prefix() {
        assert_eq!(
            format_suggestions_with_prefix(&["abs", "sin"], "math"),
            ". Did you mean: math::abs, math::sin?"
        );
    }
}
