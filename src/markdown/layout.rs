//! Document layout: turns a `Doc` into positioned runs of styled text.
//!
//! Every source line produces two candidate run lists:
//!
//! * `clean`  - Markdown markers (`#`, `**`, `>`, `[]()`...) removed, used while
//!              the caret is elsewhere (this is what makes editing "live": the
//!              document is always rendered, never in a separate preview pane).
//! * `raw`    - every character visible, markers dimmed, used for the line that
//!              holds the caret or the selection.
//!
//! Both lists are built in a single pass and cached until the document, the
//! viewport width or the font size changes.

use crate::doc::Doc;
use crate::geom::Color;
use crate::markdown::{self, Block, Flags, LineInfo};
use crate::text::{Face, TextEngine};
use crate::theme::Theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inks {
    Text,
    Dim,
    Heading,
    Code,
    Link,
    Marker,
    Quote,
}

#[derive(Clone, Copy, Debug)]
pub struct RunStyle {
    pub face: Face,
    pub size: f32,
    pub ink: Inks,
    pub underline: bool,
    pub strike: bool,
    /// `==highlight==`: painted behind the glyphs.
    pub highlight: bool,
    /// Vertical shift of the baseline, in ems (superscript positive).
    pub raise: f32,
    /// Explicit text colour (`[#rrggbb]`), overrides `ink`.
    pub fg: Option<Color>,
    /// Explicit background colour (`[bg:#rrggbb]`).
    pub bg: Option<Color>,
    /// Extra stem thickness, in pixels, for emphasis that the font has no heavier
    /// weight to express.
    ///
    /// A heading, a table's header row and inline code are already drawn with the
    /// bold face, so `**strong**` inside them used to change *nothing* on screen -
    /// which reads as "bold does not work in headings". The emphasis is drawn by
    /// thickening the glyphs instead; the advance is untouched, so nothing moves.
    pub weight: f32,
    /// An inline code span (`` `one liner` ``): painted with a chip behind it.
    ///
    /// Only the *inline* kind. A fenced block is already a card, and drawing a chip
    /// per line inside it would stripe the card.
    pub code: bool,
}

impl RunStyle {
    /// Whether this is an inline code span that wants a chip behind it.
    pub fn code(&self) -> bool {
        self.code
    }

    fn plain(face: Face, size: f32, ink: Inks) -> RunStyle {
        RunStyle {
            face,
            size,
            ink,
            underline: false,
            strike: false,
            highlight: false,
            raise: 0.0,
            fg: None,
            bg: None,
            weight: 0.0,
            code: false,
        }
    }
}

pub fn rgb_color(value: u32) -> Color {
    Color::rgba(
        ((value >> 16) & 0xff) as f32 / 255.0,
        ((value >> 8) & 0xff) as f32 / 255.0,
        (value & 0xff) as f32 / 255.0,
        1.0,
    )
}

/// How the editor paints a document.
///
/// * `Live` - rendered: markers hidden except on the line being edited.
/// * `LivePlus` - markers visible everywhere, formatting still applied.
/// * `Raw` - plain source: no formatting at all, just monospaced text.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ViewMode {
    #[default]
    Live,
    LivePlus,
    Raw,
}

impl ViewMode {
    pub const ALL: [ViewMode; 3] = [ViewMode::Live, ViewMode::LivePlus, ViewMode::Raw];

    pub fn label(&self) -> &'static str {
        match self {
            ViewMode::Live => "Live",
            ViewMode::LivePlus => "Live+",
            ViewMode::Raw => "Raw",
        }
    }

    /// Stable id for `settings.conf`.
    pub fn id(&self) -> &'static str {
        match self {
            ViewMode::Live => "live",
            ViewMode::LivePlus => "live+",
            ViewMode::Raw => "raw",
        }
    }

    pub fn parse(s: &str) -> Option<ViewMode> {
        match s.trim().to_lowercase().as_str() {
            "live" | "rendered" => Some(ViewMode::Live),
            "live+" | "liveplus" | "source" => Some(ViewMode::LivePlus),
            "raw" | "plain" => Some(ViewMode::Raw),
            _ => None,
        }
    }

    pub fn next(&self) -> ViewMode {
        match self {
            ViewMode::Live => ViewMode::LivePlus,
            ViewMode::LivePlus => ViewMode::Raw,
            ViewMode::Raw => ViewMode::Live,
        }
    }

    /// Whether Markdown markers are on screen for every line.
    pub fn shows_markers(&self) -> bool {
        !matches!(self, ViewMode::Live)
    }

    /// Whether formatting (sizes, weights, cards, decorations) is switched off.
    pub fn plain(&self) -> bool {
        matches!(self, ViewMode::Raw)
    }
}

/// Padding inside a table cell, on each side of the text.
pub const TABLE_CELL_PAD: f32 = 10.0;
/// Height of the `| --- |` alignment row (painted as the header rule).
pub const TABLE_RULE_H: f32 = 4.0;
/// Horizontal padding between a card edge and the text inside it.
pub const CODE_PAD_X: f32 = 12.0;

/// Padding either side of an inline code span's chip, in pixels.
pub const CODE_CHIP_PAD: f32 = 3.0;
/// How far a card edge sits from the first/last line it wraps.
pub const CARD_PAD_Y: f32 = 6.0;
/// Left inset of a quoted line, and the step between nesting levels.
/// Air between a picture and the source line under/over it.
pub const IMAGE_GAP: f32 = 6.0;
pub const QUOTE_INDENT: f32 = 16.0;
pub const QUOTE_GAP: f32 = 10.0;
/// Room taken by the bullet of a list item inside a quote (`> * one`), so the
/// marker has a gutter of its own between the quote's bar and the text.
pub const QUOTED_BULLET_ROOM: f32 = 14.0;

/// Vertical breathing room around a block that is drawn as a card (code,
/// table, quote). Every block gets its own space, so a card can never be drawn
/// over the lines above or below it.
/// Reference font size the fixed pixel paddings were designed against. Every
/// vertical gap scales with the font so that zooming out does not leave list
/// items and cards floating in gaps that used to be tighter than the text.
pub const REF_FONT: f32 = 17.0;

/// Vertical padding scale for a font size.
pub fn vertical_scale(font_size: f32) -> f32 {
    (font_size / REF_FONT).clamp(0.55, 2.2)
}

fn block_pad(block: Block) -> f32 {
    match block {
        Block::Code | Block::Fence | Block::Table => CARD_PAD_Y + 6.0,
        Block::Quote => 6.0,
        _ => 0.0,
    }
}

/// Face/size/ink a line renders its text with, before inline styling.
#[derive(Clone, Copy, Debug)]
pub struct BaseStyle {
    pub face: Face,
    pub size: f32,
    pub ink: Inks,
}

#[derive(Clone, Debug)]
pub struct Run {
    pub start: usize,
    pub end: usize,
    pub x: f32,
    pub w: f32,
    pub style: RunStyle,
    /// Right-to-left: the glyphs are shaped and painted right to left, so
    /// `x` is this run's **left** edge and the text inside it runs backwards.
    ///
    /// The caret inside such a run is measured from `x + w` instead of `x`, which
    /// is the whole of what "the caret follows the text" needs for an RTL run.
    pub rtl: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct WrapRow {
    /// Local char range covered by this visual row.
    pub start: usize,
    pub end: usize,
}

/// One line laid out bidirectionally, row by row.
///
/// A line is a paragraph, so UAX #9 runs over it once; the answer is kept as, for
/// every visual row, the runs **in painting order** (left to right, with `x`
/// measured from the row's left edge) and the x of every caret position in it.
///
/// Keeping the caret table here rather than recomputing it per call is what makes
/// the caret, the pointer and the glyphs agree in both directions: everything
/// downstream - painting, `x_abs`, `char_at_x`, the selection - reads this one
/// table, exactly the way a table row reads its own.
#[derive(Clone, Debug, Default)]
pub struct BidiLine {
    /// The paragraph's own direction: a line whose first strong character is
    /// Hebrew or Arabic reads right to left and is aligned to the right margin.
    pub rtl_para: bool,
    /// Runs per visual row, in painting order, x from the row's left edge.
    pub clean: Vec<Vec<Run>>,
    pub raw: Vec<Vec<Run>>,
    /// `(local char index, x)` per visual row, sorted by index: where the caret
    /// sits for every position of the row.
    pub clean_caret: Vec<Vec<(usize, f32)>>,
    pub raw_caret: Vec<Vec<(usize, f32)>>,
}

impl BidiLine {
    pub fn rows(&self, raw: bool) -> &Vec<Vec<Run>> {
        if raw {
            &self.raw
        } else {
            &self.clean
        }
    }

    pub fn caret(&self, raw: bool) -> &Vec<Vec<(usize, f32)>> {
        if raw {
            &self.raw_caret
        } else {
            &self.clean_caret
        }
    }
}

#[derive(Clone, Debug)]
pub struct LineLayout {
    pub line: usize,
    pub start_char: usize,
    pub chars: Vec<char>,
    pub clean: Vec<Run>,
    pub raw: Vec<Run>,
    /// Visual rows produced by soft wrapping (always at least one).
    pub rows: Vec<WrapRow>,
    /// Height of a single visual row.
    pub row_h: f32,
    /// Largest font size on this line (used for baseline placement).
    pub max_size: f32,
    /// Total height (rows * row_h).
    pub height: f32,
    pub space_before: f32,
    /// Room reserved *below* this line for its block's card.
    pub space_after: f32,
    // NOTE: `top` already includes `space_before`; the gap belongs to the line,
    // not to the one above it.
    pub indent: f32,
    /// Absolute y of the top of this line inside the document.
    pub top: f32,
    pub width: f32,
    /// The line's widest *row*, indent included - the number `max_row_width` is the
    /// maximum of. Kept per line so an incremental rebuild can recompute the maximum
    /// without laying anything out again.
    pub row_w: f32,
    pub info: LineInfo,
    pub flags: Vec<Flags>,
    /// Left x of every cell for a table row (empty for other blocks).
    pub table_cols: Vec<f32>,
    /// A table row's cells, already wrapped: one entry per visual band, each a
    /// list of runs whose `x` is absolute inside the table.
    ///
    /// A table cell wraps inside its own column, and the cells of a row wrap
    /// independently, so a row is not one line of text but a stack of bands that
    /// every cell contributes a piece to. Painting, the caret and the mouse all
    /// read this list, so a long cell can never draw outside its column.
    pub table_rows: Vec<Vec<Run>>,
    /// Where each character of a table row sits: (band, absolute x).
    pub table_pos: Vec<(u16, f32)>,
    /// The same row with its **inline** markers on screen (`**bold**`,
    /// `[link](url)`), while the structural pipes stay hidden.
    ///
    /// A table is edited in its grid, so the row that holds the caret shows what
    /// you would have to type to change it - exactly like every other block in the
    /// rendered view - without flipping the whole row to `| a | b |` source. Empty
    /// when the row has no inline markers, which is the common case.
    pub table_src_rows: Vec<Vec<Run>>,
    /// Character positions for [`LineLayout::table_src_rows`].
    pub table_src_pos: Vec<(u16, f32)>,
    /// Pictures drawn *inside* cells: `(cell index, box)`, positioned against the
    /// same `table_cols` the text uses.
    ///
    /// A cell whose whole content is `![alt](src)` shows the picture, exactly as a
    /// standalone image line does; putting the caret in the cell swaps it back to
    /// its source so it can be edited.
    pub table_images: Vec<(usize, ImageBox)>,
    /// Full width of the table this row belongs to.
    pub table_width: f32,
    /// The picture this line draws, if it is a standalone `![alt](src)` line.
    pub image: Option<ImageBox>,
    /// The flowchart a ```` ```mermaid ```` block draws. Only the block's opening
    /// fence line carries one: the source lines below it are folded away.
    pub diagram: Option<crate::mermaid::Diagram>,
    /// True for every line of a mermaid block that is showing its picture: those
    /// lines take no room and must not be given a code card.
    pub folded: bool,
    /// Set when the line contains right-to-left text (see [`bidi::analyse`]).
    /// `None` for every ordinary left-to-right line, which keeps the whole
    /// feature out of the way of the common case.
    pub bidi: Option<BidiLine>,
}

/// A picture placed on a line: the resolved file and the box it occupies.
#[derive(Clone, Debug)]
pub struct ImageBox {
    pub path: std::path::PathBuf,
    pub alt: String,
    /// Size of the picture on the page (logical pixels).
    pub w: f32,
    pub h: f32,
    /// True when the file could not be read (the painter shows a placeholder).
    pub missing: bool,
}

impl LineLayout {
    /// How many of the minimap's rows this line takes.
    ///
    /// The minimap is not a scale model of the page - block spacing would become empty
    /// gaps - so it counts *rows*: a paragraph that wraps over three rows takes three, a
    /// blank line takes one, a picture takes as many as its height. It lives here, next to
    /// the row data it reads, so `Layout` can total it once instead of the painter walking
    /// the whole document every frame (`ui::minimap_virtual_span` is now a thin wrapper).
    pub fn virtual_span(&self) -> f32 {
        if self.folded {
            return 0.0;
        }
        if self.image.is_some() || self.diagram.is_some() {
            let rows = (self.height / self.row_h.max(8.0)).round();
            return rows.max(self.rows.len().max(1) as f32).max(1.0);
        }
        match self.block() {
            Block::Blank => 1.0,
            _ => self.rows.len().max(1) as f32,
        }
    }

    /// Horizontal shift applied to the whole line by `{center}` / `{right}`.
    pub fn align_offset(&self, view_w: f32) -> f32 {
        let free = view_w - self.indent - self.width;
        if free <= 0.0 || self.info.align == 0 {
            return 0.0;
        }
        match self.info.align {
            1 => free * 0.5,
            2 => free,
            _ => 0.0,
        }
    }

    pub fn end_char(&self) -> usize {
        self.start_char + self.chars.len()
    }

    pub fn runs(&self, raw: bool) -> &[Run] {
        if raw {
            &self.raw
        } else {
            &self.clean
        }
    }

    pub fn row_count(&self) -> usize {
        self.rows.len().max(1)
    }

    /// The band list to paint / measure against: the rendered one, or the one
    /// with inline markers on screen.
    pub fn bands(&self, src: bool) -> &[Vec<Run>] {
        if src && !self.table_src_rows.is_empty() {
            &self.table_src_rows
        } else {
            &self.table_rows
        }
    }

    /// Character positions matching [`LineLayout::bands`].
    pub fn band_pos(&self, src: bool) -> &[(u16, f32)] {
        if src && !self.table_src_pos.is_empty() {
            &self.table_src_pos
        } else {
            &self.table_pos
        }
    }

    /// Whether this table row has inline markers that can be revealed.
    pub fn has_inline_markers(&self) -> bool {
        !self.table_src_rows.is_empty()
    }

    /// Index of the visual row containing a local char index.
    pub fn row_of_local(&self, local: usize) -> usize {
        self.row_of_local_in(local, false)
    }

    /// Index of the visual row containing a local char index, for a row showing
    /// its inline markers (`src`).
    pub fn row_of_local_in(&self, local: usize, src: bool) -> usize {
        let local = local.min(self.chars.len());
        let pos = self.band_pos(src);
        if !pos.is_empty() {
            // A table row's cells wrap independently, so which band a character
            // is on can only be answered by the wrap itself.
            return pos
                .get(local)
                .map(|t| t.0 as usize)
                .unwrap_or(0)
                .min(self.row_count().saturating_sub(1));
        }
        for (i, row) in self.rows.iter().enumerate() {
            if local < row.end {
                return i;
            }
        }
        self.rows.len().saturating_sub(1)
    }

    /// The visual band at a y inside this line, for the geometry that is actually
    /// painted (`revealed` = the row under the caret is showing its inline
    /// markers).
    ///
    /// A table row is the one block whose band count depends on that choice - a
    /// cell with a link is taller once the link's source is visible - so the click
    /// has to read its band from the same variant it then maps x through. Reading
    /// the band from the rendered row and the x from the revealed one is what made
    /// a click land two characters away.
    pub fn row_of_y_in(&self, y: f32, revealed: bool) -> usize {
        if self.row_h <= 0.0 {
            return 0;
        }
        let n = ((y - self.top) / self.row_h).floor();
        let bands = self.bands(revealed).len().max(self.rows.len()).max(1);
        (n.max(0.0) as usize).min(bands - 1)
    }

