//! Everything that knows about Markdown.
//!
//! ```text
//! mod.rs        the block + inline parser: what a line *is*, and which characters
//!               are syntax markers to be hidden while writing
//! layout.rs     the line layout: runs, wrapping, tables, pictures, the caret map
//! doc.rs        the document: text, undo, line indexes, disk identity
//! search.rs     find and replace over a document
//! highlight.rs  the syntax tokenizer for fenced code
//! mermaid.rs    flowchart parsing and layout
//! media.rs      image probing and decoding (PNG/JPEG/GIF/WebP/JXL)
//! ```
//!
//! The rule the whole layer follows: the source text is the truth, and everything
//! here is derived from it. Nothing is stored twice, so a command never has to
//! keep two representations in step.

pub mod doc;
pub mod highlight;
pub mod layout;
pub mod media;
pub mod mermaid;
pub mod search;
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Block {
    Blank,
    Paragraph,
    Heading(u8),
    Bullet,
    Ordered,
    Task(bool),
    Quote,
    Code,
    Fence,
    Hr,
    Table,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Inline {
    Bold,
    Italic,
    Strike,
    Code,
    Link,
    Underline,
    Highlight,
    Sup,
    Sub,
    /// `[#rrggbb]text[/]` - coloured text.
    Fg,
    /// `[bg:#rrggbb]text[/]` - coloured background.
    Bg,
}

#[derive(Clone, Copy, Debug)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: Inline,
    /// Colour carried by `Fg` / `Bg` spans, as `0xRRGGBB`.
    pub color: u32,
}

impl Default for Span {
    fn default() -> Self {
        Span { start: 0, end: 0, kind: Inline::Bold, color: 0 }
    }
}

#[derive(Clone, Debug, Default)]
pub struct LineInfo {
    pub block: Block,
    pub depth: usize,
    /// Number of leading chars used by the block marker.
    pub prefix: usize,
    /// Char ranges that are syntax markers (hidden in the rendered view).
    pub hides: Vec<(usize, usize)>,
    pub spans: Vec<Span>,
    pub lang: String,
    /// Cell char ranges for table rows (trimmed, excludes the pipes).
    pub cells: Vec<(usize, usize)>,
    /// Char indices of the `|` pipes of a table row.
    ///
    /// They are hidden like every other marker, but they are *structural*: the
    /// grid is drawn instead of them, so a table row keeps them hidden even while
    /// it is being edited - otherwise the whole row would flip to `| a | b |`
    /// source under the caret. Keeping them in their own list is what lets a
    /// cell's inline markers (`**bold**`, `[link](url)`) be revealed in the
    /// rendered view *without* dragging the pipes along with them.
    pub pipes: Vec<usize>,
    /// True for the `| --- | --- |` alignment row of a table.
    pub table_sep: bool,
    /// Text alignment of this line: 0 left, 1 center, 2 right, 3 justify.
    pub align: u8,
    /// Set when the line is nothing but an image: `![alt](path)`, optionally
    /// with a title. The layout renders a picture instead of text.
    pub image: Option<ImageRef>,
    /// The source of an ordered list item's marker, trimmed: `"1."`, `"12)"`.
    ///
    /// The rendered list shows the number the writer wrote. Deriving it from the
    /// item's position in the rendered run instead made a list that starts at `3.`
    /// draw "1." - the number ran backwards against the document.
    pub marker: String,
    /// Set when the line is a quote that holds a **list item**: `> * one`,
    /// `> - one`. The item's marker is hidden with the rest of the syntax, and the
    /// painter draws a bullet in the quote's gutter, so quoting a list keeps it a
    /// list instead of turning every item into a paragraph that happens to start
    /// with a star.
    pub quote_bullet: bool,
}

/// A standalone `![alt](src "title")` line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImageRef {
    pub alt: String,
    pub src: String,
    pub title: String,
}

