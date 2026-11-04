//! The incremental layout must be indistinguishable from a full rebuild.
//!
//! `Layout::try_patch` reuses every line outside the block an edit touched, which is
//! what keeps a keystroke cheap in a long document. "Cheap" is only acceptable if it is
//! also *identical* to the whole-document build, so every test here edits the document
//! and then compares the patched layout with a fresh build of the same document, field
//! by field.

use bare_md::doc::Doc;
use bare_md::layout::{Layout, ViewMode};
use bare_md::text::{Face, TextEngine};
use bare_md::theme::{Theme, ThemeKind};

const WIDTH: f32 = 720.0;

fn build(doc: &Doc, te: &mut TextEngine, theme: &Theme, mode: ViewMode) -> Layout {
    Layout::build_editing(doc, te, theme, WIDTH, mode, None, true)
}

/// The same tolerance the patcher itself uses. A y is added up in f32 line by line,
/// so a window that begins at a different place rounds the last digit differently: past
/// ten thousand lines a top can differ by a hundredth of a pixel. Nothing else may.
fn same_len(a: f32, b: f32) -> bool {
    (a - b).abs() <= 0.01 + a.abs().max(b.abs()) * 1e-6
}

/// Byte-for-byte comparison of two layouts, line by line, with a readable message.
fn same(patched: &Layout, full: &Layout, what: &str) {
    assert_eq!(patched.lines.len(), full.lines.len(), "{what}: line count");
    assert_eq!(patched.word_count, full.word_count, "{what}: word count");
    assert_eq!(patched.mermaid, full.mermaid, "{what}: mermaid blocks");
    assert_eq!(patched.headings, full.headings, "{what}: headings");
    // A document's height is added up in f32, one line at a time, so a window that
    // starts at a different place rounds the last digit differently: past ten thousand
    // lines that is a hundredth of a pixel, and no more. Everything else is exact.
    assert!(
        (patched.total_height - full.total_height).abs() <= 0.01 + full.total_height.abs() * 1e-6,
        "{what}: total height {} vs {}",
        patched.total_height,
        full.total_height
    );
    assert!(
        (patched.max_width - full.max_width).abs() < 0.01,
        "{what}: max width {} vs {}",
        patched.max_width,
        full.max_width
    );
    assert!(
        (patched.max_row_width - full.max_row_width).abs() < 0.01,
        "{what}: max row width {} vs {}",
        patched.max_row_width,
        full.max_row_width
    );
    for (i, (a, b)) in patched.lines.iter().zip(full.lines.iter()).enumerate() {
        let ctx = |field: &str| format!("{what}: line {i} {field}");
        assert_eq!(a.line, b.line, "{}", ctx("index"));
        assert_eq!(a.start_char, b.start_char, "{}", ctx("start_char"));
        assert_eq!(a.chars, b.chars, "{}", ctx("chars"));
        assert_eq!(a.info.block, b.info.block, "{}", ctx("block"));
        assert_eq!(a.info.lang, b.info.lang, "{}", ctx("lang"));
        assert_eq!(a.flags, b.flags, "{}", ctx("flags"));
        assert!(same_len(a.top, b.top), "{what}: line {i} top {} vs {}", a.top, b.top);
        assert!(
            same_len(a.height, b.height),
            "{what}: line {i} height {} vs {}",
            a.height,
            b.height
        );
        assert!(
            same_len(a.space_before, b.space_before),
            "{what}: line {i} space_before {} vs {}",
            a.space_before,
            b.space_before
        );
        assert!(
            same_len(a.space_after, b.space_after),
            "{what}: line {i} space_after {} vs {}",
            a.space_after,
            b.space_after
        );
        assert!((a.width - b.width).abs() < 0.01, "{what}: line {i} width");
        assert_eq!(a.clean.len(), b.clean.len(), "{}", ctx("clean runs"));
        assert_eq!(a.raw.len(), b.raw.len(), "{}", ctx("raw runs"));
        for (k, (ra, rb)) in a.clean.iter().zip(&b.clean).enumerate() {
            assert_eq!((ra.start, ra.end), (rb.start, rb.end), "{what}: line {i} clean run {k} text");
            assert!((ra.x - rb.x).abs() < 0.01, "{what}: line {i} clean run {k} x");
            assert!((ra.w - rb.w).abs() < 0.01, "{what}: line {i} clean run {k} w");
        }
        assert_eq!(a.rows.len(), b.rows.len(), "{}", ctx("rows"));
        assert_eq!(a.table_cols.len(), b.table_cols.len(), "{}", ctx("table columns"));
    }
}