    /// Nesting level of the line's decoration, at least 1 (a plain `>` counts as
    /// the first level, so every quote row gets a bar).
    pub fn factor_depth(&self) -> usize {
        self.info.depth.max(1)
    }

    /// Whether this line starts with a block marker the rendered view hides
    /// (`# `, `> `, `- `, `1. `, `- [ ] `).
    pub fn marks_a_block(&self) -> bool {
        matches!(
            self.info.block,
            Block::Heading(_) | Block::Quote | Block::Bullet | Block::Ordered | Block::Task(_)
        )
    }

    pub fn row(&self, idx: usize) -> WrapRow {
        self.rows
            .get(idx)
            .copied()
            .unwrap_or(WrapRow { start: self.chars.len(), end: self.chars.len() })
    }

    /// Absolute x (unwrapped, as if the line were a single row) of a local index.
    /// Distance from the start of `run` to character `up_to`, measured with the
    /// run's own per-character advances.
    ///
    /// Shaping a prefix on its own (`width(&chars[start..up_to])`) is *not* the
    /// same number: the prefix loses the kerning it had with the character that
    /// follows it, so the caret would sit a few tenths of a pixel away from the
    /// glyph it points at, and those tenths add up along the line. Everything that
    /// asks "where is this character" must go through here.
    fn run_prefix(te: &mut TextEngine, chars: &[char], run: &Run, up_to: usize) -> f32 {
        let up_to = up_to.clamp(run.start, run.end);
        if up_to == run.start {
            return 0.0;
        }
        let text: String = chars[run.start..run.end].iter().collect();
        let adv = te.char_advances(run.style.face, &text, run.style.size);
        adv.iter().take(up_to - run.start).sum()
    }

    pub fn x_abs(&self, te: &mut TextEngine, local: usize, raw: bool) -> f32 {
        let local = local.min(self.chars.len());
        if let Some(x) = self.bidi_x_abs(local, raw) {
            return x;
        }
        if let Some(x) = self.x_abs_cell(te, local, raw) {
            return x;
        }
        let runs = self.runs(raw);
        if runs.is_empty() {
            return 0.0;
        }
        let mut last_x = 0.0;
        for run in runs {
            if local < run.start {
                // The caret sits in a gap (a hidden marker, or the padding that
                // table cells are trimmed of): it belongs to the end of the text
                // before it, not to the start of the text after it.
                return last_x;
            }
            if local <= run.end {
                return run.x + Self::run_prefix(te, &self.chars, run, local);
            }
            last_x = run.x + run.w;
        }
        last_x
    }

    /// x of a caret position on a line that was laid out bidirectionally.
    ///
    /// Row-local, like the left-to-right path, and read from the table the layout
    /// recorded: the careful part (which side of a right-to-left run the caret is
    /// on) happens once, in [`bidi_line`], so the caret, the pointer and the
    /// selection cannot each get their own answer.
    fn bidi_x_abs(&self, local: usize, raw: bool) -> Option<f32> {
        let b = self.bidi.as_ref()?;
        let ri = self.row_of_local_in(local, raw);
        let pts = b.caret(raw).get(ri)?;
        if pts.is_empty() {
            return Some(0.0);
        }
        match pts.binary_search_by_key(&local, |p| p.0) {
            Ok(k) => Some(pts[k].1),
            // Between two recorded positions - a hidden marker, or the padded end
            // of a cell - the caret belongs to the text before it, the same rule
            // the left-to-right path uses.
            Err(k) => Some(pts.get(k.saturating_sub(1)).map(|p| p.1).unwrap_or(0.0)),
        }
    }

    /// The next caret position in **visual** order - what an arrow key means.
    ///
    /// Left and right move the caret the way it appears to move on screen; inside a
    /// right-to-left run that is the opposite way round in the document, which is
    /// why the arrows used to feel inverted in a Hebrew or Arabic table cell.
    ///
    /// `None` means "plain logical movement": an ordinary line, or a step that
    /// leaves the row. Two positions that share one pixel where the direction
    /// changes are both kept in the sequence, so neither becomes unreachable.
    pub fn visual_step(&self, local: usize, forward: bool, raw: bool) -> Option<usize> {
        let ri = self.row_of_local_in(local, raw);
        let mut stops: Vec<(usize, f32)> = Vec::new();
        if !self.table_cols.is_empty() {
                // A table row: the positions of **every** cell on this band, read
                // from the same per-character table the caret and the pointer use.
                //
                // Every cell, not just the caret's: the rightmost position of a
                // right-to-left cell is the *start* of its word in the document, so
                // stepping to `caret + 1` from there stayed inside the same word and
                // the arrows ping-ponged between two letters. In visual order the
                // next stop is the start of the next cell, which this list has.
                let pos = self.band_pos(raw);
                let runs = self.runs(raw);
                for (i, &(pb, x)) in pos.iter().enumerate() {
                    if pb as usize != ri || i > self.chars.len() {
                        continue;
                    }
                    // Only positions a cell owns: the padding and the pipes between
                    // cells are not places the arrows should stop at one by one.
                    if i <= self.chars.len()
                        && !self.info.cells.iter().any(|&(ca, cb)| i >= ca && i <= cb)
                    {
                        continue;
                    }
                    if !runs.iter().any(|r| i >= r.start && i <= r.end) {
                        continue;
                    }
                    stops.push((i, x));
                }
            if stops.is_empty() {
                return None;
            }
        } else if let Some(b) = &self.bidi {
            // A bidirectional paragraph already knows exactly where each position
            // is: the recorded caret table *is* the visual order.
            for &(i, x) in b.caret(raw).get(ri)? {
                stops.push((i, x));
            }
        } else {
            return None;
        }
        // Left to right; equal x keeps document order.
        stops.sort_by(|p, q| {
            p.1.partial_cmp(&q.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(p.0.cmp(&q.0))
        });
        stops.dedup_by_key(|st| st.0);
        let at = stops.iter().position(|&(i, _)| i == local)?;
        let next = if forward {
            stops.get(at + 1)
        } else {
            at.checked_sub(1).and_then(|k| stops.get(k))
        };
        if let Some(next) = next {
            return Some(next.0);
        }
        // Nothing further that way on this row. At the very edge of the paragraph the
        // caret stays where it is, the way it does at the end of any line - jumping
        // to the line's end would send it the *other* way, which is the report we
        // are fixing. In the middle of a wrapped paragraph `None` lets the caller
        // move logically into the neighbouring row.
        let edge = if forward {
            ri + 1 >= self.rows.len()
        } else {
            ri == 0
        };
        edge.then_some(local)
    }

    /// Is this line's paragraph right-to-left?
    pub fn rtl_para(&self) -> bool {
        self.bidi.as_ref().map(|b| b.rtl_para).unwrap_or(false)
    }

    /// Runs of one visual row in painting order (left to right), for a
    /// bidirectionally laid-out line. `None` for ordinary lines.
    pub fn bidi_row_runs(&self, ri: usize, raw: bool) -> Option<&Vec<Run>> {
        self.bidi.as_ref()?.rows(raw).get(ri)
    }

    /// Width of one visual row, in pixels.
    pub fn row_width_of(&self, ri: usize, raw: bool) -> f32 {
        match self.bidi_row_runs(ri, raw) {
            Some(runs) => runs.iter().map(|r| r.x + r.w).fold(0.0f32, f32::max),
            None => self.width,
        }
    }

    /// Like [`LineLayout::align_offset`], but measured per visual row.
    ///
    /// A right-to-left paragraph starts at the **right** margin, the way `dir=rtl`
    /// does in a browser - and it does so row by row, so the short last row of a
    /// wrapped Hebrew paragraph hugs the right edge instead of sitting in the
    /// middle of the page. An explicit alignment (centre, right) still wins.
    pub fn align_offset_row(&self, view_w: f32, ri: usize, raw: bool) -> f32 {
        if !self.rtl_para() || self.info.align != 0 {
            return self.align_offset(view_w);
        }
        // The indent is mirrored for an RTL paragraph: the gutter is on the right,
        // so the text lives in `[indent .. view_w - indent]` - which is also the box
        // the wrap was computed for.
        (view_w - self.indent * 2.0 - self.row_width_of(ri, raw)).max(0.0)
    }

    // ---- table cells -------------------------------------------------------
    // A table row is not one line of text: it is a set of independent cells, each
    // starting at its own x. The general code below assumes one left margin, so a
    // caret in an *empty* cell (a cell with no run at all) had nowhere to be and
    // was snapped back to the cell before it - which is why an empty cell could
    // not be clicked into, let alone typed in.

    /// Char range of a cell, widened to the next pipe so trailing spaces in the
    /// cell belong to it.
    fn cell_chars(&self, ci: usize) -> Option<(usize, usize)> {
        let &(a, b) = self.info.cells.get(ci)?;
        let mut b = b.max(a);
        while b < self.chars.len() && self.chars[b] != '|' {
            b += 1;
        }
        Some((a, b.min(self.chars.len())))
    }

    /// The cell whose column `x` falls in, for a table row.
    fn cell_at_x(&self, x: f32) -> Option<usize> {
        let n = self.info.cells.len().min(self.table_cols.len());
        if n == 0 {
            return None;
        }
        for ci in 0..n {
            let start = self.table_cols[ci];
            let end = if ci + 1 < n {
                self.table_cols[ci + 1] - TABLE_CELL_PAD * 2.0
            } else if self.table_width > 0.0 {
                self.table_width - TABLE_CELL_PAD
            } else {
                self.width
            };
            if x < end {
                return Some(ci);
            }
            let _ = start;
        }
        Some(n - 1)
    }

    /// Style to measure plain characters of a cell with (the cell's own text
    /// style where it has one).
    fn cell_style(&self, ci: usize, raw: bool) -> Option<RunStyle> {
        let (a, b) = self.cell_chars(ci)?;
        if let Some(r) = self.runs(raw).iter().find(|r| r.end > a && r.start < b) {
            return Some(r.style);
        }
        self.runs(raw).first().map(|r| r.style)
    }

    /// Advance of one character of a table row, in the style it is painted with.
    fn cell_advance(&self, te: &mut TextEngine, i: usize, raw: bool) -> f32 {
        let Some(run) = self.runs(raw).iter().find(|r| i >= r.start && i < r.end).cloned() else {
            return 0.0;
        };
        let text: String = self.chars[run.start..run.end].iter().collect();
        let adv = te.char_advances(run.style.face, &text, run.style.size);
        adv.get(i - run.start).copied().unwrap_or(0.0)
    }

    /// `x_abs` for a table row; `None` for anything that is not a laid-out table.
    fn x_abs_cell(&self, te: &mut TextEngine, local: usize, raw: bool) -> Option<f32> {
        if self.info.block != Block::Table || self.table_cols.is_empty() {
            return None;
        }
        let n = self.info.cells.len().min(self.table_cols.len());
        if n == 0 {
            return None;
        }
        // Wrapped cells carry their own per-character position, band included:
        // the caret and the mouse both read it, so they cannot drift apart.
        let pos = self.band_pos(raw);
        if !pos.is_empty() {
            if let Some((_, x)) = pos.get(local.min(pos.len() - 1)) {
                return Some(*x);
            }
        }
        // The cell this index belongs to: the last one that starts at or before it.
        let mut ci = 0;
        for i in 0..n {
            if self.info.cells[i].0 <= local {
                ci = i;
            }
        }
        let x0 = self.table_cols[ci];
        let (a, b) = self.cell_chars(ci)?;
        if local <= a {
            // In the gap before the cell's text (the padding, or an empty cell):
            // that is the cell's own left edge, not the end of the cell before.
            return Some(x0);
        }
        let local = local.min(b);
        let style = self.cell_style(ci, raw)?;
        let mut x = x0;
        let mut cursor = a;
        for run in self.runs(raw) {
            if run.end <= a || run.start >= b || run.start >= local {
                continue;
            }
            let from = run.start.max(cursor).max(a);
            let to = run.end.min(local);
            if to <= from {
                continue;
            }
            if from > cursor {
                let gap: String = self.chars[cursor..from].iter().collect();
                x += te.width(style.face, &gap, style.size);
            }
            let text: String = self.chars[from..to].iter().collect();
            x += te.width(run.style.face, &text, run.style.size);
            cursor = to;
        }
        if cursor < local {
            let rest: String = self.chars[cursor..local].iter().collect();
            x += te.width(style.face, &rest, style.size);
        }
        Some(x)
    }

    /// `char_at_row_x` for a table row: pick the cell, then the character in it.
    fn char_at_cell_x(
        &self,
        te: &mut TextEngine,
        band: usize,
        x: f32,
        raw: bool,
    ) -> Option<usize> {
        if self.info.block != Block::Table || self.table_cols.is_empty() {
            return None;
        }
        // Wrapped rows know exactly which piece of which cell is on this band, so
        // the answer comes from the wrap and not from re-deriving it. Only the
        // pieces of the cell the pointer is over take part: an empty cell draws no
        // piece at all, and without this filter the loop below would hand the
        // click to the *next* cell's start.
        // The band this click is on knows where *every character of the cell* sits:
        // `pos` is the table the painter and the caret also read, built from the
        // real per-character advances. Deriving the character from an average
        // width (`run.w / run.len()`) instead put the caret one letter to the
        // right of the pointer as soon as a cell held a run of proportional
        // letters - `i`, `l` and `m` do not have the same width, and the error
        // grew towards the end of the run.
        //
        // Only the characters this variant actually paints take part: a hidden
        // `**` sits at the same x as the word it marks, and mapping a click onto
        // it would drop the caret *before* the marker - typing there would split
        // the markup instead of editing the word.
        if let Some(ci) = self.cell_at_x(x) {
            let (ta, tb) = self.info.cells[ci];
            let pos = self.band_pos(raw);
            let runs = self.runs(raw);
            let visible = |i: usize| runs.iter().any(|r| i >= r.start && i < r.end);
            // Slots are *positions*, not characters. A hidden marker cannot hold a
            // caret - typing there would split `**bold**` - but the position just
            // after a visible character can, even when the next source character
            // is a hidden one. Offering only the visible characters as slots made
            // three positions collapse into one: before the word, inside it, and
            // the end of the cell, which is why the pointer could not be put right
            // after a bold word or a link, and why a click past the last letter of
            // a cell jumped to the cell's end instead of staying beside the glyph.
            let last = tb.min(self.chars.len());
            let mut slots: Vec<(usize, f32)> = Vec::new();
            for i in ta..=last {
                let here = visible(i);
                if !here && !(i > ta && visible(i - 1)) {
                    continue;
                }
                let Some(&(pb, at)) = pos.get(i) else { continue };
                if pb as usize != band {
                    continue;
                }
                // A hidden position sits at the *end* of the character before it,
                // not at its start: `pos` carries the last painted slot forward, so
                // a `**` would share its x with the letter in front of it, and a
                // click a fifth of the way into that letter landed *after* it - the
                // caret jumped over the character the pointer was on.
                let x = if here {
                    at
                } else {
                    let next = (i + 1..=last)
                        .find(|&q| visible(q) && pos.get(q).map(|&(qb, _)| qb as usize) == Some(band))
                        .and_then(|q| pos.get(q))
                        .map(|&(_, qx)| qx);
                    match next {
                        // The next painted slot of this band is where the glyph
                        // before this marker ends.
                        Some(qx) => qx,
                        // Nothing painted after it in this cell: measure the glyph.
                        None => at + self.cell_advance(te, i - 1, raw),
                    }
                };
                slots.push((i, x));
            }
            if slots.is_empty() {
                // Nothing painted in this cell on this band: its own edge is the
                // only caret position that keeps what you type inside it.
                return Some(ta);
            }
            // The nearest recorded position wins. This used to walk the slots in
            // index order and assume their x grew with the index - true
            // left-to-right, exactly backwards in a right-to-left cell, where a
            // click picked a position from the wrong end of the word and the middle
            // of it could not be reached at all.
            let mut best = slots[0];
            let mut best_d = (x - best.1).abs();
            for &(i, at) in slots.iter().skip(1) {
                let d = (x - at).abs();
                if d < best_d {
                    best = (i, at);
                    best_d = d;
                }
            }
            return Some(best.0);
        }
        let ci = self.cell_at_x(x)?;
        let (a, b) = self.cell_chars(ci)?;
        let x0 = self.table_cols[ci];
        // A cell with no text of its own (only the padding `|  |` leaves) has one
        // place a caret can be, and it is the start of the cell: clicking anywhere
        // in it must put the caret there.
        let (ta, tb) = self.info.cells[ci];
        if ta >= tb {
            return Some(ta);
        }
        if x <= x0 || a >= b {
            // Left of the text, or an empty cell: the caret goes to the cell's
            // own start, where a typed character lands inside this cell.
            return Some(a);
        }
        let mut prev = x0;
        for i in a..b {
            let Some(xi) = self.x_abs_cell(te, i + 1, raw) else { break };
            if x < (prev + xi) * 0.5 {
                return Some(i);
            }
            prev = xi;
        }
        Some(b)
    }

    /// x offset of a local index *inside its visual row*, measured from the row's
    /// **left** edge - the same origin the painter uses.
    pub fn x_of(&self, te: &mut TextEngine, local: usize, raw: bool) -> f32 {
        let local = local.min(self.chars.len());
        // A bidirectional row can start anywhere: in a right-to-left paragraph the
        // text begins at the row's right edge, so "the offset of the row's first
        // character" is the row's *width*, not zero. Subtracting it - which is what
        // makes the left-to-right case row-relative - would throw every caret in an
        // RTL row back to the left margin. The recorded table is already row-local,
        // so it is the answer as it stands.
        if self.bidi.is_some() {
            return self.x_abs(te, local, raw);
        }
        let row = self.row(self.row_of_local_in(local, raw));
        let base = self.x_abs(te, row.start, raw);
        self.x_abs(te, local, raw) - base
    }

    /// The pieces of **any** line a selection covers: `(row, left, right)` per visual
    /// row, in the row's own coordinates (0 = the row's left edge, whatever the
    /// direction of the text).
    ///
    /// This is what a wash draws. The older rule took the x of the first selected
    /// offset as the left edge and the x of the second as the right edge, which is
    /// only true left to right: in a right-to-left row the first selected character
    /// is the one furthest **right**, so `right - left` came out negative and the
    /// wash collapsed to a 2 px sliver - a selection in a Hebrew paragraph looked
    /// like it had not been painted at all. Two more things are fixed by measuring
    /// here rather than in the painter: each visual row gets **its own** alignment
    /// shift (wrapped right-to-left rows are not all the same width), and a row
    /// selected in full is washed from its left edge to its right one rather than
    /// from the first glyph's position.
    pub fn row_span(&self, te: &mut TextEngine, a: usize, b: usize, raw: bool) -> Vec<(usize, f32, f32)> {
        let mut out: Vec<(usize, f32, f32)> = Vec::new();
        if b < a {
            return out;
        }
        let a = a.min(self.chars.len());
        let b = b.min(self.chars.len());
        for (ri, row) in self.rows.iter().enumerate() {
            let ra = a.max(row.start).min(row.end);
            let rb = b.max(row.start).min(row.end);
            // A row the selection only touches at a boundary owns nothing: painting
            // it would put a bar on a line with no selected characters.
            if rb < ra || (ra == rb && !(a <= row.start && b >= row.end)) {
                continue;
            }
            let full = a <= row.start && b >= row.end;
            let (lo, hi) = if full {
                (0.0, self.row_width_of(ri, raw))
            } else {
                let xa = self.x_of(te, ra, raw);
                let xb = self.x_of(te, rb, raw);
                (xa.min(xb), xa.max(xb))
            };
            out.push((ri, lo, hi));
        }
        out
    }

    /// The pieces of a table row a selection covers: `(band, x, width)` per piece.
    ///
    /// A table row is not one line of text. A band carries a piece of every cell at
    /// that cell's own x, and an offset can sit on a different band in a different
    /// cell, so the wrapped-paragraph rule (one bar from the first selected offset
    /// to the second, on every visual row) painted the whole row instead of the
    /// selected words - that is what "selecting several lines in a cell" looked
    /// like. Each piece is highlighted on its own band instead, so a wash never
    /// crosses a cell or a row.
    pub fn table_sel_rects(&self, a: usize, b: usize, raw: bool) -> Vec<(usize, f32, f32)> {
        let mut out: Vec<(usize, f32, f32)> = Vec::new();
        if b <= a || self.table_cols.is_empty() {
            return out;
        }
        // The x of every character comes from the same table the caret and the
        // mouse read, never from an average width: `i`, `l` and `m` are not the
        // same width, so dividing a run by its character count slid the far end of
        // a selection away from the glyphs it was supposed to cover.
        let pos = self.band_pos(raw);
        for (bi, band) in self.bands(raw).iter().enumerate() {
            // The box each character occupies on this band: `(index, left, right)`.
            //
            // In a left-to-right run character `i` starts at its recorded x and
            // ends at the next one. In a **right-to-left** run the recorded x is the
            // character's *right* edge and it ends on the left, at the next one - so
            // `next_x - this_x` came out negative there and the floor turned the
            // wash into a 1.5 px sliver. That is why a Hebrew or Arabic symbol in a
            // table cell looked as if it could not be selected at all.
            let mut boxes: Vec<(usize, f32, f32)> = Vec::new();
            for run in band {
                let end = run.end.min(self.chars.len());
                for i in run.start..end {
                    let Some(&(pb, x)) = pos.get(i) else { continue };
                    if pb as usize != bi {
                        continue;
                    }
                    // Where this character's box ends: at the next character of the
                    // same run, or at the run's own far edge.
                    let next = if i + 1 < end {
                        pos.get(i + 1).and_then(|&(nb, nx)| (nb as usize == bi).then_some(nx))
                    } else {
                        Some(if run.rtl { run.x } else { run.x + run.w })
                    };
                    let (left, right) = match next {
                        Some(nx) => (x.min(nx), x.max(nx)),
                        None => (x, x),
                    };
                    boxes.push((i, left, right));
                }
            }
            if boxes.is_empty() {
                continue;
            }
            boxes.sort_by_key(|bx| bx.0);
            // The wash covers the boxes of the selected characters - a hidden
            // marker shares its x with the word it marks, so `a` inside one still
            // starts at the right place.
            let chosen: Vec<&(usize, f32, f32)> = boxes.iter().filter(|bx| bx.0 >= a && bx.0 < b).collect();
            let (Some(first), Some(_)) = (chosen.first(), chosen.last()) else {
                continue;
            };
            let left = chosen.iter().map(|bx| bx.1).fold(first.1, f32::min);
            let right = chosen.iter().map(|bx| bx.2).fold(first.2, f32::max);
            out.push((bi, left, (right - left).max(1.5)));
        }
        out
    }

    /// Local index closest to `x` inside visual row `row`.
    pub fn char_at_row_x(&self, te: &mut TextEngine, row_idx: usize, x: f32, raw: bool) -> usize {
        let row = self.row(row_idx);
        // Bidirectional line: the recorded caret positions *are* the answer, so a
        // click lands between the two characters whose positions it falls between,
        // whichever way the text runs there.
        //
        // Two positions can share one pixel - the start and the end of a Hebrew run
        // are both at the run's left edge - and then no hit test can tell them
        // apart. The earlier one wins, which keeps the answer stable instead of
        // depending on the order the runs happened to be walked in.
        if let Some(b) = &self.bidi {
            if let Some(pts) = b.caret(raw).get(row_idx) {
                if !pts.is_empty() {
                    let mut best = (f32::INFINITY, row.start);
                    for (idx, cx) in pts {
                        let d = (cx - x).abs();
                        if d < best.0 {
                            best = (d, *idx);
                        }
                    }
                    return best.1.clamp(row.start, row.end);
                }
            }
        }
        let base = self.x_abs(te, row.start, raw);
        let target = x + base;
        if let Some(i) = self.char_at_cell_x(te, row_idx, target, raw) {
            return i;
        }
        let runs = self.runs(raw);
        if runs.is_empty() {
            return row.start;
        }
        for run in runs {
            if run.end <= row.start {
                continue;
            }
            if run.start >= row.end {
                break;
            }
            let first = run.start.max(row.start);
            // Width already consumed inside this run before the row starts, and the
            // advances of the rest of the run - both from the run as a whole, so a
            // pointer lands on the glyph it is over.
            let text: String = self.chars[run.start..run.end].iter().collect();
            let adv = te.char_advances(run.style.face, &text, run.style.size);
            let mut acc = Self::run_prefix(te, &self.chars, run, first);
            if target <= run.x + acc {
                // Left of this run's text: stay at the end of the previous one.
                return first.min(run.start).max(row.start);
            }
            if target < run.x + run.w {
                for i in first..run.end.min(row.end) {
                    let w = adv.get(i - run.start).copied().unwrap_or(0.0);
                    if target < run.x + acc + w * 0.5 {
                        return i;
                    }
                    acc += w;
                }
                return run.end.min(row.end);
            }
        }
        row.end
    }

    /// Convenience wrapper kept for single-row callers.
    pub fn char_at_x(&self, te: &mut TextEngine, x: f32, raw: bool) -> usize {
        self.char_at_row_x(te, 0, x, raw)
    }

    pub fn block(&self) -> Block {
        self.info.block
    }
}

pub struct Layout {
    pub lines: Vec<LineLayout>,
    pub total_height: f32,
    /// Whether long lines were broken into visual rows (soft wrap).
    ///
    /// Part of the layout, not a decision the painter makes: turning wrap off means
    /// every line is one row however wide it gets, and the view scrolls sideways
    /// instead. A layout built with one answer must not be reused for the other, so
    /// the flag travels with the cache key (`is_valid_for`).
    pub wrap: bool,
    /// Widest *visual row* (what actually needs horizontal scrolling).
    pub max_row_width: f32,
    pub max_width: f32,
    /// Width the layout was built for.
    pub width: f32,
    pub font_size: f32,
    pub version: u64,
    /// (line index, level, text) for the outline panel.
    pub headings: Vec<(usize, u8, String)>,
    /// Cached word count so the status bar never re-scans the document.
    pub word_count: usize,
    /// Rows the minimap has to draw for this document (the sum of
    /// [`LineLayout::virtual_span`]), cached for the same reason as `word_count`: the map
    /// is painted every frame, and adding up ten thousand lines per frame was 235 us of a
    /// 400 us minimap.
    pub virtual_rows: f32,
    /// Characters in the widest visual row - what a full-width minimap bar means.
    pub widest_row_chars: usize,
    /// The view mode this layout was built for.
    pub mode: ViewMode,
    /// (first line, last line) of every ```` ```mermaid ```` block, open fence to
    /// closing fence inclusive.
    pub mermaid: Vec<(usize, usize)>,
    /// The mermaid block whose source is shown instead of its picture (the one
    /// holding the caret), keyed by its opening fence line.
    pub editing: Option<usize>,
}

impl Layout {
    pub fn empty() -> Layout {
        Layout {
            lines: Vec::new(),
            total_height: 0.0,
            max_row_width: 0.0,
            max_width: 0.0,
            wrap: true,
            width: 0.0,
            font_size: 0.0,
            version: u64::MAX,
            headings: Vec::new(),
            word_count: 0,
            virtual_rows: 0.0,
            widest_row_chars: 8,
            mode: ViewMode::Live,
            mermaid: Vec::new(),
            editing: None,
        }
    }