impl Default for Block {
    fn default() -> Self {
        Block::Paragraph
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Flags {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: bool,
    pub underline: bool,
    pub highlight: bool,
    pub sup: bool,
    pub sub: bool,
    pub fg: Option<u32>,
    pub bg: Option<u32>,
}

fn indent_of(chars: &[char]) -> usize {
    let mut n = 0;
    for &c in chars {
        if c == ' ' {
            n += 1;
        } else if c == '\t' {
            n += 4;
        } else {
            break;
        }
    }
    n
}

fn is_hr(chars: &[char]) -> bool {
    let mut count = 0;
    let mut ch = None;
    for &c in chars {
        if c == ' ' || c == '\t' {
            continue;
        }
        if c == '-' || c == '*' || c == '_' {
            match ch {
                None => ch = Some(c),
                Some(prev) if prev == c => {}
                _ => return false,
            }
            count += 1;
        } else {
            return false;
        }
    }
    count >= 3
}

/// Length of the backtick/tilde run at the start of `chars`, when it is at least
/// three characters (a fence marker). Returns `None` otherwise.
fn fence_run(chars: &[char]) -> Option<usize> {
    let first = *chars.first()?;
    if first != '`' && first != '~' {
        return None;
    }
    let n = chars.iter().take_while(|c| **c == first).count();
    if n >= 3 {
        Some(n)
    } else {
        None
    }
}

/// Index of a closing fence run (>=3 of the same char) in `chars`, when the rest
/// of the line after it is blank — that makes the line a self-contained one-line
/// fenced block: ```` ```code``` ````.
fn closing_run(chars: &[char]) -> Option<usize> {
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' || c == '~' {
            let n = chars[i..].iter().take_while(|x| **x == c).count();
            if n >= 3 && chars[i + n..].iter().all(|x| x.is_whitespace()) {
                return Some(i);
            }
            i += n;
        } else {
            i += 1;
        }
    }
    None
}

/// Does this (already de-indented) slice open a block that may be nested?
///
/// Used to tell an indented code block apart from an indented list item, which
/// is the whole difference between "deeper nesting" and "a grey code card".
fn starts_a_block(chars: &[char]) -> bool {
    match chars.first() {
        Some('>') => true,
        Some('-') | Some('*') | Some('+') => chars.get(1).map(|c| *c == ' ').unwrap_or(true),
        Some(c) if c.is_ascii_digit() => {
            let mut j = 0;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            matches!(chars.get(j), Some('.') | Some(')'))
        }
        _ => false,
    }
}

/// Parse one line. `fenced` carries the code-fence state across lines: when it is
/// `Some(lang)` the line is inside a fenced code block.
/// Nesting deeper than this is flattened: at 26 px per level a list would
/// otherwise walk off the right edge of the page.
pub const MAX_LIST_DEPTH: usize = 6;

