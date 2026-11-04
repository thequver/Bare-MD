//! Directional (right-to-left) text, as far as a line-at-a-time editor needs it.
//!
//! A line of Markdown is one paragraph, so the Unicode Bidirectional Algorithm
//! (UAX #9) is applied per line: [`analyse`] gives every character a level and
//! the paragraph its base direction, and [`visual_order`] turns the levels into
//! the order the pieces have to be *painted* in.
//!
//! Everything here is a pure function over characters: the layout decides what
//! to do with the answer (split a run at a level change, reverse a sequence,
//! mirror the caret). Keeping it separate means the tricky part - the numbers -
//! is testable on its own, without a window or a font.
//!
//! Levels are the UBA's own numbers: even = left-to-right, odd = right-to-left,
//! and a higher number is "more deeply nested". The paragraph's level is 0 for a
//! left-to-right paragraph and 1 for a right-to-left one, decided by the first
//! strong character (a Latin letter, an Arabic letter, …) - the same rule a
//! browser applies to `<p dir="auto">`.

/// Levels of one line, plus the direction of the paragraph itself.
#[derive(Clone, Debug, Default)]
pub struct Levels {
    /// 0 for a left-to-right paragraph, 1 for a right-to-left one.
    pub base: u8,
    /// One entry per character, in logical order.
    pub chars: Vec<u8>,
    /// True when at least one character is right-to-left. Lines without any RTL
    /// character take the plain left-to-right path through the whole layout.
    pub has_rtl: bool,
}

impl Levels {
    /// Is the paragraph itself right-to-left (its first strong character is)?
    pub fn rtl_para(&self) -> bool {
        self.base % 2 == 1
    }

    /// Level of one character, or the paragraph level past the end.
    pub fn at(&self, i: usize) -> u8 {
        self.chars.get(i).copied().unwrap_or(self.base)
    }

    /// Level of the character before `i`, used for the position *between* two
    /// characters, where the UBA takes the higher of the two levels.
    pub fn before(&self, i: usize) -> u8 {
        if i == 0 {
            self.base
        } else {
            self.at(i - 1)
        }
    }

    /// True when the position before character `i` (i.e. a caret there) sits in
    /// right-to-left context, so the caret has to be placed from the right edge
    /// of its run rather than the left.
    pub fn rtl_at_boundary(&self, i: usize) -> bool {
        if i >= self.chars.len() {
            return self.rtl_para();
        }
        // The boundary between two characters belongs to the higher level
        // (UAX #9, X10 / the "boundary type" rule): `אa` has a caret between the
        // two that belongs to the Hebrew side, so typing Hebrew before a Latin
        // word puts the new letter next to the Hebrew run.
        let l = self.before(i).max(self.at(i));
        l % 2 == 1
    }
}

/// Cheap test for a right-to-left character, used to skip the whole analysis on
/// the overwhelming majority of lines.
///
/// This covers the Hebrew and Arabic blocks (including their presentation forms
/// and the historic scripts), the RTL marks, and the RTL planes. It is only a
/// gate: whether a line really needs the UBA is decided by [`analyse`], which
/// looks at the levels it computes.
pub fn is_rtl_char(c: char) -> bool {
    matches!(c as u32,
        0x0590..=0x05FF      // Hebrew
        | 0x0600..=0x06FF    // Arabic
        | 0x0700..=0x074F    // Syriac
        | 0x0750..=0x077F    // Arabic Supplement
        | 0x0780..=0x07BF    // Thaana
        | 0x07C0..=0x07FF    // NKo
        | 0x0800..=0x083F    // Samaritan
        | 0x0840..=0x085F    // Mandaic
        | 0x0860..=0x08FF    // Syriac Supplement, Arabic Extended-A
        | 0xFB1D..=0xFB4F    // Hebrew presentation forms
        | 0xFB50..=0xFDFF    // Arabic presentation forms A
        | 0xFE70..=0xFEFF    // Arabic presentation forms B
        | 0x10800..=0x10FFF  // Cypriot, Phoenician, Kharoshthi, Avestan, …
        | 0x1E800..=0x1EFFF  // Mende Kikakui, Adlam, Arabic mathematical
    )
}

/// True when the line contains anything that could need bidirectional layout.
pub fn may_be_rtl(chars: &[char]) -> bool {
    chars.iter().copied().any(is_rtl_char)
}

/// Apply UAX #9 to one line.
pub fn analyse(chars: &[char]) -> Levels {
    analyse_with_base(chars, None)
}

/// Apply UAX #9 with the paragraph direction already decided.
///
/// The layout needs this because the *first strong character of the line* is not
/// always the first strong character the reader sees: `- [x] משימה` starts with a
/// Latin `x` inside a task marker, which would make a Hebrew line a left-to-right
/// paragraph, and a link's URL (`[שלום](https://…)`) is full of Latin letters that
/// are hidden from the reader. The caller passes the direction of the **visible**
/// text; `None` means "no strong character, let the algorithm decide".
pub fn analyse_with_base(chars: &[char], base: Option<u8>) -> Levels {
    let text: String = chars.iter().collect();
    let forced = match base {
        Some(0) => Some(unicode_bidi::Level::ltr()),
        Some(_) => Some(unicode_bidi::Level::rtl()),
        None => None,
    };
    let info = unicode_bidi::BidiInfo::new(&text, forced);
    let Some(para) = info.paragraphs.first() else {
        return Levels { base: 0, chars: vec![0; chars.len()], has_rtl: false };
    };
    // `levels` is indexed by byte; the layout counts characters.
    let mut levels = Vec::with_capacity(chars.len());
    let mut byte = 0usize;
    for c in chars {
        let l = info.levels.get(byte).copied().unwrap_or(para.level);
        levels.push(l.number());
        byte += c.len_utf8();
    }
    let base = para.level.number();
    let has_rtl = levels.iter().any(|l| l % 2 == 1) || base % 2 == 1;
    Levels { base, chars: levels, has_rtl }
}