    pub fn is_valid_for(
        &self,
        doc: &Doc,
        width: f32,
        font_size: f32,
        mode: ViewMode,
        wrap: bool,
    ) -> bool {
        self.version == doc.version
            && self.mode == mode
            && self.wrap == wrap
            && (self.width - width).abs() < 0.5
            && (self.font_size - font_size).abs() < 0.01
    }

    /// Bring this layout up to date after an edit, reusing every line outside the
    /// block that changed. `false` means "not this way": the caller rebuilds.
    ///
    /// An edit changes one block - a paragraph, a table, a fenced code block - and
    /// everything outside it lays out exactly as it did, so those lines are kept
    /// (moved, not rebuilt) and only the block itself is laid out again. That block is
    /// laid out in a throwaway document holding just its lines, with a blank line in
    /// front: a blank line is what the block really has above it, and it is also what
    /// makes the block self-contained, because no paragraph, table, fence or mermaid
    /// block crosses one.
    ///
    /// Measured on a 30 000-line file, the whole-document rebuild this replaces cost
    /// 139 ms per keystroke (`docs/ARCHITECTURE.md`); this costs the block, plus a walk
    /// over the line list to renumber what is left.
    pub fn try_patch(
        &mut self,
        doc: &Doc,
        te: &mut TextEngine,
        theme: &Theme,
        width: f32,
        mode: ViewMode,
        editing: Option<usize>,
        wrap: bool,
    ) -> bool {
        if self.mode != mode || self.editing != editing || self.wrap != wrap {
            return false;
        }
        if (self.width - width).abs() >= 0.5 || (self.font_size - theme.font_size).abs() >= 0.01 {
            return false;
        }
        let n = doc.len_lines();
        if n < 2 || self.lines.len() < 2 {
            return false;
        }
        let n_old = self.lines.len();
        // Lines added or removed below the caret move everything under them; how many
        // is `delta`, and the tail is renumbered by it.
        let delta = n as isize - n_old as isize;

        // ---- what changed ----
        // Two walks over the document's characters, one from each end, against the
        // lines this layout already holds. Asking the rope for a line at a time is a
        // tree lookup each - 30 000 of them per keystroke, which is what made finding
        // the difference alone cost 12 ms.
        let mut lo = {
            let mut chars = doc.chars();
            let mut found = n;
            for (i, line) in self.lines.iter().enumerate() {
                let mut same = line.chars.iter().all(|&c| chars.next() == Some(c));
                if same {
                    // the line break that follows the line
                    same = match chars.next() {
                        Some('\n') => true,
                        Some('\r') => {
                            chars.next();
                            true
                        }
                        None => i + 1 == n_old,
                        Some(_) => false,
                    };
                }
                if !same {
                    found = i;
                    break;
                }
            }
            found
        };
        if lo == n {
            if n == n_old {
                // Not one character moved: the version moved, that is all.
                self.version = doc.version;
                return true;
            }
            // Every line the old layout knew about is unchanged and the document has
            // grown: the new lines are at the end.
            lo = n_old;
        }
        let hi = {
            let mut chars = doc.chars_rev().peekable();
            let mut found = lo;
            for (i, line) in self.lines.iter().enumerate().rev() {
                // The line break in front of this line, if the document has one there:
                // a trailing newline in the document makes the last line empty, and the
                // walk has to step over the break to reach the line before it.
                match chars.peek() {
                    Some('\n') => {
                        chars.next();
                    }
                    Some('\r') => {
                        chars.next();
                        chars.next();
                    }
                    _ => {}
                }
                let mut same = true;
                for &c in line.chars.iter().rev() {
                    if chars.next() != Some(c) {
                        same = false;
                        break;
                    }
                }
                if !same {
                    found = i + 1;
                    break;
                }
                if i == 0 {
                    break;
                }
            }
            found.max(lo + 1)
        };
        // ---- the window: the change and a margin of context around it ----
        // Not the whole block: in a long file the "block" is often the entire document
        // (a log, a 20 000-row list, one enormous code block), and laying that out again
        // is exactly what this function exists to avoid. A margin of untouched lines is
        // enough context, because the layout of a line depends on its own text and on
        // the few lines around it - and whatever the margin cannot cover is caught
        // below by checking the rebuilt margin against the layout it replaces.
        const MARGIN: usize = 24;
        let kind = self.lines[lo.min(n_old - 1)].info.block;
        // Editing a row of a table is the one case the column hint cannot cover: a
        // column is as wide as the widest cell in it, so a cell that grew or shrank
        // moves every row in the table - including the ones outside the window. A window
        // that only *passes over* a table (a paragraph edited below one) is fine: the
        // columns it is handed are the columns the table already has.
        if self.lines[lo.min(n_old - 1)..hi.min(n_old)]
            .iter()
            .any(|l| l.info.block == Block::Table)
        {
            return false;
        }
        let mut wlo = lo.saturating_sub(MARGIN);
        let mut whi = (hi + MARGIN).min(n_old);
        // A code block ends with a fence line that belongs to the same card: keep it in
        // the window so the last code line knows it is not the last of its run.
        if matches!(kind, Block::Code | Block::Fence) {
            while whi < n_old && self.lines[whi].info.block == Block::Fence {
                whi += 1;
            }
        }
        // A table or a mermaid block cannot be cut in half: the first (its column widths
        // come from every row) would come out wrong, the second (its picture is one
        // object) would not be recognised at all. Widen over anything the window touches
        // at its edges, and give up if that makes the window the whole document.
        loop {
            let mut grew = false;
            for (a, b) in &self.mermaid {
                if *a < whi && *b >= wlo {
                    if *a < wlo {
                        wlo = *a;
                        grew = true;
                    }
                    if *b + 1 > whi {
                        whi = (*b + 1).min(n_old);
                        grew = true;
                    }
                }
            }
            if !grew {
                break;
            }
        }
        // A window this size is not worth patching: it would cost what the whole
        // rebuild costs.
        const WINDOW_CAP: usize = 1500;
        if whi - wlo > WINDOW_CAP {
            return false;
        }

        // The window in the new document: the same start (everything above the change is
        // identical) and an end that the edit may have moved.
        let end = (whi as isize + delta) as usize;
        // Lines for the throwaway document: one before the window and one after it, so
        // that "is this the first/last line of its run" and the spacing that hangs off
        // that answer come out as they do in the whole document.
        // Is the window itself inside a code block? Asked of the window's first line: the
        // changed line may sit after the block's closing fence, while the lines that give
        // the window its context are still code.
        let in_code = matches!(self.lines[wlo.min(n_old - 1)].info.block, Block::Code | Block::Fence);
        let lead_lines = usize::from(wlo > 0);
        let tail_lines = end < n;
        // The line after the window is laid out too, not just carried along as context:
        // its own spacing hangs off the line above it, and the edit may well have changed
        // that line's block. Laying it out again is one line of work; leaving it stale
        // would put the whole tail out of step.
        let tail_in_window = tail_lines;
        let mut body = String::new();
        if wlo > 0 {
            // Inside a code block a copied line would not be code: the fence opener is
            // what makes the lines below it code, and it starts the run correctly too.
            if in_code {
                body.push_str(&format!("```{}\n", self.lines[wlo].info.lang));
            } else {
                body.push_str(&doc.line_text(wlo - 1));
                body.push('\n');
            }
        }
        for i in wlo..end {
            body.push_str(&doc.line_text(i));
            body.push('\n');
        }
        if tail_lines {
            // The line *after* the window, in the new document - the window's end in old
            // coordinates plus what the edit moved it by. It is laid out and kept (see
            // `take`), and the line after that one follows as bare context, because the
            // spacing under a line is decided partly by what comes next.
            body.push_str(&doc.line_text(end));
            body.push('\n');
            if end + 1 < n {
                body.push_str(&doc.line_text(end + 1));
                body.push('\n');
            }
        }
        // A window that reaches the end of the document must end the same way the
        // document does: without a line break after its last line, so that the last
        // line's spacing comes out as it does in the whole document.
        if !tail_lines && body.ends_with('\n') {
            body.pop();
        }
        let sub_doc = Doc::new(&body, None);
        let sub_editing = editing.and_then(|e| {
            if e >= wlo && e < whi {
                Some(e - wlo + lead_lines)
            } else {
                None
            }
        });
        // A table's column widths come from every row in it, so a window of rows
        // cannot measure them for itself: it is handed the widths that are already on
        // screen. If the edit changed a column after all, the check below sees the
        // difference and the table is built again from the top.
        let table_hint = self
            .lines
            .get(wlo)
            .filter(|l| !l.table_cols.is_empty())
            .map(|l| (l.table_cols.clone(), l.table_width));
        let sub = Layout::build_with_table(
            &sub_doc,
            te,
            theme,
            width,
            mode,
            sub_editing,
            table_hint.as_ref().map(|(c, w)| (c.as_slice(), *w)),
            wrap,
        );
        let sub_headings = sub.headings.clone();
        let sub_mermaid = sub.mermaid.clone();
        let mut fresh = sub.lines;
        // Drop the context lines that were only there to give the window its surroundings.
        if end <= wlo || end > n {
            return false;
        }
        let skip = lead_lines;
        let take = end - wlo + usize::from(tail_in_window);
        if fresh.len() < skip + take {
            return false;
        }
        fresh.truncate(skip + take);
        fresh.drain(0..skip);

        // A fence is the one thing in markdown whose effect runs past the end of the
        // window: opening or closing a code block changes every line below it, and those
        // lines are not laid out here. So an edit that touches a fence gets the whole
        // rebuild it deserves.
        if self.lines[lo.min(n_old - 1)..hi.min(n_old)].iter().any(|l| l.info.block == Block::Fence) {
            return false;
        }
        if fresh
            .iter()
            .skip(lo.saturating_sub(wlo))
            .any(|l| l.info.block == Block::Fence)
        {
            return false;
        }

        // Inside a code block the window is begun again with a fence of its own, so the
        // first line of the window comes out of that throwaway document looking like the
        // first line of a block, which in the real document it is not. The spacing above
        // it belongs to a line that did not change, so it is taken from there.
        if in_code && wlo > 0 {
            fresh[0].space_before = self.lines[wlo].space_before;
        }

        // ---- put it back: y starts where the line above ends ----
        let base = if wlo == 0 {
            theme::PAD_TOP
        } else {
            self.lines[wlo - 1].top + self.lines[wlo - 1].height
        };
        let offset = (base + fresh[0].space_before) - fresh[0].top;
        for (k, l) in fresh.iter_mut().enumerate() {
            let i = wlo + k;
            l.line = i;
            l.start_char = doc.line_start(i);
            l.top += offset;
        }

        // ---- check the rebuild against what it replaces ----
        // Every line in the window that did not change must come out of the rebuild
        // exactly as it was: the same runs, the same geometry, the same place. If one
        // of them does not, the window was not self-contained (a paragraph whose
        // emphasis ran in from above the window, a table's column widths, a mermaid
        // block cut in half) and the whole rebuild is the honest answer.
        for (k, l) in fresh.iter().enumerate() {
            let Some(old) = self.lines.get(wlo + k) else {
                continue;
            };
            if old.chars != l.chars {
                // A line the edit changed has nothing to compare - but if it is still the
                // same kind of line and only its fence language moved, the window lost the
                // parse state that produced it (a fence inside a code block is not a fence
                // to a document that starts there). That is the one case a fresh line can
                // look plausible and still be wrong, so it is checked too.
                if old.info.block == l.info.block
                    && matches!(l.info.block, Block::Code | Block::Fence)
                    && old.info.lang != l.info.lang
                {
                    return false;
                }
                continue;
            }
            // The line after the edit's window is re-laid out for its spacing, so its y
            // and its spacing are expected to follow from the lines above it.
            let moves = tail_in_window && k + 1 == fresh.len();
            let same_geometry = (moves || close(old.top, l.top))
                && close(old.height, l.height)
                && (moves || close(old.space_before, l.space_before))
                && (moves || close(old.space_after, l.space_after))
                && close(old.width, l.width)
                && old.info.block == l.info.block
                && old.info.lang == l.info.lang
                && old.rows.len() == l.rows.len()
                && old.table_cols.len() == l.table_cols.len();
            let same_runs = old.clean.len() == l.clean.len()
                && old
                    .clean
                    .iter()
                    .zip(l.clean.iter())
                    .all(|(a, b)| a.start == b.start && close(a.w, b.w));
            if !same_geometry || !same_runs {
                return false;
            }
        }

        // The part of the old layout this patched window replaces, tail line included.
        let old_end = (whi + usize::from(tail_in_window)).min(n_old);
        let old_end_y = self.lines[old_end - 1].top + self.lines[old_end - 1].height;
        let new_end_y = fresh.last().map(|l| l.top + l.height).unwrap_or(old_end_y);
        let dy = new_end_y - old_end_y;

        // ---- the bookkeeping that spans lines ----
        let old_words: usize = self.lines[wlo..old_end].iter().map(|l| count_words(&l.chars)).sum();
        let new_words: usize = fresh.iter().map(|l| count_words(&l.chars)).sum();
        self.word_count = (self.word_count + new_words).saturating_sub(old_words);

        self.headings.retain(|(l, _, _)| *l < wlo || *l >= old_end);
        for h in &mut self.headings {
            if h.0 >= old_end {
                h.0 = (h.0 as isize + delta) as usize;
            }
        }
        let head_shift = wlo as isize - isize::from(lead_lines > 0);
        for (l, level, text) in sub_headings {
            let abs = l as isize + head_shift;
            if abs >= 0 && (abs as usize) >= wlo && (abs as usize) <= end {
                self.headings.push((abs as usize, level, text));
            }
        }
        self.headings.sort_by_key(|(l, _, _)| *l);

        self.mermaid.retain(|(a, b)| *b < wlo || *a >= old_end);
        for m in &mut self.mermaid {
            if m.0 >= old_end {
                m.0 = (m.0 as isize + delta) as usize;
                m.1 = (m.1 as isize + delta) as usize;
            }
        }
        let m_shift = wlo as isize - isize::from(lead_lines > 0);
        for (a, b) in sub_mermaid {
            let (a, b) = (a as isize + m_shift, b as isize + m_shift);
            if a >= 0 && b >= 0 && (a as usize) >= wlo && (a as usize) <= end {
                self.mermaid.push((a as usize, b as usize));
            }
        }
        self.mermaid.sort_by_key(|(a, _)| *a);

        self.lines.splice(wlo..old_end, fresh);

        // Everything below the window keeps its own spacing; its y moves, and its
        // characters (and its number) start somewhere else as soon as the edit was not
        // one-for-one.
        if end + 1 < n {
            // One look-up for the whole tail: every line below the window moves by the
            // same number of characters.
            let shift = doc.line_start(end + 1) as isize - self.lines[end + 1].start_char as isize;
            for (k, l) in self.lines[end + 1..].iter_mut().enumerate() {
                l.top += dy;
                l.start_char = (l.start_char as isize + shift) as usize;
                l.line = end + 1 + k;
            }
        }

        // The maxima are cheaper to recompute than to track per block.
        self.max_width = 0.0;
        self.max_row_width = 0.0;
        self.virtual_rows = 0.0;
        self.widest_row_chars = 8;
        for l in &self.lines {
            self.max_width = self.max_width.max(l.width + l.indent);
            self.max_row_width = self.max_row_width.max(l.row_w);
            self.virtual_rows += l.virtual_span();
            for r in &l.rows {
                self.widest_row_chars = self.widest_row_chars.max(r.end.saturating_sub(r.start));
            }
        }
        if let Some(last) = self.lines.last() {
            self.total_height = last.top + last.height + last.space_after + theme::PAD_BOTTOM;
        }
        self.version = doc.version;
        true
    }

