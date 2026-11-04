//! The text rendering engine: font loading, shaping (rustybuzz) and
//! rasterisation (fontdue).
//!
//! Everything that turns a `(face, size, string)` into glyphs and advances lives
//! here. The renderer (`platform::gpu`) only knows how to blit what this module
//! produced, and the layout (`markdown::layout`) only asks this module for widths
//! and line heights - which is why the two can never disagree about how wide a
//! line is.
//!
use std::collections::HashMap;
use std::sync::Arc;

pub mod bidi;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Face {
    Regular,
    Bold,
    Italic,
    BoldItalic,
    Mono,
    MonoBold,
    Fallback,
    FallbackBold,
}

impl Face {
    pub fn index(self) -> usize {
        match self {
            Face::Regular => 0,
            Face::Bold => 1,
            Face::Italic => 2,
            Face::BoldItalic => 3,
            Face::Mono => 4,
            Face::MonoBold => 5,
            Face::Fallback => 6,
            Face::FallbackBold => 7,
        }
    }

    pub fn bold(self) -> Face {
        match self {
            Face::Regular | Face::Bold => Face::Bold,
            Face::Italic | Face::BoldItalic => Face::BoldItalic,
            Face::Mono | Face::MonoBold => Face::MonoBold,
            f => f,
        }
    }

    pub fn italic(self) -> Face {
        match self {
            Face::Regular | Face::Italic => Face::Italic,
            Face::Bold | Face::BoldItalic => Face::BoldItalic,
            other => other,
        }
    }

    pub fn mono(self) -> Face {
        match self {
            Face::Bold | Face::MonoBold | Face::BoldItalic => Face::MonoBold,
            _ => Face::Mono,
        }
    }

    pub const ALL: [Face; 8] = [
        Face::Regular,
        Face::Bold,
        Face::Italic,
        Face::BoldItalic,
        Face::Mono,
        Face::MonoBold,
        Face::Fallback,
        Face::FallbackBold,
    ];
}

static LATO_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Lato-Regular.ttf");
static LATO_BOLD: &[u8] = include_bytes!("../../assets/fonts/Lato-Bold.ttf");
static LATO_ITALIC: &[u8] = include_bytes!("../../assets/fonts/Lato-Italic.ttf");
static LATO_BOLD_ITALIC: &[u8] = include_bytes!("../../assets/fonts/Lato-BoldItalic.ttf");
static MONO_REGULAR: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf");
static MONO_BOLD: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Bold.ttf");
static DEJAVU: &[u8] = include_bytes!("../../assets/fonts/DejaVuSans.ttf");
static DEJAVU_BOLD: &[u8] = include_bytes!("../../assets/fonts/DejaVuSans-Bold.ttf");

/// One shaped glyph, in em units (multiply by the pixel size to get pixels).
///
/// `PartialEq` so a test can say two engines shape a string identically - which is
/// what proves that sharing the parsed faces leaks nothing between them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub face: Face,
    pub gid: u16,
    /// Horizontal advance in em.
    pub advance: f32,
    /// Pen offset applied before drawing, in em.
    pub x_offset: f32,
    /// Vertical offset from the baseline (positive = up), in em.
    pub y_offset: f32,
    /// Byte offset of the character this glyph belongs to, inside the string that
    /// was shaped. Two glyphs with the same cluster come from one character (a
    /// ligature, a base plus its mark), which is what lets a per-character width
    /// be recovered from a shaped run.
    pub cluster: u32,
}

pub struct FaceData {
    pub font: fontdue::Font,
    pub buzz: rustybuzz::Face<'static>,
    pub ascent_em: f32,
    pub descent_em: f32,
    pub line_gap_em: f32,
}