pub fn parse(chars: &[char], fenced: &mut Option<String>) -> LineInfo {
    let len = chars.len();
    let mut info = LineInfo { block: Block::Paragraph, ..Default::default() };

    let indent = indent_of(chars);
    let mut i = 0;
    while i < len && chars[i] == ' ' {
        i += 1;
    }

    // ---- inside a fenced code block ----
    // Checked *before* the blank test so empty lines stay part of the block.
    if let Some(lang) = fenced.clone() {
        if let Some(n) = fence_run(&chars[i..]) {
            if chars[i + n..].iter().all(|c| c.is_whitespace()) {
                *fenced = None;
                info.block = Block::Fence;
                info.lang = lang;
                info.prefix = i + n;
                info.hides.push((0, len));
                return info;
            }
        }
        info.block = Block::Code;
        info.lang = lang;
        info.prefix = 0;
        return info;
    }

    if chars.iter().all(|c| c.is_whitespace()) {
        info.block = Block::Blank;
        return info;
    }

    // Deep indentation is only an indented code block when the line does not
    // start a block of its own: a nested list item is written as
    // `    - item` (two spaces per level), and treating that as code is what
    // turned every second `Tab` press into a grey code card.
    if i > 3 && !starts_a_block(&chars[i..]) {
        info.block = Block::Code;
        info.prefix = 0;
        return info;
    }

    // ---- fence markers / one-line fenced code ----
    if let Some(n) = fence_run(&chars[i..]) {
        let open_end = (i + n).min(len);
        if let Some(close_at) = closing_run(&chars[open_end..]) {
            let close_start = open_end + close_at;
            // ```code``` - a complete block on a single line.
            info.block = Block::Code;
            info.prefix = 0;
            info.hides.push((0, open_end));
            info.hides.push((close_start, len));
            parse_inline(chars, open_end, close_start, &mut info, 0);
            return info;
        }
        let lang: String = chars[open_end..].iter().collect::<String>().trim().to_string();
        *fenced = Some(lang.clone());
        info.block = Block::Fence;
        info.lang = lang;
        info.prefix = open_end;
        info.hides.push((0, open_end));
        return info;
    }

    // ---- headings ----
    if chars.get(i) == Some(&'#') {
        let mut level = 0;
        while i + level < len && chars[i + level] == '#' && level < 6 {
            level += 1;
        }
        let after = i + level;
        if after >= len || chars[after] == ' ' {
            info.block = Block::Heading(level as u8);
            info.prefix = (after + 1).min(len);
            info.hides.push((0, info.prefix));
            parse_inline(chars, info.prefix, len, &mut info, 0);
            return info;
        }
    }

    // ---- thematic break ----
    if is_hr(&chars[i..]) {
        info.block = Block::Hr;
        info.hides.push((0, len));
        return info;
    }

    // ---- block quote ----
    if chars.get(i) == Some(&'>') {
        let mut j = i;
        let mut depth = 0;
        loop {
            if chars.get(j) == Some(&'>') {
                depth += 1;
                j += 1;
                if chars.get(j) == Some(&' ') {
                    j += 1;
                }
            } else {
                break;
            }
            if depth > 8 {
                break;
            }
        }
        info.block = Block::Quote;
        info.depth = depth;
        // A list item inside the quote keeps its bullet: `> * one` is a list, not a
        // paragraph beginning with an asterisk. A task item (`> - [ ] x`) is left
        // as text on purpose - its checkbox is not drawn inside a quote, and hiding
        // the marker would hide the box with it.
        let mut content = j;
        if matches!(chars.get(j), Some('-') | Some('*') | Some('+')) {
            let mut k = j + 1;
            while k < len && chars[k] == ' ' {
                k += 1;
            }
            let is_task = chars.get(k) == Some(&'[')
                && matches!(chars.get(k + 1), Some(' ') | Some('x') | Some('X'))
                && chars.get(k + 2) == Some(&']');
            if (k > j + 1 || k >= len) && !is_task {
                content = k;
                info.quote_bullet = true;
            }
        }
        info.prefix = content;
        info.hides.push((0, content));
        parse_inline(chars, content, len, &mut info, 0);
        return info;
    }

    // ---- lists ----
    let marker_char = chars.get(i).copied();
    if matches!(marker_char, Some('-') | Some('*') | Some('+')) {
        let after = i + 1;
        let mut j = after;
        while j < len && chars[j] == ' ' {
            j += 1;
        }
        let has_space = j > after;
        // Task list: - [ ] / - [x]
        if has_space
            && chars.get(j) == Some(&'[')
            && matches!(chars.get(j + 1), Some(' ') | Some('x') | Some('X'))
            && chars.get(j + 2) == Some(&']')
        {
            let checked = matches!(chars.get(j + 1), Some('x') | Some('X'));
            let mut k = j + 3;
            if chars.get(k) == Some(&' ') {
                k += 1;
            }
            info.block = Block::Task(checked);
            info.depth = (indent / 2).min(MAX_LIST_DEPTH);
            info.prefix = k;
            info.hides.push((0, k));
            parse_inline(chars, k, len, &mut info, 0);
            return info;
        }
        if has_space || after >= len {
            info.block = Block::Bullet;
            info.depth = (indent / 2).min(MAX_LIST_DEPTH);
            info.prefix = j;
            info.hides.push((0, j));
            parse_inline(chars, j, len, &mut info, 0);
            return info;
        }
    }

    if matches!(marker_char, Some(c) if c.is_ascii_digit()) {
        let mut j = i;
        while j < len && chars[j].is_ascii_digit() {
            j += 1;
        }
        if matches!(chars.get(j), Some('.') | Some(')')) {
            let mut k = j + 1;
            while k < len && chars[k] == ' ' {
                k += 1;
            }
            if k > j + 1 {
                // The number the writer typed is the number of the item. The
                // rendered list used to *not* print it at all, so the bullets of an
                // ordered list came out numbered by their position in the rendered
                // block: an item typed as `3.` under `1.` and `2.` was drawn as
                // "3" only by luck, and a list that starts at `1.` after a paragraph
                // was drawn as "1" no matter what was written. `marker` keeps the
                // source text of the marker so the painter can show exactly that.
                info.marker = chars[i..k].iter().collect::<String>().trim_end().to_string();
                info.block = Block::Ordered;
                info.depth = (indent / 2).min(MAX_LIST_DEPTH);
                info.prefix = k;
                // The marker is a marker: hidden in the rendered view, where the
                // number is drawn in the gutter instead (see `draw_document`), and
                // shown again on the line being edited, exactly like `- ` and
                // `- [ ]`. Painting the source `1.` as if it were text put the
                // numbers inside the item's own words and made the list look
                // ragged on one theme and indented on another.
                info.hides.push((0, k));
                parse_inline(chars, k, len, &mut info, 0);
                return info;
            }
        }
    }

    // ---- trailing alignment tag: `text {center}` ----
    if let Some((align, open)) = align_tag(chars) {
        if open > 0 && chars[open - 1] == ' ' {
            info.align = align;
            info.hides.push((open - 1, len));
            parse_inline(chars, i, open - 1, &mut info, 0);
            return info;
        }
    }

    // ---- table row ----
    let trimmed_start = chars[i..].iter().position(|c| !c.is_whitespace()).map(|p| p + i);
    if chars.get(i) == Some(&'|') || trimmed_start.map(|p| chars[p] == '|').unwrap_or(false) {
        info.block = Block::Table;
        for (idx, &c) in chars.iter().enumerate().take(len) {
            if c == '|' {
                info.hides.push((idx, idx + 1));
                info.pipes.push(idx);
            }
        }
        // Split into trimmed cell ranges.
        //
        // Empty cells are kept: the number of columns a table has comes from its
        // pipes, not from the cells that happen to hold text. Skipping them meant
        // a freshly inserted column had no slot, so "+ column" changed the source
        // and left the painted grid exactly as it was.
        let mut cell_start = i;
        let mut sep_ok = true;
        let mut any_dash = false;
        for idx in i..=len {
            let at_end = idx == len;
            let is_pipe = !at_end && chars[idx] == '|';
            if at_end || is_pipe {
                let (mut a, mut b) = (cell_start, idx);
                while a < b && chars[a].is_whitespace() {
                    a += 1;
                }
                while b > a && chars[b - 1].is_whitespace() {
                    b -= 1;
                }
                // The slot before the first pipe and after the last one is the
                // table's edge, not a cell.
                let leading = is_pipe && cell_start == i && a == b;
                let trailing = at_end && a == b;
                if !(leading || trailing) {
                    let cell: String = chars[a..b].iter().collect();
                    if !cell.is_empty() {
                        let is_sep = cell.chars().all(|c| c == '-' || c == ':');
                        if is_sep {
                            any_dash = true;
                        } else {
                            sep_ok = false;
                        }
                    } else {
                        sep_ok = false;
                    }
                    info.cells.push((a, b));
                }
                cell_start = idx + 1;
            } else if !(chars[idx] == '-' || chars[idx] == ':' || chars[idx].is_whitespace()) {
                sep_ok = false;
            }
        }
        info.table_sep = sep_ok && any_dash && !info.cells.is_empty();
        parse_inline(chars, i, len, &mut info, 0);
        return info;
    }

    // ---- paragraph ----
    info.block = Block::Paragraph;
    info.prefix = i;
    parse_inline(chars, i, len, &mut info, 0);
    if let Some(img) = standalone_image(chars, i, len) {
        // A line that is only an image is drawn as the image; its syntax stays
        // hidden until the caret lands on the line, exactly like every other
        // marker.
        info.image = Some(img);
        info.hides.push((i, len));
    }
    info
}