    /// The ```` ```mermaid ```` block containing `line`, as (open, close).
    pub fn mermaid_block_at(&self, line: usize) -> Option<(usize, usize)> {
        self.mermaid.iter().copied().find(|(a, b)| line >= *a && line <= *b)
    }

    /// Index of the line whose vertical span contains `y`, or the closest one.
    pub fn line_at_y(&self, y: f32) -> usize {
        if self.lines.is_empty() {
            return 0;
        }
        let mut lo = 0usize;
        let mut hi = self.lines.len() - 1;
        while lo < hi {
            let mid = (lo + hi + 1) / 2;
            if self.lines[mid].top <= y {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        lo
    }

    /// First visible line for a scroll offset and viewport height.
    pub fn visible_range(&self, scroll: f32, height: f32) -> (usize, usize) {
        if self.lines.is_empty() {
            return (0, 0);
        }
        let first = self.line_at_y(scroll.max(0.0));
        let last = self.line_at_y(scroll.max(0.0) + height).min(self.lines.len() - 1);
        (first, last)
    }

    pub fn line_of_char(&self, ch: usize) -> usize {
        match self.lines.binary_search_by(|l| {
            if ch < l.start_char {
                std::cmp::Ordering::Greater
            } else if ch > l.end_char() {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        }) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1).min(self.lines.len().saturating_sub(1)),
        }
    }

    /// Current heading section for a caret position (used by the outline).
    pub fn active_heading(&self, ch: usize) -> Option<usize> {
        let line = self.line_of_char(ch);
        let mut best = None;
        for (i, (h_line, _, _)) in self.headings.iter().enumerate() {
            if *h_line <= line {
                best = Some(i);
            } else {
                break;
            }
        }
        best
    }

    pub fn build(doc: &Doc, te: &mut TextEngine, theme: &Theme, width: f32) -> Layout {
        Layout::build_mode(doc, te, theme, width, ViewMode::Live)
    }

    pub fn build_mode(
        doc: &Doc,
        te: &mut TextEngine,
        theme: &Theme,
        width: f32,
        mode: ViewMode,
    ) -> Layout {
        Layout::build_editing(doc, te, theme, width, mode, None, true)
    }

    /// Build with knowledge of the caret: `editing` is the opening fence line of
    /// the mermaid block whose source is being typed in, so that block shows its
    /// code instead of its picture. Every other mermaid block stays a diagram,
    /// which keeps the page from breathing in and out as the caret moves.
    pub fn build_editing(
        doc: &Doc,
        te: &mut TextEngine,
        theme: &Theme,
        width: f32,
        mode: ViewMode,
        editing: Option<usize>,
        wrap: bool,
    ) -> Layout {
        Layout::build_with_table(doc, te, theme, width, mode, editing, None, wrap)
    }

    /// The same build, told what the columns of the document's tables were the last
    /// time they were measured. A table's column widths come from every row in it, so
    /// laying out a window of rows on its own would guess them; handing over the
    /// widths that are already on screen lets a window inside a twenty-thousand-row
    /// table be rebuilt without re-measuring the table.
    fn build_with_table(
        doc: &Doc,
        te: &mut TextEngine,
        theme: &Theme,
        width: f32,
        mode: ViewMode,
        editing: Option<usize>,
        table_hint: Option<(&[f32], f32)>,
        wrap: bool,
    ) -> Layout {
        let n_lines = doc.len_lines();
        let font_size = theme.font_size;
        let plain = mode.plain();

        // ---- pass 1: parse every line (fence state has to travel downwards) ----
        struct Parsed {
            chars: Vec<char>,
            info: LineInfo,
            flags: Vec<Flags>,
            /// Runs built once in pass 1: the caret moves without touching the
            /// text, so rebuilding every line on every keystroke was wasted work.
            clean: Vec<Run>,
            raw: Vec<Run>,
            x_clean: f32,
            x_raw: f32,
        }
        let mut parsed: Vec<Parsed> = Vec::with_capacity(n_lines);
        {
            let mut fenced: Option<String> = None;
            for line in 0..n_lines {
                let chars = doc.line_chars(line);
                let info = markdown::parse(&chars, &mut fenced);
                // Runs are built *after* the join pass below: inline markup that
                // opens on this line and closes on the next only exists once the
                // paragraph has been scanned as one text.
                parsed.push(Parsed {
                    chars,
                    info,
                    flags: Vec::new(),
                    clean: Vec::new(),
                    raw: Vec::new(),
                    x_clean: 0.0,
                    x_raw: 0.0,
                });
            }
        }

        // ---- pass 2: inline markup that spans a soft line break ----
        //
        // `**bold` on one source line and `text**` on the next is **one span**:
        // a paragraph is a block, and a line break inside it is only a break on
        // screen. The scanner (`markdown::parse_inline`) is given one *line* at a
        // time, so it can never see a pair of markers across a newline, and the
        // emphasis was simply lost - bold, italic, strike, highlight, underline,
        // code, colours and links all behaved that way. Each paragraph is joined,
        // scanned once as a single text, and the spans and marker ranges are cut
        // back into per-line ranges.
        //
        // A span that opens on one line and closes on a later one covers the first
        // line to its end and the last from its start; each line keeps **its own**
        // half of the markers, so the caret line still reveals the two characters
        // it is sitting next to and nothing else.
        if !plain {
            /// Join the lines of one block, scan them as one text, and put the
            /// resulting ranges back on the lines they belong to.
            fn join_inline(block: &mut [Parsed]) {
                use crate::markdown::{LineInfo, Span};
                let mut text: Vec<char> = Vec::new();
                // Per line: (start in the joined text, first content char in the
                // line, number of content chars).
                let mut map: Vec<(usize, usize, usize)> = Vec::with_capacity(block.len());
                for p in block.iter() {
                    let from = p.info.prefix.min(p.chars.len());
                    map.push((text.len(), from, p.chars.len() - from));
                    text.extend_from_slice(&p.chars[from..]);
                    // A newline no marker can be part of, so an opener on one line
                    // and a closer on the next are seen as a pair.
                    text.push('\n');
                }
                let mut probe = LineInfo::default();
                markdown::parse_inline(&text, 0, text.len(), &mut probe, 0);
                if probe.spans.is_empty() && probe.hides.is_empty() {
                    return;
                }

                for (k, p) in block.iter_mut().enumerate() {
                    let (j0, from, len) = map[k];
                    let j1 = j0 + len;
                    for span in &probe.spans {
                        let (s, e) = (span.start.max(j0), span.end.min(j1));
                        if s < e {
                            p.info.spans.push(Span {
                                start: from + (s - j0),
                                end: from + (e - j0),
                                kind: span.kind,
                                color: span.color,
                            });
                        }
                    }
                    for &(hs, he) in &probe.hides {
                        let (s, e) = (hs.max(j0), he.min(j1));
                        if s < e {
                            p.info.hides.push((from + (s - j0), from + (e - j0)));
                        }
                    }
                    // Everything that fits on one line was already found by the
                    // per-line scan, so ranges arrive twice. Both readers are
                    // "any of these" (`is_marker`, `flags`), but a tidy list keeps
                    // the marker reveal and the hit test easy to reason about.
                    p.info.hides.sort_unstable();
                    p.info.hides.dedup();
                    p.info.spans.sort_by_key(|s| (s.start, s.end));
                    let mut unique: Vec<Span> = Vec::with_capacity(p.info.spans.len());
                    for span in p.info.spans.drain(..) {
                        let seen = unique.iter().any(|o| {
                            o.start == span.start
                                && o.end == span.end
                                && o.color == span.color
                                && std::mem::discriminant(&o.kind)
                                    == std::mem::discriminant(&span.kind)
                        });
                        if !seen {
                            unique.push(span);
                        }
                    }
                    p.info.spans = unique;
                }
            }

            // A block that inline markup may span: a paragraph, the body of a list
            // item or a quote. Never a table row, a code block or a heading - those
            // keep exactly the per-line behaviour they had - and never across a
            // blank line, which is a new paragraph in Markdown.
            let is_text = |b: Block| {
                matches!(
                    b,
                    Block::Paragraph | Block::Bullet | Block::Ordered | Block::Task(_) | Block::Quote
                )
            };
            let mut i = 0usize;
            while i < n_lines {
                if !is_text(parsed[i].info.block) {
                    i += 1;
                    continue;
                }
                let kind = std::mem::discriminant(&parsed[i].info.block);
                let mut j = i;
                while j + 1 < n_lines
                    && std::mem::discriminant(&parsed[j + 1].info.block) == kind
                {
                    j += 1;
                }
                if j > i {
                    join_inline(&mut parsed[i..=j]);
                }
                i = j + 1;
            }
        }

        // ---- pass 3: styles, runs, syntax highlighting ----
        // The syntax-highlighting state travels downwards with the fence state: a
        // comment opened on one line of a code block is still a comment on the
        // next.
        {
            let mut hl_state = crate::highlight::State::default();
            let mut hl_lang: Option<String> = None;
            for i in 0..n_lines {
                let (flags, mut clean, mut raw, x_clean, x_raw) = {
                    let p = &parsed[i];
                    let flags = markdown::flags(&p.info, p.chars.len());
                    let style = if plain {
                        plain_style(theme)
                    } else {
                        base_style(&p.info, theme, font_size)
                    };
                    let (clean, raw, x_clean, x_raw) = build_runs(
                        &p.chars,
                        &p.info,
                        &flags,
                        0,
                        p.chars.len(),
                        0.0,
                        style,
                        theme,
                        te,
                        plain,
                        false,
                    );
                    (flags, clean, raw, x_clean, x_raw)
                };
                // ---- syntax highlighting ----
                // Only inside a fenced block that names a language the highlighter
                // knows, and only in the rendered views (Raw is the source, and
                // colouring it would be a lie about the file).
                let info = &parsed[i].info;
                if !plain && info.block == Block::Code && !info.lang.is_empty() {
                    if hl_lang.as_deref() != Some(info.lang.as_str()) {
                        hl_state = crate::highlight::State::default();
                        hl_lang = Some(info.lang.clone());
                    }
                    let classes = crate::highlight::line(&info.lang, &parsed[i].chars, &mut hl_state);
                    clean = tint_runs(clean, &classes, theme);
                    // The source view of the caret's own line keeps its colours
                    // too: a colour is not a marker, and a code block that went
                    // plain the moment the caret entered it looked like the
                    // highlighting had stopped working.
                    raw = tint_runs(raw, &classes, theme);
                } else if info.block != Block::Code {
                    // A new block starts from a clean state - including at a blank line,
                    // which is where a block ends and where an incremental rebuild can
                    // start, so the state has to be the default one there.
                    if hl_lang.is_some() {
                        hl_state = crate::highlight::State::default();
                        hl_lang = None;
                    }
                }
                let p = &mut parsed[i];
                p.flags = flags;
                p.clean = clean;
                p.raw = raw;
                p.x_clean = x_clean;
                p.x_raw = x_raw;
            }
        }

        // A blank line *between* two indented-code lines belongs to the block:
        // Markdown keeps indented code going across empty lines, and treating the
        // gap as a paragraph break split one block into two cards.
        {
            // Pre-computed so the loop can mutate the block kinds below.
            let indented: Vec<bool> = parsed
                .iter()
                .map(|p| p.info.block == Block::Code && p.info.lang.is_empty())
                .collect();
            let is_code = |i: usize| indented[i];
            let mut i = 0usize;
            while i < n_lines {
                if parsed[i].info.block != Block::Blank {
                    i += 1;
                    continue;
                }
                let mut j = i;
                while j < n_lines && parsed[j].info.block == Block::Blank {
                    j += 1;
                }
                let before = i > 0 && is_code(i - 1);
                let after = j < n_lines && is_code(j);
                if before && after {
                    for k in i..j {
                        parsed[k].info.block = Block::Code;
                        parsed[k].info.lang = String::new();
                    }
                }
                i = j;
            }
        }

        // ---- table column geometry, computed once per contiguous table block ----
        let mut table_geom: Vec<Option<(Vec<f32>, f32)>> = vec![None; n_lines];
        {
            let mut k = 0usize;
            while k < n_lines && mode == ViewMode::Live {
                if parsed[k].info.block != Block::Table {
                    k += 1;
                    continue;
                }
                let start = k;
                while k < n_lines && parsed[k].info.block == Block::Table {
                    k += 1;
                }
                // Widest cell per column over the whole block, measured from the
                // cell's own characters and in *both* variants the row can be
                // painted in (markers hidden, and markers revealed - see below).
                // The column count comes from the *pipes*, not from the cells that
                // happen to hold text: a freshly inserted column is empty, and
                // sizing only the non-empty ones made "+ column" change the source
                // while the painted grid stayed exactly as it was.
                let ncols = parsed[start..k]
                    .iter()
                    .filter(|p| !p.info.table_sep)
                    .map(|p| p.info.cells.len())
                    .max()
                    .unwrap_or(0);
                // A hint that does not have a column for every pipe is no hint.
                if let Some((cols, total)) = table_hint.filter(|(c, _)| c.len() == ncols) {
                    let (cols, total) = (cols.to_vec(), total);
                    for slot in table_geom.iter_mut().take(k).skip(start) {
                        *slot = Some((cols.clone(), total));
                    }
                    continue;
                }
                let mut widths: Vec<f32> = vec![0.0; ncols];
                for p in &parsed[start..k] {
                    if p.info.table_sep {
                        continue;
                    }
                    for (ci, &(a, b)) in p.info.cells.iter().enumerate() {
                        if ci >= widths.len() {
                            break;
                        }
                        // The cell reaches up to the next pipe, exactly like the
                        // cell the painter, the caret and the mouse see (see
                        // `table_cells`): measuring only up to the last visible
                        // character made a column change width the moment a space
                        // was typed at the end of it.
                        let mut b = b.min(p.chars.len());
                        while b < p.chars.len() && p.chars[b] != '|' {
                            b += 1;
                        }
                        if a >= b {
                            continue;
                        }
                        let rendered = p
                            .clean
                            .iter()
                            .filter(|r| r.end > a && r.start < b)
                            .fold(0.0f32, |acc, r| acc + r.w);
                        // A row shows its inline markers while the caret is on it
                        // (`table_shows_markers`), and the source of a cell is
                        // wider than the text it renders to by the width of every
                        // marker in it. A column measured from the rendered cell
                        // only was therefore too narrow the moment the caret
                        // entered the row: `**bold**` no longer fitted its column,
                        // `wrap_cell` broke it into three bands and the row grew
                        // and slid sideways - until the caret left the row again,
                        // which is what "the text is one symbol off while I type
                        // in a table, and it snaps right when I click elsewhere"
                        // was. A column is as wide as the wider of the two
                        // variants, so revealing a row never re-flows it.
                        let revealed = p.raw.iter().filter(|r| r.end > a && r.start < b).fold(
                            0.0f32,
                            |acc, r| {
                                // A run can stick out of the cell (a style that
                                // runs across a pipe); count the part inside.
                                let len = (r.end - r.start).max(1) as f32;
                                let inside = (r.end.min(b) - r.start.max(a)).max(0) as f32;
                                acc + r.w * inside / len
                            },
                        );
                        widths[ci] = widths[ci].max(rendered.max(revealed));
                    }
                }
                if widths.is_empty() {
                    continue;
                }
                for w in &mut widths {
                    *w = (*w).max(24.0);
                }
                // A table is as wide as the page, not as wide as its longest
                // cell: a single long cell used to push the whole grid past the
                // right edge of the editor, where the text ran out of its column
                // and off the sheet. Columns are shrunk (widest first) until the
                // grid fits, and the cell text wraps inside its own column.
                {
                    let n = widths.len() as f32;
                    let pads = n * TABLE_CELL_PAD * 2.0;
                    let budget = (width - 8.0 - TABLE_CELL_PAD - pads).max(60.0);
                    let sum: f32 = widths.iter().sum();
                    if sum > budget {
                        let floor = (budget / n).clamp(28.0, 90.0);
                        let mut excess = sum - budget;
                        for _ in 0..6 {
                            if excess <= 0.5 {
                                break;
                            }
                            let shrinkable: f32 =
                                widths.iter().map(|w| (w - floor).max(0.0)).sum();
                            if shrinkable <= 0.5 {
                                break;
                            }
                            let take = excess.min(shrinkable);
                            let scale = take / shrinkable;
                            for w in widths.iter_mut() {
                                let s = (*w - floor).max(0.0);
                                *w -= s * scale;
                            }
                            excess -= take;
                        }
                    }
                }
                let mut xs = Vec::with_capacity(widths.len());
                let mut x = TABLE_CELL_PAD;
                for w in &widths {
                    xs.push(x);
                    x += w + TABLE_CELL_PAD * 2.0;
                }
                for slot in table_geom.iter_mut().take(k).skip(start) {
                    *slot = Some((xs.clone(), x));
                }
            }
        }

        // Which lines start or end a run of the same card block (code, table,
        // quote)? Cards are drawn per run, and the spacing around them belongs to
        // the run, not to every line inside it.
        let mut first_in_run = vec![true; n_lines];
        let mut last_in_run = vec![true; n_lines];
        for i in 1..n_lines {
            let same = parsed[i].info.block == parsed[i - 1].info.block
                && matches!(
                    parsed[i].info.block,
                    Block::Code | Block::Fence | Block::Table | Block::Quote
                );
            if same {
                first_in_run[i] = false;
                last_in_run[i - 1] = false;
            }
        }

        // ---- mermaid blocks ----
        // A ```` ```mermaid ```` fence is a picture, not code: find every block,
        // lay its source out once, and remember which line owns the drawing. The
        // block containing the caret keeps its source (see `build_editing`).
        struct MermaidBlock {
            open: usize,
            close: usize,
            diagram: Option<crate::mermaid::Diagram>,
            folded: bool,
        }
        let mut blocks: Vec<MermaidBlock> = Vec::new();
        let mut mermaid_of: Vec<Option<usize>> = vec![None; n_lines];
        if !plain {
            let is_mermaid = |i: usize| -> bool {
                let info = &parsed[i].info;
                matches!(info.block, Block::Fence | Block::Code)
                    && info
                        .lang
                        .split_whitespace()
                        .next()
                        .map(|l| l.eq_ignore_ascii_case("mermaid"))
                        .unwrap_or(false)
            };
            let mut i = 0usize;
            while i < n_lines {
                if !is_mermaid(i) {
                    i += 1;
                    continue;
                }
                // The opening fence is the first line of the run.
                let open = i;
                let mut j = open + 1;
                while j < n_lines && is_mermaid(j) {
                    j += 1;
                }
                // `j` is the closing fence when the block is closed, else the
                // first line after a still-open block.
                let close = if j < n_lines && parsed[j].info.block == Block::Fence {
                    j
                } else {
                    (j - 1).max(open)
                };
                let mut src = String::new();
                for line in (open + 1)..=close {
                    if parsed[line].info.block == Block::Fence {
                        continue;
                    }
                    if !src.is_empty() {
                        src.push('\n');
                    }
                    src.extend(parsed[line].chars.iter());
                }
                let diag = crate::mermaid::layout(&src, te, (width - 40.0).max(160.0), font_size);
                let folded = mode == ViewMode::Live
                    && diag.is_some()
                    && editing != Some(open);
                for line in open..=close {
                    if line >= n_lines {
                        break;
                    }
                    mermaid_of[line] = Some(blocks.len());
                }
                blocks.push(MermaidBlock { open, close, diagram: diag, folded });
                i = close + 1;
            }
        }

        // ---- pass 2: runs, wrapping and vertical metrics ----
        let mut lines = Vec::with_capacity(n_lines);
        let mut y = theme::PAD_TOP;
        let mut max_width = 0.0f32;
        let mut max_row_width = 0.0f32;
        let mut widest_row_chars = 8usize;
        let mut headings = Vec::new();
        let mut word_count = 0usize;
        let mut pending_after = 0.0f32;

        for (line, Parsed { chars, info, flags, clean, raw, x_clean, x_raw }) in
            parsed.into_iter().enumerate()
        {
            word_count += count_words(&chars);

            // ---- geometry per block kind ----
            let style = if plain {
                plain_style(theme)
            } else {
                base_style(&info, theme, font_size)
            };
            let indent = if plain { 0.0 } else { block_indent(&info, theme, font_size) };
            // The first row of a table block is the header: bold, like the rule
            // underneath it suggests.
            let header_row = info.block == Block::Table && !info.table_sep && first_in_run[line];
            let style = if header_row {
                BaseStyle { face: style.face.bold(), size: style.size, ink: Inks::Heading }
            } else {
                style
            };
            let base_face = style.face;
            let base_size = style.size;

            // ---- folded mermaid block ----
            // Only the opening fence survives: it carries the picture, and the
            // source lines below it take no room at all (they still exist, so
            // the caret, search and the outline keep their line numbers).
            let block = mermaid_of[line].map(|b| &blocks[b]);
            let folded_here = block.map(|b| b.folded).unwrap_or(false);
            let carries_diagram = folded_here && block.map(|b| b.open == line).unwrap_or(false);
            let diagram = if carries_diagram {
                block.and_then(|b| b.diagram.clone())
            } else {
                None
            };
            let folded_away = folded_here && !carries_diagram;

            let raw_runs = raw;
            // Table rows are re-laid out per cell so the painted grid, the caret
            // and the mouse all agree about where text lives.
            let is_table = info.block == Block::Table && mode == ViewMode::Live;
            let mut clean_runs = clean;
            let mut x_clean = x_clean;
            if folded_here {
                clean_runs.clear();
                x_clean = 0.0;
            }
            let mut table_cols: Vec<f32> = Vec::new();
            let mut table_width = 0.0f32;
            let mut table_rows: Vec<Vec<Run>> = Vec::new();
            let mut table_pos: Vec<(u16, f32)> = Vec::new();
            let mut table_src_rows: Vec<Vec<Run>> = Vec::new();
            let mut table_src_pos: Vec<(u16, f32)> = Vec::new();
            let mut table_images: Vec<(usize, ImageBox)> = Vec::new();
            let mut table_img_h = 0.0f32;
            if is_table {
                if let Some((xs, total)) = table_geom[line].clone() {
                    table_cols = xs.clone();
                    table_width = total;
                    x_clean = total;
                    clean_runs = Vec::new();
                    if !info.table_sep {
                        let rendered = table_cells(
                            &chars, &info, &flags, style, theme, te, plain, &xs, total, false,
                        );
                        clean_runs = rendered.runs;
                        table_rows = rendered.bands;
                        table_pos = rendered.pos;
                        // ---- pictures inside cells ----
                        // Resolved like a standalone image: the cell's rectangle is
                        // the column it lives in, so the picture is scaled to fit
                        // instead of pushing the grid sideways.
                        let dir = doc
                            .path
                            .as_ref()
                            .and_then(|p| p.parent())
                            .map(|d| d.to_path_buf());
                        for (ci, &(a, b)) in info.cells.iter().enumerate() {
                            let Some(src) = markdown::standalone_image(&chars, a, b) else {
                                continue;
                            };
                            let Some(&x) = xs.get(ci) else { continue };
                            let col_w = if ci + 1 < xs.len() {
                                (xs[ci + 1] - x - TABLE_CELL_PAD * 2.0).max(24.0)
                            } else {
                                (total - x - TABLE_CELL_PAD).max(24.0)
                            };
                            let path = dir
                                .as_ref()
                                .map(|d| crate::media::join(d, &src.src))
                                .unwrap_or_else(|| std::path::PathBuf::from(&src.src));
                            let size = crate::media::size_of(&path);
                            let (mut w, mut h) = match size {
                                Some((iw, ih)) if iw > 0 && ih > 0 => {
                                    // Fit the column, never enlarge: a small icon in a
                                    // cell should stay a small icon.
                                    let k = (col_w / iw as f32).min(1.0);
                                    (iw as f32 * k, ih as f32 * k)
                                }
                                _ => (col_w.min(180.0), 60.0),
                            };
                            // Never taller than a few rows: a 4K screenshot in a
                            // cell would otherwise swallow the table.
                            let cap = te.line_height(base_face, base_size) * 6.0;
                            if h > cap {
                                let k = cap / h;
                                w *= k;
                                h *= k;
                            }
                            table_img_h = table_img_h.max(h);
                            table_images.push((
                                ci,
                                ImageBox {
                                    path,
                                    alt: src.alt.clone(),
                                    w,
                                    h,
                                    missing: size.is_none(),
                                },
                            ));
                        }
                        // The row the caret is on shows what you would type. It is
                        // only worth building when the row has inline markers at
                        // all: a table of plain words (the common case) has none.
                        let has_inline = info
                            .hides
                            .iter()
                            .any(|&(a, b)| !(b == a + 1 && markdown::is_pipe(&info, a)));
                        if has_inline {
                            let src = table_cells(
                                &chars, &info, &flags, style, theme, te, plain, &xs, total, true,
                            );
                            table_src_rows = src.bands;
                            table_src_pos = src.pos;
                        }
                    }
                }
            }

            // ---- picture ----
            // A standalone image reserves a real box on the page: the layout has
            // to know its height before the painter sees it, or every scroll
            // position after it would be wrong.
            let mut image: Option<ImageBox> = None;
            if let (Some(src), false) = (info.image.clone(), plain) {
                let path = doc
                    .path
                    .as_ref()
                    .and_then(|p| p.parent())
                    .map(|dir| crate::media::join(dir, &src.src));
                let size = path.as_ref().and_then(|p| crate::media::size_of(p));
                let avail = (width - indent).max(80.0);
                let (w, h) = match size {
                    Some((iw, ih)) if iw > 0 && ih > 0 => {
                        let k = (avail / iw as f32).min(1.0);
                        let mut w = iw as f32 * k;
                        let mut h = ih as f32 * k;
                        // Never let one screenshot swallow the page.
                        let max_h = (avail * 1.3).max(160.0);
                        if h > max_h {
                            let k2 = max_h / h;
                            w *= k2;
                            h *= k2;
                        }
                        // Very small pictures (icons, 1-pixel rules) are scaled
                        // up to something visible, keeping the aspect ratio.
                        let small = (24.0 / w.max(1.0)).max(24.0 / h.max(1.0)).max(1.0);
                        (w * small, h * small)
                    }
                    _ => (avail.min(360.0), 96.0),
                };
                image = Some(ImageBox {
                    path: path.unwrap_or_else(|| std::path::PathBuf::from(&src.src)),
                    alt: src.alt.clone(),
                    w,
                    h,
                    missing: size.is_none(),
                });
            }

            // ---- soft wrap ----
            // A right-to-left paragraph is indented from the *right*, mirroring the
            // left indent of a list or a quote, so it has that much less room.
            let base_dir = visible_base(&chars, &info);
            let rtl_para = base_dir == Some(1);
            let gutters = if rtl_para { indent * 2.0 } else { indent };
            // Soft wrap off: one row per line, however wide it grows. The line is
            // still laid out for the *view* width (that is what centred and
            // right-aligned text, pictures and table columns measure against), only
            // the break is not taken; the view then scrolls sideways to reach the
            // end of the row.
            let wrap_w = if wrap {
                (width - gutters).max(120.0)
            } else {
                f32::INFINITY
            };
            let (rows, row_width) = if image.is_some() {
                (
                    vec![WrapRow { start: 0, end: chars.len() }],
                    image.as_ref().map(|i| i.w).unwrap_or(0.0),
                )
            } else if is_table {
                // One visual row per band: a cell that wraps makes the whole row
                // taller, and the ruler lines follow it. Both variants are counted,
                // so revealing a row's markers never paints bands on top of each
                // other (nor makes the table jump by a line).
                let bands = table_rows.len().max(table_src_rows.len()).max(1);
                (
                    (0..bands)
                        .map(|_| WrapRow { start: 0, end: chars.len() })
                        .collect::<Vec<_>>(),
                    table_width,
                )
            } else if plain {
                compute_rows(&chars, &info, &raw_runs, wrap_w, te, true)
            } else {
                compute_rows(&chars, &info, &clean_runs, wrap_w, te, false)
            };

            // ---- vertical metrics ----
            let lh = te.line_height(base_face, base_size);
            // A blank row is exactly as tall as a row of text. It used to be a
            // fraction of one (`0.9` in plain mode, `0.62` rendered), which saved a
            // little air between paragraphs and cost the caret: the caret is as tall
            // as its row, so it shrank and climbed towards the row above as soon as
            // the line had nothing on it - "pointer size is small and position is
            // closer to upper row; when I type 1 symbol it becomes bigger and
            // offsets lower". One empty line has to measure like one line of text
            // for the caret to stay put.
            let row_h = if plain {
                lh * 1.28
            } else {
                match info.block {
                Block::Blank => lh * 1.22,
                Block::Table if info.table_sep && !mode.shows_markers() => TABLE_RULE_H,
                // A rule is a single line of source while the caret is in it, so
                // it needs room for the text the moment it is being edited. The
                // rule itself is drawn in the middle of that box (and not at all
                // while the line shows its source), which is why the height is a
                // whole row instead of the old 10 px sliver: the sliver left the
                // `---` overlapping the paragraph above it.
                Block::Hr => lh * 1.1,
                Block::Heading(level) => match level {
                    1 => lh * 1.2,
                    2 => lh * 1.2,
                    3 => lh * 1.18,
                    _ => lh * 1.16,
                },
                // A fence row is a real line of source: the backticks are shown
                // whenever the caret is on it, so it takes exactly the room a line
                // of code takes. It used to be 0.62 of a row, which left the
                // markers sticking out of their own line and clipped the caret to
                // half its height - the "cut in half, or just tiny" report.
                Block::Code | Block::Fence => lh * 1.32,
                Block::Quote => (lh * 1.16).max(font_size * 1.3),
                Block::Bullet | Block::Ordered | Block::Task(_) => lh * 1.16,
                Block::Table => lh * 1.22,
                _ => lh * 1.22,
                }
            };
            let height = if let Some(d) = &diagram {
                d.h
            } else if folded_away {
                0.0
            } else {
                match &image {
                    Some(img) => (row_h + img.h + IMAGE_GAP).max(row_h),
                    None => {
                        let text_h = row_h * rows.len().max(1) as f32;
                        // A picture in a cell makes the whole row as tall as the
                        // picture, with a little air above and below it. Only when
                        // there is one: a row of a table without pictures keeps its
                        // text height exactly.
                        if table_img_h > 0.0 {
                            text_h.max(table_img_h + TABLE_CELL_PAD)
                        } else {
                            text_h
                        }
                    }
                }
            };

            // ---- space around the line ----
            // Everything vertical is expressed in "text units" and scaled with
            // the font, so the rhythm survives zooming in and out.
            let vs = vertical_scale(font_size);
            let pad = if plain { 0.0 } else { block_pad(info.block) * vs };
            let mut space_before = if plain {
                0.0
            } else {
                (match info.block {
                // A blank line carries the same air above it as a paragraph: an
                // empty line is an empty paragraph, and typing the first character
                // into it must not move the line (nor the caret) down by the
                // paragraph's space-before.
                Block::Blank => 6.0,
                Block::Hr => 6.0,
                Block::Heading(level) => match level {
                    1 => 22.0,
                    2 => 18.0,
                    3 => 14.0,
                    4 => 10.0,
                    5 => 8.0,
                    _ => 6.0,
                },
                Block::Code | Block::Fence | Block::Table | Block::Quote => {
                    if first_in_run[line] {
                        pad
                    } else {
                        0.0
                    }
                }
                Block::Bullet | Block::Ordered | Block::Task(_) => 2.0,
                _ => 6.0,
                }) * vs
            };
            // Space owed by the block that ended on the previous line.
            space_before = space_before.max(pending_after);
            let mut space_after = if last_in_run[line] { pad } else { 0.0 };
            if folded_here {
                // A folded mermaid block is one card: the picture owns the space
                // above and below it, and the swallowed source lines own none -
                // otherwise every hidden line would leave a hole in the page.
                if carries_diagram {
                    space_after = pad;
                } else {
                    space_before = 0.0;
                    space_after = 0.0;
                }
            }
            pending_after = space_after;
            let mut width_line = x_clean.max(x_raw);
            if let Some(img) = &image {
                width_line = width_line.max(img.w);
            }
            if let Some(d) = &diagram {
                width_line = width_line.max(d.w);
            }
            if folded_away {
                width_line = 0.0;
            }
            let row_w = row_width + indent;
            max_width = max_width.max(width_line + indent);
            for r in &rows {
                widest_row_chars = widest_row_chars.max(r.end.saturating_sub(r.start));
            }
            // After soft wrapping a row is normally `<= width`, so the horizontal
            // scrollbar stays hidden unless a single token is wider than the page.
            max_row_width = max_row_width.max(row_w);

            if let Block::Heading(level) = info.block {
                let mut t: String = chars[info.prefix.min(chars.len())..].iter().collect();
                t = t.trim().trim_end_matches('#').trim().to_string();
                if !t.is_empty() {
                    headings.push((line, level, t));
                }
            }

            let max_size = base_size.max(clean_runs.iter().map(|r| r.style.size).fold(0.0f32, f32::max));
            // ---- bidirectional text ----
            // Only lines that actually contain Hebrew/Arabic (or a bidi control)
            // pay for this; everything else keeps the plain left-to-right runs it
            // has always had. A table row is left alone: its cells are laid out
            // per column by their own code.
            let bidi = if !is_table
                && image.is_none()
                && crate::text::bidi::may_be_rtl(&chars)
            {
                let levels = crate::text::bidi::analyse_with_base(&chars, base_dir);
                if levels.has_rtl {
                    Some(bidi_line(
                        &chars, &levels, &clean_runs, &raw_runs, &rows, te,
                    ))
                } else {
                    None
                }
            } else {
                None
            };
            lines.push(LineLayout {
                line,
                start_char: doc.line_start(line),
                chars,
                clean: clean_runs,
                raw: raw_runs,
                rows,
                row_h,
                max_size,
                height,
                space_before,
                space_after,
                indent,
                top: y + space_before,
                width: width_line,
                row_w,
                info,
                flags,
                table_cols,
                table_rows,
                table_pos,
                table_src_rows,
                table_src_pos,
                table_images,
                table_width,
                image,
                diagram,
                folded: folded_here,
                bidi,
            });
            y += space_before + height;
        }
        y += pending_after;


        let virtual_rows: f32 = lines.iter().map(|l| l.virtual_span()).sum();
        Layout {
            total_height: y + theme::PAD_BOTTOM,
            lines,
            max_row_width,
            max_width,
            virtual_rows,
            widest_row_chars,
            wrap,
            width,
            font_size,
            version: doc.version,
            headings,
            word_count,
            mode,
            mermaid: blocks.iter().map(|b| (b.open, b.close)).collect(),
            editing,
        }
    }
}

/// Words in one line: one per run of non-whitespace.
/// Two lengths are the same length if they differ by less than a thousandth of their
/// own size (plus a hair), which is what f32 accumulation over twenty thousand lines
/// costs. Anything larger is a real difference and the caller rebuilds.
fn close(a: f32, b: f32) -> bool {
    let scale = a.abs().max(b.abs());
    (a - b).abs() <= 0.01 + scale * 1e-6
}

fn count_words(chars: &[char]) -> usize {
    chars
        .iter()
        .fold((0usize, false), |(n, in_word), c| {
            if c.is_whitespace() {
                (n, false)
            } else {
                (if in_word { n } else { n + 1 }, true)
            }
        })
        .0
}

/// Face/size/ink a line uses before inline styling.
/// Left inset of a line, from its block kind and list depth.
fn block_indent(info: &LineInfo, _theme: &Theme, _font_size: f32) -> f32 {
    match info.block {
        // room for the card plus its inner padding
        Block::Code | Block::Fence => CODE_PAD_X,
        Block::Table => TABLE_CELL_PAD,
        // one gutter per nesting level, so a nested bar lines up inside its parent
        // A quoted list item needs the gutter its bullet is drawn in.
        Block::Quote => {
            info.depth.max(1) as f32 * QUOTE_INDENT + QUOTE_GAP - QUOTE_INDENT
                + if info.quote_bullet { QUOTED_BULLET_ROOM } else { 0.0 }
        }
        // A checkbox is wider than a bullet, so a task needs a bigger gutter to
        // keep the text from touching the box.
        Block::Task(_) => info.depth as f32 * 22.0 + 30.0,
        Block::Bullet | Block::Ordered => info.depth as f32 * 22.0 + 20.0,
        _ => 0.0,
    }
}

/// Base style for Raw mode: everything is source text, so nothing changes size,
/// weight or colour.
fn plain_style(theme: &Theme) -> BaseStyle {
    BaseStyle { face: Face::Mono, size: theme.code_size(), ink: Inks::Text }
}

/// How much extra stem thickness the bold face carries, in pixels, at a given
/// size.
///
/// Lato ships two weights that are closer than they look: at 15 px the bold
/// face is only about one pixel heavier than the regular one, and on a light page
/// `**strong**` was almost invisible - "bold is barely different from normal
/// text". The glyphs are *stamped* a fraction of a pixel to the right on top of
/// the shaped bold face (`Renderer::text_run_weighted`), which thickens a stem
/// without moving the pen: the layout was measured from the plain advances, so
/// nothing re-flows and the caret does not shift.
pub fn bold_extra(size: f32) -> f32 {
    (size * 0.05).clamp(0.55, 1.6)
}

fn base_style(info: &LineInfo, theme: &Theme, font_size: f32) -> BaseStyle {
    let (face, size, ink) = match info.block {
        Block::Heading(level) => (Face::Bold, theme.heading_size(level), Inks::Heading),
        Block::Code | Block::Fence => (Face::Mono, theme.code_size(), Inks::Code),
        Block::Quote => (Face::Regular, font_size, Inks::Quote),
        _ => (Face::Regular, font_size, Inks::Text),
    };
    BaseStyle { face, size, ink }
}

/// Re-cut a line's runs on the syntax classes, so one run never spans two kinds
/// of token (a run is one glyph batch, so it can only have one colour).
/// Two run styles that paint the same: the same face, size, ink and colours.
fn same_style(a: &RunStyle, b: &RunStyle) -> bool {
    a.code == b.code
        && a.weight == b.weight
        && a.face == b.face
        && a.size == b.size
        && a.ink == b.ink
        && a.underline == b.underline
        && a.strike == b.strike
        && a.highlight == b.highlight
        && a.raise == b.raise
        && a.fg == b.fg
        && a.bg == b.bg
}

/// Wrap one table cell's runs into visual bands no wider than `col_w`.
///
/// A cell is an independent column of text: its own width, its own line breaks.
/// Breaking it here rather than letting the row overflow is what keeps a long
/// cell inside its column instead of drawing over the next rule and off the page.
/// Everything a table row needs: the runs of all its cells, the wrapped bands and
/// the position of every character.
pub(crate) struct TableCells {
    /// All cells' runs in source order (the line's `clean` list for a table row).
    runs: Vec<Run>,
    /// One entry per visual band; each band holds the piece of every cell that
    /// belongs on that line, at the x of its own column.
    bands: Vec<Vec<Run>>,
    /// (band, x) of every character, with `u16::MAX` marking "not painted".
    pos: Vec<(u16, f32)>,
}

/// Lay out one table row, cell by cell.
///
/// `only_pipes` builds the variant that shows the inline markers (`**bold**`,
/// `[link](url)`) while the structural pipes stay hidden - the row the caret is
/// on. Both variants come from this one function, so the paint and the caret
/// cannot disagree about where a character is.
#[allow(clippy::too_many_arguments)]
fn table_cells(
    chars: &[char],
    info: &LineInfo,
    flags: &[Flags],
    style: BaseStyle,
    theme: &Theme,
    te: &mut TextEngine,
    plain: bool,
    xs: &[f32],
    total: f32,
    only_pipes: bool,
) -> TableCells {
    let mut runs: Vec<Run> = Vec::new();
    let mut bands: Vec<Vec<Run>> = Vec::new();
    if !info.table_sep {
        for (ci, &(a, b)) in info.cells.iter().enumerate() {
            let Some(&x) = xs.get(ci) else { break };
            // The cell's run reaches up to the next pipe, not just to its last
            // visible character: the spaces a writer types at the end of a cell
            // are part of that cell, so the caret has to keep moving through them.
            let mut b = b;
            while b < chars.len() && chars[b] != '|' {
                b += 1;
            }
            let b = b.min(chars.len());
            if a >= b {
                continue;
            }
            // A cell that holds nothing but `![alt](src)` is painted as the
            // picture (see `table_images`), so its characters are not laid out at
            // all in the rendered variant - otherwise the source of the image
            // would be printed on top of it.
            if !only_pipes && markdown::standalone_image(chars, a, b).is_some() {
                continue;
            }
            let (cell, _, _, _) = build_runs(
                chars, info, flags, a, b, x, style, theme, te, plain, only_pipes,
            );
            runs.extend(cell.iter().cloned());
            // The column this cell lives in, so its text wraps inside it instead
            // of running under the next rule.
            let col_w = if ci + 1 < xs.len() {
                (xs[ci + 1] - x - TABLE_CELL_PAD * 2.0).max(24.0)
            } else {
                (total - x - TABLE_CELL_PAD).max(24.0)
            };
            // A cell is a paragraph of its own, so it is laid out bidirectionally
            // on its own: an Arabic or Hebrew cell would otherwise be painted with
            // its characters in source order - "שלום" on screen as "םולש" - and an
            // English word inside such a cell would split it in the wrong place.
            let cell_levels = if crate::text::bidi::may_be_rtl(&chars[a..b]) {
                let visible = cell_visible_base(chars, info, a, b, only_pipes);
                let lv = crate::text::bidi::analyse_with_base(&chars[a..b], visible);
                lv.has_rtl.then_some(lv)
            } else {
                None
            };
            let rtl_cell = cell_levels.as_ref().map(|l| l.rtl_para()).unwrap_or(false);
            for (bi, band) in wrap_cell(chars, &cell, col_w, te).into_iter().enumerate() {
                if bands.len() <= bi {
                    bands.push(Vec::new());
                }
                let laid = match &cell_levels {
                    // Inside a cell the offsets are rebased onto the cell's own
                    // start, which is why the levels are shifted by `a` here.
                    Some(lv) => bidi_band(chars, lv, a, band, col_w, x, te),
                    None => band
                        .into_iter()
                        .map(|mut r| {
                            r.x += x;
                            r
                        })
                        .collect(),
                };
                bands[bi].extend(laid);
            }
            let _ = rtl_cell;
        }
    }

    // Where every character of the row sits, built *after* the wrap so the caret
    // and the mouse agree with the paint. A slot nothing was painted on is
    // "unknown" until it is filled in below: `0.0` cannot be the marker, because a
    // column really can start at x = 0.
    const UNKNOWN: u16 = u16::MAX;
    let mut pos = vec![(UNKNOWN, 0.0); chars.len() + 1];
    let mut last_band = 0usize;
    let mut last_x = 0.0f32;
    for (bi, band) in bands.iter().enumerate() {
        for run in band {
            // The advances come from the run *as a whole*, the way the painter
            // draws it: kerning inside `AV` or `office` makes the sum of
            // single-character widths differ from the run by more than a
            // character over a long cell, and that difference is exactly what put
            // the text ahead of the caret and the pointer.
            let text: String = chars[run.start..run.end.min(chars.len())].iter().collect();
            let adv = te.char_advances_dir(run.style.face, &text, run.style.size, run.rtl);
            let mut acc = 0.0f32;
            for (k, i) in (run.start..run.end.min(pos.len())).enumerate() {
                // A right-to-left run is painted from its right edge, so character
                // `k` of it sits `acc` in from that edge. (The advances are always
                // in logical order, so this is the same walk either way.)
                let cx = if run.rtl { run.x + run.w - acc } else { run.x + acc };
                pos[i] = (bi as u16, cx);
                acc += adv.get(k).copied().unwrap_or(0.0);
            }
            // The position **after** the run's last character is a caret position
            // too - and it is the common one: it is where the caret sits after you
            // type the last letter of a cell, and where a click just past the last
            // glyph lands. Only the *starts* used to be recorded, so that slot was
            // left unknown and the carry-forward below filled it with the start of
            // the very character before it: the caret was drawn a whole glyph to
            // the left of the text, which is what "one letter is displayed ahead of
            // the pointer" looked like. It showed up the moment a cell's text ended
            // right before a hidden marker or a pipe - i.e. exactly when you click
            // an empty cell and type the first letter into it.
            let end_i = run.end.min(pos.len());
            if end_i < pos.len() && pos[end_i].0 == UNKNOWN {
                pos[end_i] = (bi as u16, run.x + run.w);
            }
            last_band = bi;
            last_x = run.x + acc;
        }
    }
    // Each cell's first index is its own left edge, even when the cell has no text
    // at all: that is the only position a caret can take that keeps what you type
    // inside the cell.
    for (ci, &(a, _)) in info.cells.iter().enumerate() {
        if let Some(slot) = pos.get_mut(a) {
            if slot.0 == UNKNOWN {
                *slot = (0, xs.get(ci).copied().unwrap_or(0.0));
            }
        }
    }
    // The gaps (a cell's padding, the pipe between two cells) belong to the text
    // before them, like every other caret in a run of text: carrying the last
    // known x forward stops a gap from throwing the caret back to the margin.
    let mut prev = (0u16, 0.0f32);
    for slot in pos.iter_mut() {
        if slot.0 == UNKNOWN {
            *slot = prev;
        } else {
            prev = *slot;
        }
    }
    // The slot just past the last character of the row keeps the row's own band so
    // a caret at the very end is not thrown to the first one.
    if let Some(last) = pos.last_mut() {
        *last = (last_band as u16, last_x);
    }
    TableCells { runs, bands, pos }
}

/// Lay a line out in both directions.
///
/// For each visual row: cut the logical runs at every level change (a run can only
/// be painted in one direction), put the pieces in UBA visual order, and give them
/// their x from the row's left edge. Then walk the row again to record where the
/// caret sits for every character position - measured from the right edge inside a
/// right-to-left piece, which is the one rule that makes typing feel normal in an
/// RTL paragraph.
/// Direction of a line's **visible** text: the first strong character that is not
/// part of a Markdown marker.
///
/// `None` when the text has no strongly-directional character at all (a line of
/// digits or punctuation), which leaves the choice to the algorithm.
fn visible_base(chars: &[char], info: &LineInfo) -> Option<u8> {
    use unicode_bidi::BidiClass;
    for (i, &c) in chars.iter().enumerate() {
        if markdown::is_marker(info, i) {
            continue;
        }
        match unicode_bidi::bidi_class(c) {
            BidiClass::R | BidiClass::AL => return Some(1),
            BidiClass::L => return Some(0),
            _ => {}
        }
    }
    None
}

/// Direction of one table cell's visible text (see [`visible_base`]).
fn cell_visible_base(chars: &[char], info: &LineInfo, a: usize, b: usize, only_pipes: bool) -> Option<u8> {
    use unicode_bidi::BidiClass;
    for i in a..b.min(chars.len()) {
        let hidden = if only_pipes {
            markdown::is_pipe(info, i)
        } else {
            markdown::is_marker(info, i)
        };
        if hidden {
            continue;
        }
        match unicode_bidi::bidi_class(chars[i]) {
            BidiClass::R | BidiClass::AL => return Some(1),
            BidiClass::L => return Some(0),
            _ => {}
        }
    }
    None
}

/// Lay one band of a table cell out bidirectionally, rebased onto `column_x`.
///
/// `levels` covers the whole line, so every lookup is shifted by the cell's own
/// start `a`. A right-to-left cell is aligned to the right edge of its column, the
/// way every other right-to-left paragraph in the editor is aligned to the right
/// margin.
fn bidi_band(
    chars: &[char],
    levels: &crate::text::bidi::Levels,
    a: usize,
    band: Vec<Run>,
    col_w: f32,
    column_x: f32,
    te: &mut TextEngine,
) -> Vec<Run> {
    // (1) cut at level changes and measure each piece in its own direction
    let mut split: Vec<Run> = Vec::new();
    for p in band {
        let mut i = p.start;
        while i < p.end {
            let lvl = levels.at(i.saturating_sub(a));
            let mut j = i + 1;
            while j < p.end && levels.at(j.saturating_sub(a)) == lvl {
                j += 1;
            }
            let text: String = chars[i..j.min(chars.len())].iter().collect();
            let rtl = lvl % 2 == 1;
            let w = te.width_dir(p.style.face, &text, p.style.size, rtl);
            split.push(Run { start: i, end: j, x: 0.0, w, style: p.style, rtl });
            i = j;
        }
    }
    // (2) painting order
    let levels_of: Vec<u8> = split.iter().map(|r| levels.at(r.start.saturating_sub(a))).collect();
    let order = crate::text::bidi::visual_order(&levels_of);
    let total: f32 = split.iter().map(|r| r.w).sum();
    let mut x = if levels.rtl_para() {
        column_x + (col_w - total).max(0.0)
    } else {
        column_x
    };
    let mut out: Vec<Run> = Vec::with_capacity(split.len());
    for &k in &order {
        let mut r = split[k].clone();
        r.x = x;
        x += r.w;
        out.push(r);
    }
    out
}

fn bidi_line(
    chars: &[char],
    levels: &crate::text::bidi::Levels,
    clean: &[Run],
    raw: &[Run],
    rows: &[WrapRow],
    te: &mut TextEngine,
) -> BidiLine {
    // The same work for either variant of the line: the markers shown or hidden.
    fn pass(
        chars: &[char],
        levels: &crate::text::bidi::Levels,
        runs: &[Run],
        rows: &[WrapRow],
        te: &mut TextEngine,
    ) -> (Vec<Vec<Run>>, Vec<Vec<(usize, f32)>>) {
        let mut out_rows: Vec<Vec<Run>> = Vec::with_capacity(rows.len());
        let mut carets: Vec<Vec<(usize, f32)>> = Vec::with_capacity(rows.len());
        for row in rows {
            // (1) the runs of this row, still in logical order
            let mut pieces: Vec<Run> = Vec::new();
            for r in runs {
                let a = r.start.max(row.start);
                let b = r.end.min(row.end);
                if b <= a {
                    continue;
                }
                pieces.push(Run { start: a, end: b, x: 0.0, w: 0.0, style: r.style, rtl: false });
            }
            // (2) cut at level changes, measuring each piece in its own direction
            let mut split: Vec<Run> = Vec::with_capacity(pieces.len());
            for p in pieces {
                let mut i = p.start;
                while i < p.end {
                    let lvl = levels.at(i);
                    let mut j = i + 1;
                    while j < p.end && levels.at(j) == lvl {
                        j += 1;
                    }
                    let text: String = chars[i..j].iter().collect();
                    let rtl = lvl % 2 == 1;
                    let w = te.width_dir(p.style.face, &text, p.style.size, rtl);
                    split.push(Run { start: i, end: j, x: 0.0, w, style: p.style, rtl });
                    i = j;
                }
            }
            // (3) painting order
            let levels_of: Vec<u8> = split.iter().map(|r| levels.at(r.start)).collect();
            let order = crate::text::bidi::visual_order(&levels_of);
            let mut visual: Vec<Run> = Vec::with_capacity(split.len());
            let mut x = 0.0f32;
            for &k in &order {
                let mut r = split[k].clone();
                r.x = x;
                x += r.w;
                visual.push(r);
            }
            // (4) the caret, from the pieces themselves
            let mut pts: Vec<(usize, f32, bool)> = Vec::new();
            for r in &visual {
                let text: String = chars[r.start..r.end].iter().collect();
                let adv = te.char_advances_dir(r.style.face, &text, r.style.size, r.rtl);
                let n = r.end - r.start;
                let mut acc = 0.0f32;
                for k in 0..=n {
                    let cx = if r.rtl { r.x + r.w - acc } else { r.x + acc };
                    // A position at the very start of a piece is the position
                    // *after* the piece before it, which may be the other
                    // direction: recorded so the de-duplication below can pick the
                    // side the text is on.
                    pts.push((r.start + k, cx, k == 0));
                    if k < n {
                        acc += adv.get(k).copied().unwrap_or(0.0);
                    }
                }
            }
            pts.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)));
            let mut row_caret: Vec<(usize, f32)> = Vec::with_capacity(pts.len());
            for (idx, cx, from_start) in pts {
                match row_caret.last_mut() {
                    Some(last) if last.0 == idx => {
                        // Two pieces meet here and the two candidates are different
                        // x's, which only happens where the direction changes. The
                        // caret belongs to the text *before* it - the piece that
                        // ends here - so typing continues in the direction you were
                        // already writing: at the left edge of a Hebrew run, at the
                        // right edge of a Latin one. (When both pieces run the same
                        // way the two candidates are the same pixel anyway.)
                        if !from_start {
                            last.1 = cx;
                        }
                    }
                    _ => row_caret.push((idx, cx)),
                }
            }
            out_rows.push(visual);
            carets.push(row_caret);
        }
        (out_rows, carets)
    }