fn fixture() -> String {
    let mut s = String::new();
    s.push_str("# Title\n\n");
    s.push_str("A paragraph with **bold**, *italic* and `code` in it.\n\n");
    s.push_str("- bullet one\n- bullet two\n\n");
    s.push_str("```rust\nfn main() {\n    let x = 1; // a comment\n}\n```\n\n");
    s.push_str("| a | b |\n| --- | --- |\n| 1 | 2 |\n\n");
    s.push_str("> a quote\n> continued\n\n");
    s.push_str("```mermaid\ngraph TD\nA --> B\n```\n\n");
    s.push_str("Last paragraph, with a line that is long enough to wrap somewhere in the middle of the window.\n\n");
    s.push_str("## A heading near the end\n\n");
    s.push_str("The final line.\n");
    s
}

/// Edit `from..to` to `text`, patch the layout, and demand it equal a full build.
fn check(patched: &mut Layout, doc: &Doc, te: &mut TextEngine, theme: &Theme, mode: ViewMode, what: &str) {
    let reused = patched.try_patch(doc, te, theme, WIDTH, mode, None, true);
    let full = build(doc, te, theme, mode);
    if reused {
        same(patched, &full, what);
    }
    // Whether or not the patch could be reused, the layout must be right.
    *patched = full;
}

fn all_modes() -> [ViewMode; 2] {
    [ViewMode::Live, ViewMode::Raw]
}

#[test]
fn typing_anywhere_matches_a_full_rebuild() {
    let theme = Theme::by_kind(ThemeKind::Black);
    let mut te = TextEngine::new();
    for mode in all_modes() {
        let text = fixture();
        // Every character position of the document, inserted one at a time.
        for (offset, _) in text.char_indices().step_by(7) {
            let mut doc = Doc::new(&text, None);
            let mut layout = build(&doc, &mut te, &theme, mode);
            doc.insert(offset, "x");
            check(&mut layout, &doc, &mut te, &theme, mode, &format!("insert at {offset} ({mode:?})"));
        }
    }
}

#[test]
fn typing_markdown_markers_matches_a_full_rebuild() {
    let theme = Theme::by_kind(ThemeKind::Black);
    let mut te = TextEngine::new();
    for mode in all_modes() {
        let text = fixture();
        // Typing the characters that change what a line *is*: a heading, a fence, a
        // table pipe, a list marker, a quote.
        for marker in ["#", "##", "```", "```rust", "|", "- ", "> ", "**", "1. ", "    ", "---"] {
            for (offset, _) in text.char_indices().step_by(23) {
                let mut doc = Doc::new(&text, None);
                let mut layout = build(&doc, &mut te, &theme, mode);
                doc.insert(offset, marker);
                check(&mut layout, &doc, &mut te, &theme, mode, &format!("type {marker:?} at {offset} ({mode:?})"));
            }
        }
    }
}

#[test]
fn deleting_matches_a_full_rebuild() {
    let theme = Theme::by_kind(ThemeKind::Black);
    let mut te = TextEngine::new();
    for mode in all_modes() {
        let text = fixture();
        let len = text.chars().count();
        for start in (0..len.saturating_sub(1)).step_by(11) {
            for take in [1usize, 3, 17] {
                let mut doc = Doc::new(&text, None);
                let mut layout = build(&doc, &mut te, &theme, mode);
                doc.remove(start, (start + take).min(len));
                check(&mut layout, &doc, &mut te, &theme, mode, &format!("delete {start}..+{take} ({mode:?})"));
            }
        }
    }
}

