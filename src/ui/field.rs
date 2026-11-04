//! One-line text fields.
//!
//! Every editable single-line control in the app - the two find fields, the colour
//! dialog's hex box, the folder dialog's address bar and file name - is a
//! [`TextField`]. They used to be raw `String`s with a `push_str` bolted onto each,
//! which is why the same class of bug kept coming back in a new place: a field you
//! cannot put a caret in, cannot select in, cannot delete backwards from and cannot
//! type into once it is full is not a text field at all.
//!
//! Keeping the editing rules in one type means a fix lands everywhere at once.

use crate::text::{Face, TextEngine};

/// Which characters a field takes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CharFilter {
    /// Anything printable.
    Any,
    /// Hex digits only, stored lower-case.
    Hex,
}

/// A single-line text field: the text, a caret and a selection anchor.
#[derive(Clone, Debug)]
pub struct TextField {
    pub text: String,
    /// Char offset of the caret.
    pub caret: usize,
    /// Char offset of the other end of the selection (`== caret` when none).
    pub anchor: usize,
    /// Longest accepted length, in chars.
    pub max_len: Option<usize>,
    pub filter: CharFilter,
}

impl Default for TextField {
    fn default() -> Self {
        TextField {
            text: String::new(),
            caret: 0,
            anchor: 0,
            max_len: None,
            filter: CharFilter::Any,
        }
    }
}

impl TextField {
    pub fn new() -> TextField {
        TextField::default()
    }

    /// A field with a filter and a length limit.
    pub fn limited(filter: CharFilter, max_len: usize) -> TextField {
        TextField { max_len: Some(max_len), filter, ..TextField::default() }
    }

    /// Replace the text, leaving the caret at the end (what a programmatic update
    /// - "this is the folder you are in now" - should do).
    pub fn set(&mut self, text: &str) {
        self.text.clear();
        self.text.push_str(text);
        self.caret = self.len();
        self.anchor = self.caret;
    }

    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Selected char range, `None` for a bare caret.
    pub fn selection(&self) -> Option<(usize, usize)> {
        if self.caret == self.anchor {
            None
        } else {
            Some((self.caret.min(self.anchor), self.caret.max(self.anchor)))
        }
    }

    /// Put the caret at `pos`, dropping the selection unless `extend`.
    pub fn place_caret(&mut self, pos: usize, extend: bool) {
        let pos = pos.min(self.len());
        self.caret = pos;
        if !extend {
            self.anchor = pos;
        }
    }

    pub fn move_caret(&mut self, delta: i32, extend: bool) {
        let pos = (self.caret as i32 + delta).clamp(0, self.len() as i32) as usize;
        self.place_caret(pos, extend);
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.len();
    }

    /// True when text can still be added (a full field refuses more, but its
    /// contents stay editable: that is the difference between "full" and "locked").
    fn accepts(&self, c: char) -> bool {
        match self.filter {
            CharFilter::Any => !c.is_control(),
            CharFilter::Hex => c.is_ascii_hexdigit(),
        }
    }

    /// Delete the selection, if any. Returns whether anything was removed.
    fn delete_selection(&mut self) -> bool {
        let Some((a, b)) = self.selection() else { return false };
        let chars: Vec<char> = self.text.chars().collect();
        let kept: String = chars[..a].iter().chain(chars[b..].iter()).collect();
        self.text = kept;
        self.caret = a;
        self.anchor = a;
        true
    }

    /// Insert at the caret, replacing the selection. Returns whether the text
    /// changed.
    pub fn insert(&mut self, s: &str) -> bool {
        let mut add: String = s.chars().filter(|c| self.accepts(*c)).collect();
        if self.filter == CharFilter::Hex {
            add = add.to_lowercase();
        }
        if add.is_empty() {
            return false;
        }
        self.delete_selection();
        // A field with a fixed length (the hex box) is a value, not a line: once it
        // is full, typing starts a new value instead of being swallowed. This is
        // the difference between a field you can edit and one that locks up after
        // six characters, which is exactly what the hex box used to do.
        if let Some(max) = self.max_len {
            if self.len() >= max && self.selection().is_none() {
                self.text.clear();
                self.caret = 0;
                self.anchor = 0;
            }
        }
        let chars: Vec<char> = self.text.chars().collect();
        let room = self.max_len.map(|m| m.saturating_sub(chars.len())).unwrap_or(usize::MAX);
        if room == 0 {
            return false;
        }
        let take: String = add.chars().take(room).collect();
        let n = take.chars().count();
        let mut out: String = chars[..self.caret].iter().collect();
        out.push_str(&take);
        out.extend(chars[self.caret..].iter());
        self.text = out;
        self.caret += n;
        self.anchor = self.caret;
        true
    }

    pub fn backspace(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        if self.caret == 0 {
            return false;
        }
        let chars: Vec<char> = self.text.chars().collect();
        let i = self.caret - 1;
        let out: String = chars[..i].iter().chain(chars[i + 1..].iter()).collect();
        self.text = out;
        self.caret = i;
        self.anchor = i;
        true
    }

    pub fn delete_forward(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        let len = self.len();
        if self.caret >= len {
            return false;
        }
        let chars: Vec<char> = self.text.chars().collect();
        let i = self.caret;
        let out: String = chars[..i].iter().chain(chars[i + 1..].iter()).collect();
        self.text = out;
        true
    }

    pub fn home(&mut self, extend: bool) {
        self.place_caret(0, extend);
    }

    pub fn end(&mut self, extend: bool) {
        self.place_caret(self.len(), extend);
    }

    /// Caret position for a click `x` px into the field's text, measured from the
    /// field's inner left edge.
    pub fn index_at(&self, te: &mut TextEngine, face: Face, size: f32, x: f32) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut acc = 0.0f32;
        for (i, c) in chars.iter().enumerate() {
            let w = te.width(face, &c.to_string(), size);
            if x < acc + w * 0.5 {
                return i;
            }
            acc += w;
        }
        chars.len()
    }
}
