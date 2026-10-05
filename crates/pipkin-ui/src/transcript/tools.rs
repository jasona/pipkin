//! How a tool call reads in the transcript: a friendly verb, the thing it acts on, and an icon.
//! Pure, so the mapping is tested without a window.

/// What to show for a tool call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolLabel {
    pub icon: &'static str,
    pub verb: String,
    /// The path, pattern or command it works on; may be empty.
    pub target: String,
}

/// The string value of `"key": "…"` in a JSON object's text, with the common escapes undone.
/// Tolerant: a truncated or malformed input just yields `None`.
pub fn json_string_field(input: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let mut from = 0;
    while let Some(found) = input[from..].find(&needle) {
        let after = from + found + needle.len();
        let rest = input[after..].trim_start();
        if let Some(rest) = rest.strip_prefix(':') {
            let rest = rest.trim_start();
            if let Some(body) = rest.strip_prefix('"') {
                let mut out = String::new();
                let mut chars = body.chars();
                while let Some(ch) = chars.next() {
                    match ch {
                        '"' => return Some(out),
                        '\\' => match chars.next() {
                            Some('n') => out.push('\n'),
                            Some('t') => out.push('\t'),
                            Some(other) => out.push(other),
                            None => return None,
                        },
                        _ => out.push(ch),
                    }
                }
                return None;
            }
        }
        from = after;
    }
    None
}

fn first_field(input: &str, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|k| json_string_field(input, k))
        .unwrap_or_default()
}

pub fn label(name: &str, input: &str) -> ToolLabel {
    let lower = name.to_ascii_lowercase();
    let (icon, verb, keys): (&'static str, &str, &[&str]) = match lower.as_str() {
        "read" | "read_file" | "cat" | "view" | "open" => {
            ("file-text", "Reading files", &["path", "file_path", "file"])
        }
        "grep" | "search" | "find" | "glob" | "rg" | "ls" | "list" | "list_files" => {
            ("search", "Searching", &["pattern", "query", "path", "glob"])
        }
        "edit" | "multiedit" | "apply_patch" | "patch" | "replace" => {
            ("pencil", "Editing files", &["path", "file_path", "file"])
        }
        "write" | "write_file" | "create" => (
            "file-plus",
            "Writing a file",
            &["path", "file_path", "file"],
        ),
        "bash" | "shell" | "sh" | "exec" | "run" | "terminal" => {
            ("terminal", "Running", &["command", "cmd", "script"])
        }
        _ => (
            "wrench",
            name,
            &["path", "command", "query", "pattern", "url"],
        ),
    };
    let mut target = first_field(input, keys);
    let known = icon != "wrench";
    if target.is_empty() && (!known || !input.trim_start().starts_with('{')) {
        // Not the JSON shape we know: show the input's first line, as was shown before.
        target = input.lines().next().unwrap_or("").trim().to_string();
    }
    ToolLabel {
        icon,
        verb: verb.to_string(),
        target,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_string_field_is_read_with_its_escapes_undone() {
        let input = r#"{"command":"echo \"hi\"\nls","cwd":"/tmp"}"#;
        assert_eq!(
            json_string_field(input, "command").as_deref(),
            Some("echo \"hi\"\nls")
        );
        assert_eq!(json_string_field(input, "cwd").as_deref(), Some("/tmp"));
        assert_eq!(json_string_field(input, "missing"), None);
        assert_eq!(
            json_string_field(r#"{"command":"unterminated"#, "command"),
            None
        );
        assert_eq!(
            json_string_field(r#"{"count":3}"#, "count"),
            None,
            "not a string"
        );
        assert_eq!(
            json_string_field("{ \"path\" :  \"a b\" }", "path").as_deref(),
            Some("a b")
        );
    }

    #[test]
    fn known_tools_get_a_verb_an_icon_and_their_target() {
        let l = label("read_file", r#"{"path":"src/lib.rs"}"#);
        assert_eq!(
            (l.icon, l.verb.as_str(), l.target.as_str()),
            ("file-text", "Reading files", "src/lib.rs")
        );
        let l = label("bash", r#"{"command":"cargo check"}"#);
        assert_eq!(
            (l.icon, l.verb.as_str(), l.target.as_str()),
            ("terminal", "Running", "cargo check")
        );
        let l = label("Edit", r#"{"path":"a.rs","old":"x"}"#);
        assert_eq!(
            (l.icon, l.verb.as_str(), l.target.as_str()),
            ("pencil", "Editing files", "a.rs")
        );
        let l = label("grep", r#"{"pattern":"error handling"}"#);
        assert_eq!(
            (l.icon, l.verb.as_str(), l.target.as_str()),
            ("search", "Searching", "error handling")
        );
    }

    #[test]
    fn a_plain_text_input_is_shown_as_it_is() {
        let l = label("bash", "cargo clippy -p pipkin-core -- -D warnings\nmore");
        assert_eq!(l.target, "cargo clippy -p pipkin-core -- -D warnings");
        let l = label("read_file", r#"{"unrelated":1}"#);
        assert_eq!(
            l.target, "",
            "a known tool with an unfamiliar JSON shape shows no target"
        );
    }

    #[test]
    fn an_unknown_tool_keeps_its_own_name_and_shows_its_input() {
        let l = label("deploy_preview", "{\"env\":\"staging\"}\nmore");
        assert_eq!(l.verb, "deploy_preview");
        assert_eq!(l.icon, "wrench");
        assert_eq!(l.target, "{\"env\":\"staging\"}");
    }
}
