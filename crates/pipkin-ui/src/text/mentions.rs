//! Bounded, read-only project path discovery and byte-safe @ token handling.
use std::{ops::Range, path::Path};

#[derive(Clone, Debug)]
pub struct PathMention {
    pub range: Range<usize>,
    pub path: String,
}

pub fn tokens(text: &str) -> Vec<PathMention> {
    let mut out = Vec::new();
    let mut cursor = 0;
    while cursor < text.len() {
        let c = text[cursor..].chars().next().unwrap();
        if c != '@' || (cursor > 0 && !text[..cursor].chars().next_back().unwrap().is_whitespace())
        {
            cursor += c.len_utf8();
            continue;
        }
        let start = cursor;
        cursor += 1;
        let quoted = text[cursor..].starts_with('"');
        if quoted {
            cursor += 1;
        }
        let path_start = cursor;
        while cursor < text.len() {
            let c = text[cursor..].chars().next().unwrap();
            if (quoted && c == '"') || (!quoted && c.is_whitespace()) {
                break;
            }
            cursor += c.len_utf8();
        }
        let path = text[path_start..cursor].to_string();
        if quoted && text[cursor..].starts_with('"') {
            cursor += 1;
        }
        out.push(PathMention {
            range: start..cursor,
            path,
        });
    }
    out
}

pub fn query(text: &str, caret: usize) -> Option<PathMention> {
    tokens(text).into_iter().find(|t| {
        t.range.start < caret && caret == t.range.end && !text[t.range.clone()].ends_with('"')
    })
}

pub fn insertion(path: &str) -> String {
    if path.chars().any(char::is_whitespace) {
        format!("@\"{path}\" ")
    } else {
        format!("@{path} ")
    }
}

pub fn matching(paths: &[String], query: &str) -> Vec<String> {
    let q = query.to_lowercase();
    let mut scored: Vec<_> = paths
        .iter()
        .filter_map(|p| {
            let lower = p.to_lowercase();
            lower
                .find(&q)
                .map(|pos| ((usize::from(pos != 0), pos, p.len()), p))
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    scored.into_iter().take(8).map(|(_, p)| p.clone()).collect()
}

/// Do not traverse symlinks or dependency/VCS directories. Bound both work and memory.
pub fn collect(root: &Path) -> std::io::Result<Vec<String>> {
    let mut paths = Vec::new();
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut visited = 0;
    while let Some((dir, depth)) = pending.pop() {
        if depth > 16 || visited >= 20_000 {
            continue;
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if depth == 0 => return Err(error),
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > 20_000 {
                break;
            }
            let name = entry.file_name();
            if matches!(
                name.to_str(),
                Some(".git" | "node_modules" | "target" | ".venv" | "vendor")
            ) {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() || (!kind.is_file() && !kind.is_dir()) {
                continue;
            }
            let path = entry.path();
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let Some(relative) = relative.to_str() else {
                continue;
            };
            if relative.contains(['"', '\n', '\r']) {
                continue;
            }
            paths.push(format!(
                "{relative}{}",
                if kind.is_dir() { "/" } else { "" }
            ));
            if kind.is_dir() {
                pending.push((path, depth + 1));
            }
        }
    }
    paths.sort();
    Ok(paths)
}

pub fn file_url(path: &Path) -> String {
    let mut url = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mentions_are_words_not_email_and_support_spaces_and_unicode() {
        assert!(query("mail me@example.com", 19).is_none());
        let s = "check @src/é";
        assert_eq!(query(s, s.len()).unwrap().path, "src/é");
        assert_eq!(insertion("my file.rs"), "@\"my file.rs\" ");
        assert_eq!(tokens("@\"my file.rs\" then @src/a")[0].path, "my file.rs");
        assert!(query("@\"my file.rs\"", 13).is_none());
    }
    #[test]
    fn discovery_includes_files_and_directories_but_skips_dependencies() {
        let root = std::env::temp_dir().join(format!(
            "pipkin-paths-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(root.join("src/my file.rs"), "").unwrap();
        std::fs::write(root.join("node_modules/pkg/ignored.js"), "").unwrap();
        assert_eq!(collect(&root).unwrap(), ["src/", "src/my file.rs"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn discovery_never_traverses_symlinks_and_missing_roots_are_errors() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let root = std::env::temp_dir().join(format!(
            "pipkin-path-links-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("project")).unwrap();
        std::fs::create_dir_all(root.join("outside")).unwrap();
        std::fs::write(root.join("outside/private.txt"), "synthetic").unwrap();
        std::os::unix::fs::symlink(root.join("outside"), root.join("project/link")).unwrap();
        std::os::unix::fs::symlink(root.join("outside/private.txt"), root.join("project/file"))
            .unwrap();
        assert!(collect(&root.join("project")).unwrap().is_empty());
        assert!(collect(&root.join("missing")).is_err());
        let uid = std::fs::metadata(root.join("project")).unwrap().uid();
        std::fs::set_permissions(root.join("project"), std::fs::Permissions::from_mode(0o000))
            .unwrap();
        let denied = collect(&root.join("project"));
        std::fs::set_permissions(root.join("project"), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        if uid != 0 {
            assert_eq!(
                denied.unwrap_err().kind(),
                std::io::ErrorKind::PermissionDenied
            );
        } else {
            eprintln!("permission-denial assertion unverified when run as root");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn matches_are_ranked_and_bounded() {
        let paths = vec![
            "deep/src/main.rs".into(),
            "src/lib.rs".into(),
            "other.rs".into(),
        ];
        assert_eq!(matching(&paths, "src"), ["src/lib.rs", "deep/src/main.rs"]);
        assert_eq!(
            file_url(Path::new("/tmp/a #é")),
            "file:///tmp/a%20%23%C3%A9"
        );
    }
}