    let (clean_rows, clean_caret) = pass(chars, levels, clean, rows, te);
    let (raw_rows, raw_caret) = pass(chars, levels, raw, rows, te);
    BidiLine {
        rtl_para: levels.rtl_para(),
        clean: clean_rows,
        raw: raw_rows,
        clean_caret,
        raw_caret,
    }
}

fn wrap_cell(chars: &[char], runs: &[Run], col_w: f32, te: &mut TextEngine) -> Vec<Vec<Run>> {
    if runs.is_empty() {
        return Vec::new();
    }
    // One entry per **visible** character. The runs a cell is built from never
    // contain a marker, so the gaps between them are hidden syntax and must not
    // be counted here at all: they used to get a slot of their own, which merged
    // them into the run beside them (so the painter printed `[url](url)` inside a
    // cell) and gave every band a width of zero.
    struct Piece {
        idx: usize,
        w: f32,
        style: RunStyle,
    }
    let mut pieces: Vec<Piece> = Vec::new();
    for run in runs {
        // Measured as one shaped run (see `TextEngine::char_advances`): a wrap
        // computed from single-character widths does not match the text the
        // painter draws, and the error grows with the cell's length.
        let end = run.end.min(chars.len());
        let text: String = chars[run.start..end].iter().collect();
        let adv = te.char_advances(run.style.face, &text, run.style.size);
        for i in run.start..end {
            let w = adv.get(i - run.start).copied().unwrap_or(0.0);
            pieces.push(Piece { idx: i, w, style: run.style });
        }
    }
    let n = pieces.len();
    if n == 0 {
        return Vec::new();
    }
    // Greedy wrap on the measured widths: break at the last space of the band, and
    // hard-break a single word that is longer than the column.
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut start = 0usize;
    let mut last_space: Option<usize> = None;
    let mut used = 0.0f32;
    for k in 0..n {
        let w = pieces[k].w;
        // A cell whose text is exactly as wide as its column is the normal case
        // (the columns are measured from the text), so the comparison needs a
        // tolerance: without it, one glyph of rounding wrapped the cell into a
        // second, empty band and the whole row grew a line taller.
        if used + w > col_w + 0.05 && k > start {
            let cut = match last_space {
                Some(sp) if sp > start => sp,
                _ => k,
            };
            spans.push((start, cut));
            start = if cut < n && chars[pieces[cut].idx].is_whitespace() {
                cut + 1
            } else {
                cut
            };
            last_space = None;
            used = 0.0;
            for p in &pieces[start..=k] {
                used += p.w;
            }
            if chars[pieces[k].idx].is_whitespace() {
                last_space = Some(k);
            }
        } else {
            used += w;
            if chars[pieces[k].idx].is_whitespace() {
                last_space = Some(k);
            }
        }
    }
    spans.push((start, n));
    // A trailing empty span is not a band: it would add a blank line to the row.
    while spans.len() > 1 && spans.last().map(|&(a, b)| b <= a).unwrap_or(false) {
        spans.pop();
    }

    let mut out: Vec<Vec<Run>> = Vec::with_capacity(spans.len());
    for (a, b) in spans {
        if b <= a {
            out.push(Vec::new());
            continue;
        }
        let mut band: Vec<Run> = Vec::new();
        let mut x = 0.0f32;
        let mut i = a;
        while i < b {
            let style = pieces[i].style;
            let first = pieces[i].idx;
            // A run is a *contiguous* slice of the source with one style: a hidden
            // marker in the middle ends it, so what is painted is only what the
            // reader is meant to see.
            let mut j = i;
            let mut w = 0.0f32;
            while j < b && pieces[j].idx == first + (j - i) && same_style(&pieces[j].style, &style) {
                w += pieces[j].w;
                j += 1;
            }
            band.push(Run { start: first, end: pieces[j - 1].idx + 1, x, w, style, rtl: false });
            x += w;
            i = j;
        }
        out.push(band);
    }
    out
}

