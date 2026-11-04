//! The About box: what this program is, what it is built from, and the licences of
//! everything inside it.
//!
//! Nothing here is read from disk at runtime. Two texts are compiled in with
//! `include_str!` and one list is generated into `OUT_DIR` by `build.rs`, so the
//! executable carries its own notices and a copy of the binary is enough to comply
//! with them:
//!
//! * `assets/fonts/LICENSE.txt` - the full licences of the three bundled typefaces
//!   (their text has to travel with the fonts, and the fonts are inside the binary);
//! * `LICENSE` - this program's own licence, the GNU Affero General Public License
//!   v3, in full;
//! * `third_party.rs` — `(name, version)` for every crate in `Cargo.lock`, generated
//!   at build time so the list cannot fall behind a dependency bump.
//!
//! `cargo run --example licences` writes the same three parts out as one file, and
//! the About box has a "Save licences…" button that does it from inside the app.

use crate::geom::Rect;

/// The licences of the bundled fonts, exactly as they are shipped.
pub const FONT_LICENCES: &str = include_str!("../../assets/fonts/LICENSE.txt");

/// This program's own licence.
pub const APP_LICENCE: &str = include_str!("../../LICENSE");

/// The crates, generated from `Cargo.lock` by `cargo run --example third_party`.
///
/// It used to be `include!(concat!(env!("OUT_DIR"), "/third_party.rs"))`, written by
/// `build.rs`. That tied *compiling* to a build script running: a tree whose
/// `build.rs` was its own (or a cached build directory) failed with "couldn't find
/// file .../out/third_party.rs" in the middle of a release build. A checked-in file
/// cannot go missing, and a test ([`tests::every_pinned_crate_is_named`]) fails the
/// moment it drifts from the lock file, so nothing is lost.
pub use super::third_party::CRATES;

/// The version, as one string: `0.8.17`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// One line of the header: what the program is.
pub const TAGLINE: &str =
    "A portable Markdown editor";

/// The three sections, without any heading above them: the licence, the crate list,
/// then the font licences.
fn sections() -> String {
    let mut out = String::new();

    out.push_str("================================================================================\n");
    out.push_str("LICENCE\n");
    out.push_str("================================================================================\n\n");
    out.push_str(APP_LICENCE.trim_end());
    out.push_str("\n\n");

    out.push_str("================================================================================\n");
    out.push_str(&format!("THIRD-PARTY CRATES ({} packages, as pinned by Cargo.lock)\n", CRATES.len()));
    out.push_str("================================================================================\n\n");
    out.push_str("This is the whole dependency graph - direct dependencies and everything they\n");
    out.push_str("pull in, since all of it is linked into this executable. Cargo.lock pins the\n");
    out.push_str("graph for every platform, so a few of these packages are there for a target\n");
    out.push_str("this build does not link.\n\n");
    out.push_str("Each one is under its own licence - MIT, Apache-2.0, BSD, Zlib and the like,\n");
    out.push_str("all permissive - and the text of each licence travels in that crate's own\n");
    out.push_str("package. This list is the index, names and versions:\n\n");
    for (name, version) in CRATES {
        out.push_str(&format!("  {name} {version}\n"));
    }
    out.push('\n');
    out.push_str(FONT_LICENCES.trim_end());
    out.push('\n');
    out
}

/// The whole notice - program licence, every crate, then the font licences - as one
/// text, ready to be written to a file or shipped beside an executable.
///
/// This is what *Save licences…* and `cargo run --example licences` write, so it
/// carries its own heading: a file that starts with `LICENCE` does not say what it is
/// a licence *of*.
pub fn notice() -> String {
    format!("Bare MD {VERSION}\n{TAGLINE}\n\n{}", sections())
}

/// The text the About box scrolls: a short "what is this" block, then the notices.
///
/// The box draws the name, the version and the tagline above the text itself, so the
/// pane starts with the summary rather than repeating the heading twice.
pub fn lines() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    out.push(format!(
        "Built on {} third-party crates, direct and transitive. Fonts: Lato, JetBrains Mono, DejaVu Sans.",
        CRATES.len()
    ));
    out.push("Licence: GNU Affero General Public License v3 - the full text is below.".to_string());
    out.push("Everything below is compiled into this executable.".to_string());
    out.push(String::new());
    for line in sections().lines() {
        out.push(line.trim_end().to_string());
    }
    out
}

// ---- the box, its geometry, and the text in it ---------------------------------
//
// One set of numbers, used by the painter that draws the box and by the wheel that
// scrolls it: a box whose scroll range is computed somewhere else drifts out of step
// with what is on screen the first time either is changed.