impl FaceData {
    fn new(data: &'static [u8]) -> Option<Self> {
        let font = fontdue::Font::from_bytes(data, fontdue::FontSettings::default()).ok()?;
        let buzz = rustybuzz::Face::from_slice(data, 0)?;
        let upem = buzz.units_per_em() as f32;
        let (ascent, descent, gap) = {
            let asc = buzz.ascender() as f32;
            let desc = buzz.descender() as f32;
            let gap = buzz.line_gap() as f32;
            (asc, desc, gap)
        };
        Some(FaceData {
            font,
            buzz,
            ascent_em: ascent / upem,
            descent_em: (-descent) / upem,
            line_gap_em: gap / upem,
        })
    }

    /// True if the face has a real glyph for `c`.
    fn covers(&self, c: char) -> bool {
        // Cheap check via the raw cmap lookup used by rustybuzz/ttf-parser.
        self.buzz.glyph_index(c).is_some()
    }
}

/// Longest string handed to the shaper in one piece.
///
/// See [`TextEngine::shape`]: the number is small enough that a keystroke re-shapes
/// a fraction of a long line, and large enough that ordinary text (a heading, a
/// code line, a sentence) is still shaped in one piece, kerning intact.
pub const SHAPE_CHUNK: usize = 96;

/// Byte ranges of `text`, each at most [`SHAPE_CHUNK`] characters long.
///
/// A chunk ends at a space whenever one is inside the window: the only thing
/// chunking can cost is kerning across a cut, and there is no kerning worth
/// keeping between two words.
fn shape_chunks(text: &str) -> Vec<(usize, usize)> {
    let bytes: Vec<(usize, char)> = text.char_indices().collect();
    let mut out: Vec<(usize, usize)> = Vec::with_capacity(bytes.len() / SHAPE_CHUNK + 1);
    let mut start = 0usize;
    while start < bytes.len() {
        let mut cut = (start + SHAPE_CHUNK).min(bytes.len());
        if cut < bytes.len() {
            if let Some(p) = (start + 1..cut).rev().find(|&i| bytes[i].1.is_whitespace()) {
                cut = p + 1;
            }
        }
        let a = bytes[start].0;
        let b = if cut < bytes.len() { bytes[cut].0 } else { text.len() };
        out.push((a, b));
        start = cut;
    }
    out
}

/// The chunk split for `text`, or `None` when it fits in one piece.
///
/// The byte length is only a shortcut that skips the char walk for short strings:
/// a few accented characters are longer in bytes than in characters, so a split is
/// only accepted when it really produces more than one piece. (A "split" that
/// returned the whole string would recurse for ever.)
fn split_chunks(text: &str) -> Option<Vec<(usize, usize)>> {
    if text.len() <= SHAPE_CHUNK {
        return None;
    }
    let chunks = shape_chunks(text);
    if chunks.len() > 1 {
        Some(chunks)
    } else {
        None
    }
}

pub struct TextEngine {
    /// The eight faces, parsed once per process and shared (they are read-only after
    /// construction). See [`faces`].

    /// Keyed by `(face, text, right_to_left)`: Arabic shaped right-to-left
    /// gives different glyphs (joined forms, reversed order) from the same bytes
    /// shaped left-to-right, so the direction has to be part of the key.
    shape_cache: HashMap<(u8, Arc<str>, bool), Arc<Vec<Glyph>>>,
    /// Advance of a single character, keyed by `(face, char, size in quarter px)`.
    ///
    /// Laying a table cell out measures it one character at a time - twice per
    /// variant, per keystroke - so this is the hottest width lookup in the editor.
    /// Going through `shape` for it built an `Arc<str>` cache key on every call.
    advance_cache: HashMap<(u8, char, i32), f32>,
    /// Per-character advances of a whole string, keyed like `shape_cache`.
    ///
    /// This is the *only* correct way to measure text one character at a time:
    /// shaping a string and shaping each of its characters separately do not agree
    /// (`AV` is narrower than `A` + `V` because of kerning), so a layout that adds
    /// up single-character widths drifts away from the run the painter draws.
    char_adv_cache: HashMap<(u8, Arc<str>, i32, bool), Arc<Vec<f32>>>,
    cover_cache: HashMap<(u8, char), bool>,
    /// Resolved fallback face cache.
    resolve_cache: HashMap<(u8, char), Face>,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub shapes: u64,
}

