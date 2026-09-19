use std::io::{self, IsTerminal, Write};

use crossterm::style::{Color, ResetColor, SetForegroundColor};
use serde_json::Value;

pub fn print_result(value: &Value, force_json: bool, force_pretty: bool) -> anyhow::Result<()> {
    let stdout = io::stdout();

    if force_json {
        println!("{}", serde_json::to_string(value)?);
    } else if force_pretty || stdout.is_terminal() {
        print_colored_json(value, 0)?;
        println!();
    } else {
        println!("{}", serde_json::to_string(value)?);
    }

    Ok(())
}

fn print_colored_json(value: &Value, indent: usize) -> anyhow::Result<()> {
    let mut stdout = io::stdout();
    write_colored_json(&mut stdout, value, indent)
}

fn write_colored_json(w: &mut impl Write, value: &Value, indent: usize) -> anyhow::Result<()> {
    let indent_str = "  ".repeat(indent);

    match value {
        Value::Null => {
            write!(w, "{}", SetForegroundColor(Color::Magenta))?;
            write!(w, "null")?;
            write!(w, "{}", ResetColor)?;
        }
        Value::Bool(b) => {
            write!(w, "{}", SetForegroundColor(Color::Magenta))?;
            write!(w, "{}", b)?;
            write!(w, "{}", ResetColor)?;
        }
        Value::Number(n) => {
            write!(w, "{}", SetForegroundColor(Color::Yellow))?;
            write!(w, "{}", n)?;
            write!(w, "{}", ResetColor)?;
        }
        Value::String(s) => {
            write!(w, "{}", SetForegroundColor(Color::Green))?;
            write!(w, "{}", serde_json::to_string(s)?)?;
            write!(w, "{}", ResetColor)?;
        }
        Value::Array(arr) => {
            if arr.is_empty() {
                write!(w, "[]")?;
            } else {
                writeln!(w, "[")?;
                for (i, item) in arr.iter().enumerate() {
                    write!(w, "{}  ", indent_str)?;
                    write_colored_json(w, item, indent + 1)?;
                    if i < arr.len() - 1 {
                        write!(w, ",")?;
                    }
                    writeln!(w)?;
                }
                write!(w, "{}]", indent_str)?;
            }
        }
        Value::Object(obj) => {
            if obj.is_empty() {
                write!(w, "{{}}")?;
            } else {
                writeln!(w, "{{")?;
                let entries: Vec<_> = obj.iter().collect();
                for (i, (key, val)) in entries.iter().enumerate() {
                    write!(w, "{}  ", indent_str)?;
                    write!(w, "{}", SetForegroundColor(Color::Cyan))?;
                    write!(w, "{}", serde_json::to_string(key)?)?;
                    write!(w, "{}", ResetColor)?;
                    write!(w, ": ")?;
                    write_colored_json(w, val, indent + 1)?;
                    if i < entries.len() - 1 {
                        write!(w, ",")?;
                    }
                    writeln!(w)?;
                }
                write!(w, "{}}}", indent_str)?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: render colored JSON to a String (stripping ANSI codes for assertions)
    fn render(value: &Value) -> String {
        let mut buf = Vec::new();
        write_colored_json(&mut buf, value, 0).unwrap();
        let raw = String::from_utf8(buf).unwrap();
        // Strip ANSI escape sequences for easier assertions
        let ansi_re = regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap();
        ansi_re.replace_all(&raw, "").to_string()
    }

    #[test]
    fn test_string_with_quotes_escaped() {
        let val = Value::String(r#"say "hello""#.to_string());
        let output = render(&val);
        assert_eq!(output, r#""say \"hello\"""#);
    }

    #[test]
    fn test_string_with_backslash_escaped() {
        let val = Value::String(r"path\to\file".to_string());
        let output = render(&val);
        assert_eq!(output, r#""path\\to\\file""#);
    }

    #[test]
    fn test_string_with_newline_escaped() {
        let val = Value::String("line1\nline2".to_string());
        let output = render(&val);
        assert_eq!(output, r#""line1\nline2""#);
    }

    #[test]
    fn test_object_key_escaped() {
        let val = serde_json::json!({"key\"with\"quotes": 1});
        let output = render(&val);
        assert!(output.contains(r#""key\"with\"quotes""#));
    }
}
