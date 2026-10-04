//! A small hand-written syntax highlighter. No GPUI, no dependencies.
//!
//! It scans bytes and only ever cuts at ASCII boundaries (or the end of input), so every
//! returned range falls on a UTF-8 character boundary. Non-ASCII bytes are treated as
//! identifier characters.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Keyword,
    String,
    Comment,
    Number,
    Type,
    Function,
    Added,
    Removed,
    Hunk,
}

struct Lang {
    keywords: &'static [&'static str],
    line_comments: &'static [&'static str],
    block_comment: bool,
    quotes: &'static [u8],
    triple_quotes: bool,
    /// Treat `$name` as a variable (shell).
    dollar_vars: bool,
    /// `'` is a lifetime/char in Rust rather than a string delimiter.
    rust_ticks: bool,
    toml_tables: bool,
}

const RUST: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
];
const JS: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "of",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "yield",
];
const PY: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "try", "while",
    "with", "yield",
];
const SH: &[&str] = &[
    "case", "do", "done", "elif", "else", "esac", "fi", "for", "function", "if", "in", "local",
    "return", "then", "until", "while", "export", "set",
];
const DATA: &[&str] = &["true", "false", "null", "yes", "no"];

fn lang_for(name: &str) -> Option<Lang> {
    let base = Lang {
        keywords: &[],
        line_comments: &[],
        block_comment: false,
        quotes: b"\"",
        triple_quotes: false,
        dollar_vars: false,
        rust_ticks: false,
        toml_tables: false,
    };
    Some(match name.to_ascii_lowercase().as_str() {
        "rust" | "rs" => Lang {
            keywords: RUST,
            line_comments: &["//"],
            block_comment: true,
            rust_ticks: true,
            ..base
        },
        "js" | "javascript" | "jsx" | "ts" | "typescript" | "tsx" | "mjs" => Lang {
            keywords: JS,
            line_comments: &["//"],
            block_comment: true,
            quotes: b"\"'`",
            ..base
        },
        "py" | "python" => Lang {
            keywords: PY,
            line_comments: &["#"],
            quotes: b"\"'",
            triple_quotes: true,
            ..base
        },
        "json" | "jsonc" => Lang {
            keywords: DATA,
            ..base
        },
        "sh" | "bash" | "shell" | "zsh" | "console" => Lang {
            keywords: SH,
            line_comments: &["#"],
            quotes: b"\"'",
            dollar_vars: true,
            ..base
        },
        "toml" => Lang {
            keywords: DATA,
            line_comments: &["#"],
            quotes: b"\"'",
            toml_tables: true,
            ..base
        },
        "yaml" | "yml" => Lang {
            keywords: DATA,
            line_comments: &["#"],
            quotes: b"\"'",
            ..base
        },
        _ => return None,
    })
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// Highlight `text` as `lang`. Ranges are sorted, non-overlapping, and only cover tokens;
/// everything else is plain.
pub fn highlight(lang: &str, text: &str) -> Vec<(Range<usize>, Token)> {
    let l = lang.to_ascii_lowercase();
    if matches!(l.as_str(), "diff" | "patch") {
        return diff(text);
    }
    match lang_for(&l) {
        Some(cfg) => scan(&cfg, text),
        None => Vec::new(),
    }
}

fn diff(text: &str) -> Vec<(Range<usize>, Token)> {
    let mut out = Vec::new();
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches('\n');
        let token = if body.starts_with("@@") {
            Some(Token::Hunk)
        } else if body.starts_with("+++") || body.starts_with("---") {
            Some(Token::Comment)
        } else if body.starts_with('+') {
            Some(Token::Added)
        } else if body.starts_with('-') {
            Some(Token::Removed)
        } else {
            None
        };
        if let Some(t) = token
            && !body.is_empty()
        {
            out.push((start..start + body.len(), t));
        }
        start += line.len();
    }
    out
}