/// The eight faces, parsed once for the whole process.
///
/// `fontdue::Font::from_bytes` reads every table of a font and costs about 130 ms for
/// the eight of them together (Lato x4, JetBrains Mono x2, DejaVu x2) - and it used to
/// happen inside `TextEngine::new`, i.e. in the middle of every launch, *and* in every
/// test that built a renderer. The faces never change after parsing and nothing writes
/// to them, so they are parsed once and borrowed by every engine; [`preload_faces`]
/// starts that parse on a background thread so it overlaps the window being created
/// and the graphics device being made.
/// The bundled fonts, in `Face` order.
const SOURCES: [(&str, &[u8]); 8] = [
    ("Lato Regular", LATO_REGULAR),
    ("Lato Bold", LATO_BOLD),
    ("Lato Italic", LATO_ITALIC),
    ("Lato Bold Italic", LATO_BOLD_ITALIC),
    ("JetBrains Mono", MONO_REGULAR),
    ("JetBrains Mono Bold", MONO_BOLD),
    ("DejaVu Sans", DEJAVU),
    ("DejaVu Sans Bold", DEJAVU_BOLD),
];

/// One slot per face, parsed the first time that face is asked for.
///
/// They used to be parsed as one `Vec` up front, which meant a launch paid for all
/// eight - about 110 ms of font parsing before the first frame could be drawn, half of
/// the whole launch. A frame only ever needs the faces it actually draws with, and on
/// an ordinary (Latin) screen the two DejaVu fallbacks are never touched: measured, a
/// document with an em dash, an ellipsis and a bullet asks for no fallback at all. So
/// the parse follows the drawing, one face at a time, and a Cyrillic or CJK document
/// still gets its fallback - only when it first needs it.
static SLOTS: [std::sync::OnceLock<FaceData>; 8] = [const { std::sync::OnceLock::new() }; 8];

fn face_data(face: Face) -> &'static FaceData {
    SLOTS[face.index()].get_or_init(|| {
        let (name, bytes) = SOURCES[face.index()];
        FaceData::new(bytes).unwrap_or_else(|| panic!("cannot parse {name}"))
    })
}

/// Start parsing the fonts on a background thread.
///
/// Called at the very start of a launch: the work then happens while the window is
/// being created and the graphics device made, which is time the user is already
/// waiting through. Calling it twice, or never, is fine - [`TextEngine::new`] parses
/// the faces itself if they are not ready yet.
pub fn preload_faces() {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        std::thread::Builder::new()
            .name("font-parse".into())
            .spawn(|| {
                // The faces a Latin screen needs come first, the fallbacks last: if
                // the first frame has to wait for this thread at all, it waits as
                // little as possible. Each slot is filled only if nobody has filled
                // it already, so this races with the drawing thread harmlessly.
                for face in Face::ALL {
                    let _ = face_data(face);
                }
            })
            .ok();
    });
}

/// Whether the two DejaVu fallback faces have been parsed yet.
///
/// A launch must not parse them: they exist for characters Lato and JetBrains Mono
/// do not have (Cyrillic, Greek, arrows, box drawing), and a screen of Latin text
/// draws none of those. Parsed eagerly they cost ~47 ms of a ~210 ms launch, so
/// `tests/perf.rs` holds the launch to "the first frame is on screen without them".
pub fn fallbacks_parsed() -> bool {
    SLOTS[Face::Fallback.index()].get().is_some() || SLOTS[Face::FallbackBold.index()].get().is_some()
}

impl TextEngine {
    pub fn new() -> Self {
        TextEngine {
            shape_cache: HashMap::new(),
            advance_cache: HashMap::new(),
            char_adv_cache: HashMap::new(),
            cover_cache: HashMap::new(),
            resolve_cache: HashMap::new(),
            cache_hits: 0,
            cache_misses: 0,
            shapes: 0,
        }
    }

