//! Fixture file contents and generated diffs for the scripted stories.
//!
//! Everything here is inert text: nothing is executed or written to disk.

use desktop_core::FileChange;

use super::diff::unified_diff;
use super::rng::Rng;

pub const SEARCH_PATH: &str = "crates/desktop-core/src/search.rs";
pub const NAV_PATH: &str = "crates/desktop-core/src/nav.rs";
pub const TEST_PATH: &str = "crates/desktop-core/tests/search.rs";

pub const SEARCH_OLD: &str = r#"//! Conversation search matching.

/// True when `title` matches the user's search `query`.
pub fn matches(title: &str, query: &str) -> bool {
    let query = query.trim();
    query.is_empty() || title.contains(query)
}

/// Sort key for search results; lower sorts first.
pub fn rank(title: &str, query: &str) -> usize {
    let query = query.trim().to_lowercase();
    title.to_lowercase().find(&query).unwrap_or(usize::MAX)
}
"#;

pub const SEARCH_NEW: &str = r#"//! Conversation search matching.

/// True when `title` matches the user's search `query`, ignoring case and surrounding space.
pub fn matches(title: &str, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty() || title.to_lowercase().contains(&query)
}

/// Sort key for search results; lower sorts first.
pub fn rank(title: &str, query: &str) -> usize {
    let query = query.trim().to_lowercase();
    title.to_lowercase().find(&query).unwrap_or(usize::MAX)
}
"#;

pub const NAV_OLD: &str = r#"//! Navigation list helpers.

use crate::ids::ProjectId;
use crate::state::ConversationState;

/// Conversations of one project that match `query`, newest first.
pub fn visible<'a>(
    conversations: &'a [ConversationState],
    project: ProjectId,
    query: &str,
) -> Vec<&'a ConversationState> {
    let q = query.trim();
    let mut found: Vec<_> = conversations
        .iter()
        .filter(|c| c.project == project)
        .filter(|c| q.is_empty() || c.title.contains(q))
        .collect();
    found.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    found
}
"#;

pub const NAV_NEW: &str = r#"//! Navigation list helpers.

use crate::ids::ProjectId;
use crate::search::matches;
use crate::state::ConversationState;

/// Conversations of one project that match `query`, newest first.
pub fn visible<'a>(
    conversations: &'a [ConversationState],
    project: ProjectId,
    query: &str,
) -> Vec<&'a ConversationState> {
    let mut found: Vec<_> = conversations
        .iter()
        .filter(|c| c.project == project)
        .filter(|c| matches(&c.title, query))
        .collect();
    found.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    found
}
"#;

pub const TEST_OLD: &str = r#"use desktop_core::{nav, search};
use desktop_core::{ConversationId, ConversationState, ProjectId};

fn conv(id: u64, title: &str, at: i64) -> ConversationState {
    ConversationState::new(ConversationId(id), ProjectId(1), title.into(), at)
}

#[test]
fn matches_ignores_case() {
    assert!(search::matches("Fix Billing Rounding", "billing"));
    assert!(search::matches("Fix Billing Rounding", "BILLING"));
}

#[test]
fn rank_prefers_earlier_matches() {
    assert!(search::rank("billing fix", "billing") < search::rank("fix billing", "billing"));
}

#[test]
fn visible_ignores_case() {
    let list = vec![conv(1, "Fix Billing", 10), conv(2, "billing audit", 20), conv(3, "Other", 30)];
    let found = nav::visible(&list, ProjectId(1), "BILLING");
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].id, ConversationId(2));
}
"#;

pub const TEST_NEW: &str = r#"use desktop_core::{nav, search};
use desktop_core::{ConversationId, ConversationState, ProjectId};

fn conv(id: u64, title: &str, at: i64) -> ConversationState {
    ConversationState::new(ConversationId(id), ProjectId(1), title.into(), at)
}

#[test]
fn matches_ignores_case() {
    assert!(search::matches("Fix Billing Rounding", "billing"));
    assert!(search::matches("Fix Billing Rounding", "BILLING"));
}