fn tint_runs(runs: Vec<Run>, classes: &[crate::highlight::Tok], theme: &Theme) -> Vec<Run> {
    use crate::highlight::Tok;
    let ink_of = |t: Tok| match t {
        Tok::Plain => None,
        Tok::Keyword => Some(theme.syn_keyword),
        Tok::Str => Some(theme.syn_string),
        Tok::Num => Some(theme.syn_number),
        Tok::Func => Some(theme.syn_func),
        Tok::Type => Some(theme.syn_type),
        Tok::Comment => Some(theme.text_faint),
        Tok::Meta => Some(theme.syn_keyword),
    };
    let mut out: Vec<Run> = Vec::with_capacity(runs.len() * 2);
    for run in runs {
        let mut start = run.start;
        while start < run.end {
            let class = classes.get(start).copied().unwrap_or(Tok::Plain);
            let mut end = start + 1;
            while end < run.end && classes.get(end).copied().unwrap_or(Tok::Plain) == class {
                end += 1;
            }
            // The piece keeps the run's x offset: widths are additive, and the
            // layout already measured the whole run.
            let frac = if run.end > run.start {
                (start - run.start) as f32 / (run.end - run.start) as f32
            } else {
                0.0
            };
            let wfrac = if run.end > run.start {
                (end - start) as f32 / (run.end - run.start) as f32
            } else {
                1.0
            };
            let mut piece = run.clone();
            piece.start = start;
            piece.end = end;
            piece.x = run.x + run.w * frac;
            piece.w = run.w * wfrac;
            if let Some(c) = ink_of(class) {
                // An explicit colour from a `[#rrggbb]` span wins over the theme.
                if piece.style.fg.is_none() {
                    piece.style.fg = Some(c);
                }
            }
            out.push(piece);
            start = end;
        }
    }
    out
}

