//! Incremental-friendly search & replace over the document text.

use crate::doc::Doc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FindFocus {
    Query,
    Replace,
}

#[derive(Clone, Debug)]
pub struct FindState {
    pub open: bool,
    pub show_replace: bool,
    pub query: String,
    pub replace: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub matches: Vec<(usize, usize)>,
    pub current: Option<usize>,
    pub focus: FindFocus,
    /// Cached document version the matches belong to.
    pub version: u64,
    /// Caret and selection anchor of the query field (char offsets). A text
    /// field you cannot select in is a text field you cannot edit.
    pub query_caret: usize,
    pub query_anchor: usize,
    /// Same for the replace field.
    pub replace_caret: usize,
    pub replace_anchor: usize,
}

impl Default for FindState {
    fn default() -> Self {
        FindState {
            open: false,
            show_replace: false,
            query: String::new(),
            replace: String::new(),
            case_sensitive: false,
            whole_word: false,
            matches: Vec::new(),
            current: None,
            focus: FindFocus::Query,
            version: u64::MAX,
            query_caret: 0,
            query_anchor: 0,
            replace_caret: 0,
            replace_anchor: 0,
        }
    }
}

impl FindState {
    // ---- the two text fields ----

    /// The text of a field.
    pub fn field(&self, f: FindFocus) -> &String {
        match f {
            FindFocus::Query => &self.query,
            FindFocus::Replace => &self.replace,
        }
    }

    /// Caret and anchor of a field.
    pub fn caret(&self, f: FindFocus) -> (usize, usize) {
        match f {
            FindFocus::Query => (self.query_caret, self.query_anchor),
            FindFocus::Replace => (self.replace_caret, self.replace_anchor),
        }
    }

    fn set_caret_raw(&mut self, f: FindFocus, caret: usize, anchor: usize) {
        match f {
            FindFocus::Query => {
                self.query_caret = caret;
                self.query_anchor = anchor;
            }
            FindFocus::Replace => {
                self.replace_caret = caret;
                self.replace_anchor = anchor;
            }
        }
    }

    /// Selected char range of a field, `None` when it is a bare caret.
    pub fn selection(&self, f: FindFocus) -> Option<(usize, usize)> {
        let (caret, anchor) = self.caret(f);
        if caret == anchor {
            None
        } else {
            Some((caret.min(anchor), caret.max(anchor)))
        }
    }

    /// Place the caret (clearing the selection unless `extend`).
    pub fn place_caret(&mut self, f: FindFocus, pos: usize, extend: bool) {
        let len = self.field(f).chars().count();
        let pos = pos.min(len);
        let anchor = if extend { self.caret(f).1 } else { pos };
        self.set_caret_raw(f, pos, anchor);
    }

    /// Move the caret by `delta` chars (`extend` keeps the selection growing).
    pub fn move_caret(&mut self, f: FindFocus, delta: i32, extend: bool) {
        let len = self.field(f).chars().count() as i32;
        let (caret, _) = self.caret(f);
        let pos = (caret as i32 + delta).clamp(0, len) as usize;
        self.place_caret(f, pos, extend);
    }

    /// Select the whole field.
    pub fn select_all(&mut self, f: FindFocus) {
        let len = self.field(f).chars().count();
        self.set_caret_raw(f, len, 0);
    }

    /// Insert text at the caret, replacing the selection. Returns true when the
    /// query changed (so the match list must be recomputed).
    pub fn insert(&mut self, f: FindFocus, text: &str) -> bool {
        let (a, b) = self.selection(f).unwrap_or_else(|| {
            let c = self.caret(f).0;
            (c, c)
        });
        let mut chars: Vec<char> = self.field(f).chars().collect();
        let ins: Vec<char> = text.chars().collect();
        let n = ins.len();
        chars.splice(a..b, ins);
        let caret = a + n;
        match f {
            FindFocus::Query => {
                self.query = chars.into_iter().collect();
                self.version = u64::MAX;
                self.current = None;
            }
            FindFocus::Replace => self.replace = chars.into_iter().collect(),
        }
        self.set_caret_raw(f, caret, caret);
        true
    }

    /// Backspace: delete the selection, or the char before the caret.
    pub fn backspace(&mut self, f: FindFocus) -> bool {
        if let Some((a, b)) = self.selection(f) {
            return self.delete_range(f, a, b);
        }
        let c = self.caret(f).0;
        if c == 0 {
            return false;
        }
        self.delete_range(f, c - 1, c)
    }

    /// Delete forward: the selection, or the char after the caret.
    pub fn delete_forward(&mut self, f: FindFocus) -> bool {
        if let Some((a, b)) = self.selection(f) {
            return self.delete_range(f, a, b);
        }
        let c = self.caret(f).0;
        self.delete_range(f, c, c + 1)
    }

    fn delete_range(&mut self, f: FindFocus, a: usize, b: usize) -> bool {
        let mut chars: Vec<char> = self.field(f).chars().collect();
        if a >= chars.len() || a >= b {
            return false;
        }
        let b = b.min(chars.len());
        chars.drain(a..b);
        match f {
            FindFocus::Query => {
                self.query = chars.into_iter().collect();
                self.version = u64::MAX;
                self.current = None;
            }
            FindFocus::Replace => self.replace = chars.into_iter().collect(),
        }
        self.set_caret_raw(f, a, a);
        true
    }

