//! Bounded, conversation-owned prompt recall. No UI or storage dependencies.
pub const MAX_PROMPT_HISTORY: usize = 200;
const MAX_PROMPT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Default)]
pub struct PromptHistory {
    entries: Vec<String>,
    cursor: Option<usize>,
    scratch: Option<String>,
}

impl PromptHistory {
    pub fn is_browsing(&self) -> bool {
        self.cursor.is_some()
    }

    pub fn has_entries(&self) -> bool {
        !self.entries.is_empty()
    }

    pub fn reset_navigation(&mut self) {
        self.cursor = None;
        self.scratch = None;
    }

    fn valid(text: &str) -> bool {
        !text.trim().is_empty() && text.len() <= MAX_PROMPT_BYTES
    }

    /// A deliberate submission. Consecutive repeats need only one recall slot.
    pub fn remember(&mut self, text: String) {
        self.reset_navigation();
        if !Self::valid(&text) || self.entries.last() == Some(&text) {
            return;
        }
        self.entries.push(text);
        self.trim();
    }

    /// Merge already durable transcript/journal text without doubling locally sent prompts.
    /// Earlier transcript pages are inserted before existing history, not at its recent end.
    pub fn import(&mut self, prompts: Vec<String>, older: bool) {
        let selected = self.cursor;
        let mut additions = Vec::new();
        for text in prompts {
            if Self::valid(&text)
                && !self.entries.contains(&text)
                && additions.last() != Some(&text)
            {
                additions.push(text);
            }
        }
        let added_before = if older { additions.len() } else { 0 };
        if older {
            additions.append(&mut self.entries);
            self.entries = additions;
        } else {
            self.entries.extend(additions);
        }
        let trimmed = self.entries.len().saturating_sub(MAX_PROMPT_HISTORY);
        self.trim();
        self.cursor = selected.map(|index| {
            (index + added_before)
                .saturating_sub(trimmed)
                .min(self.entries.len().saturating_sub(1))
        });
    }

    fn trim(&mut self) {
        let extra = self.entries.len().saturating_sub(MAX_PROMPT_HISTORY);
        if extra > 0 {
            self.entries.drain(..extra);
        }
    }

    pub fn previous(&mut self, draft: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let index = match self.cursor {
            Some(index) => index.saturating_sub(1),
            None => {
                self.scratch = Some(draft.to_owned());
                self.entries.len() - 1
            }
        };
        self.cursor = Some(index);
        Some(self.entries[index].clone())
    }

    pub fn next_prompt(&mut self) -> Option<String> {
        let index = self.cursor? + 1;
        if index < self.entries.len() {
            self.cursor = Some(index);
            Some(self.entries[index].clone())
        } else {
            self.cursor = None;
            self.scratch.take()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browsing_is_bounded_and_restores_the_unsent_draft() {
        let mut history = PromptHistory::default();
        history.remember("first".into());
        history.remember("second\nmultiline".into());
        assert_eq!(
            history.previous("unfinished"),
            Some("second\nmultiline".into())
        );
        assert_eq!(history.previous("ignored"), Some("first".into()));
        assert_eq!(history.previous("ignored"), Some("first".into()));
        assert_eq!(history.next_prompt(), Some("second\nmultiline".into()));
        assert_eq!(history.next_prompt(), Some("unfinished".into()));
        assert_eq!(history.next_prompt(), None);
    }

    #[test]
    fn importing_snapshots_does_not_duplicate_prompts_or_disrupt_navigation() {
        let mut history = PromptHistory::default();
        history.remember("recent".into());
        history.import(vec!["older".into(), "recent".into()], true);
        assert_eq!(history.previous("scratch"), Some("recent".into()));
        history.import(vec!["recent".into(), "new".into()], false);
        assert_eq!(history.next_prompt(), Some("new".into()));
        assert_eq!(history.next_prompt(), Some("scratch".into()));
        assert_eq!(history.entries, ["older", "recent", "new"]);
    }

    #[test]
    fn repeated_prompts_keep_their_order_when_older_history_arrives() {
        let mut history = PromptHistory::default();
        history.import(
            vec!["repeat".into(), "middle".into(), "repeat".into()],
            false,
        );
        assert_eq!(history.previous("scratch"), Some("repeat".into()));
        history.import(vec!["earlier".into()], true);
        assert_eq!(history.previous("ignored"), Some("middle".into()));
        assert_eq!(history.next_prompt(), Some("repeat".into()));
        assert_eq!(history.next_prompt(), Some("scratch".into()));
    }

    #[test]
    fn editing_a_recalled_prompt_starts_a_new_draft() {
        let mut history = PromptHistory::default();
        history.remember("sent".into());
        history.previous("scratch");
        history.reset_navigation();
        assert_eq!(history.next_prompt(), None);
        history.previous("edited recalled prompt");
        assert_eq!(history.next_prompt(), Some("edited recalled prompt".into()));
    }

    #[test]
    fn history_is_bounded_and_ignores_empty_or_oversized_text() {
        let mut history = PromptHistory::default();
        history.remember("  ".into());
        history.remember("x".repeat(MAX_PROMPT_BYTES + 1));
        assert!(!history.has_entries());
        for i in 0..MAX_PROMPT_HISTORY + 10 {
            history.remember(format!("prompt {i}"));
        }
        assert_eq!(history.entries.len(), MAX_PROMPT_HISTORY);
        assert_eq!(history.entries[0], "prompt 10");
    }
}