/// Build styled runs for `chars[from..to]`, laid out starting at `start_x`.
///
/// Returns `(clean, raw, x_clean_end, x_raw_end)`: `clean` hides the Markdown
/// markers, `raw` keeps them (dimmed) so the source can be shown in place.
#[allow(clippy::too_many_arguments)]
fn build_runs(
    chars: &[char],
    info: &LineInfo,
    flags: &[Flags],
    from: usize,
    to: usize,
    start_x: f32,
    base: BaseStyle,
    theme: &Theme,
    te: &mut TextEngine,
    plain: bool,
    only_pipes: bool,
) -> (Vec<Run>, Vec<Run>, f32, f32) {
    // In a table cell the *structural* pipes stay hidden while the inline markers
    // (`**`, `[`, `](…)`) are shown: that is how the row you are editing shows what
    // you would have to type without turning the grid back into source text.
    let hidden = |i: usize| {
        if only_pipes {
            markdown::is_pipe(info, i)
        } else {
            markdown::is_marker(info, i)
        }
    };
    let mut clean: Vec<Run> = Vec::new();
    let mut raw: Vec<Run> = Vec::new();
    let mut x_clean = start_x;
    let mut x_raw = start_x;

    #[allow(clippy::too_many_arguments)]
    fn push(
        chars: &[char],
        runs: &mut Vec<Run>,
        x: &mut f32,
        start: usize,
        end: usize,
        style: RunStyle,
        te: &mut TextEngine,
    ) {
        if end <= start {
            return;
        }
        let text: String = chars[start..end].iter().collect();
        let w = te.width(style.face, &text, style.size);
        runs.push(Run { start, end, x: *x, w, style, rtl: false });
        *x += w;
    }

    let mut i = from;
    while i < to {
        if hidden(i) {
            let mut j = i;
            while j < to && hidden(j) {
                j += 1;
            }
            let style_raw = RunStyle::plain(base.face, base.size, Inks::Marker);
            push(chars, &mut raw, &mut x_raw, i, j, style_raw, te);
            i = j;
            continue;
        }
        let f = flags[i];
        let mut j = i + 1;
        while j < to && flags[j] == f && !hidden(j) {
            j += 1;
        }
        let ink = if f.code {
            Inks::Code
        } else if f.link {
            Inks::Link
        } else {
            base.ink
        };
        let mut style = RunStyle::plain(
            if f.code { Face::Mono } else { base.face },
            if f.code { theme.code_size() } else { base.size },
            ink,
        );
        style.underline = f.link || f.underline;
        // A code span inside running text gets a chip behind it; a whole card of
        // code is already framed, and its background is the card.
        style.code = f.code && !matches!(base.ink, Inks::Code | Inks::Marker);
        style.strike = f.strike;
        style.highlight = f.highlight;
        style.fg = f.fg.map(rgb_color);
        style.bg = f.bg.map(rgb_color);
        if f.sup || f.sub {
            // Superscript and subscript keep the text readable: smaller and
            // shifted, never clipped.
            style.size = (style.size * 0.72).max(8.0);
            style.raise = if f.sup { 0.34 } else { -0.16 };
        }
        if f.bold {
            let wanted = if f.code { Face::MonoBold } else { style.face.bold() };
            if wanted == style.face {
                // Already the bold face (a heading, a table header, code): the only
                // way to show the emphasis is to put more ink under the same glyphs.
                style.weight = bold_extra(style.size);
            } else {
                // Switching to the bold face is not enough on its own: the two
                // weights are close at text sizes, so the bold face is stamped a
                // little heavier as well. The advance is untouched either way.
                style.face = wanted;
                style.weight = bold_extra(style.size);
            }
        }
        if f.italic {
            style.face = style.face.italic();
        }
        if plain {
            // Raw mode: no bold, no italics, no links - just the characters.
            style = RunStyle::plain(base.face, base.size, base.ink);
        }
        push(chars, &mut clean, &mut x_clean, i, j, style, te);
        push(chars, &mut raw, &mut x_raw, i, j, style, te);
        i = j;
    }
    (clean, raw, x_clean, x_raw)
}