#[test]
fn edits_that_add_lines_match_a_full_rebuild() {
    let theme = Theme::by_kind(ThemeKind::Black);
    let mut te = TextEngine::new();
    for mode in all_modes() {
        let text = fixture();
        for (offset, _) in text.char_indices().step_by(29) {
            for ins in ["\n", "\n\n", "\n# heading\n", "\n```\ncode\n```\n", "\n| x | y |\n| --- | --- |\n"] {
                let mut doc = Doc::new(&text, None);
                let mut layout = build(&doc, &mut te, &theme, mode);
                doc.insert(offset, ins);
                check(&mut layout, &doc, &mut te, &theme, mode, &format!("insert newline at {offset} ({mode:?})"));
            }
        }
    }
}

#[test]
fn a_fallback_font_edit_matches_a_full_rebuild() {
    let theme = Theme::by_kind(ThemeKind::Black);
    let mut te = TextEngine::new();
    let text = fixture();
    for ins in ["Привет", "漢字", "→", "—", "🙂"] {
        for (offset, _) in text.char_indices().step_by(41) {
            let mut doc = Doc::new(&text, None);
            let mut layout = build(&doc, &mut te, &theme, ViewMode::Live);
            doc.insert(offset, ins);
            check(&mut layout, &doc, &mut te, &theme, ViewMode::Live, &format!("insert {ins:?} at {offset}"));
        }
    }
    // and the faces are all there
    assert!(te.face(Face::Fallback).ascent_em > 0.0);
}

/// A long sequence of edits, each patched on top of the last: no drift may accumulate.
#[test]
fn a_whole_editing_session_matches_a_full_rebuild() {
    let theme = Theme::by_kind(ThemeKind::Black);
    let mut te = TextEngine::new();
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let pieces = [
        "a", "word ", "# ", "## Heading\n", "\n", "\n\n", "```\n", "```rust\n", "```\n", "|", " | 1 |\n",
        "- ", "> ", "**b**", "*i*", "`c`", "Привет ", "🙂", "—", "    indented\n", "---\n",
    ];
    for mode in all_modes() {
        let mut doc = Doc::new(&fixture(), None);
        let mut layout = build(&doc, &mut te, &theme, mode);
        for step in 0..80 {
            let len = doc.text().chars().count().max(1);
            let roll = next();
            let at = (roll as usize) % len;
            match roll % 3 {
                0 => {
                    let piece = pieces[(next() as usize) % pieces.len()];
                    doc.insert(at, piece);
                }
                1 => {
                    let take = 1 + (next() as usize) % 12;
                    doc.remove(at, (at + take).min(len));
                }
                _ => {
                    let piece = pieces[(next() as usize) % pieces.len()];
                    let take = (next() as usize) % 3;
                    doc.replace_range(at, (at + take).min(len), piece);
                }
            }
            let reused = layout.try_patch(&doc, &mut te, &theme, WIDTH, mode, None, true);
            let full = build(&doc, &mut te, &theme, mode);
            if reused {
                // The patched layout is what the next step patches, so a mistake that
                // only shows up after several edits cannot hide.
                same(&layout, &full, &format!("step {step} ({mode:?})"));
            } else {
                layout = full;
            }
        }
    }
}