#[test]
fn rank_prefers_earlier_matches() {
    assert!(search::rank("billing fix", "billing") < search::rank("fix billing", "billing"));
}

#[test]
fn visible_ignores_case() {
    let list = vec![conv(1, "Fix Billing", 10), conv(2, "billing audit", 20), conv(3, "Other", 30)];
    let found = nav::visible(&list, ProjectId(1), "BILLING");
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].id, ConversationId(2));
}

#[test]
fn matches_trims_surrounding_whitespace() {
    assert!(search::matches("Fix Billing Rounding", "  rounding "));
    assert!(search::matches("anything", "   "));
}
"#;

/// The three file changes reported by the normal scenario (and shown in the seeded
/// "Fix failing search test" conversation).
pub fn fix_search_changes() -> Vec<FileChange> {
    vec![
        unified_diff(SEARCH_PATH, SEARCH_OLD, SEARCH_NEW),
        unified_diff(NAV_PATH, NAV_OLD, NAV_NEW),
        unified_diff(TEST_PATH, TEST_OLD, TEST_NEW),
    ]
}

/// Five generated files with well over 2,000 changed lines in total.
pub fn large_diff(seed: u64) -> Vec<FileChange> {
    const FILES: [&str; 5] = ["messages", "session", "tools", "events", "errors"];
    FILES
        .iter()
        .enumerate()
        .map(|(n, name)| {
            let mut rng = Rng::from_parts(&[seed, 0xD1FF, n as u64]);
            let mut old = Vec::new();
            let mut new = Vec::new();
            for i in 0..720usize {
                let line = format!(
                    "    pub {name}_field_{i:04}: Option<u32>, // slot {}",
                    i * 7 % 97
                );
                old.push(line.clone());
                match rng.below(100) {
                    0..=44 => new.push(format!(
                        "    pub {name}_field_{i:04}: Option<u64>, // slot {} (widened)",
                        i * 7 % 97
                    )),
                    45..=47 => {}
                    48..=52 => {
                        new.push(line);
                        new.push(format!("    pub {name}_extra_{i:04}: bool,"));
                    }
                    _ => new.push(line),
                }
            }
            let wrap = |lines: Vec<String>| {
                format!(
                    "pub struct {}Wire {{\n{}\n}}\n",
                    name.to_uppercase(),
                    lines.join("\n")
                )
            };
            unified_diff(
                &format!("crates/protocol/src/generated/{name}.rs"),
                &wrap(old),
                &wrap(new),
            )
        })
        .collect()
}

/// Deterministic build-log text of roughly `bytes` bytes (never ends mid-character).
pub fn log_text(rng: &mut Rng, bytes: usize) -> String {
    const LEVELS: [&str; 4] = ["INFO", "INFO", "WARN", "DEBUG"];
    const MODULES: [&str; 5] = ["resolver", "codegen", "linker", "cache", "scheduler"];
    let mut out = String::with_capacity(bytes + 128);
    while out.len() < bytes {
        let n = rng.below(1_000_000);
        out.push_str(&format!(
            "[{:>5}] {:<9} unit {:06} compiled in {:>4} ms (cache {}, hash {:016x})\n",
            LEVELS[rng.below(LEVELS.len())],
            MODULES[rng.below(MODULES.len())],
            n,
            rng.below(4000),
            if rng.chance(70) { "hit" } else { "miss" },
            rng.next_u64(),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn story_changes_are_three_real_diffs() {
        let c = fix_search_changes();
        assert_eq!(c.len(), 3);
        assert!(c.iter().all(|f| f.added > 0 && !f.hunks.is_empty()));
        assert!(c[0].hunks[0].header.starts_with("@@ -"));
    }

    #[test]
    fn large_diff_has_over_two_thousand_changed_lines() {
        let total: u32 = large_diff(7).iter().map(|f| f.added + f.removed).sum();
        assert!(total >= 2000, "only {total} changed lines");
        assert_eq!(large_diff(7), large_diff(7));
    }
}