/// Greedy soft-wrap of one line into visual rows.
///
/// Break opportunities come from the Unicode line breaking algorithm applied to
/// the *visible* text (markers hidden), so wrapping matches what the reader sees.
fn compute_rows(
    chars: &[char],
    info: &LineInfo,
    clean: &[Run],
    wrap_w: f32,
    te: &mut TextEngine,
    plain: bool,
) -> (Vec<WrapRow>, f32) {
    let single = vec![WrapRow { start: 0, end: chars.len() }];
    let total_w: f32 = clean.iter().map(|r| r.w).sum();
    // Fast path: the widest possible content already fits, so there is nothing
    // to wrap and no string has to be built.
    if clean.is_empty() || total_w <= wrap_w - 9.0 {
        return (single, total_w);
    }
    // Hidden code/table lines wrap as raw text.
    let visible: String = chars
        .iter()
        .enumerate()
        .filter(|(i, _)| plain || !markdown::is_marker(info, *i))
        .map(|(_, c)| *c)
        .collect();
    if visible.is_empty() {
        return (single, total_w);
    }
    // Fast path: when even a generous estimate of the visible text fits, there is
    // nothing to wrap and the per-glyph measuring below can be skipped entirely
    // (this is what keeps long-document rebuilds cheap).
    if total_w <= wrap_w - 9.0 {
        return (single, total_w);
    }

    // Exact cumulative width of every char index. Measuring each glyph
    // individually (the shape cache makes repeat characters free) is what keeps
    // a visual row from spilling past `wrap_w`: interpolating a run's total
    // width is off by several percent on prose with mixed letter widths.
    let mut cum = vec![0.0f32; chars.len() + 1];
    {
        let mut acc = 0.0f32;
        let mut ri = 0usize;
        let mut cache: std::collections::HashMap<(u8, u32, char), f32> =
            std::collections::HashMap::new();
        for (i, c) in chars.iter().enumerate() {
            while ri < clean.len() && clean[ri].end <= i {
                ri += 1;
            }
            if ri < clean.len() && clean[ri].start <= i && i < clean[ri].end {
                let style = clean[ri].style;
                let key = (style.face as u8, style.size.to_bits(), *c);
                let w = match cache.get(&key) {
                    Some(w) => *w,
                    None => {
                        let mut buf = [0u8; 4];
                        let s: &str = c.encode_utf8(&mut buf);
                        let w = te.width(style.face, s, style.size);
                        cache.insert(key, w);
                        w
                    }
                };
                acc += w;
            }
            cum[i + 1] = acc;
        }
    }
    let x_of = |idx: usize| -> f32 { cum[idx.min(cum.len() - 1)] };
    // A row is allowed to use the full width minus a hair of slack so kerning
    // differences between per-glyph and whole-run shaping never overflow.
    let wrap_w = wrap_w - 1.5;
    if x_of(chars.len()) <= wrap_w {
        return (single, x_of(chars.len()));
    }
    // Map visible byte offsets back to char indices.
    let mut vis_to_char: Vec<usize> = Vec::with_capacity(visible.len() + 1);
    {
        let mut byte = 0usize;
        for (i, c) in chars.iter().enumerate() {
            if !plain && markdown::is_marker(info, i) {
                continue;
            }
            for _ in 0..c.len_utf8() {
                vis_to_char.push(i);
            }
            byte += c.len_utf8();
        }
        vis_to_char.push(chars.len());
        let _ = byte;
    }
    let mut breaks: Vec<usize> = unicode_linebreak::linebreaks(&visible)
        .map(|(b, _)| vis_to_char.get(b).copied().unwrap_or(chars.len()))
        .collect();
    breaks.sort_unstable();
    breaks.dedup();

    let mut rows = Vec::new();
    let mut start = 0usize;
    while start < chars.len() {
        let start_x = x_of(start);
        // Binary search the furthest break that fits.
        let mut lo = 0usize;
        let mut hi = breaks.len();
        while lo < hi {
            let mid = (lo + hi) / 2;
            if breaks[mid] > start {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        let mut end = chars.len();
        let mut found = false;
        for &b in breaks.iter().skip(lo) {
            if b <= start {
                continue;
            }
            if x_of(b) - start_x <= wrap_w {
                end = b;
                found = true;
            } else {
                break;
            }
        }
        if !found {
            // A single unbreakable token wider than the row: hard-break by width.
            let mut i = start;
            while i < chars.len() && x_of(i + 1) - start_x <= wrap_w {
                i += 1;
            }
            end = (i + 1).min(chars.len());
        }
        end = end.max(start + 1).min(chars.len());
        rows.push(WrapRow { start, end });
        start = end;
        if rows.len() > 512 {
            rows.push(WrapRow { start, end: chars.len() });
            break;
        }
    }
    if rows.is_empty() {
        return (single, x_of(chars.len()));
    }
    // Widest visual row, measured exactly - this is what decides whether the
    // horizontal scrollbar is needed at all.
    let widest = rows
        .iter()
        .map(|r| x_of(r.end) - x_of(r.start))
        .fold(0.0f32, f32::max);
    (rows, widest)
}

/// Spacing constants kept here so layout and the view agree.
pub mod theme {
    pub const PAD_TOP: f32 = 10.0;
    pub const PAD_BOTTOM: f32 = 120.0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    fn build(md: &str) -> Layout {
        let doc = Doc::new(md, None);
        let mut te = TextEngine::new();
        Layout::build(&doc, &mut te, &Theme::black(), 800.0)
    }

    #[test]
    fn clean_runs_hide_markers() {
        let l = build("# Title");
        let line = &l.lines[0];
        // Clean text must start at "Title" (markers dropped).
        assert_eq!(line.clean[0].start, 2, "runs: {:?}", line.clean);
        let visible: String = line.clean.iter().map(|r| line.chars[r.start..r.end].iter().collect::<String>()).collect();
        assert_eq!(visible, "Title");
        // Raw keeps the markers.
        let raw: String = line.raw.iter().map(|r| line.chars[r.start..r.end].iter().collect::<String>()).collect();
        assert_eq!(raw, "# Title");
    }

    #[test]
    fn caret_geometry_maps_to_chars() {
        let mut te = TextEngine::new();
        let doc = Doc::new("# Heading", None);
        let l = Layout::build(&doc, &mut te, &Theme::black(), 800.0);
        let line = &l.lines[0];
        let x0 = line.x_of(&mut te, 2, false);
        let x1 = line.x_of(&mut te, 5, false);
        assert!((x0 - 0.0).abs() < 0.01, "clean start should be at 0, got {x0}");
        assert!(x1 > x0);
        // hit-testing round-trips
        let c = line.char_at_x(&mut te, x1 + 0.5, false);
        assert_eq!(c, 5);
    }

    #[test]
    fn outline_collects_headings() {
        let l = build("# One\ntext\n## Two\n");
        assert_eq!(l.headings.len(), 2);
        assert_eq!(l.headings[0].1, 1);
        assert_eq!(l.headings[1].2, "Two");
    }

    #[test]
    fn code_blocks_are_mono_and_keep_lines() {
        let l = build("```\nlet a = 1;\n```\n");
        assert!(matches!(l.lines[0].info.block, Block::Fence));
        assert!(matches!(l.lines[1].info.block, Block::Code));
        assert_eq!(l.lines[1].clean[0].style.face, Face::Mono);
    }

    #[test]
    fn line_lookup_roundtrip() {
        let l = build("a\nbb\nccc");
        assert_eq!(l.lines.len(), 3);
        assert_eq!(l.line_of_char(0), 0);
        assert_eq!(l.line_of_char(2), 1);
        assert_eq!(l.line_of_char(3), 1);
        assert_eq!(l.line_of_char(5), 2);
    }
}

#[cfg(test)]
mod wrap_tests {
    use super::*;
    use crate::theme::Theme;

    #[test]
    fn long_paragraph_wraps_into_rows() {
        let text = "word ".repeat(80);
        let doc = Doc::new(text.trim_end(), None);
        let mut te = TextEngine::new();
        let l = Layout::build(&doc, &mut te, &Theme::black(), 420.0);
        let line = &l.lines[0];
        assert!(line.row_count() > 3, "rows = {}", line.row_count());
        // Rows tile the line without gaps or overlaps.
        let mut cursor = 0;
        for row in &line.rows {
            assert_eq!(row.start, cursor);
            assert!(row.end > row.start);
            cursor = row.end;
        }
        assert_eq!(cursor, line.chars.len());
    }

    #[test]
    fn short_lines_do_not_wrap() {
        let doc = Doc::new("short line", None);
        let mut te = TextEngine::new();
        let l = Layout::build(&doc, &mut te, &Theme::black(), 800.0);
        assert_eq!(l.lines[0].row_count(), 1);
    }

    #[test]
    fn wrapped_caret_maps_back_to_the_same_char() {
        let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu";
        let doc = Doc::new(text, None);
        let mut te = TextEngine::new();
        let l = Layout::build(&doc, &mut te, &Theme::black(), 300.0);
        let line = &l.lines[0];
        for target in [3usize, 12, 20, 35, 50] {
            if target > line.chars.len() {
                continue;
            }
            let ri = line.row_of_local(target);
            let x = line.x_of(&mut te, target, false);
            let back = line.char_at_row_x(&mut te, ri, x, false);
            assert_eq!(back, target, "row {ri} x {x}");
        }
    }
}
