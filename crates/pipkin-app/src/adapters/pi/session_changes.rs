//! Session file scope, intersected with the current uncommitted Git diff.
use super::workspace::{self, Workspace};
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
};

/// Successful file mutations establish session ownership. Results at or before the last
/// clean scan are retired, so old committed calls cannot reappear after later workspace edits.
pub fn paths(entries: &[Value], after: u64) -> Vec<String> {
    let mut calls = HashMap::new();
    let mut paths = BTreeSet::new();
    for entry in entries {
        let entry_id = entry.get("id").and_then(Value::as_u64).unwrap_or(0);
        for message in entry
            .get("model")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if message.get("role").and_then(Value::as_str) == Some("assistant") {
                for block in message
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if block.get("type").and_then(Value::as_str) == Some("toolCall")
                        && let Some(id) = block.get("id").and_then(Value::as_str)
                    {
                        calls.insert(id.to_owned(), block);
                    }
                }
            }
            if message.get("role").and_then(Value::as_str) != Some("toolResult") {
                continue;
            }
            let Some(call) = message
                .get("toolCallId")
                .and_then(Value::as_str)
                .and_then(|id| calls.remove(id))
            else {
                continue;
            };
            if entry_id <= after || message.get("isError").and_then(Value::as_bool) != Some(false) {
                continue;
            }
            if !matches!(
                call.get("name").and_then(Value::as_str),
                Some("edit" | "write")
            ) {
                continue;
            }
            if let Some(path) = call
                .pointer("/arguments/path")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())
            {
                paths.insert(path.to_owned());
            }
        }
    }
    paths.into_iter().collect()
}

pub fn collect(dir: &Path, entries: &[Value], after: u64) -> Workspace {
    workspace::collect_scoped(dir, &paths(entries, after))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub fn edit(path: &str, failed: bool) -> Vec<Value> {
        vec![
            json!({"id": 1, "model": [{"role": "assistant", "content": [{"type": "toolCall", "id": "c", "name": "edit", "arguments": {"path": path}}]}]}),
            json!({"id": 2, "model": [{"role": "toolResult", "toolCallId": "c", "isError": failed}]}),
        ]
    }

    #[test]
    fn only_successful_session_mutations_enter_the_scope() {
        assert_eq!(paths(&edit("a.rs", false), 0), ["a.rs"]);
        assert!(paths(&edit("a.rs", true), 0).is_empty());
        assert!(paths(&[], 0).is_empty());
    }

    #[test]
    fn committed_calls_are_retired_but_new_results_are_not() {
        let entries = edit("a.rs", false);
        assert!(paths(&entries, 2).is_empty());
        assert_eq!(paths(&entries, 1), ["a.rs"]);
    }

    fn git(dir: &Path, args: &[&str]) {
        assert!(
            std::process::Command::new("git")
                .args([
                    "-c",
                    "user.name=test",
                    "-c",
                    "user.email=test@example.com",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .current_dir(dir)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn uncommitted_session_changes_clear_on_commit_and_use_the_new_base() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(dir.path().join("a.rs"), "base\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "other\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "base"]);
        let entries = edit("a.rs", false);
        std::fs::write(dir.path().join("a.rs"), "session\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "unrelated session\n").unwrap();
        let Workspace::Changes { files, .. } = collect(dir.path(), &entries, 0) else {
            panic!()
        };
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "a.rs");
        assert_eq!((files[0].added, files[0].removed), (1, 1));
        git(dir.path(), &["add", "a.rs"]);
        // Staging alone must not clear it.
        assert!(
            matches!(collect(dir.path(), &entries, 0), Workspace::Changes { files, .. } if files.len() == 1)
        );
        git(dir.path(), &["commit", "-q", "-m", "session"]);
        assert!(
            matches!(collect(dir.path(), &entries, 0), Workspace::Changes { files, .. } if files.is_empty())
        );
        std::fs::write(dir.path().join("a.rs"), "next edit\n").unwrap();
        let Workspace::Changes { files, .. } = collect(dir.path(), &entries, 0) else {
            panic!()
        };
        assert!(
            files[0]
                .hunks
                .iter()
                .flat_map(|h| &h.lines)
                .any(|l| l.text == "session" && l.kind == pipkin_core::DiffKind::Remove)
        );
        assert!(
            matches!(collect(dir.path(), &entries, 2), Workspace::Changes { files, .. } if files.is_empty())
        );
    }

    #[test]
    fn new_files_are_uncommitted_until_added_and_committed() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        let entries = edit("new file.txt", false);
        std::fs::write(dir.path().join("new file.txt"), "hello\n").unwrap();
        assert!(
            matches!(collect(dir.path(), &entries, 0), Workspace::Changes { files, .. } if files.len() == 1)
        );
        git(dir.path(), &["add", "."]);
        assert!(
            matches!(collect(dir.path(), &entries, 0), Workspace::Changes { files, .. } if files.len() == 1)
        );
        git(dir.path(), &["commit", "-q", "-m", "new file"]);
        assert!(
            matches!(collect(dir.path(), &entries, 0), Workspace::Changes { files, .. } if files.is_empty())
        );
    }
}
