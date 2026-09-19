/// Returns version string with git info: "0.1.0 (a1b2c3d-dirty)"
pub fn version_string() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let sha = option_env!("VERGEN_GIT_SHA").unwrap_or("unknown");
    let dirty = option_env!("VERGEN_GIT_DIRTY")
        .map(|d| if d == "true" { "-dirty" } else { "" })
        .unwrap_or("");

    // Take first 7 chars of SHA
    let short_sha = &sha[..7.min(sha.len())];

    format!("{} ({}{})", version, short_sha, dirty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_string_format() {
        let v = version_string();
        // Should start with version number
        assert!(v.starts_with(env!("CARGO_PKG_VERSION")));
        // Should have parentheses with git info
        assert!(v.contains('('));
        assert!(v.contains(')'));
    }
}