/// The widest the box gets, in pixels.
pub const BOX_W: f32 = 700.0;
/// The box's height. It is capped to the window at draw time.
pub const BOX_H: f32 = 520.0;
/// Height of one line of the notice text.
pub const LINE_H: f32 = 17.0;
/// Size the notice is set in. Monospaced, so a line's width is predictable.
pub const TEXT_SIZE: f32 = 12.0;
/// Columns the text is wrapped to. The notice is monospaced, so a column count is
/// exact - and wrapping once, when the box is opened, keeps the scroll range honest.
pub const COLUMNS: usize = 84;
/// Height of the header above the text: title, version and the one-line summary.
pub const HEAD_H: f32 = 92.0;
/// Height of the row the two buttons sit in. The text pane stops above it: licence
/// text running underneath the buttons is text nobody can read.
pub const BUTTON_ROW: f32 = 52.0;

/// The box, centred in a `w` x `h` window and never taller than the window.
pub fn box_rect(w: f32, h: f32) -> Rect {
    let bw = BOX_W.min(w - 40.0);
    let bh = BOX_H.min(h - 60.0);
    Rect::new((w - bw) * 0.5, (h - bh) * 0.5, bw, bh)
}

/// The text pane: the box minus the header, the buttons and the scrollbar's margin.
pub fn body_rect(d: Rect) -> Rect {
    Rect::new(d.x + 20.0, d.y + HEAD_H, d.w - 40.0 - 12.0, d.h - HEAD_H - BUTTON_ROW)
}

/// The scrollbar's track: a thin column to the right of the text.
pub fn track_rect(d: Rect) -> Rect {
    let b = body_rect(d);
    Rect::new(b.right() + 4.0, b.y, 4.0, b.h)
}

/// Height of the thumb - the part of the column the notice fills.
fn thumb_height(track_h: f32, len: usize, visible: usize) -> f32 {
    let frac = (visible as f32 / len.max(1) as f32).clamp(0.05, 1.0);
    (track_h * frac).max(24.0).min(track_h)
}

/// The thumb, for a given scroll position.
pub fn thumb_rect(track: Rect, scroll: usize, len: usize, visible: usize) -> Rect {
    let th = thumb_height(track.h, len, visible);
    let max_scroll = len.saturating_sub(visible);
    let t = if max_scroll == 0 { 0.0 } else { (scroll as f32 / max_scroll as f32).clamp(0.0, 1.0) };
    Rect::new(track.x, track.y + (track.h - th) * t, track.w, th)
}

/// Where a pointer at `y` puts the notice, when the pointer grabbed the thumb at
/// `grab` (0 = top edge of the thumb, 1 = bottom edge).
///
/// This is the *inverse* of [`thumb_rect`]: the thumb's top travels the track as the
/// scroll goes from 0 to the last screen, so the pointer's travel maps onto that
/// same range. Dragging therefore tracks the pointer exactly instead of accelerating
/// with distance, and the thumb does not jump when it is grabbed off-centre.
pub fn scroll_for_pointer(track: Rect, grab: f32, y: f32, len: usize, visible: usize) -> usize {
    let th = thumb_height(track.h, len, visible);
    let travel = (track.h - th).max(0.0);
    let max_scroll = len.saturating_sub(visible);
    if travel <= 0.0 || max_scroll == 0 {
        return 0;
    }
    let top = y - grab.clamp(0.0, 1.0) * th - track.y;
    let t = (top / travel).clamp(0.0, 1.0);
    (t * max_scroll as f32).round() as usize
}

/// How many lines of text fit between the header and the buttons.
pub fn visible_lines(bh: f32) -> usize {
    (((bh - HEAD_H - BUTTON_ROW - 12.0) / LINE_H).floor() as i32).max(1) as usize
}

/// Shape the first screenful of the box, so opening it does not pay for that.
///
/// Building the notice costs ~1 ms, but *shaping* the 21 lines on the first screen
/// costs about 30 ms in a debug build (rustybuzz, once per distinct string) - and that
/// landed on the frame that first drew the box, which is exactly what "the delay is
/// loading the text in About" looks like from the outside. The shaping cache is keyed
/// by the string, not by the size, so shaping the page here makes the box's first
/// frame cost the same as every later one.
///
/// Call it once, when the app has nothing better to do - after the first frame is on
/// screen - and never at start-up.
pub fn warm(r: &mut crate::gpu::Renderer) {
    let about = About::new();
    for line in about.text.iter().take(about.visible.max(1) + 4) {
        if !line.is_empty() {
            r.warm_text(crate::text::Face::Mono, TEXT_SIZE, line);
        }
    }
    // The header line above the text is set in the UI face, and the dialog's buttons
    // and footer are that face too - a few hundred glyphs' worth of shapes is nothing,
    // but the glyphs themselves would be rasterised on the first frame otherwise.
    r.warm_text(crate::text::Face::Regular, 13.0, "Bare MD version Close Save licences… Everything above is inside this executable.");
    r.warm_text(crate::text::Face::Regular, 19.0, "Bare MD 0123456789.");
    r.warm_text(crate::text::Face::Regular, 11.5, "A portable Markdown editor: pure Rust, winit + wgpu, text rendering from scratch.");
}