    pub fn face(&self, face: Face) -> &FaceData {
        face_data(face)
    }

    /// Ascent/descent in pixels for a face at `px` size.
    pub fn vertical_metrics(&self, face: Face, px: f32) -> (f32, f32) {
        let f = self.face(face);
        (f.ascent_em * px, f.descent_em * px)
    }

    pub fn line_height(&self, face: Face, px: f32) -> f32 {
        let f = self.face(face);
        (f.ascent_em + f.descent_em + f.line_gap_em) * px
    }

    fn covers(&mut self, face: Face, c: char) -> bool {
        if let Some(v) = self.cover_cache.get(&(face as u8, c)) {
            return *v;
        }
        let v = face_data(face).covers(c);
        self.cover_cache.insert((face as u8, c), v);
        v
    }

    /// Picks the face that actually has a glyph for `c`.
    pub fn resolve(&mut self, preferred: Face, c: char) -> Face {
        if c == ' ' || c == '\t' {
            return preferred;
        }
        let key = (preferred as u8, c);
        if let Some(f) = self.resolve_cache.get(&key) {
            return *f;
        }
        // The fallback face is a real answer, not a shrug: Lato has no CJK, no
        // emoji, no arrows or box drawing, and this used to be rewritten back to
        // `Regular` right here - so every one of those characters was drawn as
        // `.notdef` from Lato and DejaVu Sans, which is bundled for exactly this
        // job, was never used at all. Advances are measured through the same
        // `shape`, so a line with a fallback glyph in it is laid out with the width
        // it is painted at.
        let chosen = if self.covers(preferred, c) {
            preferred
        } else {
            let bold = matches!(preferred, Face::Bold | Face::BoldItalic | Face::MonoBold | Face::FallbackBold);
            let mut found = None;
            for cand in [Face::Regular, Face::Bold, Face::Fallback, Face::FallbackBold] {
                let cand = if bold { cand.bold() } else { cand };
                if self.covers(cand, c) {
                    found = Some(cand);
                    break;
                }
            }
            found.unwrap_or(preferred)
        };
        self.resolve_cache.insert(key, chosen);
        chosen
    }

    /// Shape `text` for `face`, splitting into runs when a fallback face is needed.
    ///
    /// Left-to-right: the direction of almost every line. Right-to-left text goes
    /// through [`TextEngine::shape_dir`]; the two are separate caches because the
    /// same characters shape differently in the two directions.
    pub fn shape(&mut self, face: Face, text: &str) -> Arc<Vec<Glyph>> {
        self.shape_dir(face, text, false)
    }

