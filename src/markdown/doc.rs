//! Rope-backed text document with edit grouping for undo/redo.
//!
//! All public offsets are **char** offsets (not bytes), which is what the layout,
//! caret and selection code use throughout the editor.

use std::path::{Path, PathBuf};
use std::time::Instant;

use ropey::Rope;

#[derive(Clone, Debug)]
struct Edit {
    pos: usize,
    removed: String,
    inserted: String,
    /// Wall clock of the edit, used to coalesce typing bursts.
    at: Instant,
    /// Coalescing class: 1 = typing, 2 = deleting, 0 = never merge.
    class: u8,
}

pub struct Doc {
    rope: Rope,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub version: u64,
    /// Each entry is one user-visible undo step (chronological edits inside).
    undo: Vec<Vec<Edit>>,
    redo: Vec<Vec<Edit>>,
    /// Cached mapping from line index to first char, invalidated on structural edits.
    line_starts: Vec<usize>,
    line_starts_valid: bool,
    /// When set, following edits join one undo step (see `begin_compound`).
    compound_open: bool,
    compound_at: usize,
    /// `(modified time in ms, length)` of the file as Bare MD last read or wrote
    /// it. Anything else on that path with another stamp means somebody outside
    /// the editor changed the file.
    pub disk: Option<(u64, u64)>,
    /// True when the file this document came from has been deleted (or moved)
    /// behind the editor's back.
    ///
    /// Kept as a flag rather than asked on every frame: the check is a `stat`, and
    /// the tab strip asks once per tab per frame. `refresh_file_gone` is what
    /// actually looks.
    pub file_gone: bool,
}

/// Identity of a file on disk: modification time (ms since the epoch) and size.
///
/// Cheap enough to poll (one `stat`) and precise enough to notice a save from
/// another program or a sync client.
pub fn disk_stamp(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Some((mtime, meta.len()))
}

impl Doc {
    pub fn new(text: &str, path: Option<PathBuf>) -> Self {
        let mut doc = Doc {
            rope: Rope::from_str(text),
            path,
            dirty: false,
            version: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            line_starts: Vec::new(),
            line_starts_valid: false,
            compound_open: false,
            compound_at: 0,
            disk: None,
            file_gone: false,
        };
        doc.rebuild_line_starts();
        doc.note_disk();
        doc
    }

    pub fn empty() -> Self {
        Doc::new("", None)
    }

    /// Remember what the file looks like *right now* as its known-good state.
    pub fn note_disk(&mut self) {
        self.disk = self.path.as_deref().and_then(disk_stamp);
        self.file_gone = false;
    }

    /// `stat` the file again and remember whether it has gone.
    ///
    /// Only a document that *had* a file can lose it: an untitled buffer, and one
    /// opened on a path that never existed, keep `file_gone == false` so nothing
    /// starts shouting at the user about a file they never saved.
    pub fn refresh_file_gone(&mut self) -> bool {
        let gone = match (&self.path, self.disk) {
            (Some(path), Some(_)) => disk_stamp(path).is_none(),
            _ => false,
        };
        self.file_gone = gone;
        gone
    }

    /// True when the text of this document no longer exists anywhere on disk.
    pub fn file_gone(&self) -> bool {
        self.file_gone
    }

    /// True when the file changed under us since it was read or written.
    pub fn changed_on_disk(&self) -> bool {
        let Some(path) = &self.path else { return false };
        let Some(known) = self.disk else { return false };
        match disk_stamp(path) {
            Some(now) => now != known,
            // A file that vanished is not "changed": reloading would empty the
            // buffer under the user's hands. It is its own state - see
            // `refresh_file_gone` - and the tab says so in red.
            None => false,
        }
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn slice(&self, start: usize, end: usize) -> String {
        let start = start.min(self.len_chars());
        let end = end.min(self.len_chars()).max(start);
        self.rope.slice(start..end).to_string()
    }

    /// Line contents without the trailing newline.
    pub fn line_text(&self, line: usize) -> String {
        if line >= self.len_lines() {
            return String::new();
        }
        let mut s = self.rope.line(line).to_string();
        while s.ends_with('\n') || s.ends_with('\r') {
            s.pop();
        }
        s
    }

    /// Every character of the document, in order.
    ///
    /// The incremental layout compares the whole document against what it laid out
    /// last time; going through this single iterator costs one pass instead of a rope
    /// lookup per line, which on a 30 000-line document is the difference between
    /// 12 ms and 2 ms per keystroke.
    pub fn chars(&self) -> impl Iterator<Item = char> + '_ {
        self.rope.chars()
    }

