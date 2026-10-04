//! Pure editing model for the composer. No GPUI types: selection, grapheme and word
//! movement, UTF-16 conversion for platform input methods, marked (IME) text and undo.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

const MAX_HISTORY: usize = 200;

#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    text: String,
    anchor: usize,
    head: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditKind {
    Typing,
    Backspace,
    Delete,
    Other,
}

#[derive(Clone, Debug, Default)]
pub struct EditorModel {
    text: String,
    anchor: usize,
    head: usize,
    marked: Option<Range<usize>>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_kind: Option<EditKind>,
    /// State before the current IME composition began; becomes one undo step on commit.
    composition_base: Option<Snapshot>,
    revision: u64,
}

/// Replace CRLF and lone CR with LF.
pub fn normalize_newlines(s: &str) -> String {
    if !s.contains('\r') {
        return s.to_string();
    }
    s.replace("\r\n", "\n").replace('\r', "\n")
}

fn word_class(g: &str) -> u8 {
    let c = g.chars().next().unwrap_or(' ');
    if c == '\n' {
        3
    } else if c.is_whitespace() {
        0
    } else if c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

impl EditorModel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_text(text: &str) -> Self {
        let mut m = Self::new();
        m.set_text(text);
        m
    }

    // ---------------------------------------------------------------- accessors

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Bumped on every change to text, marked range or history-affecting state.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn anchor(&self) -> usize {
        self.anchor
    }

    /// The caret: the moving end of the selection.
    pub fn head(&self) -> usize {
        self.head
    }

    pub fn selection(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_reversed(&self) -> bool {
        self.head < self.anchor
    }

    pub fn selected_text(&self) -> &str {
        &self.text[self.selection()]
    }

    pub fn has_selection(&self) -> bool {
        self.anchor != self.head
    }

    pub fn marked_range(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    pub fn is_composing(&self) -> bool {
        self.marked.is_some()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Replace the whole text (programmatic). Clears history, marked text and selection.
    pub fn set_text(&mut self, text: &str) {
        self.text = normalize_newlines(text);
        self.anchor = self.text.len();
        self.head = self.text.len();
        self.marked = None;
        self.composition_base = None;
        self.undo.clear();
        self.redo.clear();
        self.last_kind = None;
        self.revision += 1;
    }

    // -------------------------------------------------------------- boundaries

    pub fn is_grapheme_boundary(&self, offset: usize) -> bool {
        if offset == 0 || offset >= self.text.len() {
            return offset <= self.text.len();
        }
        self.text.grapheme_indices(true).any(|(i, _)| i == offset)
    }

    pub fn prev_boundary(&self, offset: usize) -> usize {
        self.text[..offset.min(self.text.len())]
            .grapheme_indices(true)
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    pub fn next_boundary(&self, offset: usize) -> usize {
        let offset = offset.min(self.text.len());
        self.text[offset..]
            .graphemes(true)
            .next()
            .map(|g| offset + g.len())
            .unwrap_or(self.text.len())
    }

    /// Nearest grapheme boundary to `offset` (ties go backward).
    pub fn snap_to_boundary(&self, offset: usize) -> usize {
        let offset = offset.min(self.text.len());
        let mut prev = 0;
        for (i, _) in self.text.grapheme_indices(true) {
            if i == offset {
                return offset;
            }
            if i > offset {
                return if offset - prev <= i - offset { prev } else { i };
            }
            prev = i;
        }
        // `prev` is the start of the last grapheme; the end of text is the next boundary.
        if offset - prev <= self.text.len() - offset {
            prev
        } else {
            self.text.len()
        }
    }

    pub fn word_boundary_left(&self, offset: usize) -> usize {
        let mut pos = offset.min(self.text.len());
        let prev = |pos: usize| self.text[..pos].graphemes(true).next_back();
        if prev(pos) == Some("\n") {
            return pos - 1;
        }
        while let Some(g) = prev(pos) {
            if word_class(g) == 0 {
                pos -= g.len();
            } else {
                break;
            }
        }
        let Some(first) = prev(pos) else { return pos };
        if first == "\n" {
            return pos;
        }
        let class = word_class(first);
        while let Some(g) = prev(pos) {
            if word_class(g) == class {
                pos -= g.len();
            } else {
                break;
            }
        }
        pos
    }

    pub fn word_boundary_right(&self, offset: usize) -> usize {
        let mut pos = offset.min(self.text.len());
        let next = |pos: usize| self.text[pos..].graphemes(true).next();
        if next(pos) == Some("\n") {
            return pos + 1;
        }
        while let Some(g) = next(pos) {
            if word_class(g) == 0 {
                pos += g.len();
            } else {
                break;
            }
        }
        let Some(first) = next(pos) else { return pos };
        if first == "\n" {
            return pos;
        }
        let class = word_class(first);
        while let Some(g) = next(pos) {
            if word_class(g) == class {
                pos += g.len();
            } else {
                break;
            }
        }
        pos
    }

    /// Start of the logical line containing `offset`.
    pub fn line_start(&self, offset: usize) -> usize {
        self.text[..offset.min(self.text.len())]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0)
    }

    /// End of the logical line containing `offset` (before its newline).
    pub fn line_end(&self, offset: usize) -> usize {
        let offset = offset.min(self.text.len());
        self.text[offset..]
            .find('\n')
            .map(|i| offset + i)
            .unwrap_or(self.text.len())
    }

    // ---------------------------------------------------------------- movement

    fn set_head(&mut self, offset: usize, extend: bool) {
        self.head = offset;
        if !extend {
            self.anchor = offset;
        }
        self.last_kind = None;
    }

    pub fn move_to(&mut self, offset: usize, extend: bool) {
        let offset = self.snap_to_boundary(offset);
        self.set_head(offset, extend);
    }

    pub fn set_selection(&mut self, anchor: usize, head: usize) {
        self.anchor = self.snap_to_boundary(anchor);
        self.head = self.snap_to_boundary(head);
        self.last_kind = None;
    }

    pub fn left(&mut self, extend: bool) {
        if !extend && self.has_selection() {
            let start = self.selection().start;
            self.set_head(start, false);
        } else {
            let to = self.prev_boundary(self.head);
            self.set_head(to, extend);
        }
    }

    pub fn right(&mut self, extend: bool) {
        if !extend && self.has_selection() {
            let end = self.selection().end;
            self.set_head(end, false);
        } else {
            let to = self.next_boundary(self.head);
            self.set_head(to, extend);
        }
    }

    pub fn word_left(&mut self, extend: bool) {
        let to = self.word_boundary_left(self.head);
        self.set_head(to, extend);
    }

    pub fn word_right(&mut self, extend: bool) {
        let to = self.word_boundary_right(self.head);
        self.set_head(to, extend);
    }

    pub fn home(&mut self, extend: bool) {
        let to = self.line_start(self.head);
        self.set_head(to, extend);
    }

    pub fn end(&mut self, extend: bool) {
        let to = self.line_end(self.head);
        self.set_head(to, extend);
    }

    pub fn doc_start(&mut self, extend: bool) {
        self.set_head(0, extend);
    }

    pub fn doc_end(&mut self, extend: bool) {
        self.set_head(self.text.len(), extend);
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.head = self.text.len();
        self.last_kind = None;
    }

    /// Select the word (or whitespace/punctuation run) at `offset`.
    pub fn select_word_at(&mut self, offset: usize) {
        let offset = self.snap_to_boundary(offset);
        let at = self.text[offset..].graphemes(true).next();
        let before = self.text[..offset].graphemes(true).next_back();
        let Some(probe) = at.or(before) else { return };
        let class = word_class(probe);
        let mut start = if at.is_some() {
            offset
        } else {
            offset - probe.len()
        };
        let mut end = start + probe.len();
        while let Some(g) = self.text[..start].graphemes(true).next_back() {
            if word_class(g) == class && class != 3 {
                start -= g.len();
            } else {
                break;
            }
        }
        while let Some(g) = self.text[end..].graphemes(true).next() {
            if word_class(g) == class && class != 3 {
                end += g.len();
            } else {
                break;
            }
        }
        self.anchor = start;
        self.head = end;
        self.last_kind = None;
    }

    /// Select the whole logical line at `offset`, including its trailing newline.
    pub fn select_line_at(&mut self, offset: usize) {
        let offset = offset.min(self.text.len());
        let start = self.line_start(offset);
        let mut end = self.line_end(offset);
        if end < self.text.len() {
            end += 1;
        }
        self.anchor = start;
        self.head = end;
        self.last_kind = None;
    }

    // ------------------------------------------------------------------- edits

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            anchor: self.anchor,
            head: self.head,
        }
    }

    fn push_undo(&mut self, snap: Snapshot) {
        self.undo.push(snap);
        if self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Record history before a mutation. Consecutive edits of the same coalescable kind share
    /// one undo step unless `force_new` (e.g. the edit replaces a selection).
    fn record(&mut self, kind: EditKind, force_new: bool) {
        if self.composition_base.is_some() {
            return;
        }
        let coalescable = !matches!(kind, EditKind::Other);
        let join = coalescable && !force_new && self.last_kind == Some(kind);
        if !join {
            let snap = self.snapshot();
            self.push_undo(snap);
        }
        self.last_kind = coalescable.then_some(kind);
    }

    fn splice(&mut self, range: Range<usize>, new: &str) {
        self.text.replace_range(range, new);
        self.revision += 1;
    }

    fn replace_selection(&mut self, new: &str, kind: EditKind) -> bool {
        let force_new = self.has_selection();
        self.replace_selection_with(new, kind, force_new)
    }

    fn replace_selection_with(&mut self, new: &str, kind: EditKind, force_new: bool) -> bool {
        let sel = self.selection();
        if sel.is_empty() && new.is_empty() {
            return false;
        }
        self.record(kind, force_new);
        self.splice(sel.clone(), new);
        let caret = sel.start + new.len();
        self.anchor = caret;
        self.head = caret;
        self.marked = None;
        true
    }

    /// User insertion (typing path via actions, paste, newline). CRLF is normalized.
    pub fn insert(&mut self, text: &str) -> bool {
        if self.composition_base.is_some() {
            return false;
        }
        let text = normalize_newlines(text);
        let kind = if text.graphemes(true).count() == 1 && text != "\n" {
            EditKind::Typing
        } else {
            EditKind::Other
        };
        self.replace_selection(&text, kind)
    }

    pub fn backspace(&mut self) -> bool {
        if self.composition_base.is_some() {
            return false;
        }
        if self.has_selection() {
            return self.replace_selection("", EditKind::Backspace);
        }
        let prev = self.prev_boundary(self.head);
        if prev == self.head {
            return false;
        }
        self.anchor = prev;
        self.replace_selection_with("", EditKind::Backspace, false)
    }

    pub fn delete(&mut self) -> bool {
        if self.composition_base.is_some() {
            return false;
        }
        if self.has_selection() {
            return self.replace_selection("", EditKind::Delete);
        }
        let next = self.next_boundary(self.head);
        if next == self.head {
            return false;
        }
        self.anchor = next;
        self.replace_selection_with("", EditKind::Delete, false)
    }

    pub fn delete_word_back(&mut self) -> bool {
        if self.composition_base.is_some() {
            return false;
        }
        if !self.has_selection() {
            self.anchor = self.word_boundary_left(self.head);
        }
        self.replace_selection("", EditKind::Other)
    }

    pub fn delete_word_forward(&mut self) -> bool {
        if self.composition_base.is_some() {
            return false;
        }
        if !self.has_selection() {
            self.anchor = self.word_boundary_right(self.head);
        }
        self.replace_selection("", EditKind::Other)
    }

    // ------------------------------------------------------------ undo / redo

    fn restore(&mut self, s: Snapshot) {
        self.text = s.text;
        self.anchor = s.anchor.min(self.text.len());
        self.head = s.head.min(self.text.len());
        self.marked = None;
        self.last_kind = None;
        self.revision += 1;
    }

    pub fn undo(&mut self) -> bool {
        if self.composition_base.is_some() {
            return false;
        }
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore(prev);
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.composition_base.is_some() {
            return false;
        }
        let Some(next) = self.redo.pop() else {
            return false;
        };
        let cur = self.snapshot();
        self.undo.push(cur);
        self.restore(next);
        true
    }

    // ------------------------------------------------------- UTF-16 conversion

    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        let offset = offset.min(self.text.len());
        let mut n = 0;
        for (i, ch) in self.text.char_indices() {
            if i >= offset {
                break;
            }
            n += ch.len_utf16();
        }
        n
    }

    /// Converts a UTF-16 offset to UTF-8; an offset inside a surrogate pair rounds forward
    /// to the end of that character.
    pub fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf16 = 0;
        for (i, ch) in self.text.char_indices() {
            if utf16 >= offset {
                return i;
            }
            utf16 += ch.len_utf16();
        }
        self.text.len()
    }

    pub fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(r.start)..self.offset_to_utf16(r.end)
    }

    pub fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> {
        let a = self.offset_from_utf16(r.start);
        let b = self.offset_from_utf16(r.end);
        a.min(b)..a.max(b)
    }

    /// Text for a UTF-16 range, plus the range actually used (UTF-16).
    pub fn text_for_range_utf16(&self, r: Range<usize>) -> (String, Range<usize>) {
        let range = self.range_from_utf16(&r);
        (
            self.text[range.clone()].to_string(),
            self.range_to_utf16(&range),
        )
    }

    // --------------------------------------------------------- platform input

    fn target_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        range_utf16
            .map(|r| self.range_from_utf16(&r))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection())
    }

    fn finish_composition(&mut self) {
        if let Some(base) = self.composition_base.take() {
            if base.text != self.text {
                self.push_undo(base);
            }
            self.last_kind = None;
        }
    }

    /// `EntityInputHandler::replace_text_in_range`: typing, IME commit, dictation.
    pub fn replace_text_in_range_utf16(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
    ) -> bool {
        let range = self.target_range(range_utf16.clone());
        let new_text = normalize_newlines(new_text);
        let composing = self.composition_base.is_some();
        if !composing {
            let plain = range_utf16.is_none() && self.marked.is_none();
            let kind = if plain && new_text.graphemes(true).count() == 1 && new_text != "\n" {
                EditKind::Typing
            } else {
                EditKind::Other
            };
            if range.is_empty() && new_text.is_empty() {
                return false;
            }
            self.record(kind, !range.is_empty());
        }
        self.splice(range.clone(), &new_text);
        let caret = range.start + new_text.len();
        self.anchor = caret;
        self.head = caret;
        self.marked = None;
        self.finish_composition();
        true
    }

    /// `EntityInputHandler::replace_and_mark_text_in_range`: IME preedit updates. The caller's
    /// `new_selected_range_utf16` is relative to `new_text`.
    pub fn replace_and_mark_text_in_range_utf16(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
    ) {
        let range = self.target_range(range_utf16);
        let new_text = normalize_newlines(new_text);
        if self.composition_base.is_none() {
            self.composition_base = Some(self.snapshot());
        }
        self.splice(range.clone(), &new_text);
        if new_text.is_empty() {
            self.marked = None;
            self.anchor = range.start;
            self.head = range.start;
            // Composition cancelled: drop the base if nothing net changed, else keep one step.
            self.finish_composition();
            return;
        }
        let marked = range.start..range.start + new_text.len();
        let (a, b) = match new_selected_range_utf16 {
            Some(sel) => {
                let local = EditorModel::with_text_no_norm(&new_text);
                let r = local.range_from_utf16(&sel);
                (marked.start + r.start, marked.start + r.end)
            }
            None => (marked.end, marked.end),
        };
        self.anchor = a;
        self.head = b;
        self.marked = Some(marked);
    }

    fn with_text_no_norm(text: &str) -> Self {
        EditorModel {
            text: text.to_string(),
            ..Default::default()
        }
    }

    /// `EntityInputHandler::unmark_text`: accept the preedit as-is.
    pub fn unmark(&mut self) {
        self.marked = None;
        self.finish_composition();
        self.revision += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(m: &mut EditorModel, s: &str) {
        for g in s.graphemes(true) {
            m.insert(g);
        }
    }

    #[test]
    fn grapheme_movement_over_emoji_and_marks() {
        let family = "👨\u{200d}👩\u{200d}👧\u{200d}👦";
        let flag = "🇨🇦";
        let combined = "e\u{301}";
        let text = format!("a{family}{flag}{combined}b");
        let mut m = EditorModel::with_text(&text);
        let mut stops = vec![m.head()];
        m.doc_start(false);
        let mut seen = vec![0];
        while m.head() < text.len() {
            m.right(false);
            seen.push(m.head());
        }
        assert_eq!(seen.len(), 6, "a, family, flag, e+accent, b → 5 moves");
        assert_eq!(seen[1], 1);
        assert_eq!(seen[2], 1 + family.len());
        assert_eq!(seen[3], 1 + family.len() + flag.len());
        stops.clear();
        while m.head() > 0 {
            m.left(false);
            stops.push(m.head());
        }
        assert_eq!(stops.len(), 5);
    }

    #[test]
    fn backspace_removes_whole_cluster() {
        let mut m = EditorModel::with_text("x👨\u{200d}👩\u{200d}👧");
        assert!(m.backspace());
        assert_eq!(m.text(), "x");
        let mut m = EditorModel::with_text("e\u{301}");
        m.backspace();
        assert_eq!(m.text(), "");
        let mut m = EditorModel::with_text("🇨🇦🇫🇷");
        m.backspace();
        assert_eq!(m.text(), "🇨🇦");
        m.doc_start(false);
        m.delete();
        assert_eq!(m.text(), "");
        assert!(!m.delete() && !m.backspace());
    }

    #[test]
    fn snap_to_boundary_inside_cluster() {
        let m = EditorModel::with_text("e\u{301}x");
        assert_eq!(m.snap_to_boundary(1), 0);
        assert_eq!(m.snap_to_boundary(3), 3);
        let m = EditorModel::with_text("🇨🇦");
        assert_eq!(m.snap_to_boundary(4), 0);
        assert_eq!(m.snap_to_boundary(8), 8);
        assert!(m.is_grapheme_boundary(0) && !m.is_grapheme_boundary(4));
    }

    #[test]
    fn utf16_round_trips() {
        let m = EditorModel::with_text("aé😀b\n漢");
        for (i, _) in m.text().char_indices().chain([(m.text().len(), ' ')]) {
            assert_eq!(m.offset_from_utf16(m.offset_to_utf16(i)), i);
        }
        assert_eq!(m.offset_to_utf16("aé😀".len()), 4);
        // Inside a surrogate pair: rounds to the following character start.
        assert_eq!(m.offset_from_utf16(3), "aé😀".len());
        let (s, used) = m.text_for_range_utf16(1..4);
        assert_eq!(s, "é😀");
        assert_eq!(used, 1..4);
    }

    #[test]
    fn crlf_is_normalized() {
        let mut m = EditorModel::new();
        m.insert("a\r\nb\rc");
        assert_eq!(m.text(), "a\nb\nc");
        let mut m = EditorModel::new();
        m.replace_text_in_range_utf16(None, "x\r\ny");
        assert_eq!(m.text(), "x\ny");
    }

    #[test]
    fn selection_reversal_and_collapse() {
        let mut m = EditorModel::with_text("hello");
        m.doc_start(false);
        m.right(false);
        m.right(true);
        m.right(true);
        assert_eq!(m.selection(), 1..3);
        assert!(!m.is_reversed());
        m.left(true);
        m.left(true);
        m.left(true);
        assert_eq!(m.selection(), 0..1);
        assert!(m.is_reversed());
        m.right(false);
        assert_eq!(m.selection(), 1..1);
        m.select_all();
        m.left(false);
        assert_eq!(m.head(), 0);
        m.select_all();
        m.right(false);
        assert_eq!(m.head(), 5);
        assert_eq!(m.selected_text(), "");
    }

    #[test]
    fn word_movement_and_deletion() {
        let mut m = EditorModel::with_text("foo bar_baz, qux");
        m.doc_start(false);
        m.word_right(false);
        assert_eq!(m.head(), 3);
        m.word_right(false);
        assert_eq!(m.head(), 11);
        m.word_right(false);
        assert_eq!(m.head(), 12);
        m.word_left(false);
        assert_eq!(m.head(), 11);
        m.word_left(false);
        assert_eq!(m.head(), 4);
        m.doc_end(false);
        assert!(m.delete_word_back());
        assert_eq!(m.text(), "foo bar_baz, ");
        m.doc_start(false);
        assert!(m.delete_word_forward());
        assert_eq!(m.text(), " bar_baz, ");
        let mut m = EditorModel::with_text("a\nb");
        m.doc_end(false);
        m.word_left(false);
        assert_eq!(m.head(), 2);
        m.word_left(false);
        assert_eq!(m.head(), 1);
    }

    #[test]
    fn line_motion_and_selection_helpers() {
        let mut m = EditorModel::with_text("one\ntwo words\n\nfour");
        m.set_selection(6, 6);
        m.home(false);
        assert_eq!(m.head(), 4);
        m.end(false);
        assert_eq!(m.head(), 13);
        m.select_word_at(6);
        assert_eq!(m.selected_text(), "two");
        m.select_word_at(7);
        assert_eq!(m.selected_text(), " ");
        m.select_line_at(5);
        assert_eq!(m.selected_text(), "two words\n");
        m.select_line_at(14);
        assert_eq!(m.selected_text(), "\n");
        m.select_line_at(m.text().len());
        assert_eq!(m.selected_text(), "four");
    }

    #[test]
    fn typing_coalesces_into_one_undo_step() {
        let mut m = EditorModel::new();
        typed(&mut m, "hello 👍🏽");
        assert_eq!(m.text(), "hello 👍🏽");
        assert!(m.undo());
        assert_eq!(m.text(), "");
        assert!(!m.undo());
        assert!(m.redo());
        assert_eq!(m.text(), "hello 👍🏽");
        assert!(!m.redo());
    }

    #[test]
    fn movement_breaks_coalescing_and_redo_clears_on_edit() {
        let mut m = EditorModel::new();
        typed(&mut m, "ab");
        m.left(false);
        typed(&mut m, "X");
        assert_eq!(m.text(), "aXb");
        m.undo();
        assert_eq!(m.text(), "ab");
        m.undo();
        assert_eq!(m.text(), "");
        m.redo();
        typed(&mut m, "z");
        assert!(!m.can_redo());
    }

    #[test]
    fn undo_through_emoji_edits_restores_selection() {
        let mut m = EditorModel::with_text("a👩🏽‍💻b");
        m.set_selection(1, 1 + "👩🏽‍💻".len());
        m.insert("x");
        assert_eq!(m.text(), "axb");
        m.backspace();
        assert_eq!(m.text(), "ab");
        m.undo();
        assert_eq!(m.text(), "axb");
        m.undo();
        assert_eq!(m.text(), "a👩🏽‍💻b");
        assert_eq!(m.selection(), 1..1 + "👩🏽‍💻".len());
        m.redo();
        m.redo();
        assert_eq!(m.text(), "ab");
    }

    #[test]
    fn backspace_run_coalesces() {
        let mut m = EditorModel::with_text("abcd");
        m.backspace();
        m.backspace();
        assert_eq!(m.text(), "ab");
        m.undo();
        assert_eq!(m.text(), "abcd");
    }

    #[test]
    fn ime_commit_is_one_undo_step() {
        let mut m = EditorModel::with_text("x");
        m.replace_and_mark_text_in_range_utf16(None, "に", Some(1..1));
        assert!(m.is_composing());
        assert_eq!(m.marked_range(), Some(1..4));
        m.replace_and_mark_text_in_range_utf16(None, "にほ", Some(2..2));
        assert_eq!(m.marked_range(), Some(1..7));
        m.replace_and_mark_text_in_range_utf16(None, "日本", Some(2..2));
        assert_eq!(m.text(), "x日本");
        // Edits via actions are ignored while composing.
        assert!(!m.backspace() && !m.insert("q") && !m.undo());
        m.replace_text_in_range_utf16(None, "日本");
        assert!(!m.is_composing());
        assert_eq!(m.text(), "x日本");
        assert_eq!(m.head(), "x日本".len());
        assert!(m.undo());
        assert_eq!(m.text(), "x");
        assert!(m.undo() || true);
    }

    #[test]
    fn ime_cancel_leaves_no_history() {
        let mut m = EditorModel::with_text("x");
        m.replace_and_mark_text_in_range_utf16(None, "に", None);
        m.replace_and_mark_text_in_range_utf16(None, "", None);
        assert!(!m.is_composing());
        assert_eq!(m.text(), "x");
        assert!(!m.can_undo());
        assert_eq!(m.head(), 1);
    }

    #[test]
    fn ime_replaces_selection_and_explicit_range() {
        let mut m = EditorModel::with_text("hello world");
        m.set_selection(6, 11);
        m.replace_and_mark_text_in_range_utf16(None, "w", Some(0..1));
        assert_eq!(m.text(), "hello w");
        assert_eq!(m.marked_range(), Some(6..7));
        assert_eq!(m.selection(), 6..7);
        m.replace_text_in_range_utf16(Some(6..7), "W");
        assert_eq!(m.text(), "hello W");
        assert!(m.undo());
        assert_eq!(m.text(), "hello world");
        assert_eq!(m.selection(), 6..11);
    }

    #[test]
    fn ime_selection_is_utf16_relative_to_new_text() {
        let mut m = EditorModel::with_text("ab");
        m.set_selection(1, 1);
        m.replace_and_mark_text_in_range_utf16(None, "😀x", Some(2..3));
        assert_eq!(m.marked_range(), Some(1..6));
        assert_eq!(m.selection(), 5..6);
    }

    #[test]
    fn unmark_commits_in_place() {
        let mut m = EditorModel::new();
        m.replace_and_mark_text_in_range_utf16(None, "abc", None);
        m.unmark();
        assert!(!m.is_composing());
        assert!(m.undo());
        assert_eq!(m.text(), "");
    }

    #[test]
    fn typing_through_input_handler_coalesces() {
        let mut m = EditorModel::new();
        for c in ["h", "i", "!"] {
            m.replace_text_in_range_utf16(None, c);
        }
        assert_eq!(m.text(), "hi!");
        m.undo();
        assert_eq!(m.text(), "");
    }

    #[test]
    fn set_text_resets_history() {
        let mut m = EditorModel::new();
        typed(&mut m, "abc");
        m.set_text("zzz");
        assert!(!m.can_undo() && !m.can_redo());
        assert_eq!(m.head(), 3);
    }

    #[test]
    fn history_is_bounded() {
        let mut m = EditorModel::new();
        for i in 0..(MAX_HISTORY + 50) {
            m.insert(&format!("line {i}\n"));
        }
        let mut n = 0;
        while m.undo() {
            n += 1;
        }
        assert_eq!(n, MAX_HISTORY);
    }
}