    /// Shape `text`, right-to-left when `rtl` is set.
    ///
    /// rustybuzz returns glyphs in **visual** order: left-to-right text comes back
    /// in logical order, right-to-left text comes back reversed. That is exactly
    /// what the painter wants, because it stamps glyphs with a pen that runs from
    /// left to right; it also means Arabic gets its joined forms, which it only
    /// gets when the shaper is told the text runs right to left.
    pub fn shape_dir(&mut self, face: Face, text: &str, rtl: bool) -> Arc<Vec<Glyph>> {
        // Long strings are shaped in chunks. The cache is keyed by the whole
        // string, so typing one character in a 10 000-character table cell made
        // the cell a new key and re-shaped all of it - about 200 ms of rustybuzz
        // work per keystroke, which is the freeze this fixes. Chunking keeps the
        // work linear *and* keeps each chunk in the cache, so a keystroke only
        // shapes the chunk it landed in.
        if let Some(chunks) = split_chunks(text) {
            let key = (face as u8, Arc::<str>::from(text), rtl);
            if let Some(v) = self.shape_cache.get(&key) {
                self.cache_hits += 1;
                return v.clone();
            }
            self.cache_misses += 1;
            self.shapes += 1;
            let mut out: Vec<Glyph> = Vec::with_capacity(text.len());
            for (a, b) in chunks {
                // Each chunk is shaped on its own, so its cluster indices start at
                // zero: shift them back to where the chunk begins, so a caller can
                // still map a glyph to a character of the whole string.
                out.extend(self.shape_dir(face, &text[a..b], rtl).iter().map(|g| Glyph {
                    cluster: g.cluster + a as u32,
                    ..*g
                }));
            }
            let arc = Arc::new(out);
            if self.shape_cache.len() > 20_000 {
                self.shape_cache.clear();
            }
            self.shape_cache.insert(key, arc.clone());
            return arc;
        }
        let key = (face as u8, Arc::<str>::from(text), rtl);
        if let Some(v) = self.shape_cache.get(&key) {
            self.cache_hits += 1;
            return v.clone();
        }
        self.cache_misses += 1;
        self.shapes += 1;

        let mut out: Vec<Glyph> = Vec::with_capacity(text.len());
        let mut run_face = self.resolve(face, text.chars().next().unwrap_or(' '));
        // A fallback face has to be shaped as its own little run, so the string is
        // cut wherever the face changes. Two things follow from that, and both used
        // to be wrong: each piece's glyph clusters are byte offsets *within the
        // piece*, so they have to be rebased on where the piece starts (otherwise
        // every character after the first piece was attributed to the wrong one and
        // got no width at all), and a right-to-left string's pieces come back
        // reversed *within themselves* while still being in logical order relative
        // to each other - so the pieces themselves have to be reversed too, or a
        // Hebrew word split by a space came out in the wrong order on screen.
        let mut pieces: Vec<(Face, usize, usize)> = Vec::new();
        let mut run_start = 0usize;
        for (byte, c) in text.char_indices() {
            let f = self.resolve(face, c);
            if f != run_face {
                pieces.push((run_face, run_start, byte));
                run_face = f;
                run_start = byte;
            }
        }
        pieces.push((run_face, run_start, text.len()));
        if rtl {
            pieces.reverse();
        }
        for (f, a, b) in pieces {
            self.shape_run(f, &text[a..b], &mut out, rtl, a as u32);
        }

        let arc = Arc::new(out);
        if self.shape_cache.len() > 20_000 {
            self.shape_cache.clear();
        }
        self.shape_cache.insert(key, arc.clone());
        arc
    }

    /// Shape one piece of a string, rebasing its clusters onto `base`.
    fn shape_run(&mut self, face: Face, text: &str, out: &mut Vec<Glyph>, rtl: bool, base: u32) {
        if text.is_empty() {
            return;
        }
        let data = face_data(face);
        let upem = data.buzz.units_per_em() as f32;
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.set_direction(if rtl {
            rustybuzz::Direction::RightToLeft
        } else {
            rustybuzz::Direction::LeftToRight
        });
        let glyphs = rustybuzz::shape(&data.buzz, &[], buffer);
        for (info, pos) in glyphs
            .glyph_infos()
            .iter()
            .zip(glyphs.glyph_positions().iter())
        {
            out.push(Glyph {
                face,
                gid: info.glyph_id as u16,
                advance: pos.x_advance as f32 / upem,
                x_offset: pos.x_offset as f32 / upem,
                y_offset: pos.y_offset as f32 / upem,
                cluster: info.cluster + base,
            });
        }
    }

    /// Width of `text` in pixels at `px` size.
    ///
    /// Chunked exactly like [`TextEngine::shape`], so the width a run is laid out
    /// with is the width its glyphs add up to, and neither path has to shape a
    /// whole long line to answer.
    pub fn width(&mut self, face: Face, text: &str, px: f32) -> f32 {
        self.width_dir(face, text, px, false)
    }

    /// Width of `text` at `px`, shaped in its own direction.
    ///
    /// Measured through the same shaping the painter uses, so an Arabic word is
    /// laid out with the width its joined glyphs actually take - measured
    /// left-to-right it would keep the isolated forms' width and every caret after
    /// it would be wrong.
    pub fn width_dir(&mut self, face: Face, text: &str, px: f32, rtl: bool) -> f32 {
        if let Some(chunks) = split_chunks(text) {
            let mut w = 0.0;
            for (a, b) in chunks {
                w += self.width_dir(face, &text[a..b], px, rtl);
            }
            return w;
        }
        let shaped = self.shape_dir(face, text, rtl);
        let mut w = 0.0;
        for g in shaped.iter() {
            w += g.advance;
        }
        w * px
    }