/// `![alt](src)` — alone on a line — with an optional `"title"`.
pub fn standalone_image(chars: &[char], from: usize, to: usize) -> Option<ImageRef> {
    let mut a = from;
    let mut b = to;
    while a < b && chars[a].is_whitespace() {
        a += 1;
    }
    while b > a && chars[b - 1].is_whitespace() {
        b -= 1;
    }
    if b <= a + 4 {
        return None;
    }
    if !(chars[a] == '!' && chars[a + 1] == '[') || chars[b - 1] != ')' {
        return None;
    }
    let text: String = chars[a..b].iter().collect();
    let close_alt = text.find("](")?;
    if !text.ends_with(')') {
        return None;
    }
    let alt = text[2..close_alt].to_string();
    let mut inside = &text[close_alt + 2..text.len() - 1];
    let mut title = String::new();
    if let Some(q) = inside.rfind(" \"") {
        if inside.ends_with('"') && q + 2 <= inside.len() {
            title = inside[q + 2..inside.len() - 1].to_string();
            inside = &inside[..q];
        }
    } else if let Some(q) = inside.rfind(" '") {
        if inside.ends_with('\'') && q + 2 <= inside.len() {
            title = inside[q + 2..inside.len() - 1].to_string();
            inside = &inside[..q];
        }
    }
    let src = inside.trim().to_string();
    if src.is_empty() {
        return None;
    }
    Some(ImageRef { alt, src, title })
}

