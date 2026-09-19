pub mod apply;
pub mod export;
pub mod list;
pub mod show;

/// Escape a SQL identifier by wrapping it in backticks.
/// Any backtick inside the name is escaped as `\``.
pub fn escape_ident(name: &str) -> String {
    let escaped = name.replace('\\', "\\\\").replace('`', "\\`");
    format!("`{}`", escaped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_ident_simple() {
        assert_eq!(escape_ident("users"), "`users`");
    }

    #[test]
    fn test_escape_ident_with_backtick() {
        assert_eq!(escape_ident("my`table"), r#"`my\`table`"#);
    }

    #[test]
    fn test_escape_ident_with_backslash() {
        assert_eq!(escape_ident(r"my\table"), r"`my\\table`");
    }

    #[test]
    fn test_escape_ident_with_spaces() {
        assert_eq!(escape_ident("my table"), "`my table`");
    }

    #[test]
    fn test_escape_ident_with_backtick_and_backslash() {
        assert_eq!(escape_ident(r"a\`b"), r"`a\\\`b`");
    }
}