    /// Per-character advances of `text`, in pixels, measured **exactly the way the
    /// painter draws it**.
    ///
    /// `text_run` shapes the string once and lays the glyphs out with the advances
    /// rustybuzz returns - which include kerning and ligature substitution. Adding
    /// up single-character widths does not give the same answer: for
    /// `"AVATAR WAVE"` the whole run is 108.6 px wide while the characters on their
    /// own add up to 114.4 px, and over a long table cell the difference grows past
    /// a whole character - the text on screen then walks away from the caret and
    /// from the pointer, which is the "text is one symbol ahead" report.
    ///
    /// So this shapes the run (through the same cache the painter uses) and gives
    /// each character the advance of its own glyphs. The sum is by construction the
    /// run's own width, so every reader - the wrap, the caret, the hit test and the
    /// glyphs - is looking at one and the same measurement.
    pub fn char_advances(&mut self, face: Face, text: &str, px: f32) -> Arc<Vec<f32>> {
        self.char_advances_dir(face, text, px, false)
    }

    /// Per-character advances for text shaped in its own direction.
    ///
    /// The answer is **always in logical order**, whatever the direction: a
    /// right-to-left run comes back from rustybuzz in visual order (reversed), and
    /// the caller's caret arithmetic works on characters as they are typed, not as
    /// they are painted. Each glyph's advance is given to the character of its
    /// cluster, so the sum is the run's width by construction - the same guarantee
    /// the left-to-right version has always relied on.
    #[allow(clippy::needless_range_loop)]
    pub fn char_advances_dir(&mut self, face: Face, text: &str, px: f32, rtl: bool) -> Arc<Vec<f32>> {
        let key = (face as u8, Arc::<str>::from(text), (px * 4.0).round() as i32, rtl);
        if let Some(v) = self.char_adv_cache.get(&key) {
            return v.clone();
        }
        let shaped = self.shape_dir(face, text, rtl);
        let n_chars = text.chars().count();
        let mut out = vec![0.0f32; n_chars];
        // Byte offset -> char index, so a glyph's cluster can be attributed.
        let mut byte_to_char: Vec<(usize, usize)> = text
            .char_indices()
            .enumerate()
            .map(|(i, (b, _))| (b, i))
            .collect();
        byte_to_char.push((text.len(), n_chars));
        let char_of = |byte: u32| -> usize {
            match byte_to_char.binary_search_by_key(&(byte as usize), |e| e.0) {
                Ok(i) => byte_to_char[i].1,
                Err(i) => byte_to_char.get(i.saturating_sub(1)).map(|e| e.1).unwrap_or(0),
            }
        };
        // Every glyph belongs to exactly one cluster, so giving each glyph's
        // advance to its cluster's character makes the sum exact without caring
        // which order the glyphs arrive in - which is what lets the same code serve
        // a right-to-left run, whose clusters run downwards.
        for g in shaped.iter() {
            let c = char_of(g.cluster);
            out[c] += g.advance * px;
        }
        let total_shaped: f32 = shaped.iter().map(|g| g.advance * px).sum();
        let total_chars: f32 = out.iter().sum();
        if (total_shaped - total_chars).abs() > 0.01 && !out.is_empty() {
            // Cannot happen by construction; kept as a guard so a future change in
            // the loop cannot silently reintroduce the drift.
            out[n_chars - 1] += total_shaped - total_chars;
        }
        let arc = Arc::new(out);
        if self.char_adv_cache.len() > 20_000 {
            self.char_adv_cache.clear();
        }
        self.char_adv_cache.insert(key, arc.clone());
        arc
    }