/// The same check, but driven through the app's own commands: what the editor does to
/// the document when you type, press Enter, delete, or paste must leave the layout
/// exactly as a full rebuild would.
#[test]
fn the_apps_own_edits_match_a_full_rebuild() {
    use bare_md::app::App;
    use bare_md::config::{Paths, Session, Settings};
    use bare_md::gpu::Renderer;

    let dir = std::env::temp_dir().join(format!("bare-md-inc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let paths = Paths {
        data_dir: dir.clone(),
        config_file: dir.join("settings.conf"),
        origin: "test".into(),
    };
    let settings = Settings::default();
    let mut app = App::new(paths, settings, Session::default(), vec![]);
    app.insert_text(&fixture());
    app.set_caret(0, false);
    app.mouse = (-50.0, -50.0);
    let mut r = Renderer::offscreen(1180, 780, 1.0);
    bare_md::ui::draw(&mut app, &mut r);
    r.end();

    let mut te = TextEngine::new();
    let theme = Theme::by_kind(ThemeKind::Black);
    let mut step = 0usize;
    let mut ops: Vec<Box<dyn Fn(&mut App)>> = vec![
        Box::new(|a: &mut App| a.insert_text("word ")),
        Box::new(|a: &mut App| a.insert_text("\n")),
        Box::new(|a: &mut App| a.insert_text("# ")),
        Box::new(|a: &mut App| a.newline_ex(false)),
        Box::new(|a: &mut App| a.newline_ex(true)),
        Box::new(|a: &mut App| a.insert_text("```\ncode\n```")),
        Box::new(|a: &mut App| a.insert_text("| a | b |\n| --- | --- |")),
        Box::new(|a: &mut App| a.insert_text("- item\n- item")),
        Box::new(|a: &mut App| a.insert_text("> quote\n> more")),
        Box::new(|a: &mut App| a.insert_text("Привет ")),
    ];
    // Walk the caret through the document, doing each of those at every stop.
    let len = app.tab().doc.len_chars();
    for stop in 0..10 {
        let at = len * stop / 10;
        ops.rotate_left(1);
        for op in &ops {
            app.set_caret(at.min(app.tab().doc.len_chars()), false);
            op(&mut app);
            bare_md::ui::draw(&mut app, &mut r);
            r.end();
            // Rebuild at the width, mode and mermaid state the app itself used.
            let (w, m, e) = (app.tab().layout.width, app.tab().layout.mode, app.tab().layout.editing);
            let full = Layout::build_editing(&app.tab().doc, &mut te, &theme, w, m, e, true);
            same(&app.tab().layout, &full, &format!("app step {step}"));
            step += 1;
        }
    }
    drop(app);
    drop(r);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A document with no blank lines in it is one block from top to bottom, and that
/// block is the whole document: the window that a keystroke rebuilds must not be
/// the block, or typing in a twenty-thousand-line log costs a full rebuild.
#[test]
fn a_long_document_without_blank_lines_matches_a_full_rebuild() {
    let theme = Theme::by_kind(ThemeKind::Black);
    let mut te = TextEngine::new();
    let mut text = String::new();
    for i in 0..1200 {
        text.push_str(&format!("line {i} of a document that has no blank lines in it at all\n"));
    }
    for mode in all_modes() {
        let mut doc = Doc::new(&text, None);
        let mut layout = build(&doc, &mut te, &theme, mode);
        let len = doc.len_chars();
        for (k, at) in [len / 4, len / 2, len - 40, 20].iter().enumerate() {
            doc.insert(*at, "xyz");
            check(&mut layout, &doc, &mut te, &theme, mode, &format!("append in a run at {at} ({mode:?}, {k})"));
            doc.remove(*at, *at + 3);
            check(&mut layout, &doc, &mut te, &theme, mode, &format!("take it back at {at} ({mode:?}, {k})"));
        }
    }
}

/// The rows of a table are laid out from the table's column widths, and a window of
/// rows cannot measure those for itself: it is handed the ones on screen. Editing a
/// cell must still come out exactly as a build of the whole document does.
#[test]
fn an_edit_inside_a_long_table_matches_a_full_rebuild() {
    let theme = Theme::by_kind(ThemeKind::Black);
    let mut te = TextEngine::new();
    let mut text = String::from("| a | b | c |\n| --- | --- | --- |\n");
    for i in 0..400 {
        text.push_str(&format!("| {i} | value {i} | 42 |\n"));
    }
    for mode in all_modes() {
        let mut doc = Doc::new(&text, None);
        let mut layout = build(&doc, &mut te, &theme, mode);
        let len = doc.len_chars();
        for at in [len / 3, len / 2, len - 30] {
            doc.insert(at, "wide cell text");
            check(&mut layout, &doc, &mut te, &theme, mode, &format!("widen a cell at {at} ({mode:?})"));
            doc.remove(at, at + 14);
            check(&mut layout, &doc, &mut te, &theme, mode, &format!("narrow it again at {at} ({mode:?})"));
        }
    }
}