    /// The text as `&str` for drawing.
    pub fn field_str(&self, f: FindFocus) -> &str {
        self.field(f).as_str()
    }

    pub fn ensure_matches(&mut self, doc: &Doc) {
        if self.version == doc.version {
            return;
        }
        self.matches = find_all(&doc.text(), &self.query, self.case_sensitive, self.whole_word);
        self.version = doc.version;
        if self.matches.is_empty() {
            self.current = None;
        } else if let Some(c) = self.current {
            if c >= self.matches.len() {
                self.current = Some(self.matches.len() - 1);
            }
        }
    }

    /// Picks the first match at or after the caret, wrapping around.
    pub fn select_at_or_after(&mut self, caret: usize) {
        if self.matches.is_empty() {
            self.current = None;
            return;
        }
        let idx = self
            .matches
            .iter()
            .position(|&(s, e)| caret <= s || (caret >= s && caret < e))
            .unwrap_or(0);
        self.current = Some(idx);
    }

    pub fn step(&mut self, forward: bool) -> Option<(usize, usize)> {
        if self.matches.is_empty() {
            return None;
        }
        let n = self.matches.len();
        let cur = self.current.unwrap_or(0);
        let next = if forward { (cur + 1) % n } else { (cur + n - 1) % n };
        self.current = Some(next);
        Some(self.matches[next])
    }

    pub fn current_match(&self) -> Option<(usize, usize)> {
        self.current.and_then(|i| self.matches.get(i).copied())
    }
}

fn matches_at(text: &[char], needle: &[char], case: bool) -> bool {
    if needle.len() > text.len() || needle.is_empty() {
        return false;
    }
    for (i, &nc) in needle.iter().enumerate() {
        let hc = text[i];
        let (a, b) = if case {
            (hc, nc)
        } else {
            (hc.to_lowercase().next().unwrap_or(hc), nc.to_lowercase().next().unwrap_or(nc))
        };
        if a != b {
            return false;
        }
    }
    true
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// All non-overlapping matches of `needle` as char ranges.
pub fn find_all(haystack: &str, needle: &str, case: bool, whole_word: bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if needle.is_empty() || haystack.is_empty() {
        return out;
    }
    let hay: Vec<char> = haystack.chars().collect();
    let pat: Vec<char> = needle.chars().collect();
    let mut i = 0usize;
    while i + pat.len() <= hay.len() {
        if matches_at(&hay[i..], &pat, case) {
            let start_ok = !whole_word || i == 0 || !is_word(hay[i - 1]);
            let end = i + pat.len();
            let end_ok = !whole_word || end >= hay.len() || !is_word(hay[end]);
            if start_ok && end_ok {
                out.push((i, end));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Replace every match in the document; returns how many were replaced.
pub fn replace_all(doc: &mut Doc, needle: &str, replacement: &str, case: bool, whole_word: bool) -> usize {
    let matches = find_all(&doc.text(), needle, case, whole_word);
    if matches.is_empty() {
        return 0;
    }
    // Replace from the end so earlier offsets stay valid; all in one undo step.
    doc.begin_compound();
    for &(s, e) in matches.iter().rev() {
        doc.replace_range(s, e, replacement);
    }
    doc.end_compound();
    matches.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_all_occurrences() {
        let m = find_all("foo bar foo foobar", "foo", true, false);
        assert_eq!(m, vec![(0, 3), (8, 11), (12, 15)]);
    }

    #[test]
    fn case_insensitive() {
        let m = find_all("Foo fOO", "foo", false, false);
        assert_eq!(m.len(), 2);
        let m = find_all("Foo fOO", "foo", true, false);
        assert_eq!(m.len(), 0);
    }

    #[test]
    fn whole_word_only() {
        let m = find_all("cat category cat", "cat", true, true);
        assert_eq!(m, vec![(0, 3), (13, 16)]);
    }

    #[test]
    fn replace_all_rewrites_document() {
        let mut d = Doc::new("a cat and a cat", None);
        let n = replace_all(&mut d, "cat", "dog", true, true);
        assert_eq!(n, 2);
        assert_eq!(d.text(), "a dog and a dog");
        d.undo();
        assert_eq!(d.text(), "a cat and a cat");
    }

    #[test]
    fn navigation_wraps() {
        let mut f = FindState { query: "x".into(), ..Default::default() };
        f.matches = vec![(0, 1), (5, 6), (9, 10)];
        f.current = Some(2);
        assert_eq!(f.step(true), Some((0, 1)));
        assert_eq!(f.step(false), Some((9, 10)));
    }

    #[test]
    fn selections_follow_caret() {
        let mut f = FindState::default();
        f.matches = vec![(2, 4), (10, 12)];
        f.select_at_or_after(5);
        assert_eq!(f.current, Some(1));
        f.select_at_or_after(0);
        assert_eq!(f.current, Some(0));
    }
}