/// The order to paint pieces in, given their levels in logical order.
///
/// This is the UBA's rule L2: from the highest level down to the lowest odd
/// level, reverse every contiguous sequence of pieces at that level or deeper.
/// A run has to have a single level (the layout splits at level changes), which
/// is why the input here is one level per piece.
///
/// Returns indices into `levels`, in visual left-to-right order.
pub fn visual_order(levels: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    let Some(max) = levels.iter().copied().max() else { return order };
    let Some(lowest_odd) = levels.iter().copied().filter(|l| l % 2 == 1).min() else {
        return order;
    };
    let mut level = max;
    loop {
        let mut i = 0;
        while i < order.len() {
            if levels[order[i]] >= level {
                let start = i;
                while i < order.len() && levels[order[i]] >= level {
                    i += 1;
                }
                if i - start > 1 {
                    order[start..i].reverse();
                }
            } else {
                i += 1;
            }
        }
        if level <= lowest_odd {
            break;
        }
        level -= 1;
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn latin_is_one_left_to_right_run() {
        let l = analyse(&chars("hello world"));
        assert_eq!(l.base, 0);
        assert!(!l.has_rtl);
        assert!(l.chars.iter().all(|&x| x == 0));
        assert_eq!(visual_order(&l.chars), (0..11).collect::<Vec<_>>());
    }

    #[test]
    fn hebrew_is_right_to_left_and_reversed() {
        // "שלום" - four Hebrew letters, one RTL run.
        let l = analyse(&chars("שלום"));
        assert_eq!(l.base, 1, "first strong character is Hebrew");
        assert!(l.has_rtl);
        assert!(l.chars.iter().all(|&x| x % 2 == 1));
        assert_eq!(visual_order(&l.chars), vec![3, 2, 1, 0]);
    }

    #[test]
    fn a_paragraph_of_digits_stays_left_to_right() {
        // No strong character at all: the paragraph level falls back to LTR.
        let l = analyse(&chars("123 456"));
        assert_eq!(l.base, 0);
        assert!(!l.has_rtl);
    }

    #[test]
    fn a_number_inside_hebrew_keeps_its_digits_in_order() {
        // "abc" in Hebrew letters around a number: the number keeps level 2
        // (left-to-right) inside the level-1 paragraph, so its digits stay in
        // reading order while the words around it are reversed.
        let text = "אב 123 גד";
        let l = analyse(&chars(text));
        assert_eq!(l.base, 1);
        let n = text.chars().count();
        let order = visual_order(&l.chars);
        assert_eq!(order.len(), n);
        // The digits must appear in ascending logical order inside the visual line.
        let digit_idx: Vec<usize> = text
            .chars()
            .enumerate()
            .filter(|(_, c)| c.is_ascii_digit())
            .map(|(i, _)| i)
            .collect();
        let mut seen: Vec<usize> = order.iter().copied().filter(|i| digit_idx.contains(i)).collect();
        assert_eq!(seen, digit_idx, "digits were reversed with the text around them");
        seen.clear();
    }

    #[test]
    fn a_latin_word_inside_hebrew_stays_left_to_right() {
        let text = "שלום world";
        let l = analyse(&chars(text));
        assert_eq!(l.base, 1);
        let order = visual_order(&l.chars);
        // The Latin letters keep their own order among themselves.
        let latin: Vec<usize> = text
            .chars()
            .enumerate()
            .filter(|(_, c)| c.is_ascii_alphabetic())
            .map(|(i, _)| i)
            .collect();
        let seen: Vec<usize> = order.iter().copied().filter(|i| latin.contains(i)).collect();
        assert_eq!(seen, latin, "the Latin word was reversed");
    }

    #[test]
    fn a_caret_at_a_boundary_follows_the_text_before_it() {
        let text = "abc שלום";
        let l = analyse(&chars(text));
        // Index 4 is the space before the Hebrew word: caret there is in RTL
        // context (the Hebrew to its right wins), so it is placed from the right.
        assert!(l.rtl_at_boundary(4));
        // Index 1, inside the Latin word, is not.
        assert!(!l.rtl_at_boundary(1));
    }

    #[test]
    fn reordering_leaves_a_plain_ltr_line_alone() {
        assert_eq!(visual_order(&[0, 0, 0]), vec![0, 1, 2]);
        assert_eq!(visual_order(&[]), Vec::<usize>::new());
    }

    #[test]
    fn an_rtl_run_inside_ltr_text_is_reversed_in_place() {
        // LTR LTR [RTL RTL RTL] LTR: only the RTL stretch reverses.
        let levels = [0u8, 0, 1, 1, 1, 0];
        assert_eq!(visual_order(&levels), vec![0, 1, 4, 3, 2, 5]);
    }
}