    /// Width of one character, in pixels, from a per-character cache.
    ///
    /// The size is quantised to a quarter pixel, exactly like the glyph cache, so
    /// zooming cannot grow this table without bound.
    ///
    /// Only for callers that genuinely measure a character on its own (the search
    /// field's caret, say). A *layout* must use [`TextEngine::char_advances`], or
    /// its numbers will not add up to what is painted.
    pub fn advance(&mut self, face: Face, ch: char, px: f32) -> f32 {
        let key = (face as u8, ch, (px * 4.0).round() as i32);
        if let Some(w) = self.advance_cache.get(&key) {
            return *w;
        }
        let mut buf = [0u8; 4];
        let w = self.width(face, ch.encode_utf8(&mut buf), px);
        if self.advance_cache.len() > 4096 {
            self.advance_cache.clear();
        }
        self.advance_cache.insert(key, w);
        w
    }

    /// Rasterise a glyph; returns (bitmap, width, height, left, top) in pixels.
    /// `left`/`top` are offsets from the pen position / baseline (top positive up).
    pub fn rasterize(
        &mut self,
        face: Face,
        gid: u16,
        px: f32,
    ) -> Option<(Vec<u8>, u32, u32, i32, i32)> {
        let data = face_data(face);
        let (metrics, bitmap) = data.font.rasterize_indexed(gid, px);
        if metrics.width == 0 || metrics.height == 0 {
            return None;
        }
        Some((
            bitmap,
            metrics.width as u32,
            metrics.height as u32,
            metrics.xmin,
            metrics.ymin + metrics.height as i32,
        ))
    }

