//! Session-owned tool evidence. Never infer historical changes from today's working tree.
use std::collections::{BTreeMap, HashMap};

use pipkin_core::{DiffKind, DiffLine, FileChange, Hunk};
use serde_json::Value;

use super::workspace::{self, Workspace};

const MAX_LINES: usize = 2000;
const MAX_FILES: usize = 100;

fn lines(text: &str, kind: DiffKind) -> Vec<DiffLine> {
    text.lines()
        .take(MAX_LINES)
        .map(|line| DiffLine {
            kind,
            old_no: None,
            new_no: None,
            text: line.to_owned(),
        })
        .collect()
}

/// Entries are ordered and deduplicated by durable entry id by the caller.
pub fn collect(entries: &[Value], history_complete: bool) -> Workspace {
    let mut calls = HashMap::new();
    let mut files: BTreeMap<String, FileChange> = BTreeMap::new();
    let mut missing = !history_complete;
    for entry in entries {
        let Some(messages) = entry.get("model").and_then(Value::as_array) else {
            continue;
        };
        for message in messages {
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
                        calls.insert(id.to_owned(), block.clone());
                    }
                }
            }
            if message.get("role").and_then(Value::as_str) != Some("toolResult") {
                continue;
            }
            let Some(id) = message.get("toolCallId").and_then(Value::as_str) else {
                continue;
            };
            let Some(call) = calls.remove(id) else {
                missing = true;
                continue;
            };
            let name = call.get("name").and_then(Value::as_str).unwrap_or("");
            // Read-only tools do not imply changes, and failed calls are not successful edits.
            if name == "read" {
                continue;
            }
            if message.get("isError").and_then(Value::as_bool) != Some(false) {
                if name != "read" {
                    missing = true;
                }
                continue;
            }
            if name != "write" && name != "edit" {
                missing = true;
                continue;
            }
            let args = &call["arguments"];
            let Some(path) = args
                .get("path")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())
            else {
                missing = true;
                continue;
            };
            if files.len() >= MAX_FILES && !files.contains_key(path) {
                missing = true;
                continue;
            }
            let file = files.entry(path.to_owned()).or_insert_with(|| FileChange {
                path: path.to_owned(),
                added: 0,
                removed: 0,
                hunks: vec![],
            });
            if name == "write" {
                let Some(content) = args.get("content").and_then(Value::as_str) else {
                    missing = true;
                    continue;
                };
                // A recorded write may overwrite a file: do not claim every line was added.
                file.hunks.push(Hunk {
                    header: "Recorded write · previous contents unavailable".into(),
                    lines: lines(content, DiffKind::Context),
                });
                missing = true;
            } else if let Some(patch) = message.pointer("/details/patch").and_then(Value::as_str) {
                missing |= patch.lines().take(MAX_LINES + 1).count() > MAX_LINES;
                // The tool supplies a standard unified patch, without Git's framing header.
                let mut parsed = workspace::parse_diff(&format!(
                    "diff --git a/session-file b/session-file\n{patch}"
                ));
                if let Some(change) = parsed.pop().filter(|f| !f.hunks.is_empty()) {
                    file.added += change.added;
                    file.removed += change.removed;
                    file.hunks.extend(change.hunks);
                } else {
                    missing = true;
                }
            } else {
                // Older engines recorded exact replacements but not line-numbered patches.
                let legacy = args.get("oldText").zip(args.get("newText"));
                let replacements: Vec<(&Value, &Value)> = args
                    .get("edits")
                    .and_then(Value::as_array)
                    .map(|edits| {
                        edits
                            .iter()
                            .filter_map(|edit| edit.get("oldText").zip(edit.get("newText")))
                            .collect()
                    })
                    .unwrap_or_else(|| legacy.into_iter().collect());
                if replacements.is_empty() {
                    missing = true;
                }
                for (old, new) in replacements {
                    let (Some(old), Some(new)) = (old.as_str(), new.as_str()) else {
                        missing = true;
                        continue;
                    };
                    missing |= old.lines().take(MAX_LINES + 1).count() > MAX_LINES
                        || new.lines().take(MAX_LINES + 1).count() > MAX_LINES;
                    let removed = lines(old, DiffKind::Remove);
                    let added = lines(new, DiffKind::Add);
                    file.removed += removed.len() as u32;
                    file.added += added.len() as u32;
                    file.hunks.push(Hunk {
                        header: "Recorded replacement · line numbers unavailable".into(),
                        lines: removed.into_iter().chain(added).collect(),
                    });
                }
            }
            let line_count: usize = file.hunks.iter().map(|h| h.lines.len()).sum();
            if line_count > MAX_LINES {
                missing = true;
                let mut remaining = MAX_LINES;
                for hunk in &mut file.hunks {
                    hunk.lines.truncate(remaining);
                    remaining = remaining.saturating_sub(hunk.lines.len());
                }
            }
        }
    }
    let files = files
        .into_values()
        .filter(|file| !file.hunks.is_empty())
        .collect();
    let detail = if missing {
        "Recorded edits from this session only. Coverage is incomplete: shell/custom tools, missing history, and previous contents of writes may not have file diffs. Later workspace edits are not included."
    } else {
        "Recorded edits from this session only, in execution order—not a net repository diff. Later workspace edits are not included."
    };
    Workspace::Recorded {
        files,
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn edit(path: &str, old: &str, new: &str, failed: bool) -> Vec<Value> {
        vec![
            json!({"model": [{"role": "assistant", "content": [{"type": "toolCall", "id": "c", "name": "edit", "arguments": {"path": path, "edits": [{"oldText": old, "newText": new}]}}]}]}),
            json!({"model": [{"role": "toolResult", "toolCallId": "c", "isError": failed}]}),
        ]
    }

    #[test]
    fn sessions_in_the_same_directory_have_independent_recorded_changes() {
        for (path, old, new) in [("a.rs", "a", "b"), ("b.rs", "x", "y")] {
            let Workspace::Recorded { files, .. } = collect(&edit(path, old, new, false), true)
            else {
                panic!()
            };
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].path, path);
            assert_eq!(files[0].hunks[0].lines[0].text, old);
            assert_eq!(files[0].hunks[0].lines[1].text, new);
        }
        let Workspace::Recorded { files, .. } = collect(&[], true) else {
            panic!()
        };
        assert!(files.is_empty());
    }

    #[test]
    fn failed_edits_are_not_reported_as_successful_changes() {
        let Workspace::Recorded { files, detail } = collect(&edit("a", "a", "b", true), true)
        else {
            panic!()
        };
        assert!(files.is_empty());
        assert!(detail.contains("incomplete"));
    }

    #[test]
    fn uses_the_recorded_patch_not_current_file_contents() {
        let mut entries = edit("a.rs", "old", "new", false);
        entries[1]["model"][0]["details"] =
            json!({"patch": "--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n"});
        let Workspace::Recorded { files, .. } = collect(&entries, true) else {
            panic!()
        };
        assert_eq!((files[0].added, files[0].removed), (1, 1));
        assert_eq!(files[0].hunks[0].lines[0].text, "old");
    }

    #[test]
    fn shell_calls_and_missing_history_disclose_incomplete_coverage() {
        let entries = [
            json!({"model": [{"role": "assistant", "content": [{"type": "toolCall", "id": "c", "name": "bash", "arguments": {"command": "echo hi > file"}}]}]}),
            json!({"model": [{"role": "toolResult", "toolCallId": "c", "isError": false}]}),
        ];
        for (entries, complete) in [(&entries[..], true), (&[][..], false)] {
            let Workspace::Recorded { files, detail } = collect(entries, complete) else {
                panic!()
            };
            assert!(files.is_empty());
            assert!(detail.contains("incomplete"));
        }
    }

    #[test]
    fn writes_do_not_claim_a_complete_net_diff() {
        let entries = vec![
            json!({"model": [{"role": "assistant", "content": [{"type": "toolCall", "id": "c", "name": "write", "arguments": {"path": "file", "content": "replacement"}}]}]}),
            json!({"model": [{"role": "toolResult", "toolCallId": "c", "isError": false}]}),
        ];
        let Workspace::Recorded { files, detail } = collect(&entries, true) else {
            panic!()
        };
        assert_eq!((files[0].added, files[0].removed), (0, 0));
        assert_eq!(files[0].hunks[0].lines[0].kind, DiffKind::Context);
        assert!(detail.contains("incomplete"));
    }
}