fn scan(cfg: &Lang, text: &str) -> Vec<(Range<usize>, Token)> {
    let b = text.as_bytes();
    let mut out: Vec<(Range<usize>, Token)> = Vec::new();
    let mut i = 0;
    let mut line_start = true;
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            line_start = true;
            i += 1;
            continue;
        }
        let at_line_start = line_start;
        if !c.is_ascii_whitespace() {
            line_start = false;
        }
        // Comments.
        if let Some(_p) = cfg
            .line_comments
            .iter()
            .find(|p| b[i..].starts_with(p.as_bytes()))
        {
            let end = b[i..]
                .iter()
                .position(|&x| x == b'\n')
                .map_or(b.len(), |n| i + n);
            out.push((i..end, Token::Comment));
            i = end;
            continue;
        }
        if cfg.block_comment && b[i..].starts_with(b"/*") {
            let end = text[i + 2..].find("*/").map_or(b.len(), |n| i + 2 + n + 2);
            out.push((i..end, Token::Comment));
            i = end;
            continue;
        }
        if cfg.toml_tables && at_line_start && c == b'[' {
            let end = b[i..]
                .iter()
                .position(|&x| x == b'\n')
                .map_or(b.len(), |n| i + n);
            out.push((i..end, Token::Type));
            i = end;
            continue;
        }
        // Rust lifetimes and char literals.
        if cfg.rust_ticks && c == b'\'' {
            let rest = &b[i + 1..];
            let is_char = (rest.len() >= 2 && rest[1] == b'\'' && rest[0] != b'\\')
                || (rest.len() >= 3 && rest[0] == b'\\' && rest[2] == b'\'');
            if is_char {
                let end = if rest[0] == b'\\' { i + 4 } else { i + 3 };
                out.push((i..end, Token::String));
                i = end;
            } else {
                i += 1;
            }
            continue;
        }
        // Strings.
        if cfg.quotes.contains(&c) {
            let triple = cfg.triple_quotes && b[i..].starts_with(&[c, c, c]);
            let end = if triple {
                let close = [c, c, c];
                let mut j = i + 3;
                loop {
                    if j >= b.len() {
                        break b.len();
                    }
                    if b[j..].starts_with(&close) {
                        break j + 3;
                    }
                    j += 1;
                }
            } else {
                let mut j = i + 1;
                loop {
                    if j >= b.len() || b[j] == b'\n' {
                        break j;
                    }
                    if b[j] == b'\\' {
                        j += 2;
                        continue;
                    }
                    if b[j] == c {
                        break j + 1;
                    }
                    j += 1;
                }
            };
            let end = end.min(b.len());
            // JSON keys read better as type color.
            let is_key = cfg.keywords == DATA
                && !cfg.toml_tables
                && b[end..].iter().find(|x| !x.is_ascii_whitespace()) == Some(&b':');
            out.push((i..end, if is_key { Token::Type } else { Token::String }));
            i = end;
            continue;
        }
        // Shell variables.
        if cfg.dollar_vars && c == b'$' {
            let mut j = i + 1;
            if j < b.len() && b[j] == b'{' {
                j = b[j..]
                    .iter()
                    .position(|&x| x == b'}')
                    .map_or(b.len(), |n| j + n + 1);
            } else {
                while j < b.len() && is_ident(b[j]) {
                    j += 1;
                }
            }
            if j > i + 1 {
                out.push((i..j, Token::Type));
            }
            i = j.max(i + 1);
            continue;
        }
        // Numbers.
        if c.is_ascii_digit() {
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] == b'.') {
                // Stop before a method call or range (`1..2`, `1.max`).
                if b[j] == b'.' && (j + 1 >= b.len() || !b[j + 1].is_ascii_digit()) {
                    break;
                }
                j += 1;
            }
            out.push((i..j, Token::Number));
            i = j;
            continue;
        }
        // Identifiers.
        if is_ident(c) {
            let mut j = i + 1;
            while j < b.len() && is_ident(b[j]) {
                j += 1;
            }
            let word = &text[i..j];
            let next_non_space = b[j..].iter().find(|x| **x != b' ');
            if cfg.keywords.contains(&word) {
                out.push((i..j, Token::Keyword));
            } else if word.as_bytes()[0].is_ascii_uppercase()
                && !cfg.toml_tables
                && cfg.keywords != DATA
            {
                out.push((i..j, Token::Type));
            } else if next_non_space == Some(&b'(') && cfg.keywords != DATA {
                out.push((i..j, Token::Function));
            }
            i = j;
            continue;
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens<'a>(lang: &str, text: &'a str) -> Vec<(&'a str, Token)> {
        highlight(lang, text)
            .into_iter()
            .map(|(r, t)| (&text[r], t))
            .collect()
    }

    #[test]
    fn rust_basics() {
        let t = tokens(
            "rust",
            "fn main() { let x = 42; // hi\n let s = \"a\\\"b\"; }",
        );
        assert!(t.contains(&("fn", Token::Keyword)));
        assert!(t.contains(&("main", Token::Function)));
        assert!(t.contains(&("42", Token::Number)));
        assert!(t.contains(&("// hi", Token::Comment)));
        assert!(t.contains(&("\"a\\\"b\"", Token::String)));
    }

    #[test]
    fn rust_lifetimes_are_not_strings() {
        let t = tokens("rs", "fn f<'a>(x: &'a str) -> char { 'x' }");
        assert!(t.contains(&("'x'", Token::String)));
        assert!(
            !t.iter()
                .any(|(s, k)| *k == Token::String && s.contains("a>"))
        );
    }

    #[test]
    fn python_triple_quotes_and_comments() {
        let t = tokens("python", "def f():\n    \"\"\"doc\nmore\"\"\"  # c\n");
        assert!(t.contains(&("def", Token::Keyword)));
        assert!(
            t.iter().any(|(s, k)| *k == Token::String
                && s.starts_with("\"\"\"")
                && s.ends_with("\"\"\""))
        );
        assert!(t.contains(&("# c", Token::Comment)));
    }

    #[test]
    fn json_keys_and_values() {
        let t = tokens("json", "{\"a\": [1, true, \"x\"]}");
        assert!(t.contains(&("\"a\"", Token::Type)));
        assert!(t.contains(&("true", Token::Keyword)));
        assert!(t.contains(&("\"x\"", Token::String)));
    }

    #[test]
    fn shell_vars_and_toml_tables() {
        assert!(
            tokens("bash", "echo \"$HOME\" $X ${Y}")
                .iter()
                .any(|(s, k)| *s == "$X" && *k == Token::Type)
        );
        assert!(tokens("toml", "[package]\nname = \"x\"\n").contains(&("[package]", Token::Type)));
    }

    #[test]
    fn diff_lines() {
        let t = tokens("diff", "--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n ctx\n");
        assert!(t.contains(&("@@ -1 +1 @@", Token::Hunk)));
        assert!(t.contains(&("-old", Token::Removed)));
        assert!(t.contains(&("+new", Token::Added)));
    }

    #[test]
    fn unknown_language_is_plain_and_ranges_are_char_boundaries() {
        assert!(highlight("cobol", "MOVE 1 TO X").is_empty());
        let text = "let s = \"héllo 👨‍👩‍👧‍👦\"; // ünï 中文\nfn ñ() {}";
        for (r, _) in highlight("rust", text) {
            assert!(text.is_char_boundary(r.start) && text.is_char_boundary(r.end));
        }
    }

    #[test]
    fn unterminated_constructs_do_not_panic() {
        for src in ["\"open", "/* open", "'", "\"\"\"x", "$", "${", "1.", "'\\"] {
            for lang in ["rust", "python", "bash", "js", "json", "toml"] {
                for (r, _) in highlight(lang, src) {
                    assert!(r.end <= src.len() && src.is_char_boundary(r.end));
                }
            }
        }
    }
}