/// Break `text` into display lines of at most `width` columns, preferring a space.
///
/// The licence text is prose with a few long runs (URLs, licence clauses in caps); a
/// long word is broken rather than allowed to run past the box.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut out: Vec<String> = Vec::new();
    for raw in text.lines() {
        let raw = raw.trim_end();
        if raw.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in raw.split(' ') {
            if line.is_empty() {
                line.push_str(word);
            } else if line.chars().count() + 1 + word.chars().count() <= width {
                line.push(' ');
                line.push_str(word);
            } else {
                out.push(std::mem::take(&mut line));
                line.push_str(word);
            }
            // A single word longer than the box is broken where it stands.
            while line.chars().count() > width {
                let head: String = line.chars().take(width).collect();
                out.push(head);
                line = line.chars().skip(width).collect();
            }
        }
        out.push(line);
    }
    out
}

/// The state the About dialog keeps between frames: the wrapped text, and where it is
/// scrolled to.
#[derive(Clone, Debug)]
pub struct About {
    /// The notice, already wrapped to [`COLUMNS`].
    pub text: Vec<String>,
    /// First visible line.
    pub scroll: usize,
    /// How many lines the box showed last time it was drawn (`scroll` is clamped
    /// against it).
    pub visible: usize,
}

impl Default for About {
    fn default() -> Self {
        Self::new()
    }
}

impl About {
    pub fn new() -> About {
        let mut a = About { text: wrap(&lines().join("\n"), COLUMNS), scroll: 0, visible: 1 };
        a.visible = visible_lines(BOX_H);
        a
    }

    /// Tell the box how many lines fit. A window resized smaller must not leave the
    /// text scrolled past its own end.
    pub fn clamp(&mut self, visible: usize) {
        self.visible = visible.max(1);
        let max = self.text.len().saturating_sub(self.visible);
        self.scroll = self.scroll.min(max);
    }