    /// The same, from the end: used to find where a change stops as cheaply as where
    /// it starts.
    pub fn chars_rev(&self) -> impl Iterator<Item = char> + '_ {
        self.rope.chars_at(self.rope.len_chars()).reversed()
    }

    /// The characters of `line`, without the line break.
    ///
    /// The layout asks for this for every line of the document on every rebuild, so
    /// it avoids the `String` that `line_text` would allocate and collect in between:
    /// on a 30 000-line document that is 30 000 fewer allocations per keystroke.
    pub fn line_chars(&self, line: usize) -> Vec<char> {
        if line >= self.len_lines() {
            return Vec::new();
        }
        let mut chars: Vec<char> = self.rope.line(line).chars().collect();
        while matches!(chars.last(), Some('\n') | Some('\r')) {
            chars.pop();
        }
        chars
    }

    pub fn rebuild_line_starts(&mut self) {
        let mut v = Vec::with_capacity(self.len_lines());
        let mut acc = 0usize;
        for line in self.rope.lines() {
            v.push(acc);
            acc += line.len_chars();
        }
        self.line_starts = v;
        self.line_starts_valid = true;
    }

    pub fn line_start(&self, line: usize) -> usize {
        if !self.line_starts_valid {
            // Safe fallback; callers normally operate on a valid cache.
            return self.rope.line_to_char(line.min(self.len_lines().saturating_sub(1)));
        }
        self.line_starts.get(line).copied().unwrap_or_else(|| self.len_chars())
    }

    /// Length of `line` in chars excluding the trailing newline.
    pub fn line_len(&self, line: usize) -> usize {
        let start = self.line_start(line);
        let raw_end = if line + 1 < self.len_lines() {
            self.line_start(line + 1)
        } else {
            self.len_chars()
        };
        let mut end = raw_end;
        while end > start {
            let c = self.rope.char(end - 1);
            if c == '\n' || c == '\r' {
                end -= 1;
            } else {
                break;
            }
        }
        end.saturating_sub(start)
    }

    pub fn line_of_char(&self, ch: usize) -> usize {
        let ch = ch.min(self.len_chars());
        if self.line_starts_valid {
            match self.line_starts.binary_search(&ch) {
                Ok(i) => i,
                Err(i) => i.saturating_sub(1),
            }
        } else {
            self.rope.char_to_line(ch)
        }
    }

    fn invalidate(&mut self) {
        self.line_starts_valid = false;
    }

    /// Insert `text` at `pos` (char offset). Returns the caret position after the insert.
    pub fn insert(&mut self, pos: usize, text: &str) -> usize {
        self.insert_with_class(pos, text, 1)
    }

    /// Insert from a non-typing source (paste, toolbar formatting, auto-indent).
    pub fn insert_atomic(&mut self, pos: usize, text: &str) -> usize {
        self.insert_with_class(pos, text, 0)
    }

    fn insert_with_class(&mut self, pos: usize, text: &str, class: u8) -> usize {
        if text.is_empty() {
            return pos;
        }
        let pos = pos.min(self.len_chars());
        let char_pos = self.rope.byte_to_char(self.rope.char_to_byte(pos));
        self.rope.insert(char_pos, text);
        self.invalidate();
        self.version += 1;
        self.dirty = true;
        let n = text.chars().count();
        self.push_edit(Edit {
            pos,
            removed: String::new(),
            inserted: text.to_string(),
            at: Instant::now(),
            class,
        });
        self.redo.clear();
        pos + n
    }

    /// Remove the char range `[start, end)`.
    pub fn remove(&mut self, start: usize, end: usize) -> usize {
        self.remove_with_class(start, end, 2)
    }

    pub fn remove_atomic(&mut self, start: usize, end: usize) -> usize {
        self.remove_with_class(start, end, 0)
    }

    fn remove_with_class(&mut self, start: usize, end: usize, class: u8) -> usize {
        let start = start.min(self.len_chars());
        let end = end.min(self.len_chars()).max(start);
        if start == end {
            return start;
        }
        let removed = self.slice(start, end);
        self.rope.remove(start..end);
        self.invalidate();
        self.version += 1;
        self.dirty = true;
        self.push_edit(Edit { pos: start, removed, inserted: String::new(), at: Instant::now(), class });
        self.redo.clear();
        start
    }

    /// Replace a range with new text as a single undo step.
    pub fn replace_range(&mut self, start: usize, end: usize, text: &str) -> usize {
        let start = start.min(self.len_chars());
        let end = end.min(self.len_chars()).max(start);
        let removed = self.slice(start, end);
        if removed.is_empty() && text.is_empty() {
            return start;
        }
        self.rope.remove(start..end);
        self.rope.insert(start, text);
        self.invalidate();
        self.version += 1;
        self.dirty = true;
        self.push_edit(Edit {
            pos: start,
            removed,
            inserted: text.to_string(),
            at: Instant::now(),
            class: 0,
        });
        self.redo.clear();
        start + text.chars().count()
    }

    fn push_edit(&mut self, edit: Edit) {
        if self.compound_open && self.undo.len() > self.compound_at {
            if let Some(g) = self.undo.last_mut() {
                g.push(edit);
                return;
            }
        }
        // Merge with the previous edit when it is a continuation of the same burst.
        if let Some(last) = self.undo.last_mut().and_then(|g| g.last_mut()) {
            let close_in_time = last.at.elapsed().as_millis() < 900;
            let same_class = last.class != 0 && last.class == edit.class;
            if close_in_time && same_class {
                match edit.class {
                    1 => {
                        // typing: append if directly after the previous insertion
                        if last.inserted.is_empty() == false
                            && last.pos + last.inserted.chars().count() == edit.pos
                            && edit.removed.is_empty()
                        {
                            last.inserted.push_str(&edit.inserted);
                            last.at = edit.at;
                            return;
                        }
                    }
                    2 => {
                        // backspacing: extend the removed range backwards
                        if last.inserted.is_empty()
                            && edit.inserted.is_empty()
                            && edit.pos + edit.removed.chars().count() == last.pos
                        {
                            last.removed = format!("{}{}", edit.removed, last.removed);
                            last.pos = edit.pos;
                            last.at = edit.at;
                            return;
                        }
                        // forward deleting: extend forwards
                        if last.inserted.is_empty()
                            && edit.inserted.is_empty()
                            && edit.pos == last.pos
                        {
                            last.removed.push_str(&edit.removed);
                            last.at = edit.at;
                            return;
                        }
                    }
                    _ => {}
                }
            }
        }
        self.undo.push(vec![edit]);
        if self.undo.len() > 2000 {
            self.undo.remove(0);
        }
    }

    /// Undo one step (which may contain several primitive edits); returns the caret.
    pub fn undo(&mut self) -> Option<usize> {
        let group = self.undo.pop()?;
        let mut caret = None;
        for edit in group.iter().rev() {
            let end = edit.pos + edit.inserted.chars().count();
            if !edit.inserted.is_empty() {
                self.rope.remove(edit.pos..end);
            }
            if !edit.removed.is_empty() {
                self.rope.insert(edit.pos, &edit.removed);
            }
            caret = Some(edit.pos);
        }
        self.invalidate();
        self.version += 1;
        self.dirty = true;
        self.redo.push(group);
        caret
    }

    pub fn redo(&mut self) -> Option<usize> {
        let group = self.redo.pop()?;
        let mut caret = None;
        for edit in group.iter() {
            let end = edit.pos + edit.removed.chars().count();
            if !edit.removed.is_empty() {
                self.rope.remove(edit.pos..end);
            }
            if !edit.inserted.is_empty() {
                self.rope.insert(edit.pos, &edit.inserted);
            }
            caret = Some(edit.pos + edit.inserted.chars().count());
        }
        self.invalidate();
        self.version += 1;
        self.dirty = true;
        self.undo.push(group);
        caret
    }

    /// Wrap the most recent edit *and* whatever follows into a single undo step.
    /// Used by "replace all", line operations and other multi-edit commands.
    pub fn begin_compound(&mut self) {
        self.break_undo_group();
        self.compound_open = true;
        self.compound_at = self.undo.len();
    }

    pub fn end_compound(&mut self) {
        self.compound_open = false;
    }

    /// Break undo coalescing, e.g. after moving the caret.
    pub fn break_undo_group(&mut self) {
        if let Some(last) = self.undo.last_mut().and_then(|g| g.last_mut()) {
            last.class = 0;
        }
        self.compound_open = false;
    }

    // ---- line-oriented helpers used by commands and layout ----

    /// Index of the line containing `ch` and the char offset within that line.
    pub fn line_and_col(&self, ch: usize) -> (usize, usize) {
        let line = self.line_of_char(ch);
        (line, ch.saturating_sub(self.line_start(line)))
    }

    /// Convenience for tests and for the search engine.
    pub fn as_str(&self) -> String {
        self.text()
    }
}