fn find_from(chars: &[char], from: usize, to: usize, needle: &[char]) -> Option<usize> {
    if needle.is_empty() || to < needle.len() {
        return None;
    }
    let mut i = from;
    while i + needle.len() <= to {
        if chars[i..i + needle.len()] == *needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Parses inline markup in `chars[from..to]`, appending spans and marker ranges.
/// Parse `#rrggbb` (with or without a leading `bg:`) out of `[..]` at `i`.
/// Every link in one line of source, as `(label_start, label_end, target)` in
/// **char** offsets. The target is the text between the parentheses, exactly as
/// written (no escaping, no resolution against the document's folder).
///
/// The layout only keeps the *span* of a link (so it can be underlined and
/// tinted); clicking needs the destination, and re-reading the line is cheaper
/// than threading the target through `LineInfo` and every run.
pub fn link_targets(chars: &[char]) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let n = chars.len();
    while i < n {
        if chars[i] != '[' {
            i += 1;
            continue;
        }
        let Some(close) = find_from(chars, i + 1, n, &[']']) else {
            i += 1;
            continue;
        };
        if chars.get(close + 1) != Some(&'(') {
            i = close + 1;
            continue;
        }
        let Some(paren) = matching_paren(chars, close + 1, n) else {
            i = close + 1;
            continue;
        };
        let raw: String = chars[close + 2..paren].iter().collect();
        let target = link_target_of(&raw);
        // A `![alt](image)` is a picture, not a link: it is drawn in place.
        let image = i > 0 && chars[i - 1] == '!';
        if !image && !target.is_empty() {
            out.push((i + 1, close, target));
        }
        i = paren + 1;
    }
    out
}

/// Returns `(color, bytes consumed, is_background)`.
fn color_tag(chars: &[char], i: usize, _to: usize) -> Option<(u32, usize, bool)> {
    let mut k = i + 1;
    let mut bg = false;
    if chars.get(k..k + 3) == Some(&['b', 'g', ':']) {
        bg = true;
        k += 3;
    }
    if chars.get(k) != Some(&'#') {
        return None;
    }
    k += 1;
    let mut value = 0u32;
    let mut digits = 0usize;
    while digits < 6 {
        let c = *chars.get(k)?;
        let d = c.to_digit(16)?;
        value = value * 16 + d;
        k += 1;
        digits += 1;
    }
    if chars.get(k) != Some(&']') {
        return None;
    }
    Some((value, k + 1 - i, bg))
}

/// Parse `{center}` / `{right}` / `{left}` / `{justify}` at the end of a line.
fn align_tag(chars: &[char]) -> Option<(u8, usize)> {
    let n = chars.len();
    if chars.get(n.wrapping_sub(1)) != Some(&'}') {
        return None;
    }
    let open = chars.iter().rposition(|c| *c == '{')?;
    let word: String = chars[open + 1..n - 1].iter().collect();
    let value = match word.trim().to_lowercase().as_str() {
        "center" | "centre" => 1,
        "right" => 2,
        "justify" => 3,
        "left" => 0,
        _ => return None,
    };
    Some((value, open))
}

/// Scan inline markup (emphasis, code, links, colours) in `chars[from..to]`.
///
/// `pub(crate)` because it is also used on a whole *paragraph*: emphasis that
/// opens on one source line and closes on the next is one span, and only a scan
/// that can see past the newline finds it (see `join_inline` in
/// `markdown/layout.rs`).
pub(crate) fn parse_inline(
    chars: &[char],
    from: usize,
    to: usize,
    info: &mut LineInfo,
    depth: u32,
) {
    if depth > 6 || from >= to {
        return;
    }
    let mut i = from;
    while i < to {
        let c = chars[i];
        // Escapes: skip the escaped char.
        if c == '\\' && i + 1 < to {
            i += 2;
            continue;
        }
        // Inline code
        if c == '`' {
            if let Some(j) = find_from(chars, i + 1, to, &['`']) {
                info.spans.push(Span { start: i + 1, end: j, kind: Inline::Code, color: 0 });
                // An empty pair (` `` `) would vanish completely if its markers
                // were hidden, leaving the user with nothing to see or click.
                if j > i + 1 {
                    info.hides.push((i, i + 1));
                    info.hides.push((j, j + 1));
                }
                i = j + 1;
                continue;
            }
        }
        // Coloured spans: [#rrggbb]text[/] and [bg:#rrggbb]text[/]
        if c == '[' {
            if let Some((color, tag_len, bg)) = color_tag(chars, i, to) {
                let tag_end = i + tag_len;
                // Find the closing `[/]`.
                let mut k = tag_end;
                let mut close = None;
                while k + 2 < to {
                    if chars[k] == '[' && chars[k + 1] == '/' && chars[k + 2] == ']' {
                        close = Some(k);
                        break;
                    }
                    k += 1;
                }
                if let Some(close) = close {
                    if close > tag_end {
                        info.spans.push(Span {
                            start: tag_end,
                            end: close,
                            kind: if bg { Inline::Bg } else { Inline::Fg },
                            color,
                        });
                        info.hides.push((i, tag_end));
                        info.hides.push((close, close + 3));
                        parse_inline(chars, tag_end, close, info, depth + 1);
                        i = close + 3;
                        continue;
                    }
                }
            }
        }
        // Images and links: [text](url) / ![alt](url)
        let (bracket_start, image) = if c == '!' && chars.get(i + 1) == Some(&'[') {
            (i + 1, true)
        } else if c == '[' {
            (i, false)
        } else {
            (usize::MAX, false)
        };
        if bracket_start != usize::MAX {
            if let Some(close) = find_from(chars, bracket_start + 1, to, &[']']) {
                let k = close + 1;
                if chars.get(k) == Some(&'(') {
                    if let Some(paren) = matching_paren(chars, k, to) {
                        info.spans.push(Span {
                            start: bracket_start + 1,
                            end: close,
                            kind: Inline::Link,
                            color: 0,
                        });
                        let hide_start = if image { i } else { i };
                        info.hides.push((hide_start, bracket_start + 1));
                        info.hides.push((close, paren + 1));
                        parse_inline(chars, bracket_start + 1, close, info, depth + 1);
                        i = paren + 1;
                        continue;
                    }
                }
            }
        }
        // Strong / emphasis / strikethrough
        let markers: [(&[char], Inline); 9] = [
            (&['*', '*'], Inline::Bold),
            (&['_', '_'], Inline::Bold),
            (&['~', '~'], Inline::Strike),
            (&['=', '='], Inline::Highlight),
            (&['+', '+'], Inline::Underline),
            (&['*'], Inline::Italic),
            (&['_'], Inline::Italic),
            (&['^'], Inline::Sup),
            (&['~'], Inline::Sub),
        ];
        let mut matched = false;
        for (m, kind) in markers {
            if i + m.len() <= to && chars[i..i + m.len()] == *m {
                // Single markers must not match a doubled one (handled above).
                if m.len() == 1 && chars.get(i + 1) == Some(&m[0]) {
                    continue;
                }
                if let Some(close) = find_from(chars, i + m.len(), to, m) {
                    if close > i + m.len() {
                        // A subscript's closing `~` must not be the first half of a
                        // `~~` striker.
                        if kind == Inline::Sub && chars.get(close + 1) == Some(&'~') {
                            continue;
                        }
                        info.spans.push(Span { start: i + m.len(), end: close, kind, color: 0 });
                        info.hides.push((i, i + m.len()));
                        info.hides.push((close, close + m.len()));
                        parse_inline(chars, i + m.len(), close, info, depth + 1);
                        i = close + m.len();
                        matched = true;
                        break;
                    }
                }
            }
        }
        if matched {
            continue;
        }
        i += 1;
    }
}

/// Per-character inline flags for a line of `len` chars.
pub fn flags(info: &LineInfo, len: usize) -> Vec<Flags> {
    let mut v = vec![Flags::default(); len];
    for span in &info.spans {
        let end = span.end.min(len);
        for f in v.iter_mut().take(end).skip(span.start) {
            match span.kind {
                Inline::Bold => f.bold = true,
                Inline::Italic => f.italic = true,
                Inline::Strike => f.strike = true,
                Inline::Code => f.code = true,
                Inline::Link => f.link = true,
                Inline::Underline => f.underline = true,
                Inline::Highlight => f.highlight = true,
                Inline::Sup => f.sup = true,
                Inline::Sub => f.sub = true,
                Inline::Fg => f.fg = Some(span.color),
                Inline::Bg => f.bg = Some(span.color),
            }
        }
    }
    v
}

/// True if `ch` (a char index within the line) is part of a marker range.
pub fn is_marker(info: &LineInfo, ch: usize) -> bool {
    info.hides.iter().any(|&(a, b)| ch >= a && ch < b)
}

/// Whether `ch` is one of a table row's `|` pipes.
pub fn is_pipe(info: &LineInfo, ch: usize) -> bool {
    info.pipes.contains(&ch)
}

/// The closing `)` that matches the `(` at `open`, counting nesting, or `None`.
///
/// Without the nesting count, `[Link]([https://a.b](https://a.b))` - which is what
/// pasting a link into a link produces - closed at the *inner* `)`, so the outer
/// label was hidden up to there, the trailing `))` was left visible, and the
/// destination came out truncated.
pub fn matching_paren(chars: &[char], open: usize, to: usize) -> Option<usize> {
    if chars.get(open) != Some(&'(') {
        return None;
    }
    let mut depth = 0usize;
    let mut i = open;
    while i < to.min(chars.len()) {
        match chars[i] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// A destination as written between the parentheses, cleaned up for use.
///
/// `[text](url "title")` keeps the url, and a link that was pasted *as* a link
/// (`[url](url)`) is unwrapped to the url, because that is the address the writer
/// meant.
pub fn link_target_of(raw: &str) -> String {
    let t = raw.trim();
    let first = t.split_whitespace().next().unwrap_or("");
    // `[url](url)` pasted as a destination.
    if let Some(rest) = first.strip_prefix('[') {
        if let Some(close) = rest.find(']') {
            let label = &rest[..close];
            let after = rest[close + 1..].trim_start();
            if let Some(inner) = after.strip_prefix('(') {
                if let Some(inner) = inner.strip_suffix(')') {
                    if !inner.trim().is_empty() {
                        return link_target_of(inner);
                    }
                }
            }
            if !label.trim().is_empty() {
                return link_target_of(label);
            }
        }
    }
    first.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    fn p(s: &str) -> LineInfo {
        let mut f = None;
        parse(&chars(s), &mut f)
    }

    #[test]
    fn headings() {
        let i = p("## Hello world");
        assert_eq!(i.block, Block::Heading(2));
        assert_eq!(i.prefix, 3);
        assert_eq!(i.hides, vec![(0, 3)]);
        assert_eq!(p("#nothashtag").block, Block::Paragraph);
    }

    #[test]
    fn lists_and_tasks() {
        let b = p("- item");
        assert_eq!(b.block, Block::Bullet);
        assert_eq!(b.prefix, 2);
        let t = p("- [x] done");
        assert_eq!(t.block, Block::Task(true));
        assert_eq!(t.prefix, 6);
        let u = p("  - nested");
        assert_eq!(u.block, Block::Bullet);
        assert_eq!(u.depth, 1);
        let o = p("12. twelve");
        assert_eq!(o.block, Block::Ordered);
        assert_eq!(o.prefix, 4);
    }

    #[test]
    fn quote_and_hr() {
        assert_eq!(p("> quoted").block, Block::Quote);
        assert_eq!(p("---").block, Block::Hr);
        assert_eq!(p("***").block, Block::Hr);
        assert_eq!(p("--").block, Block::Paragraph);
    }

    #[test]
    fn fenced_code_state_machine() {
        let mut f = None;
        let open = parse(&chars("```rust"), &mut f);
        assert_eq!(open.block, Block::Fence);
        assert_eq!(open.lang, "rust");
        assert!(f.is_some());
        let body = parse(&chars("let x = 1;"), &mut f);
        assert_eq!(body.block, Block::Code);
        let close = parse(&chars("```"), &mut f);
        assert_eq!(close.block, Block::Fence);
        assert!(f.is_none());
        let after = parse(&chars("normal"), &mut f);
        assert_eq!(after.block, Block::Paragraph);
    }

    #[test]
    fn bold_italic_code_links() {
        let i = p("a **bold** b");
        assert!(i.spans.iter().any(|s| s.kind == Inline::Bold));
        assert!(i.hides.contains(&(2, 4)));
        let i = p("*it* and `code`");
        assert!(i.spans.iter().any(|s| s.kind == Inline::Italic));
        assert!(i.spans.iter().any(|s| s.kind == Inline::Code));
        let i = p("see [docs](https://x.y) now");
        let link = i.spans.iter().find(|s| s.kind == Inline::Link).unwrap();
        let text: String = chars("see [docs](https://x.y) now")[link.start..link.end].iter().collect();
        assert_eq!(text, "docs");
        // the URL is hidden
        assert!(i.hides.iter().any(|&(a, b)| b > a + 8));
    }

    #[test]
    fn flags_are_applied() {
        let i = p("**bo**ld");
        let f = flags(&i, i.prefix.max(6));
        assert!(f[2].bold, "flags: {f:?}");
        assert!(!f[5].bold);
    }

    #[test]
    fn table_pipes_are_hidden() {
        let i = p("| a | b |");
        assert_eq!(i.block, Block::Table);
        assert_eq!(i.hides.len(), 3);
        assert_eq!(i.cells.len(), 2);
        assert!(!i.table_sep);
        let a: String = "| a | b |".chars().collect::<Vec<_>>()[i.cells[0].0..i.cells[0].1].iter().collect();
        assert_eq!(a, "a");
    }

    #[test]
    fn table_separator_row_is_detected() {
        assert!(p("| --- | :-- |").table_sep);
        assert!(p("|---|---|").table_sep);
        assert!(!p("| a | b |").table_sep);
        assert!(!p("| cell | --- |").table_sep);
    }

    #[test]
    fn fence_markers_are_hidden() {
        let mut f = None;
        let open = parse(&chars("```rust"), &mut f);
        // markers hidden, language label kept
        assert_eq!(open.hides, vec![(0, 3)]);
        let close = parse(&chars("```"), &mut f);
        assert_eq!(close.hides, vec![(0, 3)]);
    }

    #[test]
    fn unmatched_markers_are_literal() {
        let i = p("2 * 3 = 6");
        assert!(i.spans.is_empty(), "{:?}", i.spans);
    }
}