    /// Scroll by `delta` lines (negative = up). Returns true when anything moved.
    pub fn scroll_by(&mut self, delta: i32) -> bool {
        let max = self.text.len().saturating_sub(self.visible);
        let next = (self.scroll as i64 + delta as i64).clamp(0, max as i64) as usize;
        let moved = next != self.scroll;
        self.scroll = next;
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The font notices are the whole licence text, not a summary: the OFL and the
    /// DejaVu/Arev texts both have to be in the binary.
    #[test]
    fn the_font_licences_are_compiled_in_whole() {
        assert!(FONT_LICENCES.contains("SIL OPEN FONT LICENSE, VERSION 1.1"));
        assert!(FONT_LICENCES.contains("SECTION B: DEJAVU FONTS LICENSE"));
        assert!(FONT_LICENCES.contains("Bitstream Vera"));
        assert!(FONT_LICENCES.len() > 8000, "the licence text is only {} bytes", FONT_LICENCES.len());
        // …and the program's own licence too: AGPL v3 or later, in full.
        assert!(APP_LICENCE.contains("GNU AFFERO GENERAL PUBLIC LICENSE"));
        assert!(APP_LICENCE.contains("Version 3, 19 November 2007"));
        assert!(APP_LICENCE.contains("Remote Network Interaction"));
        assert!(APP_LICENCE.len() > 30000, "the licence text is only {} bytes", APP_LICENCE.len());
    }

    /// Every crate in `Cargo.lock` is named, with the version the lock pins.
    #[test]
    fn every_pinned_crate_is_named() {
        let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock")).unwrap();
        let mut want: Vec<(String, String)> = Vec::new();
        let (mut name, mut version) = (String::new(), String::new());
        for line in lock.lines().map(str::trim) {
            if line == "[[package]]" {
                if !name.is_empty() && !version.is_empty() {
                    want.push((name.clone(), version.clone()));
                }
                name.clear();
                version.clear();
            } else if let Some(rest) = line.strip_prefix("name = \"") {
                name = rest.trim_end_matches('"').to_string();
            } else if let Some(rest) = line.strip_prefix("version = \"") {
                version = rest.trim_end_matches('"').to_string();
            }
        }
        if !name.is_empty() && !version.is_empty() {
            want.push((name, version));
        }
        want.retain(|(n, _)| n != env!("CARGO_PKG_NAME"));
        want.sort();
        want.dedup();
        let got: Vec<(String, String)> =
            CRATES.iter().map(|(n, v)| (n.to_string(), v.to_string())).collect();
        assert_eq!(
            got, want,
            "src/ui/third_party.rs is stale - run `cargo run --example third_party` to refresh it"
        );
        assert!(CRATES.len() > 50, "only {} crates listed", CRATES.len());
        // The heavyweights this editor is built on are all there.
        for needle in ["wgpu", "winit", "ropey", "rustybuzz"] {
            assert!(CRATES.iter().any(|(n, _)| *n == needle), "{needle} is missing");
        }
    }

    /// Wrapping keeps every line inside the box and loses nothing: what the box shows
    /// has to be the licence, not a mangled copy of it.
    #[test]
    fn wrapping_keeps_every_word_and_every_line_inside_the_box() {
        let text = notice();
        let wrapped = wrap(&text, COLUMNS);
        for line in &wrapped {
            assert!(line.chars().count() <= COLUMNS, "line too long: {line:?}");
            assert_eq!(line.trim_end(), line, "trailing space in {line:?}");
        }
        // Words survive, in order, allowing for lines broken mid-word.
        let flat: String = wrapped.concat();
        let squashed: String = flat.split_whitespace().collect::<Vec<_>>().join("");
        let want: String = text.split_whitespace().collect::<Vec<_>>().join("");
        assert_eq!(squashed, want, "wrapping changes the text");
        // Blank lines are kept: the notice is read in sections.
        assert!(wrapped.iter().any(|l| l.is_empty()));
        // A width of zero must not hang: it clamps to something usable.
        assert!(wrap("aaaa", 0).iter().all(|l| l.chars().count() <= 8));
    }

    /// The box fits the window it is drawn in, and the line count it reports is the
    /// number of lines that actually fit.
    #[test]
    fn the_box_fits_the_window() {
        let d = box_rect(1600.0, 1000.0);
        assert!(d.w <= BOX_W && d.h <= BOX_H);
        assert!((d.x + d.w * 0.5 - 800.0).abs() < 0.01, "not centred horizontally");
        assert!((d.y + d.h * 0.5 - 500.0).abs() < 0.01, "not centred vertically");
        // A window smaller than the box shrinks the box rather than overflowing.
        let small = box_rect(600.0, 400.0);
        assert!(small.w <= 560.0 && small.h <= 340.0);
        assert!(small.x >= 0.0 && small.y >= 0.0);
        assert!(small.right() <= 600.0 && small.bottom() <= 400.0);
        // More height, more lines, and never zero.
        assert!(visible_lines(BOX_H) >= 20);
        assert!(visible_lines(HEAD_H + 5.0) >= 1);
        assert_eq!(visible_lines(0.0), 1);
    }

    /// The box opens at the top, and the whole notice is reachable by scrolling.
    #[test]
    fn the_whole_notice_can_be_reached() {
        let mut a = About::new();
        a.clamp(visible_lines(BOX_H));
        assert_eq!(a.scroll, 0, "the box opens at the top");
        let last = a.text.len().saturating_sub(a.visible);
        for _ in 0..1000 {
            a.scroll_by(a.visible as i32);
        }
        assert_eq!(a.scroll, last);
        // …and the last page of text really is the end of the licence.
        let want = wrap(&notice(), COLUMNS);
        assert!(a.visible < want.len());
        assert_eq!(
            a.text.iter().rev().take(a.visible).rev().cloned().collect::<Vec<_>>(),
            want.iter().rev().take(a.visible).rev().cloned().collect::<Vec<_>>(),
            "the last screen is not the end of the notice"
        );
        assert!(want.last().map_or(false, |l| l.contains("tavmjong")), "the notice ends elsewhere");
    }

    /// The scrollbar is one mapping, and it goes both ways: the thumb drawn for a
    /// scroll position is the thumb that, grabbed, gives that position back. A drawn
    /// thumb and a grabbed thumb that disagree are a scrollbar that fights the pointer.
    #[test]
    fn the_thumb_and_the_drag_agree_about_where_the_notice_is() {
        let d = box_rect(1600.0, 900.0);
        let track = track_rect(d);
        assert!(track.w > 0.0 && track.h > 100.0);
        // The track sits to the right of the text pane, inside the box.
        assert!(track.x > body_rect(d).right() && track.right() <= d.right());
        assert!((track.y - body_rect(d).y).abs() < 0.01);

        let len = 600;
        let visible = 22;
        let max = len - visible;
        for scroll in [0, 1, max / 3, max / 2, max - 1, max] {
            let thumb = thumb_rect(track, scroll, len, visible);
            assert!((thumb.h - thumb_rect(track, 0, len, visible).h).abs() < 0.01, "thumb size varies");
            // Grabbing the thumb at its top edge and holding still does not move it.
            let back = scroll_for_pointer(track, 0.0, thumb.y, len, visible);
            assert!(
                back.abs_diff(scroll) <= 1,
                "the thumb for {scroll} maps back to {back}"
            );
            // …nor does grabbing it in the middle.
            let mid = scroll_for_pointer(track, 0.5, thumb.y + thumb.h * 0.5, len, visible);
            assert!(mid.abs_diff(scroll) <= 1, "grabbed mid-thumb, {scroll} became {mid}");
        }
        // The thumb never leaves the track, and it fills it when everything fits.
        let top = thumb_rect(track, 0, len, visible);
        let last = thumb_rect(track, max, len, visible);
        assert!((top.y - track.y).abs() < 0.01);
        assert!((last.bottom() - track.bottom()).abs() < 0.01);
        let all = thumb_rect(track, 0, 20, 20);
        assert!((all.h - track.h).abs() < 0.01, "a notice that fits should fill the track");
        assert_eq!(scroll_for_pointer(track, 0.5, track.y + 5.0, 20, 20), 0);
        // Dragging past either end stops at the end instead of running off.
        assert_eq!(scroll_for_pointer(track, 0.0, track.y - 400.0, len, visible), 0);
        assert_eq!(scroll_for_pointer(track, 0.0, track.bottom() + 400.0, len, visible), max);
    }

    /// Compiling must never depend on the build script having produced a file.
    ///
    /// This is the bug a release build hit: `include!(concat!(env!("OUT_DIR"),
    /// "/third_party.rs"))` needs `build.rs` to have run, and a tree with its own (or
    /// a cached) build script stopped with *"couldn't find file .../out/third_party.rs"*.
    /// Every generated thing in this crate is now a checked-in source file, so the
    /// rule is worth pinning: no `src/*.rs` may include from `OUT_DIR`, and `build.rs`
    /// must not write `.rs` files at all.
    #[test]
    fn nothing_compiles_a_file_the_build_script_writes() {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("a source directory") {
                let p = entry.expect("a directory entry").path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().map(|e| e == "rs").unwrap_or(false) {
                    out.push(p);
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        walk(&root.join("src"), &mut files);
        assert!(files.len() > 10, "the source walk found {} files", files.len());
        // Built at run time so that this test's own source does not contain the
        // pattern it looks for (the first version matched itself).
        let needle = ["OUT", "_DIR"].concat();
        for f in &files {
            let src = std::fs::read_to_string(f).expect("a source file");
            for (i, line) in src.lines().enumerate() {
                // Comments explain the rule; only code can break it.
                let code = line.trim_start();
                if code.starts_with("//") || code.starts_with('*') {
                    continue;
                }
                if line.contains(&needle) {
                    panic!(
                        "{}:{} compiles a build-script output: {}",
                        f.display(),
                        i + 1,
                        line.trim()
                    );
                }
            }
        }
        let build = std::fs::read_to_string(root.join("build.rs")).expect("build.rs");
        for (i, line) in build.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") || code.starts_with('*') {
                continue;
            }
            if line.contains(&needle) {
                panic!("build.rs:{} still writes into a build output: {}", i + 1, line.trim());
            }
        }
    }

    /// Scrolling stops at both ends instead of running off the page.
    #[test]
    fn scrolling_stops_at_the_ends() {
        let mut a = About::new();
        a.clamp(20);
        assert!(a.text.len() > 40, "this test needs a long text");
        assert!(a.scroll_by(-5) == false, "the top is not a scroll position");
        assert_eq!(a.scroll, 0);
        assert!(a.scroll_by(60));
        assert_eq!(a.scroll, 60, "a small scroll moves by exactly that much");
        let end = a.text.len() - 20;
        assert!(a.scroll_by(a.text.len() as i32 * 2));
        assert_eq!(a.scroll, end, "the last screen is the end");
        assert!(!a.scroll_by(60), "there is nothing below the end");
        assert_eq!(a.scroll, end);
        // A taller box means less to scroll.
        a.clamp(a.text.len().saturating_sub(5).max(1));
        assert_eq!(a.scroll, 5.min(a.text.len().saturating_sub(a.visible)));
    }
}