/// Find the end of the word at `pos` (used for ctrl+arrow / double click).
pub fn word_bounds(text: &[char], pos: usize) -> (usize, usize) {
    let n = text.len();
    if n == 0 {
        return (0, 0);
    }
    let pos = pos.min(n);
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    // If we're at the end of a word, select that word.
    let mut start = pos;
    if start == n || !is_word(text[start]) {
        while start > 0 && !is_word(text[start - 1]) {
            start -= 1;
        }
    }
    while start > 0 && is_word(text[start - 1]) {
        start -= 1;
    }
    let mut end = pos;
    while end < n && is_word(text[end]) {
        end += 1;
    }
    if start == end && end < n {
        end += 1;
    }
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_vanishes_is_marked_but_not_treated_as_changed() {
        let dir = std::env::temp_dir().join(format!("bare-md-gone-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("gone.md");
        std::fs::write(&path, "text\n").unwrap();
        let mut d = Doc::new("text\n", Some(path.clone()));
        assert!(!d.file_gone(), "a file that is there is not gone");
        assert!(!d.changed_on_disk());

        std::fs::remove_file(&path).unwrap();
        assert!(d.refresh_file_gone(), "the deletion was not noticed");
        assert!(d.file_gone());
        // Not "changed": reloading would empty the buffer under the user's hands,
        // which is why a vanished file gets its own state.
        assert!(!d.changed_on_disk(), "a deleted file must not look like an edit");

        // An untitled buffer, and a path that never existed, are not "gone" - there
        // is nothing to have lost.
        let mut untitled = Doc::new("", None);
        assert!(!untitled.refresh_file_gone());
        let mut never = Doc::new("", Some(dir.join("never-existed.md")));
        assert!(!never.refresh_file_gone(), "a file that never existed is not deleted");

        // Saving puts the file back, so the mark goes with it.
        d.note_disk();
        std::fs::write(&path, "text\n").unwrap();
        d.note_disk();
        assert!(!d.refresh_file_gone(), "a re-created file is still marked as gone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn line_math() {
        let d = Doc::new("one\ntwo\nthree", None);
        assert_eq!(d.len_lines(), 3);
        assert_eq!(d.line_text(1), "two");
        assert_eq!(d.line_start(1), 4);
        assert_eq!(d.line_of_char(5), 1);
        assert_eq!(d.line_len(0), 3);
    }

    #[test]
    fn insert_and_remove() {
        let mut d = Doc::new("abc", None);
        let c = d.insert(3, "def");
        assert_eq!(c, 6);
        assert_eq!(d.text(), "abcdef");
        d.remove(1, 3);
        assert_eq!(d.text(), "adef");
    }

    #[test]
    fn undo_redo_roundtrip() {
        let mut d = Doc::new("hello", None);
        d.insert_atomic(5, " world");
        assert_eq!(d.text(), "hello world");
        d.undo().unwrap();
        assert_eq!(d.text(), "hello");
        d.redo().unwrap();
        assert_eq!(d.text(), "hello world");
    }

    #[test]
    fn typing_coalesces_into_one_undo_step() {
        let mut d = Doc::new("", None);
        let mut caret = 0;
        for c in ["h", "e", "l", "l", "o"] {
            caret = d.insert(caret, c);
        }
        assert_eq!(d.text(), "hello");
        d.undo();
        assert_eq!(d.text(), "", "a typing burst should be a single undo step");
        d.redo();
        assert_eq!(d.text(), "hello");
    }

    #[test]
    fn backspace_coalesces() {
        let mut d = Doc::new("abcdef", None);
        d.remove(5, 6);
        d.remove(4, 5);
        d.remove(3, 4);
        assert_eq!(d.text(), "abc");
        d.undo();
        assert_eq!(d.text(), "abcdef");
    }

    #[test]
    fn unicode_offsets_are_chars() {
        let mut d = Doc::new("Привет", None);
        d.insert(6, " мир");
        assert_eq!(d.text(), "Привет мир");
        assert_eq!(d.len_chars(), 10);
    }

    #[test]
    fn word_bounds_finds_word() {
        let t: Vec<char> = "foo bar baz".chars().collect();
        assert_eq!(word_bounds(&t, 5), (4, 7));
        assert_eq!(word_bounds(&t, 4), (4, 7));
    }
}