    pub fn trim(&mut self) {
        if self.shape_cache.len() > 8_000 {
            self.shape_cache.clear();
        }
        if self.char_adv_cache.len() > 8_000 {
            self.char_adv_cache.clear();
        }
        if self.cover_cache.len() > 4_000 {
            self.cover_cache.clear();
        }
    }
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}


    /// The faces are parsed **once** for the process and shared: the parse costs about
    /// 130 ms (fontdue reads every table of every font), and it used to be paid by
    /// every `TextEngine::new` - once per launch, and once per renderer in the test
    /// suite. Two engines must therefore borrow the same data, and the second must be
    /// essentially free.
    #[test]
    fn the_faces_are_parsed_once_and_shared() {
        let t = std::time::Instant::now();
        let a = TextEngine::new();
        let first = t.elapsed();
        let t = std::time::Instant::now();
        let b = TextEngine::new();
        let second = t.elapsed();
        // Same allocation, not a copy of the same bytes.
        for f in [Face::Regular, Face::Bold, Face::Italic, Face::Mono, Face::Fallback] {
            assert!(
                std::ptr::eq(a.face(f) as *const FaceData, b.face(f) as *const FaceData),
                "{f:?} was parsed twice"
            );
        }
        assert!(
            second * 4 < first.max(std::time::Duration::from_millis(5)),
            "a second engine cost {second:?} against {first:?} for the first - the faces are being reparsed"
        );
        // …and the metrics are still the ones the fonts have.
        assert!(a.face(Face::Regular).ascent_em > 0.5);
        assert!(b.face(Face::Mono).ascent_em > 0.5);
    }

    /// Shaping the same text through two engines that share the faces agrees exactly:
    /// sharing must not leak state between engines.
    #[test]
    fn two_engines_shape_identically() {
        let mut a = TextEngine::new();
        let mut b = TextEngine::new();
        for text in ["Hello, world", "Привет", "שלום", "a **bold** word", "# Heading"] {
            let ga = a.shape(Face::Regular, text);
            let gb = b.shape(Face::Regular, text);
            assert_eq!(ga, gb, "engines disagree about {text:?}");
            // Right-to-left too, which goes through a different path and cache.
            assert_eq!(
                a.shape_dir(Face::Regular, text, true),
                b.shape_dir(Face::Regular, text, true),
                "engines disagree about right-to-left {text:?}"
            );
        }
    }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_and_measures_latin() {
        let mut t = TextEngine::new();
        let w = t.width(Face::Regular, "Hello", 16.0);
        assert!(w > 20.0 && w < 100.0, "width was {w}");
        let wb = t.width(Face::Bold, "Hello", 16.0);
        assert!((wb - w).abs() < 20.0);
    }

    #[test]
    fn measures_are_linear_in_size() {
        let mut t = TextEngine::new();
        let a = t.width(Face::Regular, "abcdefg", 10.0);
        let b = t.width(Face::Regular, "abcdefg", 20.0);
        assert!((b - 2.0 * a).abs() < 0.5, "{a} {b}");
    }

    #[test]
    fn cyrillic_falls_back() {
        let mut t = TextEngine::new();
        let w = t.width(Face::Regular, "Привет", 16.0);
        assert!(w > 10.0);
        let glyphs = t.shape(Face::Regular, "Привет");
        assert!(glyphs.iter().all(|g| g.gid != 0), "missing glyphs: {glyphs:?}");
    }

    #[test]
    fn rasterizes_a_glyph() {
        let mut t = TextEngine::new();
        let g = t.shape(Face::Regular, "A");
        let glyph = g[0];
        let r = t.rasterize(glyph.face, glyph.gid, 24.0);
        assert!(r.is_some());
        let (bmp, w, h, _, _) = r.unwrap();
        assert!(w > 0 && h > 0 && bmp.iter().any(|&p| p > 32));
    }

    /// (R28) A right-to-left word that has to be split between two faces - here
    /// Latin in Lato and Hebrew in the fallback - must measure and shape like one
    /// text: every character keeps its advance, and the pieces come out in the
    /// order they are painted (reversed for RTL).
    #[test]
    fn a_mixed_face_string_keeps_every_character_wide() {
        let mut te = TextEngine::new();
        let text = "שלום עולם";
        let adv = te.char_advances_dir(Face::Regular, text, 20.0, true);
        assert_eq!(adv.len(), text.chars().count());
        for (i, w) in adv.iter().enumerate() {
            assert!(*w > 0.5, "character {i} was given {w} px: its glyph lost its advance");
        }
        let sum: f32 = adv.iter().sum();
        let width = te.width_dir(Face::Regular, text, 20.0, true);
        assert!((sum - width).abs() < 0.05, "the advances add up to {sum} but the run is {width} wide");
    }

    /// The glyphs handed to the painter must be in **paint order**: left-to-right
    /// for Latin, reversed for Hebrew, so the pen can simply run forwards.
    #[test]
    fn right_to_left_glyphs_are_returned_in_paint_order() {
        let mut te = TextEngine::new();
        let text = "אבג";
        let glyphs = te.shape_dir(Face::Regular, text, true);
        let clusters: Vec<u32> = glyphs.iter().map(|g| g.cluster).collect();
        let mut sorted = clusters.clone();
        sorted.sort_unstable();
        sorted.reverse();
        assert_eq!(clusters, sorted, "RTL glyphs must come back in reverse (visual) order: {clusters:?}");
        // …and left-to-right they are in logical order.
        let glyphs = te.shape_dir(Face::Regular, text, false);
        let clusters: Vec<u32> = glyphs.iter().map(|g| g.cluster).collect();
        let mut sorted = clusters.clone();
        sorted.sort_unstable();
        assert_eq!(clusters, sorted, "LTR glyphs must stay in logical order");
    }

    /// Arabic only gets its joined forms when the shaper is told the direction:
    /// shaped left-to-right the letters keep their isolated shapes.
    #[test]
    fn arabic_shapes_differently_in_its_own_direction() {
        let mut te = TextEngine::new();
        let text = "مرحبا";
        let rtl: Vec<u16> = te.shape_dir(Face::Regular, text, true).iter().map(|g| g.gid).collect();
        let ltr: Vec<u16> = te.shape_dir(Face::Regular, text, false).iter().map(|g| g.gid).collect();
        assert_ne!(rtl, ltr, "Arabي shaped left-to-right kept its isolated forms");
    }
}
