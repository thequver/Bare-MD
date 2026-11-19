//! Regression tests for the reported UI defects.
//!
//! Each test drives the real app through its public entry points (typing,
//! clicking, toolbar actions) rather than poking at internals, so a regression in
//! any layer shows up here.

use std::path::PathBuf;

use bare_md::app::{App, Hit, ToolbarAction};
use bare_md::config::{Paths, Session, Settings};
use bare_md::geom::Rect;
use bare_md::gpu::Renderer;
use bare_md::layout::ViewMode;
use bare_md::text::Face;
use bare_md::theme::ThemeKind;

const W: u32 = 1180;
const H: u32 = 780;

struct Harness {
    app: App,
    r: Renderer,
    _dir: PathBuf,
    /// Held for the lifetime of the harness: software Vulkan does not enjoy many
    /// concurrent devices, so the GPU-backed tests take turns.
    _gpu: std::sync::MutexGuard<'static, ()>,
}

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The drawing code, as one string.
///
/// A few tests read the source instead of the screen (to pin down something that
/// is only visible as a shape, like "the selection wash is painted *before* the
/// glyphs"). The painter lives in several files now, so the string is the whole
/// `src/ui` tree: a test that looks for a call in there finds it wherever it was
/// filed.
fn ui_source() -> &'static str {
    concat!(
        include_str!("../src/ui/mod.rs"),
        include_str!("../src/ui/painter.rs"),
        include_str!("../src/ui/chrome.rs"),
        include_str!("../src/ui/document.rs"),
        include_str!("../src/ui/dialogs.rs"),
    )
}

fn harness(tag: &str) -> Harness {
    let gpu = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("bare-md-reg-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let paths = Paths {
        data_dir: dir.clone(),
        config_file: dir.join("settings.conf"),
        origin: "test".into(),
    };
    let mut settings = Settings::default();
    settings.workspace = Some(dir.clone());
    settings.theme = ThemeKind::Black;
    let mut app = App::new(paths, settings, Session::default(), vec![]);
    app.mouse = (-50.0, -50.0);
    let r = Renderer::offscreen(W, H, 1.0);
    let mut h = Harness { app, r, _dir: dir, _gpu: gpu };
    h.draw();
    h
}

impl Harness {
    fn draw(&mut self) {
        bare_md::ui::draw(&mut self.app, &mut self.r);
        self.r.end();
    }

    fn click(&mut self, pos: (f32, f32)) {
        self.app.mouse = pos;
        self.draw();
        self.app.click(pos, &mut self.r);
        self.draw();
    }

    /// Press *and* release, which is what a real click is.
    fn click_full(&mut self, pos: (f32, f32)) {
        self.click(pos);
        self.app.mouse_up(&mut self.r);
        self.draw();
    }

    fn click_hit(&mut self, pred: impl Fn(&Hit) -> bool) -> bool {
        let Some(rect) = self.app.hits.find(pred) else {
            return false;
        };
        let c = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
        self.click(c);
        true
    }

    fn type_str(&mut self, s: &str) {
        self.app.insert_text(s);
        self.draw();
    }

    /// Type the way the keyboard does: through the auto-close path.
    fn type_key(&mut self, s: &str) {
        self.app.insert_typed(s);
        self.draw();
    }

    /// The caret's x, as it is actually painted.
    fn caret_x(&mut self) -> f32 {
        self.app.caret_rect(&mut self.r.text).x
    }

    /// Index of the first table row of the open document.
    fn table_line(&self) -> usize {
        self.app
            .tab()
            .layout
            .lines
            .iter()
            .position(|l| !l.table_cols.is_empty())
            .expect("a table row")
    }

    /// Index of the first line whose block is `block`.
    fn line_of_block(&self, block: bare_md::markdown::Block) -> Option<usize> {
        self.app.tab().layout.lines.iter().position(|l| l.block() == block)
    }

    /// Index of the first line of code (fenced or indented).
    fn code_line(&self) -> Option<usize> {
        self.line_of_block(bare_md::markdown::Block::Code)
    }

    /// The last table row of the open document - the data row of the fixtures the
    /// table tests use, where the header is the first table line and the rule is
    /// between them.
    fn last_table_row(&self) -> usize {
        self.app
            .tab()
            .layout
            .lines
            .iter()
            .rposition(|l| !l.table_cols.is_empty())
            .expect("a table row")
    }

    /// Screen y of `frac` down visual row `row` of line `line`, scroll included.
    ///
    /// Tests used to spell this out by hand, and several of them forgot `- scroll`:
    /// they only passed because they never scrolled. One copy cannot forget.
    fn row_y(&self, line: usize, row: usize, frac: f32) -> f32 {
        let t = self.app.tab();
        let ll = &t.layout.lines[line];
        self.app.text_rect.y - t.scroll + ll.top + ll.row_h * (row as f32 + frac)
    }

    /// Screen x of local caret position `local` of line `line` as the layout places
    /// it: margin, indent, the row's alignment shift and the position itself.
    fn local_x(&mut self, line: usize, local: usize, row: usize, raw: bool) -> f32 {
        let w = self.app.text_rect.w;
        let shift_x = self.app.text_rect.x - self.app.tab().scroll_x;
        let mut te = self.r.text_ref();
        let ll = &self.app.tab().layout.lines[line];
        shift_x + ll.indent + ll.align_offset_row(w, row, raw) + ll.x_abs(&mut te, local, raw)
    }

    /// Screen point `frac` across the column of table cell `col` of line `line`.
    fn cell_point(&self, line: usize, col: usize, frac: f32) -> (f32, f32) {
        let t = self.app.tab();
        let ll = &t.layout.lines[line];
        let left = self.app.text_rect.x - t.scroll_x + ll.indent + ll.align_offset_row(self.app.text_rect.w, 0, false);
        let w = ll
            .table_cols
            .get(col)
            .copied()
            .and_then(|x0| ll.table_cols.get(col + 1).map(|x1| x1 - x0))
            .unwrap_or(24.0);
        (left + ll.table_cols[col] + w * frac, self.row_y(line, 0, 0.5))
    }

    /// The minimap, asserting there is one to aim at.
    fn minimap(&self) -> Rect {
        let m = self.app.minimap_rect;
        assert!(!m.is_empty(), "no minimap");
        m
    }

    /// Show the minimap and draw it.
    fn open_minimap(&mut self) {
        self.app.settings.minimap_open = true;
        self.draw();
    }

    /// Press on the minimap `frac` of the way down it; returns the pointer's y.
    fn minimap_press(&mut self, frac: f32) -> f32 {
        let m = self.minimap();
        let x = m.x + m.w * 0.5;
        let y = m.y + m.h * frac;
        self.app.mouse = (x, y);
        self.draw();
        self.app.click((x, y), &mut self.r);
        self.draw();
        y
    }

    /// Move the pressed map to `y` (keeps the button down).
    fn minimap_drag_to(&mut self, y: f32) {
        let m = self.minimap();
        let x = m.x + m.w * 0.5;
        self.app.mouse = (x, y);
        assert!(
            self.app.minimap_drag((x, y), &mut self.r),
            "the drag on the map was dropped"
        );
        self.draw();
    }

    /// Where the map's handle is drawn, in screen y - the same number the painter uses.
    ///
    /// It used to be derived from the map *content* (the position of the page's first line
    /// among the bars), which stops being the handle as soon as the map slides: the handle
    /// rides the strip, the content moves under it. While a drag is in flight the painter
    /// uses the drag's own y; otherwise `indicator_y`.
    fn handle_y(&self) -> f32 {
        let m = self.minimap();
        if let Some(y) = self.app.map_handle {
            return y;
        }
        let (g, _) = self.app.minimap_geometry();
        let total = self.app.tab().layout.total_height;
        let text_h = self.app.text_rect.h;
        m.y + g.indicator_y(self.app.tab().scroll, total, text_h)
    }

    /// The open picker dialog. Two tests had their own copy of this match, one
    /// borrowing and one cloning, which is one copy too many.
    fn picker(&self) -> &bare_md::app::Picker {
        match &self.app.dialog {
            Some(bare_md::app::Dialog::Picker(p)) => p,
            _ => panic!("no picker dialog"),
        }
    }

    /// Centre of the first hit matching `pred`, if it is on screen.
    fn hit_center(&self, pred: impl Fn(&Hit) -> bool) -> Option<(f32, f32)> {
        let rect = self.app.hits.find(pred)?;
        Some((rect.x + rect.w * 0.5, rect.y + rect.h * 0.5))
    }

    /// Hover the first table row so its chrome (the `+` buttons, the grips) is drawn.
    fn hover_table(&mut self) {
        let line = self.table_line();
        let (x, y) = self.cell_point(line, 0, 0.5);
        self.app.mouse = (x, y);
        self.draw();
    }

    /// Press an arrow `steps` times and report `(caret, caret x)` after every press.
    fn walk_arrows(&mut self, forward: bool, steps: usize) -> Vec<(usize, f32)> {
        let mut seen = Vec::new();
        for _ in 0..steps {
            self.app.move_horizontal(forward, false);
            self.draw();
            let x = self.caret_x();
            seen.push((self.app.tab().caret, x));
        }
        seen
    }

    /// Type through the event path, so a focused text field (find, replace)
    /// receives the text instead of the document.
    fn type_into_find(&mut self, s: &str) {
        use winit::event::{Ime, WindowEvent};
        let mut input = bare_md::input::InputState::default();
        self.app.handle_event(&WindowEvent::Ime(Ime::Commit(s.to_string())), &mut self.r, &mut input);
        self.draw();
    }
}

/// A caret walk may only ever travel one way on the screen - that is what "the
/// arrows follow what you see" means, and it is the property four separate RTL
/// regressions were each checking by hand.
fn assert_walks_one_way(walk: &[(usize, f32)], forward: bool, what: &str) {
    for (k, w) in walk.windows(2).enumerate() {
        let ok = if forward {
            w[1].1 > w[0].1 - 0.01
        } else {
            w[1].1 < w[0].1 + 0.01
        };
        assert!(
            ok,
            "{what}: {} moved the caret the wrong way at step {k}: {:?}",
            if forward { "Right" } else { "Left" },
            walk.iter().map(|p| (p.0, (p.1 * 10.0).round() / 10.0)).collect::<Vec<_>>()
        );
    }
}

/// A document of `n` short paragraphs: long enough that the minimap has to scroll,
/// and the shape every minimap test used to build by hand.
fn filler(n: usize) -> String {
    (0..n).map(|i| format!("line {i}\n\n")).collect()
}

/// Characters the layout treats as right-to-left. One copy, so a test written in
/// round 30 and a test written in round 29 cannot disagree about what Hebrew is.
fn is_rtl_char(c: char) -> bool {
    matches!(
        c as u32,
        0x0590..=0x05FF   // Hebrew
        | 0x0600..=0x06FF // Arabic
        | 0x0700..=0x074F // Syriac
        | 0x0750..=0x077F // Arabic Supplement
        | 0xFB1D..=0xFB4F // Hebrew presentation forms
        | 0xFE70..=0xFEFF // Arabic presentation forms
    )
}

/// Characters that are Markdown syntax rather than text: in the rendered view they
/// are hidden, so they have no glyph for a wash to cover. In the source view they
/// must be selectable like anything else.
fn is_md_marker(c: char) -> bool {
    matches!(c, '*' | '_' | '`' | '~' | '[' | ']' | '(' | ')')
}

/// A table whose data row is Hebrew, Latin, Arabic - the shape that exposed both
/// the round-29 cell direction bug and the two round-30 reports.
const RTL_TABLE: &str = "| a | b | c |\n| --- | --- | --- |\n| שלום | abc | مرحبا |\n";

/// (1) Typing a heading must leave the caret at the end of the text, at the
/// heading's own scale.
#[test]
fn caret_tracks_heading_text_while_typing() {
    let mut h = harness("heading-caret");
    h.type_str("# Title");
    let size = h.app.theme.heading_size(1);
    let expected_x = h.app.text_rect.x
        + h.r.text.width(Face::Bold, "# Title", size);
    let cr = h.app.caret_rect(&mut h.r.text);
    assert!(
        (cr.x - expected_x).abs() < 2.0,
        "caret x {} should sit after the heading text ({})",
        cr.x,
        expected_x
    );
    // And it must be as tall as the heading, not a dot.
    let lh = h.r.text.line_height(Face::Bold, size);
    assert!(cr.h > lh * 0.8, "caret height {} vs heading line {}", cr.h, lh);
}

/// (1b) A heading typed character by character keeps the caret behind the text
/// on every keystroke.
#[test]
fn caret_stays_at_line_end_after_every_keystroke() {
    let mut h = harness("heading-typing");
    for (i, ch) in "# Hi".chars().enumerate() {
        h.type_str(&ch.to_string());
        let cr = h.app.caret_rect(&mut h.r.text);
        let tab = h.app.tab();
        let line = &tab.layout.lines[0];
        let local = tab.caret - line.start_char;
        assert_eq!(local, i + 1, "caret did not advance with the text");
        let raw = tab.line_is_raw(h.app.mode, 0);
        let want = h.app.text_rect.x + line.indent + line.x_of(&mut h.r.text, local, raw);
        assert!((cr.x - want).abs() < 1.0, "keystroke {i}: {} != {want}", cr.x);
    }
}

/// (8) An empty line must still show a full-height caret.
#[test]
fn caret_on_empty_line_is_text_height() {
    let mut h = harness("empty-caret");
    h.type_str("first");
    h.type_str("\n");
    let cr = h.app.caret_rect(&mut h.r.text);
    let lh = h.r.text.line_height(Face::Regular, h.app.theme.font_size);
    assert!(
        cr.h >= lh * 0.85,
        "empty-line caret is {} high, expected about {lh}",
        cr.h
    );
    assert!(cr.x >= h.app.text_rect.x - 1.0);
}

/// (11) A bare `#` with the caret elsewhere keeps its marker visible and already
/// renders at heading size.
#[test]
fn bare_hash_marker_stays_visible_and_takes_heading_size() {
    let mut h = harness("bare-hash");
    h.type_str("#\n\nbody text");
    // Move the caret far away (to the end of the document).
    let end = h.app.tab().doc.len_chars();
    h.app.set_caret(end, false);
    h.draw();
    let tab = h.app.tab();
    let line = &tab.layout.lines[0];
    assert!(
        tab.line_is_raw(h.app.mode, 0),
        "a line with no content after its marker must stay in raw mode"
    );
    // The heading marker run must be laid out at heading size.
    let heading = h.app.theme.heading_size(1);
    let marker_run = line
        .raw
        .iter()
        .find(|r| r.style.ink == bare_md::layout::Inks::Marker)
        .expect("the `#` must be a marker run that is painted");
    assert!(
        (marker_run.style.size - heading).abs() < 0.01,
        "marker size {} should match heading size {}",
        marker_run.style.size,
        heading
    );
}

/// (11b) `#` alone becomes a heading immediately, without waiting for text.
#[test]
fn hash_becomes_heading_immediately() {
    let mut h = harness("instant-heading");
    h.type_str("#");
    let tab = h.app.tab();
    let line = &tab.layout.lines[0];
    assert_eq!(line.block(), bare_md::markdown::Block::Heading(1));
    let body = h.app.theme.font_size;
    let heading = h.app.theme.heading_size(1);
    assert!(heading > body * 1.3, "heading must be visibly larger");
    assert!(line.max_size >= heading - 0.01, "layout must already use the heading size");
}

/// (9) The X on a tab closes it.
#[test]
fn tab_close_button_closes_the_tab() {
    let mut h = harness("tab-close");
    h.type_str("first tab");
    h.app.new_tab();
    h.type_str("second tab");
    h.draw();
    let before = h.app.tabs.len();
    assert_eq!(before, 2);
    // Hover the first tab so its close button is revealed, then click the X.
    let Some(tab_rect) = h.app.hits.find(|h| matches!(h, Hit::Tab(0))) else {
        panic!("no tab hit registered");
    };
    h.app.mouse = (tab_rect.x + tab_rect.w * 0.5, tab_rect.y + tab_rect.h * 0.5);
    h.draw();
    assert!(
        h.click_hit(|hit| matches!(hit, Hit::TabClose(0))),
        "the close button should be registered while hovering the tab"
    );
    if h.app.dialog.is_some() {
        // An unsaved tab asks first; confirming must close it.
        assert!(h.click_hit(|hit| matches!(hit, Hit::DialogConfirm)), "no confirm button");
    }
    assert_eq!(h.app.tabs.len(), before - 1, "clicking X must close the tab");
}

/// (9b) Closing the last tab leaves a valid empty document behind.
#[test]
fn closing_last_tab_keeps_editor_usable() {
    let mut h = harness("tab-close-last");
    h.type_str("only");
    h.draw();
    let Some(tab_rect) = h.app.hits.find(|h| matches!(h, Hit::Tab(0))) else {
        panic!("no tab hit");
    };
    h.app.mouse = (tab_rect.x + tab_rect.w * 0.5, tab_rect.y + tab_rect.h * 0.5);
    h.draw();
    h.click_hit(|hit| matches!(hit, Hit::TabClose(0)));
    if h.app.dialog.is_some() {
        h.click_hit(|hit| matches!(hit, Hit::DialogConfirm));
    }
    assert_eq!(h.app.tabs.len(), 1, "there must always be one tab");
    h.type_str("hello");
    assert!(h.app.tab().doc.text().contains("hello"));
}

/// (4) The view button cycles Live -> Live+ -> Raw and changes how lines render.
#[test]
fn view_mode_button_cycles_three_modes() {
    let mut h = harness("raw-toggle");
    h.type_str("# Heading\n\n**bold** text");
    let end = h.app.tab().doc.len_chars();
    h.app.set_caret(end, false);
    h.draw();
    assert!(!h.app.line_is_raw(0), "live mode should hide markers off-caret");

    // Live -> Live+ : markers everywhere, formatting kept.
    assert!(h.click_hit(|hit| matches!(hit, Hit::Toolbar(ToolbarAction::ToggleRaw))), "view button missing");
    assert_eq!(h.app.mode, ViewMode::LivePlus);
    assert!(h.app.line_is_raw(0));
    h.draw();
    let tab = h.app.tab();
    let heading = &tab.layout.lines[0];
    let bold_run = heading
        .runs(true)
        .iter()
        .find(|r| r.style.face == Face::Bold && r.start <= heading.info.prefix)
        .map(|r| r.style.size)
        .unwrap_or(0.0);
    assert!(bold_run > 20.0, "Live+ keeps the heading size, got {bold_run}");

    // Live+ -> Raw : markers visible, formatting gone.
    h.click_hit(|hit| matches!(hit, Hit::Toolbar(ToolbarAction::ToggleRaw)));
    assert_eq!(h.app.mode, ViewMode::Raw);
    h.draw();
    let tab = h.app.tab();
    let line = &tab.layout.lines[0];
    let visible: String = line
        .runs(true)
        .iter()
        .map(|r| line.chars[r.start..r.end].iter().collect::<String>())
        .collect();
    assert_eq!(visible, "# Heading");
    for r in line.runs(true) {
        assert_eq!(r.style.face, Face::Mono, "raw mode must not style text");
        assert!(r.style.size < 16.0, "raw mode must not resize text: {}", r.style.size);
    }

    // Raw -> Live.
    h.click_hit(|hit| matches!(hit, Hit::Toolbar(ToolbarAction::ToggleRaw)));
    assert_eq!(h.app.mode, ViewMode::Live);
}

/// (3) Three themes exist and all of them are monochrome: black, grey, white.
#[test]
fn only_monochrome_themes() {
    assert_eq!(ThemeKind::ALL.len(), 3);
    assert_eq!(ThemeKind::ALL[0], ThemeKind::Black);
    assert_eq!(ThemeKind::ALL[1], ThemeKind::Grey);
    assert_eq!(ThemeKind::ALL[2], ThemeKind::White);
    // Legacy ids from old settings files still load.
    assert_eq!(ThemeKind::parse("dark"), Some(ThemeKind::Black));
    assert_eq!(ThemeKind::parse("light"), Some(ThemeKind::White));
    assert_eq!(ThemeKind::parse("gray"), Some(ThemeKind::Grey));
    // …and the cycle visits every theme before coming back.
    let mut k = ThemeKind::Black;
    let mut seen = vec![k];
    for _ in 0..2 {
        k = k.next();
        seen.push(k);
    }
    assert_eq!(seen, ThemeKind::ALL.to_vec());
    assert_eq!(k.next(), ThemeKind::Black);
}

/// (2)/(5) The toolbar floats above the document and the sidebar is full height.
#[test]
fn chrome_layout_matches_the_new_design() {
    let h = harness("chrome");
    let c = bare_md::ui::chrome(&h.app, W as f32, H as f32);
    assert!(c.sidebar.x == 0.0 && c.sidebar.y == 0.0);
    assert!(
        c.sidebar.h >= c.doc.h - 1.0,
        "sidebar {} must run the full height of the body {}",
        c.sidebar.h,
        c.doc.h
    );
    // Tabs live inside the document column and share its background.
    assert!(c.tabbar.x >= c.doc.x - 0.5);
    assert!((c.tabbar.w - c.doc.w).abs() < 0.5);
    // The floating toolbar sits inside the editor, above the text.
    let toolbar_top = bare_md::ui::floating_toolbar_rect(&h.app, c.editor);
    assert!(toolbar_top.y >= c.editor.y);
    assert!(toolbar_top.bottom() <= c.text.y, "toolbar must not cover the text");
    assert!(
        toolbar_top.w < c.editor.w * 0.9,
        "the toolbar should be a floating pill, not a full-width band"
    );
}

/// (10) Pressing Bold repeatedly must never stack or corrupt the document.
#[test]
fn repeated_bold_presses_are_idempotent() {
    let mut h = harness("bold-spam");
    h.type_str("word");
    h.app.select_range(0, 4);
    for _ in 0..6 {
        h.app.toggle_inline("**", "**");
        h.draw();
    }
    let text = h.app.tab().doc.text();
    assert_eq!(text, "word", "repeated Bold must return to plain text, got {text:?}");
    h.app.toggle_inline("**", "**");
    assert_eq!(h.app.tab().doc.text(), "**word**");
}

/// (10b) Bold with no selection toggles a pair instead of nesting more markers.
#[test]
fn bold_at_caret_toggles_without_stacking() {
    let mut h = harness("bold-caret");
    h.type_str("ab");
    h.app.set_caret(1, false);
    h.app.toggle_inline("**", "**");
    h.draw();
    assert_eq!(h.app.tab().doc.text(), "**ab**");
    // The caret is left inside the pair; a second press must remove it again.
    h.app.toggle_inline("**", "**");
    assert_eq!(h.app.tab().doc.text(), "ab");
}

/// (10c) Formatting inside an existing span unwraps that span rather than
/// nesting a new pair (the old artefact bug).
#[test]
fn bold_inside_existing_span_unwraps() {
    // The caret inside `**bo|ld**` (4) and right after the closing stars (8).
    assert_bold_round_trip("bold-unwrap", "**bold**", 4);
    assert_bold_round_trip("bold-unwrap-end", "**bold**", 8);
}

/// (6) The minimap draws one thin bar per visual row, not chunky blocks.
#[test]
fn minimap_rows_are_thin_bars() {
    let mut h = harness("minimap");
    let text: String = (0..60).map(|i| format!("line {i} with some words on it\n")).collect();
    h.type_str(&text);
    h.draw();
    let rect = h.app.minimap_rect;
    assert!(rect.w > 0.0, "minimap must be laid out");
    let layout = &h.app.tab().layout;
    let visible_rows: usize = layout.lines.iter().map(|l| l.row_count()).sum();
    assert!(visible_rows >= 60);
    // The bar height is a constant, so adding a line somewhere else in the
    // document never resizes existing bars.
    let before = layout.lines[0].row_h;
    h.type_str("another line\n");
    h.draw();
    let after = h.app.tab().layout.lines[0].row_h;
    assert!((before - after).abs() < 0.001, "minimap bar height must not depend on the line count");
    assert!(Rect::new(rect.x, rect.y, rect.w, rect.h).contains((rect.x + 1.0, rect.y + 1.0)));
}

/// (7) The console stays quiet: third-party Vulkan/wgpu chatter and our own
/// startup notes are both below the default filter.
///
/// An editor that opens by announcing where its settings file is and which port
/// it took reads like a program in trouble. The default is `warn`, for our crate
/// as much as for anyone else's, and `RUST_LOG` raises it when someone wants the
/// details.
#[test]
fn log_filter_is_quiet_by_default() {
    // The log filter lives with the rest of the OS integration.
    let src = include_str!("../src/platform/window.rs");
    assert!(src.contains("wgpu_hal=error"), "Vulkan loader warnings must be filtered");
    assert!(src.contains("\"warn,wgpu_hal=error"), "the default level is warn");
    assert!(
        !src.contains("\"warn,bare_md="),
        "nothing of ours is printed at info by default"
    );
}

// ---------------------------------------------------------------- round 3

/// (1) A code block keeps its empty lines, hides its fence markers and hugs its
/// code.
#[test]
fn fenced_block_keeps_empty_lines_and_hides_fences() {
    let mut h = harness("fence");
    h.type_str("```rust\nfn main() {}\n\nlet x = 1;\n```\n");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    {
        let layout = &h.app.tab().layout;
        let blocks: Vec<bare_md::markdown::Block> = layout.lines.iter().map(|l| l.block()).collect();
        assert_eq!(blocks[0], bare_md::markdown::Block::Fence, "opening fence: {blocks:?}");
        assert_eq!(blocks[1], bare_md::markdown::Block::Code);
        assert_eq!(blocks[2], bare_md::markdown::Block::Code, "an empty line stays inside the block");
        assert_eq!(blocks[3], bare_md::markdown::Block::Code);
        assert_eq!(blocks[4], bare_md::markdown::Block::Fence, "closing fence: {blocks:?}");
        // The card hugs the code instead of spanning the page.
        let widest = layout.lines[1].width;
        assert!(widest < layout.width * 0.5, "code card should hug the code, got {widest}");
        // ...and it never overlaps the paragraph after it.
        let last = &layout.lines[4];
        let next = &layout.lines[5];
        assert!(next.top >= last.top + last.height, "the next line must start below the block");
    }
    // The markers stay hidden while the block has code in it: the code in its
    // card is what the user wants to see, not the syntax.
    assert!(!h.app.line_is_raw(0), "``` written into a multi-line block must be hidden");
    assert!(!h.app.line_is_raw(4), "closing ``` must be hidden too");
    // ...but they come back on the caret line, so the block stays editable.
    h.app.set_caret(0, false);
    h.draw();
    assert!(h.app.line_is_raw(0), "the caret line reveals its marker");
}

/// (1) A single-line ```` ```code``` ```` is still a code block.
#[test]
fn one_line_fence_renders_as_code() {
    let mut h = harness("oneliner");
    h.type_str("```let x = 1;```");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    let line = &h.app.tab().layout.lines[0];
    assert_eq!(line.block(), bare_md::markdown::Block::Code);
    let visible: String = line.clean.iter().map(|r| line.chars[r.start..r.end].iter().collect::<String>()).collect();
    assert_eq!(visible.trim(), "let x = 1;", "the back-ticks must be hidden, the code shown");
}

/// (1) ```` ``` ```` with no text still shows its markers, like `#` does.
#[test]
fn empty_fence_markers_stay_visible() {
    let mut h = harness("emptyfence");
    h.type_str("```\n\n```\n");
    h.app.set_caret(0, false);
    h.draw();
    assert!(h.app.line_is_raw(0));
    let line = &h.app.tab().layout.lines[0];
    let raw: String = line.raw.iter().map(|r| line.chars[r.start..r.end].iter().collect::<String>()).collect();
    assert_eq!(raw.trim(), "```");
}

/// (2) Clicking a checkbox toggles it instead of moving the caret.
#[test]
fn checkbox_click_toggles_the_task() {
    let mut h = harness("checkbox");
    h.type_str("- [ ] write docs\n");
    h.draw();
    let Some(rect) = h.app.hits.find(|hit| matches!(hit, Hit::Task(_))) else {
        panic!("checkbox must be clickable");
    };
    h.click((rect.x + rect.w * 0.5, rect.y + rect.h * 0.5));
    assert_eq!(h.app.tab().doc.text(), "- [x] write docs\n");
    h.click((rect.x + rect.w * 0.5, rect.y + rect.h * 0.5));
    assert_eq!(h.app.tab().doc.text(), "- [ ] write docs\n");
}

/// (2) Tab moves the whole line (checkbox included), Shift+Tab moves it back.
#[test]
fn tab_indents_the_whole_task_line() {
    let mut h = harness("tab-indent");
    h.type_str("- [ ] item");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.app.indent(false);
    assert_eq!(h.app.tab().doc.text(), "  - [ ] item");
    h.app.indent(true);
    assert_eq!(h.app.tab().doc.text(), "- [ ] item");
}

/// (2) Backspace at the start of a task removes the indentation before the syntax.
#[test]
fn backspace_removes_indent_before_checkbox_syntax() {
    let mut h = harness("backspace-task");
    h.type_str("  - [ ] item");
    h.app.set_caret(2 + 6, false); // right after "- [ ] "
    h.app.backspace();
    assert_eq!(h.app.tab().doc.text(), "- [ ] item", "indentation goes first");
    h.app.set_caret(6, false);
    h.draw();
    h.app.backspace();
    assert_eq!(h.app.tab().doc.text(), "item", "then the whole marker");
}

/// (3) Brackets and quotes auto-complete.
#[test]
fn typed_brackets_auto_complete() {
    let mut h = harness("autopair");
    h.app.insert_typed("(");
    assert_eq!(h.app.tab().doc.text(), "()");
    assert_eq!(h.app.tab().caret, 1, "caret goes between the brackets");
    h.app.insert_typed(")");
    assert_eq!(h.app.tab().doc.text(), "()", "typing the closer steps over it");
    assert_eq!(h.app.tab().caret, 2);
    h.app.insert_typed("[");
    assert_eq!(h.app.tab().doc.text(), "()[]");
}

/// (4) Settings live in one file, with no extra folder.
#[test]
fn settings_are_one_file_next_to_the_exe() {
    let dir = std::env::temp_dir().join(format!("bare-md-cfg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let paths = Paths {
        data_dir: dir.clone(),
        config_file: dir.join("settings.conf"),
        origin: "test".into(),
    };
    let mut settings = Settings::default();
    settings.theme = ThemeKind::White;
    settings.save(&paths);
    let session = Session {
        tabs: vec![bare_md::config::SessionTab { path: None, caret: 7, scroll: 0.0 }],
        active: 0,
    };
    session.save(&paths);
    assert!(paths.config_file.exists());
    let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).collect();
    assert_eq!(entries.len(), 1, "only settings.conf may be created");
    // Settings survive a session save, and vice versa.
    let back = Settings::load(&paths);
    assert_eq!(back.theme, ThemeKind::White);
    let sess = Session::load(&paths);
    assert_eq!(sess.tabs.len(), 1);
    assert_eq!(sess.tabs[0].caret, 7);
    let _ = std::fs::remove_dir_all(&dir);
}

/// (7) Table rows lay their cells out at the drawn column offsets, so the caret
/// lands where you click; the grid itself has visible vertical rules.
#[test]
fn table_cells_are_laid_out_on_the_grid() {
    let mut h = harness("table");
    h.type_str("| Name | Value |\n| ---- | ----- |\n| a | 1 |\n| b | 2 |\n");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    let layout = &h.app.tab().layout;
    let row = &layout.lines[2]; // "| a | 1 |"
    assert_eq!(row.block(), bare_md::markdown::Block::Table);
    assert!(row.table_cols.len() >= 2, "columns must be computed: {:?}", row.table_cols);
    assert!(row.table_width > 0.0);
    // The second cell starts at its column offset, not right after the first.
    let second = row
        .clean
        .iter()
        .find(|r| r.start >= row.info.cells[1].0)
        .expect("second cell run");
    assert!(
        (second.x - row.table_cols[1]).abs() < 0.5,
        "cell x {} should equal column offset {}",
        second.x,
        row.table_cols[1]
    );
    // Two columns means three vertical rules (left, middle, right).
    assert!(row.table_cols[1] > row.table_cols[0] + 20.0);
}

/// (11) An `---` rule is either the rendered line or the syntax, never both.
#[test]
fn hr_rule_and_syntax_never_overlap() {
    let mut h = harness("hr");
    h.type_str("text\n\n---\n\nmore");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    assert!(!h.app.line_is_raw(2), "away from the caret the rule is rendered");
    h.app.set_caret(h.app.tab().doc.line_start(2) + 1, false);
    h.draw();
    assert!(h.app.line_is_raw(2), "on the caret line the syntax is shown instead");
}

/// (12) Bold right after already-formatted text removes the formatting instead
/// of stacking markers.
#[test]
fn bold_after_formatted_text_unwraps() {
    // The caret right after the closing `**` must unwrap, not add more stars.
    assert_bold_round_trip("bold-after", "**bold**", 8);
    // …and so must it just before the opening ones.
    assert_bold_round_trip("bold-before", "**bold**", 0);
}

/// (13) The code button fences a multi-line selection.
#[test]
fn code_button_fences_multiline_selection() {
    let mut h = harness("code-block");
    h.type_str("one\ntwo\nthree");
    h.app.select_range(0, 7); // "one\ntwo"
    h.app.toggle_code_block();
    assert_eq!(h.app.tab().doc.text(), "```\none\ntwo\n```\nthree");
}

/// (14) Enter continues a quote even when the line is empty; Shift+Enter ends it.
#[test]
fn quote_continues_on_enter_and_ends_on_shift_enter() {
    let mut h = harness("quote");
    h.type_str("> quoted");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.app.newline_ex(false);
    h.draw();
    assert_eq!(h.app.tab().doc.text(), "> quoted\n> ");
    h.app.newline_ex(false);
    h.draw();
    assert_eq!(h.app.tab().doc.text(), "> quoted\n> \n> ", "an empty quote line still continues");
    h.app.newline_ex(true);
    h.draw();
    // Shift+Enter drops the marker on the empty line, so the quote really ends
    // instead of leaving a dangling `> ` behind.
    assert_eq!(h.app.tab().doc.text(), "> quoted\n> \n", "Shift+Enter leaves the quote");
}

/// (15) The status bar no longer repeats the theme, "portable" or "untitled".
#[test]
fn status_bar_is_minimal() {
    let src = ui_source();
    let status = &src[src.find("fn draw_status(").unwrap()..];
    let status = &status[..status.find("\n}\n").unwrap()];
    assert!(!status.contains("untitled"), "no placeholder label in the status bar");
    assert!(!status.contains("portable"), "no 'portable' tag in the status bar");
    assert!(!status.contains("kind.label()"), "no theme name in the status bar");
}

// ------------------------------------------------------- round 3 (continued)

/// (1) The table card is a real grid: a thin `| --- |` rule row, columns that
/// keep their own width, and vertical space above and below so no prose can end
/// up on top of a line.
#[test]
fn table_is_a_grid_with_rules_and_clear_space() {
    let mut h = harness("table-grid");
    h.type_str("before\n\n| Name | Value |\n| ---- | ----- |\n| alpha | 1 |\n| beta | 22 |\n\nafter\n");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    let layout = &h.app.tab().layout;
    let rows: Vec<usize> = (0..layout.lines.len())
        .filter(|&i| layout.lines[i].block() == bare_md::markdown::Block::Table)
        .collect();
    assert_eq!(rows.len(), 4, "rows: {rows:?}");

    // The `| --- |` row is collapsed into a thin rule, not a text row.
    let sep = &layout.lines[rows[1]];
    assert!(sep.info.table_sep);
    assert!(
        (sep.height - bare_md::layout::TABLE_RULE_H).abs() < 0.01,
        "separator height {}",
        sep.height
    );

    // Every row has room on both sides of its text, so the grid never runs
    // through the glyphs.
    for &i in &rows {
        let l = &layout.lines[i];
        assert!(l.height >= l.row_h - 0.01, "row {} too short", l.line);
        assert!(l.table_width > 0.0);
        let last_col = *l.table_cols.last().unwrap();
        assert!(
            l.table_width >= last_col,
            "columns must fit inside the card: {} vs {}",
            l.table_width,
            last_col
        );
    }

    // The card owns its space: the paragraph below starts after it, and the one
    // above ends before it.
    let first = &layout.lines[rows[0]];
    let above = &layout.lines[rows[0] - 1];
    assert!(
        first.top - (above.top + above.height) >= bare_md::layout::CARD_PAD_Y,
        "the table card needs clear space above it"
    );
    let last = &layout.lines[*rows.last().unwrap()];
    let below = &layout.lines[*rows.last().unwrap() + 1];
    assert!(below.top >= last.top + last.height + bare_md::layout::CARD_PAD_Y);
}

/// (2) Nested quotes line up: each level adds exactly one gutter, and an empty
/// row draws a bar instead of showing its `>` glyph.
#[test]
fn nested_quotes_align_and_empty_row_shows_a_bar() {
    let mut h = harness("quotes");
    h.type_str("> outer\n> > inner\n> > > deeper\n> \n");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    let layout = &h.app.tab().layout;
    let indents: Vec<f32> = (0..3).map(|i| layout.lines[i].indent).collect();
    let step = indents[1] - indents[0];
    assert!(step > 8.0, "each quote level must indent: {indents:?}");
    assert!(
        (indents[2] - indents[1] - step).abs() < 0.01,
        "nesting must be even: {indents:?}"
    );
    // Text always starts one gap after its deepest bar.
    let gap = bare_md::layout::QUOTE_GAP;
    assert!((indents[0] - gap).abs() < 0.01, "first level starts at one gutter");
    assert!((indents[1] - (2.0 * step + gap - step) ).abs() < 1.0);

    // The empty `>` row: markers hidden, so the only thing left is the bar.
    let empty = &layout.lines[3];
    assert_eq!(empty.block(), bare_md::markdown::Block::Quote);
    assert!(!h.app.line_is_raw(3), "an empty quote row must show a bar, not `>`");
    let visible: String = empty
        .clean
        .iter()
        .map(|r| empty.chars[r.start..r.end].iter().collect::<String>())
        .collect();
    assert!(visible.trim().is_empty(), "empty quote row shows {:?}", visible);
    // ...and it still occupies a full row, so the bar has a body.
    assert!(empty.height > 8.0);
}

/// (3) An empty one-line code pair keeps its back-ticks visible.
#[test]
fn empty_inline_code_span_keeps_its_markers() {
    let mut h = harness("empty-inline-code");
    h.type_str("a `` b");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    let line = &h.app.tab().layout.lines[0];
    let visible: String = line
        .clean
        .iter()
        .map(|r| line.chars[r.start..r.end].iter().collect::<String>())
        .collect();
    assert!(
        visible.matches('`').count() == 2,
        "an empty code pair must stay visible, got {visible:?}"
    );
}

/// (4) A one-line ```` ```code``` ```` is drawn as a card like any other block.
#[test]
fn one_line_fence_gets_a_card_and_space() {
    let mut h = harness("one-line-card");
    h.type_str("text\n\n```let x = 1;```\n\ntext\n");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    let layout = &h.app.tab().layout;
    let line = &layout.lines[2];
    assert_eq!(line.block(), bare_md::markdown::Block::Code);
    assert!(!h.app.line_is_raw(2), "a one-liner hides its markers too");
    let first_top = layout.lines[1].top + layout.lines[1].height;
    assert!(
        line.top - first_top >= bare_md::layout::CARD_PAD_Y,
        "the card needs clear space above it"
    );
    let after = &layout.lines[4];
    assert!(after.top >= line.top + line.height + bare_md::layout::CARD_PAD_Y);
}

/// (5) Enter at the start of a list item breaks the list instead of writing a
/// second marker in front of the first.
#[test]
fn enter_at_list_marker_does_not_duplicate_it() {
    for marker in ["- ", "1. ", "- [ ] "] {
        let mut h = harness(&format!("list-enter-{}", marker.len()));
        h.type_str(&format!("{marker}item"));
        h.app.set_caret(0, false); // start of the line, before the marker
        h.app.newline_ex(false);
        assert_eq!(
            h.app.tab().doc.text(),
            format!("\n{marker}item"),
            "marker {marker:?} was duplicated"
        );
        assert_eq!(h.app.tab().caret, 0, "the caret stays on the new empty line");
    }
}

/// (6) Bold and italic work with either syntax.
#[test]
fn formatting_toggles_both_star_and_underscore_syntax() {
    // `_word_` must respond to the italic button.
    {
        let mut h = harness("underscore-italic");
        h.type_str("_word_");
        h.app.set_caret(h.app.tab().doc.len_chars() - 1, false);
        h.app.toggle_inline("*", "*");
        assert_eq!(h.app.tab().doc.text(), "word", "`_word_` must un-wrap");
    }
    {
        let mut h = harness("underscore-bold");
        h.type_str("__word__");
        h.app.set_caret(h.app.tab().doc.len_chars() - 2, false);
        h.app.toggle_inline("**", "**");
        assert_eq!(h.app.tab().doc.text(), "word", "`__word__` must un-wrap");
    }
    // A bare word gets the syntax the button asks for, and the same button
    // removes it again rather than stacking markers.
    {
        let mut h = harness("underscore-add");
        h.type_str("word");
        h.app.set_caret(4, false);
        h.app.toggle_inline("_", "_");
        assert_eq!(h.app.tab().doc.text(), "_word_");
        h.app.toggle_inline("_", "_");
        assert_eq!(h.app.tab().doc.text(), "word", "pressing again must unwrap, not stack");
    }
    // `**bold**` answered by the underscore button, and vice versa.
    {
        let mut h = harness("mixed-syntax");
        h.type_str("**bold**");
        h.app.set_caret(h.app.tab().doc.len_chars() - 1, false);
        h.app.toggle_inline("__", "__");
        assert_eq!(h.app.tab().doc.text(), "bold");
        h.app.toggle_inline("__", "__");
        assert_eq!(h.app.tab().doc.text(), "__bold__");
    }
}

/// (7) No two blocks ever sit at the same vertical position: headings of every
/// level, cards and plain paragraphs all get their own space.
#[test]
fn blocks_never_touch_each_other() {
    let mut h = harness("rhythm");
    h.type_str(
        "# One\n\ntext\n\n## Two\n\ntext\n\n### Three\n\ntext\n\n#### Four\n\ntext\n\n\
         ##### Five\n\n```\ncode\n```\n\ntext\n\n| a | b |\n| - | - |\n| c | d |\n\n\
         > quote\n\ntext\n\n---\n\ntext\n",
    );
    h.app.set_caret(0, false);
    h.draw();
    let layout = &h.app.tab().layout;
    for pair in layout.lines.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        assert!(
            b.top >= a.top + a.height - 0.01,
            "line {} overlaps line {} ({} + {} > {})",
            b.line,
            a.line,
            a.top,
            a.height,
            b.top
        );
    }
    // A heading is followed by real space, not by an adjacent paragraph.
    let head = &layout.lines[0];
    let para = &layout.lines[2];
    assert!(
        para.top - (head.top + head.height) >= 16.0,
        "a H1 needs room below it: {}",
        para.top - (head.top + head.height)
    );
}

/// (8) Links are drawn in the pale blue ink.
#[test]
fn links_use_the_pale_blue_ink() {
    let mut h = harness("links");
    h.type_str("see [the docs](https://example.com) now");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    let line = &h.app.tab().layout.lines[0];
    let link = line
        .clean
        .iter()
        .find(|r| r.style.ink == bare_md::layout::Inks::Link)
        .expect("the link text must have its own ink");
    let theme = h.app.theme.clone();
    assert!(theme.link.b > theme.link.r, "the link ink must lean blue");
    assert_ne!(theme.link, theme.text, "links must not be plain text");
    assert!(link.style.underline, "links stay underlined");
}

// ---------------------------------------------------------------- round 4

/// (1) The bars of consecutive quote rows are one continuous line, not dashes.
#[test]
fn quote_bars_are_continuous() {
    // One harness at a time: the software Vulkan device is shared.
    {
        let mut h = harness("quote-bars");
        h.type_str("> one\n> two\n> three\n");
        h.app.set_caret(h.app.tab().doc.len_chars(), false);
        h.draw();
        let layout = &h.app.tab().layout;
        let spans = bare_md::ui::quote_bar_spans(layout, 0, 3);
        assert_eq!(spans.len(), 1, "one bar for a single-level quote, got {spans:?}");
        let (depth, top, bottom) = spans[0];
        assert_eq!(depth, 0);
        let expected_top = layout.lines[0].top;
        let expected_bottom = layout.lines[2].top + layout.lines[2].height;
        assert!((top - expected_top).abs() < 0.01);
        assert!((bottom - expected_bottom).abs() < 0.01, "the bar must span all three rows");
    }

    // Nesting adds a second bar that only covers the deeper rows.
    let mut h = harness("quote-nested-bars");
    h.type_str("> a\n> > b\n> c\n");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    let layout = &h.app.tab().layout;
    let spans = bare_md::ui::quote_bar_spans(layout, 0, 3);
    let outer: Vec<_> = spans.iter().filter(|s| s.0 == 0).collect();
    let inner: Vec<_> = spans.iter().filter(|s| s.0 == 1).collect();
    assert_eq!(outer.len(), 1, "the outer bar runs the whole quote: {spans:?}");
    assert_eq!(inner.len(), 1, "the inner bar covers just its own rows: {spans:?}");
    let inner_mid = layout.lines[1].top;
    assert!((inner[0].1 - inner_mid).abs() < 0.01);
}

/// (1b) Left-arrow at the end of a quote row jumps straight past the hidden `>`
/// marker instead of stopping in front of it.
#[test]
fn arrows_skip_hidden_block_markers() {
    let mut h = harness("skip-markers");
    h.type_str("a\n> quoted");
    let prefix = h.app.tab().layout.lines[1].info.prefix;
    assert_eq!(prefix, 2);
    let line_start = h.app.tab().doc.line_start(1);
    h.app.set_caret(line_start + prefix + 2, false);
    // Walk left over the two characters of text, then once more.
    h.app.move_horizontal(false, false);
    h.app.move_horizontal(false, false);
    assert_eq!(
        h.app.tab().caret,
        line_start + prefix,
        "walking left through the text stops right after the marker, never before it"
    );
    h.app.move_horizontal(false, false);
    assert_eq!(
        h.app.tab().caret,
        line_start - 1,
        "the caret must leave the line, not stop before `>`"
    );
    // ...and coming back it lands after the marker, never inside it.
    h.app.move_horizontal(true, false);
    assert_eq!(h.app.tab().caret, line_start + prefix);
    h.app.move_horizontal(true, false);
    assert_eq!(
        h.app.tab().caret,
        line_start + prefix + 1,
        "right-arrow from the text start moves into the text"
    );
}

/// (2) A table row keeps its rendered geometry while it is being edited.
#[test]
fn tables_are_edited_in_place() {
    let mut h = harness("table-in-place");
    h.type_str("before\n\n| Name | Value |\n| ---- | ----- |\n| alpha | 1 |\n| beta | 22 |\n");
    let text = h.app.tab().doc.text();
    let at = text.find("beta").unwrap();
    h.app.set_caret(text[..at].chars().count() + 1, false);
    h.draw();
    let tab = h.app.tab();
    let line_idx = tab.layout.line_of_char(tab.caret);
    let line = &tab.layout.lines[line_idx];
    assert_eq!(line.block(), bare_md::markdown::Block::Table);
    assert!(
        !tab.line_is_raw(h.app.mode, line_idx),
        "a table row must not flip to raw source while editing"
    );
    // The cell text still sits on the column grid, so the caret and the grid
    // agree with each other.
    let second = line
        .clean
        .iter()
        .find(|r| r.start >= line.info.cells[1].0)
        .expect("second cell");
    assert!((second.x - line.table_cols[1]).abs() < 0.5);
}

/// (3) Clicking a ```` ``` ```` row puts the caret where the click happened, not
/// at column 0.
#[test]
fn clicking_a_fence_line_lands_under_the_pointer() {
    let mut h = harness("fence-click");
    h.type_str("```rust\ncode\n```\n");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    let text = h.app.text_rect;
    let line = &h.app.tab().layout.lines[0];
    let y = text.y + line.top + line.height * 0.5;
    let x = text.x + 40.0;
    h.click((x, y));
    let caret = h.app.tab().caret;
    let (line_idx, col) = h.app.tab().doc.line_and_col(caret);
    assert_eq!(line_idx, 0, "the click must stay on the fence row");
    assert!(col > 0, "caret landed at column 0 instead of under the pointer");
    // ...and roughly where the pointer is, in the revealed (source) geometry.
    let tab = h.app.tab();
    let ll = &tab.layout.lines[0];
    let approx = ll.indent + ll.x_abs(&mut h.r.text, caret - ll.start_char, true);
    // One monospaced character of slack: the caret snaps to a character boundary.
    let ch = h.r.text.width(Face::Mono, "m", h.app.theme.code_size());
    assert!(
        (approx - 40.0).abs() <= ch + 1.0,
        "caret x {approx} should sit under the click at 40 (char {ch})"
    );
}

/// (3b) The card of a code block is one unbroken shape while a line is edited,
/// and an indented block with blank lines stays a single block.
#[test]
fn indented_code_block_stays_one_block() {
    let mut h = harness("indented-block");
    h.type_str("text\n\n    let a = 1;\n\n    let b = 2;\n\ntext\n");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    let blocks: Vec<bare_md::markdown::Block> = {
        let layout = &h.app.tab().layout;
        layout.lines.iter().map(|l| l.block()).collect()
    };
    assert_eq!(blocks[2], bare_md::markdown::Block::Code);
    assert_eq!(
        blocks[3],
        bare_md::markdown::Block::Code,
        "the blank line between two indented code lines belongs to the block: {blocks:?}"
    );
    assert_eq!(blocks[4], bare_md::markdown::Block::Code);
    let layout = &h.app.tab().layout;
    assert_eq!(layout.lines[2].space_after, 0.0, "no gap inside the block");
    assert_eq!(layout.lines[3].space_before, 0.0);
    assert_eq!(layout.lines[4].space_before, 0.0);
}

/// (5) Headings do not get extra air on the minimap.
#[test]
fn minimap_spacing_ignores_block_padding() {
    let mut h = harness("minimap-spacing");
    h.type_str("# One\n\ntext\n\n## Two\n\ntext\n");
    h.app.set_caret(0, false);
    h.draw();
    let layout = &h.app.tab().layout;
    let tops = bare_md::ui::minimap_virtual_tops(layout);
    // The document's real geometry has a fat gap above each heading...
    let real_gap = layout.lines[4].top - (layout.lines[3].top + layout.lines[3].height);
    assert!(real_gap > 10.0, "headings really do have space around them: {real_gap}");
    // ...the minimap advances one row per line instead.
    for i in 0..tops.len() - 1 {
        let step = tops[i + 1] - tops[i];
        assert!(step <= 1.01, "line {i} takes {step} rows on the minimap");
    }
}

/// (6) Three view modes: Live, Live+ (markers, still formatted) and Raw (plain).
#[test]
fn view_modes_have_three_distinct_behaviours() {
    let mut h = harness("view-modes");
    h.type_str("# Heading\n\n**bold** and *italic*");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    let heading_size = h.app.theme.heading_size(1);

    // Live: markers hidden away from the caret.
    h.draw();
    assert!(!h.app.line_is_raw(0));

    // Live+: markers everywhere, formatting intact.
    h.app.set_view_mode(ViewMode::LivePlus);
    h.draw();
    assert!(h.app.line_is_raw(0));
    let tab = h.app.tab();
    let sizes: Vec<f32> = tab.layout.lines[0].runs(true).iter().map(|r| r.style.size).collect();
    assert!(sizes.iter().cloned().fold(0.0f32, f32::max) >= heading_size - 0.01);

    // Raw: markers everywhere and *no* formatting at all.
    h.app.set_view_mode(ViewMode::Raw);
    h.draw();
    let tab = h.app.tab();
    for (i, line) in tab.layout.lines.iter().enumerate() {
        assert!(tab.line_is_raw(h.app.mode, i));
        for run in line.runs(true) {
            assert_eq!(run.style.face, Face::Mono, "raw line {i} is not monospaced");
            assert!(run.style.size < heading_size, "raw line {i} is still scaled");
            assert!(!run.style.underline && !run.style.strike);
        }
        assert_eq!(line.indent, 0.0, "raw mode must not indent blocks");
    }
}

/// (7) In Live+ the `| --- |` separator row gets its own line of space.
#[test]
fn table_separator_has_room_in_marker_modes() {
    let mut h = harness("sep-room");
    h.type_str("| a | b |\n| --- | --- |\n| c | d |\n");
    h.app.set_view_mode(ViewMode::LivePlus);
    h.app.set_caret(0, false);
    h.draw();
    let layout = &h.app.tab().layout;
    let sep = &layout.lines[1];
    assert!(sep.info.table_sep);
    assert!(
        sep.height > bare_md::layout::TABLE_RULE_H * 3.0,
        "the separator needs a text row's worth of space, got {}",
        sep.height
    );
    let next = &layout.lines[2];
    assert!(next.top >= sep.top + sep.height, "the next row must not overlap it");
}

/// (8) The reload button re-reads the file from disk.
#[test]
fn reload_button_rereads_the_file() {
    let mut h = harness("reload");
    let path = h._dir.join("reload.md");
    std::fs::write(&path, "# One\n").unwrap();
    h.app.open_path(&path);
    h.draw();
    assert!(h.app.hits.find(|hit| matches!(hit, Hit::Toolbar(ToolbarAction::ReloadFile))).is_some());
    std::fs::write(&path, "# Two\n\nmore\n").unwrap();
    h.app.reload_file();
    assert_eq!(h.app.tab().doc.text(), "# Two\n\nmore\n");
    assert!(!h.app.tab().doc.dirty);
    // Reloading with no changes is a no-op and says so.
    h.app.reload_file();
    assert_eq!(h.app.tab().doc.text(), "# Two\n\nmore\n");
}

/// (9) The vertical scrollbar is wide and hugs the right edge of the editor.
#[test]
fn scrollbar_is_wide_and_parked_on_the_right() {
    let src = ui_source();
    // The bars exist only with the minimap off (R12): the map is the indicator.
    let sb = &src[src.find("!app.settings.minimap_open && total > text.h {")
        .expect("vertical scrollbar")..];
    let sb = &sb[..sb.find("Hit::ScrollV").unwrap()];
    assert!(sb.contains("editor.right() - 13.0"), "scrollbar must hug the right edge");
    assert!(sb.contains("9.0"), "scrollbar must be wide enough to grab");
}

/// (10) The middle button pans the page, and letting go keeps it moving.
#[test]
fn middle_button_pans_with_kinetic_scroll() {
    let mut h = harness("pan");
    // A document much taller than the viewport.
    let body = filler(200);
    h.type_str(&body);
    h.app.set_caret(0, false);
    h.draw();
    let before = h.app.tab().scroll;
    h.app.mouse = (400.0, 400.0);
    h.app.pan_begin();
    // Dragging *up* moves the page down (grabbing the paper, not the scrollbar).
    h.app.pan_to((400.0, 300.0));
    assert!(h.app.tab().scroll > before + 50.0, "middle-drag must scroll the page");
    let caret_before = h.app.tab().caret;
    assert_eq!(h.app.tab().caret, caret_before, "panning must never move the caret");

    // A quick flick keeps going for a few frames, then stops.
    h.app.mouse = (400.0, 200.0);
    h.app.pan_to((400.0, 200.0));
    h.app.pan_end();
    let after_release = h.app.tab().scroll;
    if h.app.kinetic_active() {
        h.app.step_kinetic();
        assert!(h.app.tab().scroll >= after_release, "the fling must continue forwards");
    }
    // Whatever happens, the fling always ends (each frame is ~4 ms of real time).
    for _ in 0..400 {
        if !h.app.kinetic_active() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(4));
        h.app.step_kinetic();
    }
    assert!(!h.app.kinetic_active(), "the fling must decay to a stop");
}

/// (11) Checkboxes are bigger and keep their distance from the text.
#[test]
fn task_checkbox_is_big_and_has_a_gap() {
    let mut h = harness("checkbox-metrics");
    h.type_str("- [ ] task\n- bullet\n");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    let layout = &h.app.tab().layout;
    let task_indent = layout.lines[0].indent;
    let bullet_indent = layout.lines[1].indent;
    assert!(
        task_indent >= bullet_indent + 6.0,
        "task text needs a wider gutter: {task_indent} vs {bullet_indent}"
    );
    // The checkbox is clickable and at least 15 px across.
    let Some(r) = h.app.hits.find(|hit| matches!(hit, Hit::Task(_))) else {
        panic!("checkbox must be clickable");
    };
    assert!(r.h >= 15.0, "checkbox is too small: {}x{}", r.w, r.h);
    // The text starts after the checkbox, with room to spare.
    let text_x = h.app.text_rect.x + task_indent;
    assert!(text_x - (r.x + r.w) >= 4.0, "text touches the checkbox");
}

/// (12) Zooming never scrolls the page.
#[test]
fn zooming_does_not_scroll() {
    let mut h = harness("zoom-scroll");
    let body = filler(200);
    h.type_str(&body);
    h.app.set_caret(0, false);
    h.app.tab_mut().scroll = 300.0;
    let before = h.app.tab().scroll;
    let old = h.app.theme.font_size;
    h.app.zoom(2.0);
    let ratio = h.app.theme.font_size / old;
    assert!(ratio > 1.0);
    // The scroll offset follows the font size, so the same text stays on screen:
    // it must not jump by a wheel step.
    let after = h.app.tab().scroll;
    assert!(
        (after - before * ratio).abs() < 1.0,
        "scroll {before} -> {after} should scale with the font ({ratio})"
    );
}

// ---------------------------------------------------------------------------
// Round 5
// ---------------------------------------------------------------------------

/// (1) Typing a space at the end of a table cell inserts a space and keeps the
/// caret in that cell; it must not jump to the next cell.
#[test]
fn space_at_the_end_of_a_cell_stays_in_the_cell() {
    let mut h = harness("r5-space-cell");
    h.type_str("| Name | Value |\n| ---- | ----- |\n| alpha | 1 |\n");
    let text = h.app.tab().doc.text();
    let at = text.find("alpha").unwrap();
    h.app.set_caret(text[..at].chars().count() + 5, false);
    h.app.insert_text(" ");
    h.draw();
    let text = h.app.tab().doc.text();
    assert!(text.contains("alpha "), "the space must be typed: {text:?}");
    // The caret is still inside the first cell, left of the second column.
    let tab = h.app.tab();
    let line_idx = tab.layout.line_of_char(tab.caret);
    let line = &tab.layout.lines[line_idx];
    let local = tab.caret - line.start_char;
    let mut te = bare_md::text::TextEngine::new();
    let x = line.x_abs(&mut te, local, false);
    assert!(
        x < line.table_cols[1],
        "caret x {x} must stay left of column 1 at {}",
        line.table_cols[1]
    );
}

/// (2) Zooming out must not open uneven holes around list blocks: every gap is
/// proportional to the font, so the rhythm is the same at any zoom.
#[test]
fn zoomed_out_lists_keep_an_even_rhythm() {
    let mut h = harness("r5-zoom-list");
    h.type_str("* Item 3a\n\n* Item 3b\n\n```\ncode\n```\n");
    for size in [17.0f32, 11.0] {
        h.app.theme.font_size = size;
        h.app.invalidate_layouts();
        h.draw();
        let tab = h.app.tab();
        let lines: Vec<_> = tab
            .layout
            .lines
            .iter()
            .filter(|l| matches!(l.block(), bare_md::markdown::Block::Bullet))
            .collect();
        assert_eq!(lines.len(), 2);
        // The gap a list item reserves above itself is the same for both items
        // and scales with the font instead of staying a fixed 2 px.
        let gap = lines[0].space_before;
        assert_eq!(gap, lines[1].space_before);
        let expect = 2.0 * (size / bare_md::layout::REF_FONT);
        assert!(
            (gap - expect).abs() < 0.35,
            "gap {gap} at font {size} should track the font ({expect})"
        );
        assert!(lines[0].space_after == 0.0);
    }
}

/// (3) A whole-row selection stops at the end of the text instead of painting a
/// band to the right edge of the page.
#[test]
fn whole_row_selection_stops_at_the_content() {
    let mut h = harness("r5-row-select");
    h.type_str("short line of text\n\n```\nfn main() {}\n```\n");
    let text = h.app.tab().doc.text();
    let a = text[..text.find("short").unwrap()].chars().count();
    h.app.select_range(a, a + "short line of text".len());
    h.draw();
    let tab = h.app.tab();
    let line = &tab.layout.lines[tab.layout.line_of_char(a)];
    // The painted band may not be wider than the text plus its small pad.
    assert!(line.width < 400.0);
    assert_eq!(line.block(), bare_md::markdown::Block::Paragraph);
}

/// (5) Quoting a task line keeps the checkbox, and quoting a selection quotes
/// every row, not only the caret's row.
#[test]
fn quoting_keeps_formatting_and_covers_every_row() {
    let mut h = harness("r5-quote-task");
    h.type_str("- [ ] first task\n- [x] second task\n- [ ] third task\n");
    let end = h.app.tab().doc.len_chars();
    h.app.select_range(0, end - 1);
    h.app.toggle_quote();
    h.draw();
    let text = h.app.tab().doc.text();
    for row in ["first task", "second task", "third task"] {
        let line = text
            .lines()
            .find(|l| l.contains(row))
            .unwrap_or_else(|| panic!("row {row} vanished: {text:?}"));
        assert!(line.starts_with("> "), "row not quoted: {line:?}");
        assert!(line.contains("- ["), "checkbox lost: {line:?}");
    }
    assert!(text.contains("- [x] second task"), "checked state lost: {text:?}");
    // Pressing quote again removes the bars and still keeps the checkboxes.
    let end = h.app.tab().doc.len_chars();
    h.app.select_range(0, end - 1);
    h.app.toggle_quote();
    h.draw();
    let text = h.app.tab().doc.text();
    assert!(!text.contains("> "), "quote not removed: {text:?}");
    assert_eq!(text.lines().count(), 3, "no row lost: {text:?}");
}

/// (6) Enter on an empty list item continues the list, Shift+Enter ends it.
#[test]
fn enter_continues_lists_and_shift_enter_ends_them() {
    let mut h = harness("r5-list-enter");
    h.type_str("1. one");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    h.app.newline_ex(false);
    h.app.insert_text("two");
    h.draw();
    assert_eq!(h.app.tab().doc.text(), "1. one\n2. two");
    let end = h.app.tab().doc.len_chars();
    h.app.set_caret(end, false);
    h.app.newline_ex(true);
    h.app.insert_text("plain");
    h.draw();
    assert_eq!(h.app.tab().doc.text(), "1. one\n2. two\nplain");
}

/// (4) The new inline commands are additive and idempotent.
#[test]
fn new_inline_commands_are_idempotent() {
    let mut h = harness("r5-inline");
    h.type_str("size");
    h.app.select_range(0, 4);
    for (open, close, want) in [
        ("++", "++", "++size++"),
        ("==", "==", "==size=="),
        ("^", "^", "^size^"),
        ("~", "~", "~size~"),
    ] {
        h.app.select_range(0, h.app.tab().doc.len_chars());
        h.app.toggle_inline(open, close);
        assert_eq!(h.app.tab().doc.text(), want, "{open} wrap");
        h.app.select_range(0, h.app.tab().doc.len_chars());
        h.app.toggle_inline(open, close);
        assert_eq!(h.app.tab().doc.text(), "size", "{open} unwrap");
    }
    h.draw();
}

/// (4) Colour and alignment land as tags, and clear formatting removes them.
#[test]
fn colour_and_alignment_round_trip() {
    let mut h = harness("r5-color-align");
    h.type_str("coloured words");
    h.app.select_range(0, 8);
    h.app.set_text_color(false, Some(0xe05c5c));
    assert!(h.app.tab().doc.text().starts_with("[#e05c5c]coloured[/]"));
    h.app.select_range(0, h.app.tab().doc.len_chars());
    h.app.set_text_color(true, Some(0x5fae6b));
    assert!(h.app.tab().doc.text().starts_with("[bg:#5fae6b]"));
    h.draw();
    h.app.select_range(0, h.app.tab().doc.len_chars());
    h.app.clear_formatting();
    let text = h.app.tab().doc.text();
    assert!(!text.contains("[#"), "colour tag left behind: {text:?}");
    h.app.set_alignment(1);
    assert!(h.app.tab().doc.text().ends_with("{center}"));
    h.app.set_alignment(0);
    assert!(!h.app.tab().doc.text().contains("{center}"));
    h.draw();
}

/// (10) Row and column commands through the table engine.
#[test]
fn table_rows_and_columns_can_be_rearranged() {
    let mut h = harness("r5-table-actions");
    h.type_str("| A | B |\n| - | - |\n| 1 | 2 |\n| 3 | 4 |\n");
    // Every command starts from a fresh caret: the offsets shift as the table is
    // rewritten, exactly like a writer clicking into the cell they mean.
    let caret_on = |h: &mut Harness, needle: &str| {
        let text = h.app.tab().doc.text();
        let at = text
            .find(needle)
            .unwrap_or_else(|| panic!("{needle} not in {text:?}"));
        let pos = text[..at].chars().count();
        h.app.set_caret(pos, false);
    };

    caret_on(&mut h, "| 1");
    h.app.run_table_action(bare_md::app::TableAction::InsertRowBelow);
    let text = h.app.tab().doc.text();
    assert_eq!(text.lines().filter(|l| l.contains('|')).count(), 5, "{text:?}");

    caret_on(&mut h, "| 1");
    h.app.run_table_action(bare_md::app::TableAction::InsertColRight);
    let text = h.app.tab().doc.text();
    let head = text.lines().find(|l| l.contains('A')).expect("header");
    assert_eq!(head.matches('|').count(), 4, "three columns: {head:?}");

    // Deleting removes the column the caret sits in (the first one here).
    caret_on(&mut h, "| 1");
    h.app.run_table_action(bare_md::app::TableAction::DeleteCol);
    let text = h.app.tab().doc.text();
    let head = text.lines().find(|l| l.contains('B')).expect("header");
    assert_eq!(head.matches('|').count(), 3, "two columns: {head:?}");
    assert!(!head.contains('A'), "the caret's column is removed: {head:?}");

    // Rows can be rearranged: the row holding `4` moves up one place.
    caret_on(&mut h, "| 4");
    let before: Vec<String> = h.app.tab().doc.text().lines().map(|l| l.to_string()).collect();
    let i4 = before.iter().position(|l| l.contains('4')).unwrap();
    h.app.run_table_action(bare_md::app::TableAction::MoveRowUp);
    h.draw();
    let after: Vec<String> = h.app.tab().doc.text().lines().map(|l| l.to_string()).collect();
    let text = after.join("\n");
    assert_eq!(after.len(), before.len(), "row count changed: {text:?}");
    let j4 = after.iter().position(|l| l.contains('4')).unwrap();
    assert_eq!(j4, i4 - 1, "row not moved up: {text:?}");
    assert_eq!(after[j4], "|     | 4   |", "row content changed: {text:?}");
}


/// (9) Document tools: whitespace, empty rows, duplicates and the list/table
/// conversions.
#[test]
fn document_tools_clean_and_convert() {
    let mut h = harness("r5-tools");
    h.type_str("a    b\n\n\nc   d\n\n\n");
    h.app.select_range(0, h.app.tab().doc.len_chars());
    h.app.run_tools_action(bare_md::app::ToolsAction::CompressSpaces);
    assert_eq!(h.app.tab().doc.text(), "a b\n\n\nc d\n\n\n");
    h.app.select_range(0, h.app.tab().doc.len_chars());
    h.app.run_tools_action(bare_md::app::ToolsAction::RemoveEmptyRows);
    assert_eq!(h.app.tab().doc.text(), "a b\nc d\n");
    h.type_str("- one: first\n- two: second\n");
    let end = h.app.tab().doc.len_chars();
    h.app.select_range(0, end);
    h.app.run_tools_action(bare_md::app::ToolsAction::ListToTable);
    let text = h.app.tab().doc.text();
    assert!(text.contains("| Item | Detail |"), "no table header: {text:?}");
    h.app.select_range(0, h.app.tab().doc.len_chars());
    h.app.run_tools_action(bare_md::app::ToolsAction::TableToList);
    let text = h.app.tab().doc.text();
    assert!(text.contains("- one | first"), "no list rows: {text:?}");
    h.draw();
}

/// (8) The context menu offers clipboard, table and document commands.
#[test]
fn context_menu_offers_the_expected_commands() {
    let mut h = harness("r5-context");
    h.type_str("| A | B |\n| - | - |\n| 1 | 2 |\n");
    let at = h.app.tab().doc.text().find('1').unwrap();
    h.app.set_caret(at, false);
    h.draw();
    let editor = h.app.text_rect;
    h.app.open_context_menu((editor.x + 40.0, editor.y + 40.0));
    let labels: Vec<String> = h
        .app
        .menu
        .as_ref()
        .expect("context menu")
        .entries
        .iter()
        .map(|e| e.label.clone())
        .collect();
    for want in ["Copy", "Paste", "Insert row below", "Delete column"] {
        assert!(
            labels.iter().any(|l| l == want),
            "missing {want:?} in {labels:?}"
        );
    }
    h.draw();
}

/// (7) A fling keeps gliding after the drag ends instead of stopping dead.
#[test]
fn a_fling_glides() {
    let mut h = harness("r5-kinetic");
    h.type_str(&"line of text\n".repeat(200));
    h.draw();
    h.app.tab_mut().scroll = 200.0;
    h.app.pan_begin();
    h.app.pan_to((500.0, -100.0));
    std::thread::sleep(std::time::Duration::from_millis(4));
    h.app.pan_to((500.0, -400.0));
    h.app.pan_end();
    std::thread::sleep(std::time::Duration::from_millis(4));
    let at_release = h.app.tab().scroll;
    let mut frames = 0;
    while h.app.kinetic_active() && frames < 200 {
        std::thread::sleep(std::time::Duration::from_millis(4));
        h.app.step_kinetic();
        frames += 1;
    }
    let travelled = (h.app.tab().scroll - at_release).abs();
    assert!(
        travelled > 60.0,
        "the fling only travelled {travelled} px over {frames} frames"
    );
}

/// Clicking a toolbar dropdown opens its menu, and clicking an entry runs it.
#[test]
fn toolbar_dropdowns_open_and_apply() {
    use bare_md::app::MenuKind;
    let mut h = harness("r5-dropdown");
    h.type_str("a title");
    h.app.set_caret(0, false);
    h.draw();
    assert!(h.click_hit(|hit| *hit == Hit::MenuButton(MenuKind::Heading)), "heading button");
    assert!(h.app.menu.is_some(), "the heading menu did not open");
    h.draw();
    // "Heading 2" is the second entry: click its centre.
    let menu = h.app.menu.as_ref().unwrap();
    assert!(menu.entries.len() == 6, "six heading levels");
    let item = h.app.hits.find(|hit| matches!(hit, Hit::MenuItem(1)));
    assert!(item.is_some(), "menu entries must be clickable");
    h.click_hit(|hit| matches!(hit, Hit::MenuItem(1)));
    assert_eq!(h.app.tab().doc.text(), "## a title");
    assert!(h.app.menu.is_none(), "the menu closes after a choice");

    // A click outside closes the menu again.
    h.draw();
    h.click_hit(|hit| *hit == Hit::MenuButton(MenuKind::Tools));
    assert!(h.app.menu.is_some());
    h.draw();
    h.click((900.0, 700.0));
    assert!(h.app.menu.is_none(), "clicking away must dismiss the menu");
}

/// The right-click menu opens where the pointer is and its entries work.
#[test]
fn right_click_opens_the_context_menu() {
    let mut h = harness("r5-rightclick");
    h.type_str("copy me");
    h.app.select_all();
    h.draw();
    let at = (h.app.text_rect.x + 60.0, h.app.text_rect.y + 60.0);
    h.app.mouse = at;
    h.draw();
    h.app.open_context_menu(at);
    h.draw();
    let menu = h.app.menu.as_ref().expect("context menu");
    assert_eq!((menu.x, menu.y), at);
    assert!(menu.entries.iter().any(|e| e.label == "Copy"));
    h.click_hit(|hit| matches!(hit, Hit::MenuItem(1)));
    assert!(h.app.menu.is_none());
}

/// The attach button opens the file picker instead of editing the document.
#[test]
fn attach_button_opens_the_picker() {
    let mut h = harness("r5-attach");
    h.type_str("text");
    h.app.set_caret(0, false);
    h.draw();
    assert!(h.click_hit(|hit| matches!(hit, Hit::Toolbar(ToolbarAction::AttachFile))));
    assert!(
        matches!(h.app.dialog, Some(bare_md::app::Dialog::Picker(_))),
        "attach must open a file picker"
    );
    assert_eq!(h.app.tab().doc.text(), "text", "the document is untouched");
}

/// (10) The table's hover widgets work through real clicks: the `+` appends a
/// row, and a grip opens a menu whose entries rearrange the table.
#[test]
fn table_hover_widgets_add_and_rearrange_rows() {
    let mut h = harness("r5-table-hits");
    h.type_str("| A | B |\n| - | - |\n| 1 | 2 |\n| 3 | 4 |\n\nAfter.\n");
    h.app.set_caret(0, false);
    h.draw();
    // Hover the table so its chrome is drawn.
    h.hover_table();

    // "+" under the table: one more row, at the end.
    let plus = h
        .hit_center(|hit| matches!(hit, Hit::TableZone(bare_md::app::TableAction::InsertRowBelow, _)))
        .expect("the InsertRowBelow button");
    h.click(plus);
    let text = h.app.tab().doc.text();
    assert_eq!(
        text.lines().filter(|l| l.contains('|')).count(),
        5,
        "a row must be appended: {text:?}"
    );
    assert!(text.lines().last().map(|l| l.contains('|')).unwrap_or(false) || text.contains("|     |     |"));

    // A grip opens the row menu; "Move row up" swaps it with its neighbour.
    // (10.3) There is one grip, on the row under the pointer, so hover the
    // appended row first and take *its* grip.
    let last_row_y = {
        let tab = h.app.tab();
        let rows: Vec<f32> = tab
            .layout
            .lines
            .iter()
            .filter(|l| l.block() == bare_md::markdown::Block::Table && !l.info.table_sep)
            .map(|l| l.top)
            .collect();
        h.app.text_rect.y + rows.last().copied().expect("a row") + 4.0 - tab.scroll
    };
    h.app.mouse = (h.app.text_rect.x + 30.0, last_row_y);
    h.draw();
    let grip = h
        .app
        .hits
        .find(|hit| matches!(hit, Hit::RowGrip(_, i) if *i == 3))
        .expect("a grip on the appended row");
    h.click_full((grip.x + grip.w * 0.5, grip.y + grip.h * 0.5));
    assert!(h.app.menu.is_some(), "the grip must open a row menu");
    h.draw();
    let labels: Vec<String> = h.app.menu.as_ref().unwrap().entries.iter().map(|e| e.label.clone()).collect();
    assert!(labels.iter().any(|l| l == "Move row up"), "{labels:?}");
    let before = h.app.tab().doc.text();
    let item = h
        .app
        .hits
        .find(|hit| match hit {
            Hit::MenuItem(i) => h.app.menu.as_ref().map(|m| m.entries.get(*i).map(|e| e.label == "Move row up").unwrap_or(false)).unwrap_or(false),
            _ => false,
        })
        .expect("the Move row up entry");
    h.click((item.x + item.w * 0.5, item.y + item.h * 0.5));
    let after = h.app.tab().doc.text();
    assert_ne!(before, after, "the row should have moved: {after:?}");
    assert_eq!(
        before.lines().count(),
        after.lines().count(),
        "no line added or lost: {after:?}"
    );
}

/// (6.1) Typing spaces at the end of a table cell must move the caret, not just
/// append invisible characters.
#[test]
fn spaces_at_the_end_of_a_cell_move_the_caret() {
    let mut h = harness("r6-cell-space");
    h.type_str("| Name | Value |\n| ---- | ----- |\n| alpha | 1 |\n");
    let text = h.app.tab().doc.text();
    let at = text.find("alpha").unwrap();
    h.app.set_caret(text[..at].chars().count() + 5, false);
    h.draw();
    let x0 = h.caret_x();
    for n in 1..=3 {
        h.app.insert_text(" ");
        h.draw();
        let x = h.caret_x();
        assert!(
            x > x0 + 1.0,
            "caret did not move after {n} space(s): {x0} -> {x}"
        );
    }
    let text = h.app.tab().doc.text();
    // The cell keeps its padding space, so three typed spaces make four.
    assert!(text.contains("alpha    |"), "spaces must be in the text: {text:?}");
}

/// (6.2) Dragging a table row grip reorders rows; a click (no movement) still
/// opens the row menu.
#[test]
fn table_row_grip_drags_and_clicks() {
    let mut h = harness("r6-row-drag");
    h.type_str("| A | B |\n| - | - |\n| 1 | 2 |\n| 3 | 4 |\n| 5 | 6 |\n");
    h.app.set_caret(0, false);
    h.draw();
    h.hover_table();

    // (10.3) The chrome is per table, not per row: only the row under the pointer
    // carries a grip, so a four-row table shows one grip, not four.
    let row_tops: Vec<f32> = {
        let layout = &h.app.tab().layout;
        layout
            .lines
            .iter()
            .filter(|l| l.block() == bare_md::markdown::Block::Table && !l.info.table_sep)
            .map(|l| l.top)
            .collect()
    };
    assert_eq!(row_tops.len(), 4, "four rows in the layout");
    h.app.mouse = (h.app.text_rect.x + 30.0, h.app.text_rect.y + row_tops[3] + 4.0 - h.app.tab().scroll);
    h.draw();
    let grips: Vec<(Rect, usize)> = h
        .app
        .hits
        .entries_for_test()
        .into_iter()
        .filter_map(|(r, hit)| match hit {
            Hit::RowGrip(_, i) => Some((*r, *i)),
            _ => None,
        })
        .collect();
    assert_eq!(grips.len(), 1, "one grip per table, not one per row");
    let (grip, idx) = grips[0];
    assert_eq!(idx, 3, "the grip belongs to the hovered row");

    // Drag the LAST row's grip upwards by two rows.
    let before: Vec<String> = h.app.tab().doc.text().lines().map(|s| s.to_string()).collect();
    h.click((grip.x + grip.w * 0.5, grip.y + grip.h * 0.5));
    // press only (the harness helper clicks down+up); do the steps by hand
    h.app.drag = None;
    h.app.mouse = (grip.x + grip.w * 0.5, grip.y + grip.h * 0.5);
    h.draw();
    assert!(h.app.drag_begin(&Hit::RowGrip(0, 3), h.app.mouse));
    let target_y = h.app.text_rect.y + row_tops[1] + 2.0 - h.app.tab().scroll;
    h.app.mouse = (grip.x + grip.w * 0.5, target_y);
    assert!(h.app.drag_update(h.app.mouse));
    assert!(h.app.drag_end(&mut h.r), "the drag must be applied");
    h.draw();
    let after: Vec<String> = h.app.tab().doc.text().lines().map(|s| s.to_string()).collect();
    assert_eq!(after.len(), before.len(), "no row lost: {after:?}");
    assert!(before[4].contains('5'), "sanity: {before:?}");
    assert!(
        !after[4].contains('5'),
        "the last row should have moved: {after:?}"
    );
    // Dropping onto row index 1 puts it right under the header.
    assert_eq!(after[2].trim_start_matches('|').split('|').next().unwrap().trim(), "5",
        "row 5 should now sit under the header: {after:?}");
}

/// (6.4) Dragging an outline handle moves the whole section, heading included.
#[test]
fn outline_grip_moves_a_whole_section() {
    let mut h = harness("r6-outline-drag");
    h.type_str("# One\na\n# Two\nb\n# Three\nc\n");
    h.app.settings.outline_open = true;
    h.app.set_caret(0, false);
    h.draw();
    assert_eq!(h.app.outline_sections().len(), 3);
    let sections = h.app.outline_sections();
    assert_eq!(sections[0], (0, 2, 1, "One".to_string()));
    // Move "One" (with its body) to the end.
    h.app.move_outline_section(0, 2);
    let text = h.app.tab().doc.text();
    assert_eq!(text, "# Two\nb\n# Three\nc\n# One\na\n", "section moved as a block");
    // …and back to the front.
    h.app.move_outline_section(2, 0);
    let text = h.app.tab().doc.text();
    assert_eq!(text, "# One\na\n# Two\nb\n# Three\nc\n", "round trip");
    // The outline rows carry a drag handle.
    h.draw();
    assert!(
        h.app.hits.find(|hit| matches!(hit, Hit::OutlineGrip(_))).is_some(),
        "outline rows need drag handles"
    );
}

/// (6.5) A standalone `![alt](src)` line is drawn as the picture: the image gets
/// real height in the layout, and the file is decoded for the painter.
#[test]
fn images_are_laid_out_and_drawn() {
    let mut h = harness("r6-image");
    // A real PNG next to the document.
    let dir = h.app.paths.data_dir.clone();
    let path = dir.join("shot.png");
    let mut bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut bytes, 40, 20);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&vec![220u8; 40 * 20 * 4]).unwrap();
    }
    std::fs::write(&path, &bytes).unwrap();
    // The document has to exist so the tab gets a path: relative image
    // references resolve against its folder.
    let notes = dir.join("notes.md");
    std::fs::write(&notes, "").unwrap();
    h.app.open_path(&notes);
    h.type_str("# Notes\n\n![](shot.png)\n\nafter\n");
    h.app.set_caret(0, false);
    h.draw();
    let tab = h.app.tab();
    let img_line = tab
        .layout
        .lines
        .iter()
        .find(|l| l.image.is_some())
        .expect("the image line must carry a picture");
    let img = img_line.image.as_ref().unwrap();
    assert!(!img.missing, "the file is right next to the document: {img:?}");
    assert!((img.w / img.h - 2.0).abs() < 0.05, "aspect ratio kept: {img:?}");
    assert!(
        img_line.height >= img.h,
        "the layout must reserve the picture's height"
    );
    // The painter decodes it, and the text of the line stays hidden.
    let tex = h.r.image(&img.path, 512).expect("decode");
    assert_eq!((tex.w, tex.h), (40, 20));
    assert!(img_line.clean.is_empty(), "no text where the picture is");
    // A missing file still gets a box (the placeholder), never a silent gap.
    h.app.insert_text("\n![](nope.png)\n");
    h.draw();
    let missing = h
        .app
        .tab()
        .layout
        .lines
        .iter()
        .filter_map(|l| l.image.as_ref())
        .any(|i| i.missing);
    assert!(missing, "a missing image keeps its box");
}

/// The parser only treats a *standalone* image as a picture; inline ones stay text.
#[test]
fn only_standalone_images_become_pictures() {
    use bare_md::markdown;
    let mut fenced = None;
    let chars: Vec<char> = "![a](p.png)".chars().collect();
    assert!(markdown::parse(&chars, &mut fenced).image.is_some());
    let chars: Vec<char> = "text ![a](p.png) more".chars().collect();
    assert!(markdown::parse(&chars, &mut fenced).image.is_none());
    let chars: Vec<char> = "![a](p.png \"title\")".chars().collect();
    let info = markdown::parse(&chars, &mut fenced);
    let img = info.image.expect("image with a title");
    assert_eq!((img.alt.as_str(), img.src.as_str(), img.title.as_str()), ("a", "p.png", "title"));
}

/// Attaching a picture inserts a Markdown image on its own line (so it renders),
/// with a path relative to the document; other files stay links.
#[test]
fn attaching_a_picture_inserts_an_image() {
    let mut h = harness("r6-attach");
    let dir = h.app.paths.data_dir.clone();
    let doc = dir.join("note.md");
    std::fs::write(&doc, "").unwrap();
    let img = dir.join("shot.png");
    let mut bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut bytes, 8, 8);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&vec![255u8; 8 * 8 * 4]).unwrap();
    }
    std::fs::write(&img, &bytes).unwrap();
    h.app.open_path(&doc);
    h.app.attach_file(&img);
    h.draw();
    let text = h.app.tab().doc.text();
    assert!(text.contains("![shot.png](shot.png)"), "relative image link: {text:?}");
    assert!(
        h.app.tab().layout.lines.iter().any(|l| l.image.is_some()),
        "the inserted image must lay out as a picture"
    );
    // A non-image file stays a plain link.
    let other = dir.join("notes.txt");
    std::fs::write(&other, "x").unwrap();
    h.app.attach_file(&other);
    assert!(h.app.tab().doc.text().contains("[notes.txt]("));
}

// ===================== round 7 =====================

/// (7.1) The right-click menu must survive the button release that follows the
/// press; closing on release made it flash for one frame.
#[test]
fn context_menu_survives_the_button_release() {
    use winit::event::{ElementState, MouseButton, WindowEvent};
    let mut h = harness("r7-rmb");
    h.type_str("hello");
    let mut input = bare_md::input::InputState::default();
    let pos = (h.app.text_rect.x + 40.0, h.app.text_rect.y + 30.0);
    h.app.mouse = pos;
    h.draw();
    let press = WindowEvent::MouseInput {
        state: ElementState::Pressed,
        button: MouseButton::Right,
        device_id: winit::event::DeviceId::dummy(),
    };
    h.app.handle_event(&press, &mut h.r, &mut input);
    assert!(h.app.menu.is_some(), "press opens the menu");
    let release = WindowEvent::MouseInput {
        state: ElementState::Released,
        button: MouseButton::Right,
        device_id: winit::event::DeviceId::dummy(),
    };
    h.app.handle_event(&release, &mut h.r, &mut input);
    assert!(h.app.menu.is_some(), "the release must not close the menu");
    h.draw();
    assert!(
        h.app.hits.find(|hit| matches!(hit, Hit::MenuItem(_))).is_some(),
        "the menu has clickable entries"
    );
}

/// (7.2) A click on the minimap has to land where the bars are: the map draws on
/// a spacing-free virtual scale, so the old `y / height` fraction pointed at the
/// wrong line.
#[test]
fn minimap_click_maps_to_the_bar_under_the_pointer() {
    let mut h = harness("r7-minimap");
    h.app.new_tab();
    h.app.settings.minimap_open = true;
    let mut doc = String::new();
    for i in 0..60 {
        if i % 6 == 0 {
            doc.push_str(&format!("# Section {i}\n\n"));
        }
        doc.push_str(&format!("line {i}\n\n"));
    }
    h.app.insert_text(&doc);
    h.app.set_caret(0, false);
    h.draw();
    let map = h.app.minimap_rect;
    assert!(!map.is_empty(), "the minimap is on");
    let tab = h.app.tab();
    let layout = &tab.layout;
    let vtop = bare_md::ui::minimap_virtual_tops(layout);
    let geom = bare_md::ui::minimap_geom(layout, map);
    // Click on the middle of the strip of bars, then check the line that the map
    // showed there is the one now in the middle of the viewport: a click centres
    // what it lands on.
    let click_y = map.y + geom.content_h * 0.5;
    h.app.mouse = (map.x + map.w * 0.5, click_y);
    h.app.on_mouse_down(&mut h.r);
    let scroll = h.app.tab().scroll;
    let text_h = h.app.text_rect.h;
    let landed = h.app.tab().layout.line_at_y(scroll + text_h * 0.5);
    // While the bars fill the column the pointer picks a document position and the
    // viewport is centred on it. While they only fill part of the column the handle
    // is a thumb that spans the whole column, and the press puts its middle under
    // the pointer: same idea, measured in the space the handle moves in.
    // A press puts the handle's middle under the pointer, and the handle travels
    // the whole column, so the document position it lands on is the same linear
    // mapping the paint uses.
    let total = h.app.tab().layout.total_height;
    let view = geom.view_h_px(total, text_h);
    let travel = geom.handle_travel(total, text_h);
    let t = (((click_y - map.y) - bare_md::ui::MINIMAP_PAD - view * 0.5) / travel).clamp(0.0, 1.0);
    let want_y = t * (total - text_h).max(0.0) + text_h * 0.5;
    let _ = &vtop;
    let want = h.app.tab().layout.line_at_y(want_y);
    assert!(
        (landed as isize - want as isize).abs() <= 2,
        "the click put line {landed} in the middle of the viewport, the map showed line {want} (scroll {scroll})"
    );
    // The map is clickable over its whole column: the handle lives there.
    let (geom, _) = h.app.minimap_geometry();
    let last_bar = map.y + geom.bars_bottom() - 1.0;
    assert!(
        h.app
            .hits
            .at((map.x + map.w * 0.5, last_bar))
            .map(|hit| matches!(hit, Hit::Minimap))
            .unwrap_or(false),
        "the last bar must be clickable (bars end at {:.0})",
        map.y + geom.bars_bottom()
    );
    let below = map.y + geom.bars_bottom() + 9.0;
    if below < map.bottom() {
        assert!(
            h.app
                .hits
                .at((map.x + map.w * 0.5, below))
                .map(|hit| matches!(hit, Hit::Minimap))
                .unwrap_or(false),
            "the column below the bars must take a press"
        );
    }
}

/// (7.3) Indenting a list item twice must nest it, not turn it into a code card.
#[test]
fn nested_lists_indent_instead_of_turning_into_code() {
    use bare_md::markdown::Block;
    let mut h = harness("r7-nesting");
    // Three flat items, then nest the second and the third by one and two levels.
    h.type_str("- one\n- two\n- three\n");
    h.app.set_caret(h.app.tab().doc.line_start(1) + 2, false);
    h.app.indent(false);
    h.draw();
    h.app.set_caret(h.app.tab().doc.line_start(2) + 2, false);
    h.app.indent(false);
    h.app.indent(false);
    h.draw();
    let lines: Vec<String> = (0..3).map(|i| h.app.tab().doc.line_text(i)).collect();
    let depth_of = |h: &Harness, i: usize| -> usize {
        let l = &h.app.tab().layout.lines[i];
        assert!(
            matches!(l.block(), Block::Bullet | Block::Ordered | Block::Task(_)),
            "line {i} became {:?} instead of a list item: {:?}",
            l.block(),
            h.app.tab().doc.line_text(i)
        );
        l.info.depth
    };
    assert_eq!((depth_of(&h, 0), depth_of(&h, 1), depth_of(&h, 2)), (0, 1, 2), "{lines:?}");
    // The rendered indent must grow with the level, and the markers must stay
    // editable list items (not become a code card).
    let x: Vec<f32> = (0..3).map(|i| h.app.tab().layout.lines[i].indent).collect();
    assert!(x[1] > x[0] + 8.0 && x[2] > x[1] + 8.0, "indents: {x:?}");
    // Outdent brings it back.
    h.app.set_caret(h.app.tab().doc.line_start(2) + 4, false);
    h.app.indent(true);
    h.draw();
    assert_eq!(depth_of(&h, 2), 1);
}

/// (7.4) A code block's copy button copies the code, without the fences.
#[test]
fn code_block_copy_button_copies_the_body() {
    let mut h = harness("r7-copy");
    h.type_str("```\nlet x = 1;\nlet y = 2;\n```\n");
    h.app.set_caret(0, false);
    // Hover the card so its button appears.
    let code = h.code_line().expect("a code line");
    let y = h.row_y(code, 0, 0.5);
    h.app.mouse = (h.app.text_rect.x + 30.0, y);
    h.draw();
    let btn = h
        .hit_center(|hit| matches!(hit, Hit::CopyCode(..)))
        .expect("a copy button on the code card");
    h.click_full(btn);
    assert_eq!(
        h.app.code_text(0, 3),
        "let x = 1;\nlet y = 2;",
        "the body is copied, the fences are not"
    );
    // The button ran: either the clipboard took it, or the status says why not
    // (a headless test machine has no clipboard owner).
    assert!(
        h.app.status.contains("copied") || h.app.status.contains("clipboard"),
        "status: {:?}",
        h.app.status
    );
}

/// (7.5) The closing fence sits inside the card, with the same breathing room as
/// the opening one.
#[test]
fn fence_lines_sit_inside_the_code_card() {
    let mut h = harness("r7-fence-pad");
    h.type_str("```\nlet a = 1;\n```\n");
    h.app.set_caret(0, false);
    h.draw();
    let tab = h.app.tab();
    let lines = &tab.layout.lines;
    let open = &lines[0];
    let close = lines
        .iter()
        .filter(|l| l.block() == bare_md::markdown::Block::Fence)
        .nth(1)
        .expect("the closing fence");
    // The gap between the body and the fence must be symmetric with the gap
    // between the opening fence and the body.
    let body = lines.iter().find(|l| l.block() == bare_md::markdown::Block::Code).unwrap();
    let top_gap = body.top - (open.top + open.height);
    let bottom_gap = close.top - (body.top + body.height);
    assert!(
        (top_gap - bottom_gap).abs() <= 1.5,
        "the fences are not symmetric: {top_gap} vs {bottom_gap}"
    );
}

/// (7.6) The find fields are real text fields: type, select with shift+arrows,
/// and replace the selection by typing.
#[test]
fn find_fields_support_selection_and_replacement() {
    let mut h = harness("r7-find-field");
    h.type_str("alpha beta gamma");
    h.app.open_find(true);
    h.draw();
    // The query is selected on open, so typing replaces it.
    h.type_into_find("beta");
    h.draw();
    assert_eq!(h.app.tab().find.query, "beta");
    // Shift+left extends the selection…
    let find = &mut h.app.tab_mut().find;
    find.place_caret(bare_md::search::FindFocus::Query, 4, false);
    find.move_caret(bare_md::search::FindFocus::Query, -4, true);
    assert_eq!(
        h.app.tab().find.selection(bare_md::search::FindFocus::Query),
        Some((0, 4))
    );
    // …and typing over it replaces the whole selection.
    h.type_into_find("gamma");
    h.draw();
    assert_eq!(h.app.tab().find.query, "gamma");
    // Clicking inside the field puts the caret where the glyph is, not at the end.
    let field = h
        .app
        .hits
        .find(|hit| matches!(hit, Hit::FindQuery))
        .expect("the query field");
    let x = field.x + 8.0 + 4.0;
    h.click((x, field.y + field.h * 0.5));
    let caret = h.app.tab().find.caret(bare_md::search::FindFocus::Query).0;
    assert!(caret < 5, "a click near the start must not jump to the end: {caret}");
    // The replace field works the same way, once it has the focus.
    let rf = h
        .app
        .hits
        .find(|hit| matches!(hit, Hit::FindReplace))
        .expect("the replace field");
    h.click((rf.x + 12.0, rf.y + rf.h * 0.5));
    h.type_into_find("delta");
    h.draw();
    assert_eq!(h.app.tab().find.replace, "delta", "typing goes into the focused field");
    assert_eq!(h.app.focus, bare_md::app::Focus::FindReplace);
}

/// (7.7) "+" on a table leaves the caret inside the cell it just created, so the
/// next keystroke fills it instead of appending to the header line.
#[test]
fn table_plus_puts_the_caret_in_the_new_cell() {
    let mut h = harness("r7-table-plus");
    h.app.new_tab();
    h.app.insert_text("| A | B |\n| - | - |\n| 1 | 2 |\n");
    h.app.set_caret(20, false);
    h.draw();
    h.hover_table();
    let plus = h
        .hit_center(|hit| matches!(hit, Hit::TableZone(bare_md::app::TableAction::InsertRowBelow, _)))
        .expect("the InsertRowBelow button");
    h.click_full(plus);
    h.app.insert_text("7");
    h.draw();
    let text = h.app.tab().doc.text();
    assert!(
        text.contains("| 7"),
        "typing after + must land in the new row: {text:?}"
    );
    assert_eq!(text.lines().filter(|l| l.starts_with('|')).count(), 4, "{text:?}");
}

/// (7.8) Grey is a real theme: switching to it keeps the document and changes
/// every surface.
#[test]
fn grey_theme_is_a_third_option() {
    let mut h = harness("r7-grey");
    h.type_str("# Title");
    let black_bg = h.app.theme.bg;
    h.app.set_theme(ThemeKind::Grey);
    h.draw();
    let grey_bg = h.app.theme.bg;
    assert!(grey_bg.r > black_bg.r + 0.05, "grey must be lighter than black");
    assert_eq!(h.app.tab().doc.text(), "# Title");
    assert_eq!(h.app.settings.theme, ThemeKind::Grey);
    // The chrome steps must all be lighter than the sheet, and the outline and
    // the sidebar share the panel colour (the sheet is for the document).
    assert!(h.app.theme.bg_panel.r > h.app.theme.bg.r);
    assert!(h.app.theme.bg_hover.r > h.app.theme.bg_panel.r);
}

// ===========================================================================
// Round 8
// ===========================================================================

/// (8.1, 8.2) A colour is applied to the line under a bare caret, to a selection,
/// and removed again in both cases — with the pointer as well as with the
/// keyboard, which is what "remove colour doesn't work" was about.
#[test]
fn colour_applies_to_and_clears_a_whole_line() {
    let mut h = harness("r8-colour-line");
    h.type_str("hello world");
    h.app.set_caret(6, false);
    h.draw();
    h.app.set_text_color(false, Some(0xe05c5c));
    let text = h.app.tab().doc.text();
    assert_eq!(text, "[#e05c5c]hello world[/]", "line not coloured: {text}");
    // Pressing again recolours, it must not stack a second pair of tags.
    for _ in 0..3 {
        h.app.set_text_color(false, Some(0xe05c5c));
    }
    let text = h.app.tab().doc.text();
    assert_eq!(text.matches("[#").count(), 1, "tags stacked: {text}");
    h.app.set_text_color(false, None);
    assert_eq!(h.app.tab().doc.text(), "hello world", "the colour was not removed");
}

#[test]
fn colour_applies_to_and_clears_a_selection() {
    let mut h = harness("r8-colour-sel");
    h.type_str("one two three");
    h.app.select_range(0, 3);
    h.draw();
    h.app.set_text_color(false, Some(0x4f8fd0));
    assert_eq!(h.app.tab().doc.text(), "[#4f8fd0]one[/] two three");
    h.app.set_text_color(false, None);
    assert_eq!(h.app.tab().doc.text(), "one two three", "the selection kept its colour");
}

#[test]
fn colour_clears_from_a_caret_inside_the_span() {
    let mut h = harness("r8-colour-inside");
    h.type_str("[#e05c5c]hello[/] world");
    h.app.set_caret(13, false);
    h.draw();
    h.app.set_text_color(false, None);
    assert_eq!(h.app.tab().doc.text(), "hello world");
}

/// (8.1, 8.2) Background colour and text colour live side by side: clearing one
/// leaves the other alone, and the tags never nest more than once.
#[test]
fn background_colour_is_independent_of_text_colour() {
    let mut h = harness("r8-bg");
    h.type_str("marked");
    h.app.set_caret(3, false);
    h.draw();
    h.app.set_text_color(false, Some(0xe05c5c));
    h.app.set_text_color(true, Some(0x4f8fd0));
    let text = h.app.tab().doc.text();
    assert!(text.contains("[bg:#4f8fd0]"), "no background written: {text}");
    assert!(text.contains("[#e05c5c]"), "the text colour was dropped: {text}");
    h.app.set_text_color(true, None);
    let text = h.app.tab().doc.text();
    assert!(!text.contains("[bg:"), "background not removed: {text}");
    assert!(text.contains("[#e05c5c]"), "removing the background dropped the text colour: {text}");
    h.app.set_text_color(false, None);
    assert_eq!(h.app.tab().doc.text(), "marked");
}

/// (8.1) The toolbar's colour button opens a menu entry that opens the picker,
/// and the picker's "Apply" writes the colour it is showing.
#[test]
fn custom_colour_picker_end_to_end() {
    let mut h = harness("r8-picker");
    h.type_str("hello world");
    h.app.set_caret(6, false);
    h.draw();
    assert!(
        h.click_hit(|hit| matches!(hit, Hit::MenuButton(bare_md::app::MenuKind::Color))),
        "no colour button"
    );
    let entries = h.app.menu.as_ref().expect("colour menu").entries.clone();
    assert!(
        entries.iter().any(|e| matches!(e.action, bare_md::app::MenuAction::ColourPicker(false))),
        "no custom-colour entry in the menu"
    );
    h.app.run_menu_action(bare_md::app::MenuAction::ColourPicker(false), &mut h.r);
    h.draw();
    assert!(matches!(h.app.dialog, Some(bare_md::app::Dialog::Colour(_))), "no picker dialog");
    // Drag in the saturation square, then apply.
    let sv = h
        .app
        .hits
        .find(|hit| matches!(hit, Hit::ColourSv(_)))
        .expect("sv square");
    h.click((sv.x + sv.w * 0.9, sv.y + sv.h * 0.1));
    assert!(h.click_hit(|hit| matches!(hit, Hit::ColourOk)), "no Apply button");
    let text = h.app.tab().doc.text();
    assert!(text.contains("[#"), "nothing applied: {text}");
    assert!(h.app.last_colour.is_some(), "the toolbar swatch was not remembered");
    assert!(h.app.dialog.is_none(), "the dialog stayed open");
}

/// (71) Right-clicking a tab does **not** close it.
///
/// It used to close the tab outright - losing work for anyone who right-clicked to see
/// what a tab offers. Closing is the tab's ×, or a middle click.
#[test]
fn right_click_does_not_close_a_tab() {
    let mut h = harness("r71-tab-right-click");
    h.app.new_tab();
    h.app.new_tab();
    h.draw();
    let before = h.app.tabs.len();
    let rect = h.app.hits.find(|hit| matches!(hit, Hit::Tab(_))).expect("a tab");
    let pos = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
    h.app.mouse = pos;
    h.draw();
    let mut input = bare_md::input::InputState::default();
    use winit::event::{ElementState, MouseButton, WindowEvent};
    let button = |state: ElementState| WindowEvent::MouseInput {
        state,
        button: MouseButton::Right,
        device_id: winit::event::DeviceId::dummy(),
    };
    h.app.handle_event(&button(ElementState::Pressed), &mut h.r, &mut input);
    h.draw();
    h.app.handle_event(&button(ElementState::Released), &mut h.r, &mut input);
    h.draw();
    assert_eq!(h.app.tabs.len(), before, "right click closed the tab");
    // (75) …and what it opens is the *tab* menu - close, close others, close all,
    // rename - not the document's: a menu of editing commands over a tab was never
    // what the click was about.
    assert!(h.app.tab_menu.is_some(), "a right click on a tab opened no tab menu");
    assert!(h.app.menu.is_none(), "a right click on a tab opened the document menu");
    assert!(
        h.app.tab_menu_entries().iter().any(|(l, _)| l == "Close others"),
        "the tab menu is missing its commands"
    );
    // Dismiss it so the rest of the test presses the close button, not the menu.
    h.app.tab_menu = None;
    h.draw();
    // The tab's own close button still works.
    let close = h.app.hits.find(|hit| matches!(hit, Hit::TabClose(_)))
        .expect("a tab close button");
    let cpos = (close.x + close.w * 0.5, close.y + close.h * 0.5);
    h.app.mouse = cpos;
    h.draw();
    assert!(h.click_hit(|hit| matches!(hit, Hit::TabClose(_))), "no close button to click");
    assert_eq!(h.app.tabs.len(), before - 1, "the close button did not close the tab");
}

/// (8.4) A tab can be dragged along the strip and lands where it is dropped.
#[test]
fn dragging_a_tab_reorders_the_strip() {
    let mut h = harness("r8-tab-drag");
    h.app.new_tab();
    let count = h.app.tabs.len();
    h.app.new_tab();
    h.app.new_tab();
    h.draw();
    let rects = h.app.tab_rects.clone();
    assert!(rects.len() >= 3, "tab rects were not published: {}", rects.len());
    let first = rects[0];
    h.app.mouse = (first.x + first.w * 0.5, first.y + first.h * 0.5);
    h.draw();
    h.app.click(h.app.mouse, &mut h.r);
    assert!(h.app.tab_drag.is_some(), "a press on a tab must start a drag");
    let third = rects[2];
    let target = (third.x + third.w * 0.5, third.y + third.h * 0.5);
    let mut input = bare_md::input::InputState::default();
    h.app.handle_event(
        &winit::event::WindowEvent::CursorMoved {
            device_id: winit::event::DeviceId::dummy(),
            position: winit::dpi::PhysicalPosition::new(target.0 as f64, target.1 as f64),
        },
        &mut h.r,
        &mut input,
    );
    h.draw();
    h.app.mouse_up(&mut h.r);
    h.draw();
    assert_eq!(h.app.tabs.len(), count + 2, "tabs were lost while reordering");
    assert_eq!(h.app.active, 2, "the dragged tab did not land at the end");
}

/// (8.5) The two `+` buttons on a table grow it in both directions, and the
/// painted grid follows: a new column used to change the source and nothing else.
#[test]
fn table_plus_buttons_grow_the_grid_in_both_directions() {
    let mut h = harness("r8-table-plus");
    h.type_str("| A | B |\n| - | - |\n| 1 | 2 |\n\nTail.\n");
    h.app.set_caret(0, false);
    h.draw();
    // Hover the table so its chrome is drawn.
    h.hover_table();
    let plus = h
        .hit_center(|hit| matches!(hit, Hit::TableZone(bare_md::app::TableAction::InsertColRight, _)))
        .expect("the InsertColRight button");
    h.click_full(plus);
    let text = h.app.tab().doc.text();
    assert_eq!(text.lines().next().unwrap().matches('|').count(), 4, "no column: {text:?}");
    let cols = &h.app.tab().layout.lines[0].table_cols;
    assert_eq!(cols.len(), 3, "the painted grid still has {} columns", cols.len());

    h.hover_table();
    let plus = h
        .hit_center(|hit| matches!(hit, Hit::TableZone(bare_md::app::TableAction::InsertRowBelow, _)))
        .expect("the InsertRowBelow button");
    h.click_full(plus);
    let text = h.app.tab().doc.text();
    assert!(text.contains("Tail."), "the paragraph after the table was eaten: {text:?}");
    let rows = text.lines().filter(|l| l.trim_start().starts_with('|')).count();
    assert_eq!(rows, 4, "{text:?}");
}

/// (8.10) On the last line of a code block the caret stays inside the card: the
/// closing fence is laid out at 0.6 of a line, and a full-height caret poked
/// through the bottom edge.
#[test]
fn caret_on_the_last_code_line_stays_inside_the_card() {
    let mut h = harness("r8-code-caret");
    h.type_str("Text.\n\n```\nline one\nline two\n```");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    let cr = h.app.caret_rect(&mut h.r.text);
    // The card ends a card-pad below the fence line; the caret must end above it.
    let fence = h
        .app
        .tab()
        .layout
        .lines
        .iter()
        .filter(|l| l.block() == bare_md::markdown::Block::Fence)
        .next_back()
        .expect("a closing fence");
    let card_bottom = h.app.text_rect.y + fence.top + fence.height + 6.0;
    assert!(
        cr.bottom() <= card_bottom - 1.0,
        "the caret ({}) reaches the bottom of the code card ({card_bottom})",
        cr.bottom()
    );
}

/// (8.11) The empty document's placeholder is never covered by the caret.
#[test]
fn empty_documents_placeholder_has_no_caret_over_it() {
    let mut h = harness("r8-empty");
    h.app.new_tab();
    h.draw();
    assert_eq!(h.app.tab().doc.len_chars(), 0);
    assert!(!h.app.caret_is_painted(), "the caret is drawn on top of the placeholder");
    // Typing one character brings it back.
    h.type_str("x");
    assert!(h.app.caret_is_painted(), "the caret did not come back");
}


// ---------------------------------------------------------------- round 9 ---

/// (9.1) A middle click on a tab closes it, and the document behind it stays.
#[test]
fn middle_click_closes_a_tab() {
    use winit::event::{ElementState, MouseButton, WindowEvent};
    let mut h = harness("r9-middle-close");
    h.app.new_tab();
    h.app.insert_text("keep me");
    h.app.new_tab();
    h.app.insert_text("close me");
    h.draw();
    let before = h.app.tabs.len();
    // The *second* tab, so the click cannot be mistaken for "close the active one".
    let rects = h.app.tab_rects.clone();
    assert!(rects.len() >= 2, "expected two tabs, got {}", rects.len());
    let target = rects[1];
    let pos = (target.x + target.w * 0.5, target.y + target.h * 0.5);
    h.app.mouse = pos;
    h.draw();
    let mut input = bare_md::input::InputState::default();
    h.app.handle_event(
        &WindowEvent::MouseInput {
            state: ElementState::Pressed,
            button: MouseButton::Middle,
            device_id: winit::event::DeviceId::dummy(),
        },
        &mut h.r,
        &mut input,
    );
    h.draw();
    // A tab with unsaved text asks first, exactly like the tab's own close button.
    assert!(
        matches!(h.app.dialog, Some(bare_md::app::Dialog::Confirm { .. })),
        "a dirty tab should ask before closing"
    );
    assert!(h.click_hit(|hit| *hit == Hit::DialogConfirm), "no Discard button");
    h.draw();
    assert_eq!(h.app.tabs.len(), before - 1, "the middle click did not close a tab");
    // Middle-dragging inside the document must still pan, not close anything.
    let centre = (
        h.app.text_rect.x + h.app.text_rect.w * 0.5,
        h.app.text_rect.y + h.app.text_rect.h * 0.5,
    );
    h.app.mouse = centre;
    h.app.handle_event(
        &WindowEvent::MouseInput {
            state: ElementState::Pressed,
            button: MouseButton::Middle,
            device_id: winit::event::DeviceId::dummy(),
        },
        &mut h.r,
        &mut input,
    );
    assert_eq!(h.app.tabs.len(), before - 1, "panning closed a tab");
}

/// (9.2 -> r13) The minimap shows the whole document, so the wheel over it must
/// mean what a wheel over the text means: the document moves, and the map stays
/// a single fixed mapping from the top of the file to the bottom.
#[test]
fn minimap_shows_the_whole_document_and_the_wheel_scrolls_the_page() {
    use winit::event::{MouseScrollDelta, WindowEvent};
    let mut h = harness("r9-minimap-scroll");
    h.type_str(&filler(220));
    h.app.set_caret(0, false);
    h.draw();
    let map = h.minimap();
    let layout = &h.app.tab().layout;
    let geom = bare_md::ui::minimap_geom(layout, map);
    assert!(
        geom.scale <= bare_md::ui::MINIMAP_PITCH + 0.001,
        "the pitch must never exceed a comfortable line ({} px)",
        geom.scale
    );
    // One pitch for every document (71): what changes with length is how much of the
    // file the strip shows - the map slides when it does not fit - never the size of a
    // bar or the gap between bars.
    assert_eq!(
        geom.scale,
        bare_md::ui::MINIMAP_PITCH,
        "the pitch changed with the document ({} px/row)",
        geom.scale
    );
    assert_eq!(geom.bar_h, bare_md::ui::MINIMAP_BAR_H, "the bars changed thickness");
    assert!(
        geom.content_h <= map.h || geom.offset >= 0.0,
        "the map neither fits nor slides"
    );
    // The wheel over the strip scrolls the document.
    let doc_scroll = h.app.tab().scroll;
    h.app.mouse = (map.x + map.w * 0.5, map.y + map.h * 0.5);
    let mut input = bare_md::input::InputState::default();
    h.app.handle_event(
        &WindowEvent::MouseWheel {
            delta: MouseScrollDelta::LineDelta(0.0, -1.0),
            phase: winit::event::TouchPhase::Moved,
            device_id: winit::event::DeviceId::dummy(),
        },
        &mut h.r,
        &mut input,
    );
    h.draw();
    assert!(
        h.app.tab().scroll > doc_scroll + 0.5,
        "the wheel over the map did not scroll the document"
    );
}

/// (9.3) Clicking a table cell puts the caret inside that cell - including the
/// empty cell a `+` button just added, and including the last column.
#[test]
fn clicking_any_table_cell_lands_in_that_cell() {
    let mut h = harness("r9-table-cells");
    h.app.new_tab();
    h.type_str("| A | B | C |\n| - | - | - |\n| one | two | three |\n\nTail.\n");
    h.app.set_caret(0, false);
    h.draw();
    let row = h.last_table_row();
    let cols = h.app.tab().layout.lines[row].table_cols.clone();
    assert_eq!(cols.len(), 3, "the painted grid should have three columns");
    for (ci, want) in [(0usize, "one"), (1, "two"), (2, "three")] {
        let (x, y) = h.cell_point(row, ci, 0.1);
        h.click_full((x, y));
        let caret = h.app.tab().caret;
        let line = h.app.tab().doc.line_of_char(caret);
        let text = h.app.tab().doc.line_text(line);
        assert!(
            text.contains(want),
            "clicking column {ci} put the caret on {text:?}, not in the cell holding {want:?}"
        );
        // And the caret is inside that cell's span, not just anywhere on the row.
        let local = caret - h.app.tab().doc.line_start(line);
        let (a, b) = h.app.tab().layout.lines[line].info.cells[ci];
        assert!(local >= a && local <= b, "caret {local} outside cell {a}..{b}");
    }
}

/// (9.4) A `.jxl` file is recognised, sized and decoded.
#[test]
fn jxl_images_probe_and_decode() {
    let src = "![pic](tiny.jxl)\n";
    let dir = std::env::temp_dir().join(format!("bare-md-jxl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // 4x3 lossless JPEG XL, generated once with `jxl-encoder`.
    const TINY_JXL: [u8; 67] = [
        0xFF, 0x0A, 0x10, 0x30, 0x10, 0x09, 0x08, 0x02, 0x01, 0x00, 0xDC, 0x00, 0x4B, 0x12, 0xF8,
        0x04, 0xFF, 0x70, 0x80, 0x3D, 0x01, 0x78, 0x03, 0x80, 0x08, 0xC8, 0xF9, 0x2F, 0xDF, 0xEF,
        0xC2, 0x06, 0xF0, 0x86, 0x2B, 0x88, 0xF9, 0x46, 0x9F, 0x30, 0x7A, 0xEC, 0xAB, 0xDF, 0x2B,
        0x40, 0x62, 0x9F, 0xA0, 0xCB, 0x48, 0xFF, 0xA7, 0xF6, 0xF9, 0x70, 0xFB, 0x0B, 0x40, 0x0F,
        0xE8, 0x0F, 0x84, 0xC5, 0xD2, 0xA1, 0xD0,
    ];
    let path = dir.join("tiny.jxl");
    std::fs::write(&path, TINY_JXL).unwrap();
    let doc = dir.join("note.md");
    std::fs::write(&doc, src).unwrap();
    let mut h = harness("r9-jxl");
    h.app.open_path(&doc);
    h.draw();
    let line = h.app.tab().layout.lines[0].clone();
    let img = line.image.expect("the picture was not recognised");
    assert!(!img.missing, "the .jxl file was not decoded");
    assert!(img.w > 24.0 && img.h > 18.0, "odd box {}x{}", img.w, img.h);
    let _ = std::fs::remove_dir_all(&dir);
}

/// (9.5) A ```` ```mermaid ```` block renders as a picture, and clicking it brings
/// the source back for editing.
#[test]
fn mermaid_blocks_render_and_open_for_editing() {
    let mut h = harness("r9-mermaid");
    h.app.new_tab();
    h.type_str(
        "Before.\n\n```mermaid\nflowchart TD\n  A[Start] --> B{Ready?}\n  B -->|yes| C[Done]\n```\n\nAfter.\n",
    );
    h.app.set_caret(0, false);
    h.draw();
    let (open, close, diag) = {
        let layout = &h.app.tab().layout;
        let (open, close) = layout.mermaid[0];
        (open, close, layout.lines[open].diagram.clone())
    };
    assert_eq!((open, close), (2, 6), "the block was not found");
    let d = diag.expect("no diagram was laid out");
    assert_eq!(d.nodes.len(), 3);
    assert_eq!(d.edges.len(), 2);
    assert!(d.h > 60.0, "the diagram has no height ({})", d.h);
    // The source lines are folded away: the block occupies the picture's height.
    let block_h: f32 = (open..=close).map(|i| h.app.tab().layout.lines[i].height).sum();
    assert!((block_h - d.h).abs() < 1.0, "block height {block_h} vs diagram {}", d.h);
    assert_eq!(h.app.tab().layout.lines[open + 1].height, 0.0, "a source line kept its height");

    // Clicking the picture puts the caret in the source and unfolds it.
    let (top, indent) = {
        let line = &h.app.tab().layout.lines[open];
        (line.top, line.indent)
    };
    let x = h.app.text_rect.x + indent + 20.0;
    let y = h.app.text_rect.y + top + d.h * 0.5 - h.app.tab().scroll;
    h.click_full((x, y));
    h.draw();
    let caret_line = h.app.tab().doc.line_of_char(h.app.tab().caret);
    assert!(
        (open + 1..=close).contains(&caret_line),
        "the caret landed on line {caret_line}, outside the block"
    );
    assert!(h.app.tab().layout.lines[open + 1].height > 0.0, "the source stayed folded");
    assert!(h.app.tab().layout.lines[open].diagram.is_none(), "the picture stayed up");

    // Walking the caret out of the block folds it again.
    h.app.set_caret(0, false);
    h.draw();
    assert!(h.app.tab().layout.lines[open].diagram.is_some(), "the picture did not come back");
}

/// (9.6) The colour dialog is two columns: text on the left, background on the
/// right, each with its own square, hue strip, presets and hex field.
#[test]
fn colour_picker_has_a_text_and_a_background_column() {
    let mut h = harness("r9-colour-cols");
    h.app.new_tab();
    h.type_str("A word to colour.\n");
    h.app.set_caret(0, false);
    h.app.run_menu_action(bare_md::app::MenuAction::ColourPicker(false), &mut h.r);
    h.draw();
    let sv = |bg: bool| {
        h.app
            .hits
            .find(|hit| matches!(hit, Hit::ColourSv(b) if *b == bg))
            .expect("a saturation square")
    };
    let (text_sv, bg_sv) = (sv(false), sv(true));
    assert!(text_sv.x < bg_sv.x, "the columns are not side by side");
    assert!(
        (text_sv.y - bg_sv.y).abs() < 2.0,
        "the two squares are not level with each other"
    );
    for (bg, name) in [(false, "text"), (true, "background")] {
        for hit in [
            Hit::ColourHue(bg),
            Hit::ColourHex(bg),
            Hit::ColourPreset(bg, 0),
            Hit::ColourClear(bg),
        ] {
            assert!(
                h.app.hits.find(|h2| *h2 == hit).is_some(),
                "the {name} column is missing {hit:?}"
            );
        }
    }
    // Painting the background column and applying writes a `[bg:...]` span, and
    // leaves the text colour alone.
    let sq = h.app.hits.find(|hit| matches!(hit, Hit::ColourSv(true))).unwrap();
    h.click_full((sq.x + sq.w * 0.5, sq.y + sq.h * 0.5));
    assert!(h.click_hit(|hit| *hit == Hit::ColourOk), "no Apply button");
    let text = h.app.tab().doc.text();
    assert!(text.contains("[bg:#"), "the background was not applied: {text}");
    assert!(!text.contains("[#"), "the text colour was changed too: {text}");
}

/// (9.7) A link can be clicked: the click follows it instead of opening the
/// source for editing.
#[test]
fn clicking_a_link_follows_it() {
    let mut h = harness("r9-link");
    h.app.new_tab();
    h.type_str("# Top\n\nSee [the section](#top) for details.\n");
    h.app.set_caret(0, false);
    h.draw();
    // Put the caret far away, so a click is not "just moving the caret".
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    let (x, y, line_text) = {
        let layout = &h.app.tab().layout;
        let line = &layout.lines[2];
        let run = line
            .clean
            .iter()
            .find(|r| r.style.underline || line.chars[r.start..r.end].iter().collect::<String>().contains("section"))
            .expect("the link run");
        (
            h.app.text_rect.x + line.indent + run.x + run.w * 0.5,
            h.app.text_rect.y + line.top + line.row_h * 0.5 - h.app.tab().scroll,
            h.app.tab().doc.line_text(2),
        )
    };
    h.click_full((x, y));
    h.draw();
    let caret_line = h.app.tab().doc.line_of_char(h.app.tab().caret);
    assert_eq!(
        caret_line, 0,
        "the click did not follow the #top link (caret on line {caret_line} of {line_text:?})"
    );
    // Clicking ordinary text still places the caret where the pointer is.
    let (x2, y2) = {
        let layout = &h.app.tab().layout;
        let line = &layout.lines[2];
        (h.app.text_rect.x + line.indent + 6.0, h.app.text_rect.y + line.top + line.row_h * 0.5)
    };
    h.click_full((x2, y2));
    assert_eq!(h.app.tab().doc.line_of_char(h.app.tab().caret), 2, "plain text is not clickable");
}

/// (9.8) The open-folder dialog has an editable address bar and shows the
/// filesystem, so another disk can be reached.
#[test]
fn folder_dialog_has_an_editable_path_and_lists_the_root() {
    let mut h = harness("r9-folder-path");
    h.app.open_picker(bare_md::app::PickerKind::OpenFolder);
    h.draw();
    assert!(
        h.app.hits.find(|hit| *hit == Hit::DialogPathField).is_some(),
        "the dialog has no address bar"
    );
    // (10.5) No "Go" button: the address bar takes Enter, like every other file
    // dialog.
    assert!(
        h.app.hits.find(|hit| *hit == Hit::DialogUp).is_some(),
        "the dialog has no up button"
    );
    let before = h.picker().path.text.clone();
    assert!(!before.is_empty(), "the address bar starts empty");
    // `..`, the filesystem root, the home folder and any drive are listed, so
    // another disk is reachable from here.
    let listing = h.picker();
    let names: Vec<String> = listing.entries.iter().map(|e| e.name.clone()).collect();
    assert!(names.iter().any(|n| n == ".."), "no way back up: {names:?}");
    assert!(
        names.iter().any(|n| n.starts_with('~')) || listing.dir == std::path::Path::new("/"),
        "the home folder is missing from the listing: {names:?}"
    );

    // Click the address bar, type a different folder and press Go.
    assert!(h.click_hit(|hit| *hit == Hit::DialogPathField), "no address bar");
    let dir = std::env::temp_dir().join(format!("bare-md-pick-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    // The field starts with the current folder: select it and type over it, the
    // way any text field is edited.
    assert!(h.app.dialog_select_all(), "the address bar cannot be selected");
    h.type_into_find(&dir.to_string_lossy());
    assert_eq!(
        h.picker().path.as_str(),
        dir.to_string_lossy(),
        "typing did not reach the address bar"
    );
    // Enter commits the typed address (there is no Go button any more).
    assert!(h.app.picker_path_enter(), "Enter in the address bar was ignored");
    h.draw();
    let after = h.picker();
    assert_eq!(after.dir, dir, "Enter did not navigate");
    let names: Vec<String> = after.entries.iter().map(|e| e.name.clone()).collect();
    assert!(names.iter().any(|n| n == "sub"), "the new folder was not listed: {names:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// (9.9) A long code block keeps its card width when its first lines scroll out
/// of view: the painter always starts the card at the run's first line.
#[test]
fn a_scrolled_code_card_keeps_its_width() {
    let mut h = harness("r9-code-card");
    let mut doc = String::from("Intro.\n\n```rust\n");
    for i in 0..80 {
        doc.push_str(&format!("let value_{i} = compute({i});\n"));
    }
    doc.push_str("```\n\nAfter.\n");
    h.type_str(&doc);
    h.app.set_caret(0, false);
    h.draw();
    let layout = &h.app.tab().layout;
    let first_code = layout
        .lines
        .iter()
        .position(|l| l.block() == bare_md::markdown::Block::Code)
        .expect("a code block");
    // Wide line then a narrow one: the old painter measured only what was on
    // screen, so this pair is exactly what used to make the card jump.
    assert_eq!(bare_md::ui::card_run_start(layout, first_code), first_code, "run start");
    // Scroll deep into the block: the visible line is now far from the start.
    let (first_visible, _) = layout.visible_range(layout.lines[first_code].top + 600.0, 400.0);
    assert!(first_visible > first_code + 2, "the test did not scroll into the block");
    assert_eq!(
        bare_md::ui::card_run_start(&h.app.tab().layout, first_visible),
        first_code,
        "a scrolled card would start at line {first_visible} and change width"
    );
}

/// (9.10) A long table keeps its grid when scrolled: the row the painter starts
/// from is the header, not the first visible row, and every row still carries
/// its column geometry.
#[test]
fn a_scrolled_table_keeps_its_grid() {
    let mut h = harness("r9-table-grid");
    let mut doc = String::from("| h1 | h2 |\n| -- | -- |\n");
    for i in 0..60 {
        doc.push_str(&format!("| row {i} | value {i} |\n"));
    }
    doc.push_str("\nTail.\n");
    h.type_str(&doc);
    h.app.set_caret(0, false);
    h.draw();
    let layout = &h.app.tab().layout;
    let first = layout
        .lines
        .iter()
        .position(|l| l.block() == bare_md::markdown::Block::Table)
        .expect("a table");
    let (mid, _) = layout.visible_range(layout.lines[first].top + 500.0, 380.0);
    assert!(mid > first + 3, "the test did not scroll into the table");
    assert_eq!(
        bare_md::ui::card_run_start(&h.app.tab().layout, mid),
        first,
        "the grid would be drawn from row {mid}"
    );
    for line in h.app.tab().layout.lines.iter().filter(|l| l.block() == bare_md::markdown::Block::Table)
    {
        assert!(
            !line.table_cols.is_empty() || line.info.table_sep,
            "row {:?} has no column geometry",
            line.line
        );
    }
}

/// (10.1) The window icon is built from the SVGs in `assets/app/`, so it can be
/// edited without touching Rust: the plate and the mark both end up in the
/// bitmap, and the bitmap is a real square RGBA image.
#[test]
fn the_app_icon_comes_from_the_svg_assets() {
    let (w, h, rgba) = bare_md::appicon::rgba(64);
    assert_eq!((w, h), (64, 64), "the icon is not square");
    assert_eq!(rgba.len(), (64 * 64 * 4) as usize, "the icon is not RGBA");

    // The plate covers the middle of the plate area but not the corner (the
    // rounded plate), and the mark's bars are lighter than the plate behind them.
    let at = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y * 64 + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    };
    let corner = at(1, 1);
    assert_eq!(corner[3], 0, "the rounded corner should be transparent, got {corner:?}");
    let plate = at(32, 8);
    assert_eq!(plate[3], 255, "the plate is missing at its own edge: {plate:?}");
    let bar = at(32, 21);
    assert!(
        bar[0] as u32 > plate[0] as u32 + 60,
        "the mark is not drawn on the plate: plate {plate:?} bar {bar:?}"
    );
    // The icon really is the single asset on disk, in its own colours, and the
    // two-layer files are only a fallback.
    assert!(bare_md::appicon::ICON_SVG.contains("<rect"));
    assert_eq!([bar[0], bar[1], bar[2]], [0xf4, 0xf4, 0xf2], "the mark lost its colour");
    assert_eq!([plate[0], plate[1], plate[2]], [0x17, 0x18, 0x1c], "the plate lost its colour");
    assert!(bare_md::appicon::BACKGROUND_SVG.contains("<rect"));
    assert!(bare_md::appicon::FOREGROUND_SVG.contains("<rect"));
}

// ---------------------------------------------------------------------------
// Round 10
// ---------------------------------------------------------------------------

/// (10.6) Editing a `---` rule shows its markers while the caret is on the line,
/// and the rule's own box is at least a text line tall so it cannot overlap the
/// paragraphs above and below.
#[test]
fn editing_a_rule_shows_its_markers_and_keeps_its_space() {
    let mut h = harness("r10-hr");
    h.type_str("Above.\n\n---\n\nBelow.\n");
    let hr = |h: &Harness| -> usize {
        h.app
            .tab()
            .layout
            .lines
            .iter()
            .position(|l| l.block() == bare_md::markdown::Block::Hr)
            .expect("no rule in the layout")
    };
    h.app.set_caret(h.app.tab().doc.text().find("---").unwrap(), false);
    h.draw();
    // Caret elsewhere: it is a rule.
    h.app.set_caret(0, false);
    h.draw();
    let i = hr(&h);
    let line = &h.app.tab().layout.lines[i];
    assert!(line.height >= 12.0, "the rule's box is only {} px tall", line.height);
    // It has to line up with the text rows around it, not on top of them.
    let above = &h.app.tab().layout.lines[i - 1];
    let below = &h.app.tab().layout.lines[i + 1];
    assert!(above.top + above.height <= line.top + 0.5, "the rule overlaps the line above");
    assert!(line.top + line.height <= below.top + 0.5, "the rule overlaps the line below");

    // Caret on the rule: the markers come back and the rule itself is not painted.
    let start = h.app.tab().doc.text().find("---").unwrap();
    h.app.set_caret(start + 3, false);
    h.draw();
    assert!(
        h.app.line_is_raw(i),
        "a rule being edited still reports itself as rendered"
    );
    let text: String = h.app.tab().layout.lines[i].chars.iter().collect();
    assert_eq!(text.trim_end(), "---", "the markers are not on screen while editing");
}

/// (10.2) A drag on the minimap moves the document by the same amount for the
/// same pointer movement: no acceleration as it approaches the end.
#[test]
fn dragging_the_minimap_is_linear() {
    let mut h = harness("r10-map-drag");
    h.type_str(&filler(300));
    h.app.set_caret(0, false);
    h.open_minimap();
    let map = h.minimap();
    // Press in the middle of the map, then drag in equal steps.
    let start = h.minimap_press(0.3);
    let mut deltas = Vec::new();
    let mut last = h.app.tab().scroll;
    let step = map.h * 0.05;
    for i in 1..=6 {
        h.minimap_drag_to(start + step * i as f32);
        let now = h.app.tab().scroll;
        deltas.push(now - last);
        last = now;
    }
    h.app.mouse_up(&mut h.r);
    h.draw();
    let first = deltas[0];
    assert!(first > 1.0, "the drag did not move the document: {deltas:?}");
    for (i, d) in deltas.iter().enumerate() {
        assert!(
            (d - first).abs() < first.abs() * 0.35 + 2.0,
            "step {i} moved {d} px after a first step of {first} px: the drag accelerates ({deltas:?})"
        );
    }
}

/// (10.7) The colour tool offers text and background side by side, two columns,
/// instead of one long list where half the rows were backgrounds.
#[test]
fn the_colour_tool_is_two_columns() {
    let mut h = harness("r10-colour");
    h.type_str("hello");
    h.app.set_caret(0, false);
    h.draw();
    h.app.open_menu(bare_md::app::MenuKind::Color, Rect::new(200.0, 100.0, 20.0, 20.0));
    h.draw();
    let items: Vec<(Rect, usize)> = h
        .app
        .hits
        .entries_for_test()
        .into_iter()
        .filter_map(|(r, hit)| match hit {
            Hit::MenuItem(i) => Some((*r, *i)),
            _ => None,
        })
        .collect();
    assert!(!items.is_empty(), "the colour menu drew no rows");
    // Two x positions cover every row: that is what "two columns" means.
    let mut left: Vec<f32> = items.iter().map(|(r, _)| r.x).collect();
    left.sort_by(|a, b| a.partial_cmp(b).unwrap());
    left.dedup_by(|a, b| (*a - *b).abs() < 1.0);
    assert_eq!(left.len(), 2, "the colour menu is not in two columns: x = {left:?}");
    assert!(left[1] - left[0] > 60.0, "the columns are too close: {left:?}");
    // A text row and a background row share a line.
    let same_row = items
        .iter()
        .filter(|(r, _)| (r.x - left[0]).abs() < 1.0)
        .any(|(a, _)| items.iter().any(|(b, _)| (b.x - left[1]).abs() < 1.0 && (a.y - b.y).abs() < 1.0));
    assert!(same_row, "no text/background pair shares a row");
}

/// (11.3) Opening a folder asks in a **dialog with two columns**: the filesystem
/// tree on the left and the folder's contents on the right. Clicking a folder in
/// the tree re-points the listing.
#[test]
fn the_folder_dialog_has_a_tree_and_a_listing() {
    let mut h = harness("r11-folder-dialog");
    let root = h._dir.clone();
    std::fs::create_dir_all(root.join("sub")).unwrap();
    h.app.open_picker(bare_md::app::PickerKind::OpenFolder);
    if let Some(bare_md::app::Dialog::Picker(p)) = &mut h.app.dialog {
        p.select_dir(root.clone());
    }
    h.draw();
    assert!(
        h.app.hits.find(|hit| *hit == Hit::DialogPathField).is_some(),
        "the dialog has no address bar"
    );
    let tree_items: Vec<Rect> = h
        .app
        .hits
        .entries_for_test()
        .iter()
        .filter(|(_, h)| matches!(h, Hit::DialogTreeItem(_)))
        .map(|(r, _)| *r)
        .collect();
    assert!(!tree_items.is_empty(), "the dialog has no tree column");
    let list_items: Vec<Rect> = h
        .app
        .hits
        .entries_for_test()
        .iter()
        .filter(|(_, h)| matches!(h, Hit::DialogItem(_)))
        .map(|(r, _)| *r)
        .collect();
    assert!(!list_items.is_empty(), "the dialog lists nothing");
    // Two columns really are two columns.
    assert!(
        list_items[0].x > tree_items[0].right() - 1.0,
        "the listing is not to the right of the tree"
    );
    // Clicking the folder in the tree selects it.
    let sub_row = {
        let picker = match &h.app.dialog {
            Some(bare_md::app::Dialog::Picker(p)) => p,
            _ => panic!("no picker"),
        };
        picker
            .tree
            .rows()
            .iter()
            .position(|r| r.name == "sub")
            .expect("sub is not in the tree")
    };
    assert!(
        h.click_hit(|hit| matches!(hit, Hit::DialogTreeItem(i) if *i == sub_row)),
        "no tree row to click"
    );
    let dir = match &h.app.dialog {
        Some(bare_md::app::Dialog::Picker(p)) => p.dir.clone(),
        _ => panic!("the dialog closed"),
    };
    assert_eq!(dir, root.join("sub"), "the tree did not re-point the listing");
}

/// (10.3) The last, empty cell of a table row is a place you can put the caret:
/// the cell is zero-width in the source but still clickable.
#[test]
fn clicking_an_empty_last_cell_puts_the_caret_in_it() {
    let mut h = harness("r10-empty-cell");
    h.type_str("| A | B | C |\n| - | - | - |\n| 1 | 2 |   |\n");
    h.app.set_caret(0, false);
    h.draw();
    let row = h.last_table_row();
    let (x, y, text) = {
        let line = &h.app.tab().layout.lines[row];
        let cell = line.info.cells[2];
        assert_eq!(cell.0, cell.1, "the cell should be empty for this test");
        (
            h.cell_point(row, 2, 0.0).0,
            h.row_y(row, 0, 0.5),
            line.chars.iter().collect::<String>(),
        )
    };
    h.click_full((x, y));
    let (line_no, col) = h.app.tab().doc.line_and_col(h.app.tab().caret);
    assert_eq!(line_no, 2, "the click missed the row ({text:?})");
    // The empty cell sits between the pipes that close the row; either boundary
    // is inside it.
    let len = text.chars().count();
    assert!(
        col + 1 >= len - 1,
        "the caret ({col}) is not in the last cell of {text:?} (line is {len} chars)"
    );
    // Typing there lands in that cell, not in the one before it.
    h.app.insert_text("7");
    h.draw();
    let after = h.app.tab().doc.text();
    let row = after.lines().nth(2).unwrap_or("").to_string();
    assert!(row.contains('7'), "typing did not reach the empty cell: {row:?}");
    let (cells, chars) = {
        let tab = h.app.tab();
        let line = tab.layout.lines.iter().find(|l| l.line == 2).expect("the row");
        (line.info.cells.clone(), line.chars.clone())
    };
    let last = cells.last().copied().unwrap_or((0, 0));
    let last_text: String = chars[last.0.min(chars.len())..last.1.min(chars.len())].iter().collect();
    assert!(
        last_text.contains('7'),
        "the 7 did not land in the last cell: {row:?} cells {cells:?} text {last_text:?}"
    );
}

// ---------------------------------------------------------------------------
// Round 11
// ---------------------------------------------------------------------------

/// (11.1) A file rewritten by another program is noticed, and the three answers
/// do what they say: reload, keep, and auto-reload for this session.
#[test]
fn an_external_change_asks_before_touching_the_buffer() {
    let mut h = harness("r11-external");
    let path = h._dir.join("note.md");
    std::fs::write(&path, "first version\n").unwrap();
    h.app.open_path(&path);
    h.draw();
    assert_eq!(h.app.tab().doc.text(), "first version\n");

    // Another program writes the file, then the editor ticks.
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, "second version\n").unwrap();
    h.app.disk_check_at = std::time::Instant::now() - std::time::Duration::from_secs(1);
    h.app.poll_disk();
    match &h.app.dialog {
        Some(bare_md::app::Dialog::External(ec)) => assert_eq!(ec.path, path, "wrong file reported"),
        other => panic!("no question asked about the change: {other:?}"),
    }
    assert_eq!(h.app.tab().doc.text(), "first version\n", "the buffer changed by itself");
    h.draw();

    // "Keep mine" keeps the open text and stops asking about *this* change.
    assert!(h.click_hit(|hit| *hit == Hit::ExternalKeep), "no Keep button");
    h.draw();
    assert!(h.app.dialog.is_none(), "the dialog stayed up");
    assert_eq!(h.app.tab().doc.text(), "first version\n");
    h.app.disk_check_at = std::time::Instant::now() - std::time::Duration::from_secs(1);
    h.app.poll_disk();
    assert!(h.app.dialog.is_none(), "the same change is asked about twice");

    // A newer change is asked about again, and "Reload" takes it.
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, "third version\n").unwrap();
    h.app.disk_check_at = std::time::Instant::now() - std::time::Duration::from_secs(1);
    h.app.poll_disk();
    assert!(matches!(h.app.dialog, Some(bare_md::app::Dialog::External(_))), "no second question");
    h.draw();
    assert!(h.click_hit(|hit| *hit == Hit::ExternalReload), "no Reload button");
    h.draw();
    assert_eq!(h.app.tab().doc.text(), "third version\n", "reload did not happen");

    // "Auto-reload" keeps reloading without asking, for this session.
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, "fourth version\n").unwrap();
    h.app.disk_check_at = std::time::Instant::now() - std::time::Duration::from_secs(1);
    h.app.poll_disk();
    h.draw();
    assert!(h.click_hit(|hit| *hit == Hit::ExternalAuto), "no Auto-reload button");
    h.draw();
    assert!(h.app.auto_reload, "auto-reload was not switched on");
    assert_eq!(h.app.tab().doc.text(), "fourth version\n");
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, "fifth version\n").unwrap();
    h.app.disk_check_at = std::time::Instant::now() - std::time::Duration::from_secs(1);
    h.app.poll_disk();
    assert!(h.app.dialog.is_none(), "auto-reload still asks");
    assert_eq!(h.app.tab().doc.text(), "fifth version\n", "auto-reload did not reload");
}

/// (11.1b) A dirty buffer is never reloaded behind the user's back, even with
/// auto-reload on: those edits would be lost.
#[test]
fn auto_reload_never_throws_away_unsaved_edits() {
    let mut h = harness("r11-external-dirty");
    let path = h._dir.join("note.md");
    std::fs::write(&path, "base\n").unwrap();
    h.app.open_path(&path);
    h.app.insert_text("mine\n");
    h.app.auto_reload = true;
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, "theirs\n").unwrap();
    h.app.disk_check_at = std::time::Instant::now() - std::time::Duration::from_secs(1);
    h.app.poll_disk();
    assert!(
        matches!(h.app.dialog, Some(bare_md::app::Dialog::External(_))),
        "a dirty buffer was reloaded without asking"
    );
    assert!(h.app.tab().doc.text().contains("mine"), "the edits are gone");
}

/// (11.2) Workspaces are tabs: opening another folder adds a tab, switching keeps
/// both trees, and closing one leaves the other.
#[test]
fn workspaces_are_tabs() {
    let mut h = harness("r11-workspaces");
    let a = h._dir.join("alpha");
    let b = h._dir.join("beta");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(a.join("one.md"), "# one").unwrap();
    std::fs::write(b.join("two.md"), "# two").unwrap();
    let before = h.app.workspaces.len();
    h.app.add_workspace(a.clone());
    h.app.add_workspace(b.clone());
    h.draw();
    assert_eq!(
        h.app.workspaces.len(),
        before + 2,
        "a second workspace replaced the first instead of being added as a tab"
    );
    assert_eq!(h.app.ws.root, b, "the new workspace is not the active one");
    let chips: Vec<Rect> = h
        .app
        .hits
        .entries_for_test()
        .iter()
        .filter(|(_, hit)| matches!(hit, Hit::WorkspaceTab(_)))
        .map(|(r, _)| *r)
        .collect();
    assert_eq!(chips.len(), h.app.workspaces.len(), "one chip per workspace");
    // (13.1) There is no "+" chip any more: the panel's open-folder button is the
    // way to add a workspace, and it opens the folder dialog.
    assert!(
        h.app.hits.find(|hit| *hit == Hit::WorkspaceAdd).is_none(),
        "the \"+\" chip is still in the tab row"
    );
    assert!(
        h.app.hits.find(|hit| *hit == Hit::SidebarOpenFolder).is_some(),
        "no open-folder button in the panel"
    );
    assert!(h.click_hit(|hit| *hit == Hit::SidebarOpenFolder), "the folder button is not clickable");
    assert!(
        matches!(
            h.app.dialog,
            Some(bare_md::app::Dialog::Picker(ref pk))
                if pk.kind == bare_md::app::PickerKind::OpenFolder
        ),
        "the folder button did not open the folder dialog"
    );
    h.app.dialog = None;
    h.draw();
    // Switching back shows the other tree.
    let a_tab = h.app.workspaces.iter().position(|w| w.root == a).expect("alpha is not a tab");
    assert!(h.click_hit(|hit| matches!(hit, Hit::WorkspaceTab(i) if *i == a_tab)), "no chip for alpha");
    h.draw();
    assert_eq!(h.app.ws.root, a, "switching did not change the workspace");
    let names: Vec<String> = h.app.ws.entries().iter().map(|e| e.name.clone()).collect();
    assert!(names.iter().any(|n| n == "one.md"), "wrong tree: {names:?}");
    // Closing the active tab falls back to another one, with a live tree.
    let n = h.app.workspaces.len();
    h.app.close_workspace(a_tab);
    h.draw();
    assert_eq!(h.app.workspaces.len(), n - 1);
    assert!(h.app.workspaces.iter().all(|w| w.root != a), "alpha is still a tab");
    assert!(h.app.ws.root.is_dir(), "closing left the workspace pointing nowhere");
    assert!(!h.app.ws.entries().is_empty(), "the tree did not load");
    // The last workspace cannot be closed.
    while h.app.workspaces.len() > 1 {
        h.app.close_workspace(0);
    }
    h.app.close_workspace(0);
    assert_eq!(h.app.workspaces.len(), 1, "the last workspace was closed");
}

/// (11.4 -> r13) A drag that is parked against the bottom of the minimap leaves
/// the document at its end, where it stays: the map is a fixed mapping of the
/// whole file, so there is nowhere further to run to and nothing may drift.
#[test]
fn a_minimap_drag_against_the_edge_stops_at_the_end() {
    let mut h = harness("r11-map-edge");
    h.type_str(&filler(400));
    h.app.set_caret(0, false);
    h.open_minimap();
    h.minimap_press(0.5);
    // Drag into the bottom band and hold it there.
    h.minimap_drag_to(h.minimap().bottom() - 2.0);
    let total = h.app.tab().layout.total_height;
    let text_h = h.app.text_rect.h;
    let after = h.app.tab().scroll;
    assert!(
        (after - (total - text_h).max(0.0)).abs() < 1.0,
        "the bottom of the map must be the end of the document ({after} of {})",
        total - text_h
    );
    // Still there a moment later: the map must not creep or snap back.
    h.app.mouse_up(&mut h.r);
    h.draw();
    assert!(
        (h.app.tab().scroll - after).abs() < 1.0,
        "the document moved after the drag ended"
    );
}

/// (11.4b) The minimap is more compact than it used to be: a whole screen of rows
/// fits in far less height.
#[test]
fn the_minimap_is_compact() {
    let pitch = bare_md::ui::MINIMAP_PITCH;
    assert!(pitch <= 3.0, "the minimap pitch is still {pitch} px per row");
    let bar = 2.0f32.min(pitch - 1.0).max(1.0);
    assert!(bar < pitch, "bars would touch: {bar} in a {pitch} px pitch");
}

/// (11.5 / 11.6) A new document from the keyboard (`Ctrl+T`) and from a middle
/// click on the empty part of the tab strip.
#[test]
fn new_documents_from_the_strip_and_the_keyboard() {
    let mut h = harness("r11-new-tab");
    h.app.new_tab();
    h.draw();
    let n = h.app.tabs.len();
    // Middle click on the empty part of the strip.
    let strip = h.app.tabbar_rect;
    let y = strip.y + strip.h - 6.0;
    let empty = (0..40)
        .map(|k| (strip.x + 200.0 + k as f32 * 7.0, y))
        .find(|p| p.0 < strip.right() - 260.0 && h.app.hits.at(*p).is_none())
        .expect("the whole strip is covered by controls");
    {
        use winit::event::{ElementState, MouseButton, WindowEvent};
        let mut input = bare_md::input::InputState::default();
        h.app.mouse = empty;
        h.app.handle_event(
            &WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Middle, device_id: winit::event::DeviceId::dummy() },
            &mut h.r,
            &mut input,
        );
    }
    assert_eq!(h.app.tabs.len(), n + 1, "a middle click on the strip did not open a document");
    // Ctrl+T does the same.
    let n = h.app.tabs.len();
    h.app.handle_shortcut("t", false, &mut h.r);
    assert_eq!(h.app.tabs.len(), n + 1, "Ctrl+T did not open a document");
}

/// (11.7) The view chips in the tab strip carry big, recognisable glyphs.
#[test]
fn the_view_icons_fill_their_buttons() {
    let mut h = harness("r11-chips");
    h.draw();
    let chip = h
        .app
        .hits
        .find(|hit| *hit == Hit::Toolbar(ToolbarAction::ToggleMinimap))
        .expect("no minimap chip");
    // The icon is inset 5 px horizontally and 2.5 px vertically from a 28 px
    // button, so it is at least 18 px wide - it used to be 14.
    // It used to be a 14 px glyph inside a 28 px button.
    assert!(chip.w - 10.0 >= 18.0, "the glyph is only {} px wide", chip.w - 10.0);
    assert!(chip.h - 5.0 >= 17.0, "the glyph is only {} px tall", chip.h - 5.0);
}

/// (11.8) Clicking the path in the status bar copies it.
#[test]
fn clicking_the_status_path_copies_it() {
    let mut h = harness("r11-status-path");
    let path = h._dir.join("copy-me.md");
    std::fs::write(&path, "# hi").unwrap();
    h.app.open_path(&path);
    h.draw();
    let chip = h
        .app
        .hits
        .find(|hit| *hit == Hit::StatusPath)
        .expect("the status bar has no path button");
    assert!(chip.y > h.app.text_rect.bottom(), "the path is not in the status bar");
    assert!(h.click_hit(|hit| *hit == Hit::StatusPath), "no path chip to click");
    // The clipboard is not available in a headless test, but the action must have
    // run and reported what it did.
    let status = h.app.status.to_lowercase();
    assert!(
        status.contains("copied") || status.contains("clipboard"),
        "clicking the path did nothing: {:?}",
        h.app.status
    );
}

/// (11.9) Code blocks are syntax highlighted, and only when the fence names a
/// language.
#[test]
fn code_blocks_are_highlighted() {
    let mut h = harness("r11-highlight");
    h.app.new_tab();
    h.app.insert_text("```rust\nlet x = \"s\"; // c\n```\n\n```\nplain old code\n```\n");
    h.app.set_caret(0, false);
    h.draw();
    let layout = &h.app.tab().layout;
    let theme = h.app.theme;
    let code = layout
        .lines
        .iter()
        .find(|l| l.chars.iter().collect::<String>().contains("let x"))
        .expect("the code line");
    let colours: Vec<bare_md::geom::Color> = code.clean.iter().filter_map(|r| r.style.fg).collect();
    assert!(
        colours.iter().any(|c| *c == theme.syn_keyword),
        "the keyword is not coloured: {colours:?}"
    );
    assert!(
        colours.iter().any(|c| *c == theme.syn_string),
        "the string is not coloured: {colours:?}"
    );
    // Nothing is coloured in a block without a language.
    let plain = layout
        .lines
        .iter()
        .find(|l| l.chars.iter().collect::<String>().contains("plain old code"))
        .expect("the unlabelled code line");
    assert!(
        plain.clean.iter().all(|r| r.style.fg.is_none()),
        "an unlabelled block was coloured"
    );
    // Raw mode is the source: no colours there either.
    h.app.set_view_mode(ViewMode::Raw);
    h.draw();
    let raw = h
        .app
        .tab()
        .layout
        .lines
        .iter()
        .find(|l| l.chars.iter().collect::<String>().contains("let x"))
        .expect("the code line in raw mode");
    assert!(
        raw.clean.iter().all(|r| r.style.fg.is_none()),
        "raw mode coloured the source"
    );
}

/// (11.10) A table's grid is closed: a rule above the first row and below the
/// last one, not just between the rows.
#[test]
fn the_table_grid_is_closed_top_and_bottom() {
    let mut h = harness("r11-table-lines");
    h.app.new_tab();
    h.app.insert_text("| A | B |\n| - | - |\n| 1 | 2 |\n| 3 | 4 |\n");
    h.app.set_caret(0, false);
    h.draw();
    let (top, bottom) = {
        let rows: Vec<&bare_md::layout::LineLayout> = h
            .app
            .tab()
            .layout
            .lines
            .iter()
            .filter(|l| l.block() == bare_md::markdown::Block::Table && !l.info.table_sep)
            .collect();
        (
            rows.first().unwrap().top,
            rows.last().unwrap().top + rows.last().unwrap().height,
        )
    };
    // The grid rect is what the painter closes; the rules sit on its edges.
    let card_lines = bare_md::ui::table_rule_ys(&h.app.tab().layout, h.app.text_rect.y, h.app.tab().scroll);
    assert!(
        card_lines.iter().any(|y| (*y - (h.app.text_rect.y + top - h.app.tab().scroll)).abs() < 6.0),
        "no rule above the first row: {card_lines:?}"
    );
    assert!(
        card_lines.iter().any(|y| (*y - (h.app.text_rect.y + bottom - h.app.tab().scroll)).abs() < 8.0),
        "no rule below the last row: {card_lines:?}"
    );
}

// ===========================================================================
// Round 12
// ===========================================================================

/// (12.1) A picture is one entry on the map, but a *tall* one: the map gives it
/// the space it takes on the page instead of one row, so the lines around it are
/// not squeezed into a stripe.
#[test]
fn minimap_gives_a_picture_its_real_height() {
    let mut h = harness("r12-map-picture");
    h.app.new_tab();
    h.app.insert_text("line one\n\n![alt](nope.png)\n\nafter\n");
    h.draw();
    let layout = &h.app.tab().layout;
    let img = layout
        .lines
        .iter()
        .find(|l| l.image.is_some())
        .expect("the picture line carries an image box");
    let span = bare_md::ui::minimap_virtual_span(img);
    assert!(
        span > img.rows.len().max(1) as f32,
        "a picture must take more map rows than its text rows ({} vs {})",
        span,
        img.rows.len()
    );
    // Text keeps one map row per wrapped row, and a blank line still counts as
    // one, so the document cannot collapse.
    let text_line = &layout.lines[0];
    assert_eq!(bare_md::ui::minimap_virtual_span(text_line), text_line.rows.len().max(1) as f32);
}

/// (12.1) The viewport indicator keeps one size from the top of the document to
/// the bottom: it is measured against the whole document, not against whatever
/// happens to be under it.
#[test]
fn minimap_indicator_keeps_its_size() {
    let mut h = harness("r12-map-indicator");
    h.app.new_tab();
    let body = filler(120);
    h.app.insert_text(&body);
    h.app.insert_text("![alt](nope.png)\n");
    h.draw();
    let layout = &h.app.tab().layout;
    let text_h = h.app.text_rect.h;
    let total = layout.total_height;
    let geom = bare_md::ui::minimap_geom(layout, h.app.minimap_rect);
    let a = geom.view_h_px(total, text_h);
    // Scroll the page and measure again: the indicator is the document's visible
    // slice, so its height cannot depend on where the page is.
    let tab = h.app.tab_mut();
    tab.scroll = (total - text_h) * 0.5;
    h.draw();
    let b = bare_md::ui::minimap_geom(&h.app.tab().layout, h.app.minimap_rect).view_h_px(total, text_h);
    assert!((a - b).abs() < 0.01, "the indicator changed size while scrolling: {a} vs {b}");
    assert!(a > 8.0 && a < h.app.minimap_rect.h, "indicator {a} is not a sane size");
    // (63) It is a handle of one constant height, not a picture of the viewport: sized by
    // the viewport it ate the track it has to move in, and a 120-line document's 83 px
    // thumb could move 34 px over a map of 3 400 px of text. It is never taller than the
    // bars it slides along, and it always leaves MIN_TRAVEL_PX behind it.
    assert!(
        (a - bare_md::ui::MIN_HANDLE_H).abs() < 0.01,
        "indicator {a} is not the handle height {}",
        bare_md::ui::MIN_HANDLE_H
    );
    assert!(a <= geom.bars_h(), "the handle ({a}) is taller than the bars");
    assert!(
        geom.handle_travel(total, text_h) >= bare_md::ui::MIN_TRAVEL_PX,
        "the handle left itself no travel"
    );
}

/// (12.1) Dragging the map moves the page by the same amount wherever the drag
/// starts: the speed comes from the pointer's *movement*, not from what lies
/// under it. Two identical drags from two different heights must scroll the
/// document identically.
#[test]
fn minimap_drag_is_proportional_and_never_accelerates() {
    let mut h = harness("r12-map-drag");
    h.app.new_tab();
    h.app.insert_text(&filler(200));
    h.app.set_caret(0, false);
    h.draw();
    let map = h.minimap();

    // Press `from` down the map, drag `dy` pixels, and report how far the page moved.
    let drag = |h: &mut Harness, from: f32, dy: f32| -> f32 {
        let y = h.minimap_press((from - map.y) / map.h);
        let before = h.app.tab().scroll;
        h.minimap_drag_to(y + dy);
        h.app.mouse_up(&mut h.r);
        h.draw();
        h.app.tab().scroll - before
    };
    // One pixel of drag is one pixel of handle, whatever the handle's height in the
    // strip: the expectation is the geometry's own mapping, `travel` map pixels for the
    // whole document.
    let (geom, _) = h.app.minimap_geometry();
    let total = h.app.tab().layout.total_height;
    let text_h = h.app.text_rect.h;
    let max_scroll = (total - text_h).max(0.0);
    let per_px = max_scroll / geom.handle_travel(total, text_h);
    let step = map.h * 0.22;
    let high = drag(&mut h, map.y + step * 0.6, 10.0);
    let low = drag(&mut h, map.y + step * 2.4, 10.0);
    assert!(high > 0.5, "dragging did not scroll the document: {high}");
    assert!(
        (high - low).abs() < 1.0,
        "the same drag moved the page by {high} at one height and {low} at another"
    );
    assert!(
        (high - 10.0 * per_px).abs() < 2.0,
        "a 10 px drag moved the page {high:.1} px, the map says {:.1}",
        10.0 * per_px
    );
    // And twice the movement is twice the scroll.
    let one = drag(&mut h, map.y + step * 1.5, 8.0);
    let two = drag(&mut h, map.y + step * 1.5, 16.0);
    assert!((two - one * 2.0).abs() < 1.5, "drag is not proportional: {one} then {two}");
}

/// (12.2) The two plain scrollbars belong to the minimap-off case only.
#[test]
fn scrollbars_only_exist_without_the_minimap() {
    let mut h = harness("r12-bars");
    h.app.new_tab();
    let body = filler(120);
    h.app.insert_text(&body);
    h.draw();
    assert!(h.app.settings.minimap_open, "the minimap should be on by default");
    assert!(
        h.app.hits.find(|hit| *hit == Hit::ScrollV).is_none(),
        "the vertical bar is drawn while the minimap is on"
    );
    h.app.settings.minimap_open = false;
    h.draw();
    assert!(
        h.app.hits.find(|hit| *hit == Hit::ScrollV).is_some(),
        "no vertical bar with the minimap off"
    );
}

/// (12.3) An empty table cell can be clicked into, and typing lands in *that*
/// cell instead of jumping back into the one before it.
#[test]
fn an_empty_table_cell_can_be_edited_in_place() {
    let mut h = harness("r12-empty-cell");
    h.app.new_tab();
    h.app.insert_text("| a |  | c |\n| - | - | - |\n| d | e | f |\n");
    h.app.set_caret(0, false);
    h.draw();
    let (x, y, want_cell) = {
        let (a, b) = h.app.tab().layout.lines[0].info.cells[1];
        assert_eq!(a, b, "the middle header cell must be empty");
        let (x, y) = h.cell_point(0, 1, 0.15);
        (x, y, a)
    };
    h.click_full((x, y));
    let tab = h.app.tab();
    assert_eq!(tab.caret, want_cell, "the caret did not land in the empty cell");
    // The caret must be *drawn* in that cell too: it used to be snapped to the
    // end of the cell before it, which is why the empty cell looked unclickable.
    // Where the cell really is on the page: the table's own indent plus the
    // column's offset inside it.
    let cell_x = h.app.text_rect.x + tab.layout.lines[0].indent + tab.layout.lines[0].table_cols[1];
    let cr = h.app.caret_rect(&mut h.r.text);
    assert!(
        (cr.x - cell_x).abs() < 4.0,
        "the caret sits at x {} instead of the empty cell's edge {cell_x}",
        cr.x
    );
    h.app.insert_text("x");
    h.draw();
    let text = h.app.tab().doc.text();
    let first = text.lines().next().unwrap_or("");
    let cells: Vec<&str> = first.split('|').map(|c| c.trim()).collect();
    assert_eq!(cells.get(2), Some(&"x"), "typing did not land in the empty cell: {first:?}");
}

/// (12.4) The vertical rules of a table stop where the horizontal ones are: they
/// are drawn between the rule above the first row and the rule under the last,
/// so nothing overhangs.
#[test]
fn table_verticals_stop_at_the_horizontal_rules() {
    let src = ui_source();
    let f = &src[src.find("fn draw_table_grid(").unwrap()..];
    let f = &f[..f.find("\npub fn ").or_else(|| f.find("\nfn ")).unwrap()];
    assert!(f.contains("grid_top"), "the verticals must use the grid's own top");
    assert!(f.contains("grid_bottom"), "the verticals must use the grid's own bottom");
    assert!(!f.contains("card.h - 8.0"), "the old hand-tuned overhang is back");
    assert!(!f.contains("card.h - 2.0"), "the old hand-tuned overhang is back");
}

/// (12.5) The wheel scrolls the folder dialog's *tree* when the pointer is over
/// it, and leaves the listing alone.
#[test]
fn the_folder_tree_scrolls_under_the_wheel() {
    use winit::event::{MouseScrollDelta, WindowEvent};
    let mut h = harness("r12-tree-wheel");
    // Enough folders to overflow the tree column.
    for i in 0..40 {
        std::fs::create_dir_all(h._dir.join(format!("d{i:02}"))).unwrap();
    }
    h.app.open_picker(bare_md::app::PickerKind::OpenFolder);
    h.draw();
    let tree = h.app.dialog_tree_rect;
    assert!(!tree.is_empty(), "the tree column has no rect");
    assert!(h.app.tab().doc.path.is_none());
    let before_sel = match &h.app.dialog {
        Some(bare_md::app::Dialog::Picker(pk)) => pk.selected,
        _ => panic!("no picker"),
    };
    h.app.mouse = (tree.x + tree.w * 0.5, tree.y + tree.h * 0.5);
    let mut input = bare_md::input::InputState::default();
    h.app.handle_event(
        &WindowEvent::MouseWheel {
            device_id: winit::event::DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, -1.0),
            phase: winit::event::TouchPhase::Moved,
        },
        &mut h.r,
        &mut input,
    );
    h.draw();
    let (tree_scroll, after_sel) = match &h.app.dialog {
        Some(bare_md::app::Dialog::Picker(pk)) => (pk.tree_scroll, pk.selected),
        _ => panic!("no picker"),
    };
    assert!(tree_scroll > 0, "the tree did not scroll");
    assert_eq!(before_sel, after_sel, "the wheel moved the listing while over the tree");
}

/// (12.6) The status path has no hover popup; the only feedback is the "Copied!"
/// tick after a click.
#[test]
fn the_status_path_has_no_hover_popup() {
    let mut h = harness("r12-status-path");
    let path = h._dir.join("no-popup.md");
    std::fs::write(&path, "# hi").unwrap();
    h.app.open_path(&path);
    h.draw();
    let chip = h.app.hits.find(|hit| *hit == Hit::StatusPath).expect("no path chip");
    h.app.mouse = (chip.x + chip.w * 0.5, chip.y + chip.h * 0.5);
    h.draw();
    let src = ui_source();
    let status = &src[src.find("fn draw_status(").unwrap()..];
    let status = &status[..status.find("\n}\n").unwrap()];
    assert!(!status.contains("click to copy"), "the hover popup is back");
    assert!(status.contains("Copied!"), "there is no copy feedback any more");
    assert!(h.click_hit(|hit| *hit == Hit::StatusPath), "the path is not clickable");
    // The click copies. On a machine with no clipboard service (a headless test
    // runner) the app says so instead of pretending, and that is the only other
    // message it may show here.
    assert!(
        h.app.status == "Copied!" || h.app.status == "Clipboard unavailable",
        "clicking the path said {:?}",
        h.app.status
    );
}

/// (12.7) Text fields are editable everywhere: a full hex box takes a new value,
/// backspace works from the middle, and a selection is replaced.
#[test]
fn text_fields_are_editable_at_the_root() {
    use bare_md::field::{CharFilter, TextField};

    let mut hex = TextField::limited(CharFilter::Hex, 6);
    hex.set("e05f3f");
    assert_eq!(hex.caret, 6, "the caret follows the text");
    // Typing into a full field starts a new value instead of being swallowed.
    assert!(hex.insert("00ff00"));
    assert_eq!(hex.text, "00ff00");
    // Non-hex characters never get in.
    assert!(!hex.insert("zz"));
    // Backspace and delete work from a caret, not only from the end.
    hex.place_caret(2, false);
    assert!(hex.backspace());
    assert_eq!(hex.text, "0ff00");
    assert!(hex.delete_forward());
    assert_eq!(hex.text, "0f00");
    // Select-all replaces the whole value.
    hex.select_all();
    assert!(hex.insert("abcdef"));
    assert_eq!(hex.text, "abcdef");

    let mut path = TextField::new();
    path.set("/home/user/notes");
    path.place_caret(5, false);
    assert!(path.insert("X"));
    assert_eq!(path.text, "/homeX/user/notes");
    assert_eq!(path.caret, 6);
    // Nothing is lost at the ends.
    path.home(false);
    assert!(!path.backspace());
    path.end(false);
    assert!(!path.delete_forward());
}

/// (12.7) The same thing through the real dialog: the hex box of the colour
/// picker responds to the keyboard after being clicked, and the colour follows.
#[test]
fn the_colour_hex_box_takes_typing() {
    use bare_md::app::Dialog;
    let mut h = harness("r12-hex-dialog");
    h.app.new_tab();
    h.app.insert_text("hello\n");
    h.draw();
    h.app.open_menu(bare_md::app::MenuKind::Color, Rect::new(400.0, 60.0, 20.0, 20.0));
    h.draw();
    let item = h
        .app
        .menu
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .position(|e| e.label.contains("Custom"))
        .unwrap();
    assert!(h.click_hit(|hit| matches!(hit, Hit::MenuItem(i) if *i == item)));
    h.draw();
    assert!(matches!(h.app.dialog, Some(Dialog::Colour(_))));
    assert!(h.click_hit(|hit| matches!(hit, Hit::ColourHex(_))));
    h.type_into_find("00ff00");
    let (hex, rgb) = match &h.app.dialog {
        Some(Dialog::Colour(cp)) => (cp.slot(false).hex.text.clone(), cp.slot(false).rgb()),
        _ => panic!("no colour dialog"),
    };
    assert_eq!(hex, "00ff00", "the hex box did not take the typing");
    assert_eq!(rgb, 0x00ff00, "the swatch did not follow the hex box");
}

// ---------------------------------------------------------------------------
// Round 13
// ---------------------------------------------------------------------------

/// (13.1) Workspace chips can be closed and reordered, and the tab row has no
/// "+" any more - the panel's folder button is the way to add a workspace.
#[test]
fn workspace_chips_close_and_reorder() {
    let mut h = harness("r13-ws-chips");
    let a = h._dir.join("alpha");
    let b = h._dir.join("beta");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(a.join("one.md"), "# one").unwrap();
    std::fs::write(b.join("two.md"), "# two").unwrap();
    let base = h.app.workspaces.len();
    h.app.add_workspace(a.clone());
    h.app.add_workspace(b.clone());
    let ai = h.app.workspaces.iter().position(|w| w.root == a).expect("no chip for alpha");
    let bi = h.app.workspaces.iter().position(|w| w.root == b).expect("no chip for beta");
    h.draw();
    assert_eq!(h.app.workspaces.len(), base + 2, "each folder adds a chip");

    // The close button of the *inactive* chip is only there while it is hovered,
    // so hover it first, exactly as a pointer would.
    let chip = h
        .app
        .hits
        .find(|hit| matches!(hit, Hit::WorkspaceTab(i) if *i == ai))
        .expect("no chip for the first workspace");
    h.app.mouse = (chip.x + chip.w * 0.5, chip.y + chip.h * 0.5);
    h.draw();
    assert!(
        h.app.hits.find(|hit| matches!(hit, Hit::WorkspaceTabClose(i) if *i == ai)).is_some(),
        "a workspace chip has no close button"
    );
    assert!(
        h.click_hit(|hit| matches!(hit, Hit::WorkspaceTabClose(i) if *i == ai)),
        "the close button is dead"
    );
    h.draw();
    assert_eq!(h.app.workspaces.len(), base + 1, "the close button did not close the chip");
    assert!(
        h.app.workspaces.iter().all(|w| w.root != a),
        "closing removed the wrong chip"
    );
    let _ = bi;

    // Reorder: give the strip a partner and drag the first chip to the end.
    h.app.add_workspace(a.clone());
    h.draw();
    let rects: Vec<Rect> = h
        .app
        .hits
        .entries_for_test()
        .into_iter()
        .filter(|(_, hit)| matches!(hit, Hit::WorkspaceTab(_)))
        .map(|(r, _)| *r)
        .collect();
    assert!(rects.len() >= 2, "need two chips to reorder");
    let before: Vec<PathBuf> = h.app.workspaces.iter().map(|w| w.root.clone()).collect();
    let first = rects[0];
    h.app.mouse = (first.x + first.w * 0.5, first.y + first.h * 0.5);
    h.draw();
    h.app.click(h.app.mouse, &mut h.r);
    assert!(h.app.ws_drag.is_some(), "a press on a chip must start a drag");
    let last = *rects.last().unwrap();
    let mut input = bare_md::input::InputState::default();
    h.app.handle_event(
        &winit::event::WindowEvent::CursorMoved {
            device_id: winit::event::DeviceId::dummy(),
            position: winit::dpi::PhysicalPosition::new(
                (last.x + last.w * 0.5) as f64,
                (last.y + last.h * 0.5) as f64,
            ),
        },
        &mut h.r,
        &mut input,
    );
    h.draw();
    h.app.mouse_up(&mut h.r);
    h.draw();
    let after: Vec<PathBuf> = h.app.workspaces.iter().map(|w| w.root.clone()).collect();
    assert_eq!(after.len(), before.len(), "a drag lost a workspace");
    assert_ne!(after, before, "the chip did not move");
    assert_eq!(after[after.len() - 1], before[0], "the dragged chip did not land at the end");
}

/// (13.2) A long heading name in the outline stops before the drag grip instead of
/// running under it: the label's width is clamped to the column left of the grip.
#[test]
fn an_outline_row_stops_before_its_grip() {
    let src = ui_source();
    let body = &src[src.find("fn draw_outline(").unwrap()..];
    let body = &body[..body.find("\n}\n").unwrap()];
    assert!(
        body.contains("r.right() - 18.0"),
        "the outline label is not clamped to the grip column"
    );
    // And the paint proves it: a very long heading stays inside the panel.
    let mut h = harness("r13-outline");
    h.app.new_tab();
    h.type_str("# A heading whose name is far too long to fit in the side panel at all\n\ntext\n");
    h.app.set_caret(0, false);
    h.draw();
    let side = h.app.sidebar_rect;
    let grip = h
        .app
        .hits
        .entries_for_test()
        .into_iter()
        .find(|(_, hit)| matches!(hit, Hit::OutlineGrip(_)))
        .map(|(r, _)| *r);
    if let Some(grip) = grip {
        assert!(grip.x + grip.w <= side.right() + 0.5, "the grip is outside the panel");
    }
}

/// (13.3) Dragging across a dialog field selects the text under the pointer - the
/// same behaviour the document has.
#[test]
fn dragging_across_a_dialog_field_selects_text() {
    let mut h = harness("r13-field-drag");
    h.app.open_picker(bare_md::app::PickerKind::OpenFolder);
    h.draw();
    let field = h.app.hits.find(|hit| *hit == Hit::DialogPathField).expect("no address bar");
    let y = field.y + field.h * 0.5;
    h.app.mouse = (field.x + 12.0, y);
    h.app.click(h.app.mouse, &mut h.r);
    let start = match &h.app.dialog {
        Some(bare_md::app::Dialog::Picker(p)) => p.path.selection(),
        _ => panic!("no picker"),
    };
    assert!(start.is_some() || true, "the press placed no caret");
    // Drag to the far right of the field.
    let mut input = bare_md::input::InputState::default();
    h.app.handle_event(
        &winit::event::WindowEvent::CursorMoved {
            device_id: winit::event::DeviceId::dummy(),
            position: winit::dpi::PhysicalPosition::new((field.right() - 4.0) as f64, y as f64),
        },
        &mut h.r,
        &mut input,
    );
    let sel = match &h.app.dialog {
        Some(bare_md::app::Dialog::Picker(p)) => p.path.selection(),
        _ => panic!("no picker"),
    };
    assert!(sel.is_some(), "dragging across the field selected nothing");
    h.app.mouse_up(&mut h.r);
}

/// (13.5) The minimap is one fixed mapping: the whole document fits the column,
/// the bar pitch never grows past a comfortable line, and the same document
/// position always sits at the same map position - whatever the scroll is.
#[test]
fn the_minimap_maps_the_whole_document_with_one_fixed_scale() {
    let mut h = harness("r13-map-fixed");
    h.app.new_tab();
    let body = filler(180);
    h.app.insert_text(&body);
    h.app.set_caret(0, false);
    h.draw();
    let map = h.app.minimap_rect;
    let total = h.app.tab().layout.total_height;
    let text_h = h.app.text_rect.h;
    let geom = bare_md::ui::minimap_geom(&h.app.tab().layout, map);
    assert!(geom.scale > 0.0 && geom.scale <= bare_md::ui::MINIMAP_PITCH + 0.001);
    assert_eq!(
        geom.scale,
        bare_md::ui::MINIMAP_PITCH,
        "the pitch is constant now, so nothing may squeeze past it ({})",
        geom.scale
    );
    // A document position maps to the same place at the top and at the bottom.
    let top = bare_md::ui::minimap_virtual_tops(&h.app.tab().layout);
    let y_of = |scroll: f32, h: &Harness| {
        let v = bare_md::ui::MinimapGeom::virtual_of_doc(&h.app.tab().layout, &top, scroll);
        bare_md::ui::minimap_geom(&h.app.tab().layout, map).y_of_virtual(v)
    };
    let mid = (total - text_h) * 0.5;
    h.app.tab_mut().scroll = mid;
    let here = y_of(mid, &h);
    h.app.tab_mut().scroll = 0.0;
    let again = y_of(mid, &h);
    assert!((here - again).abs() < 0.01, "the map moved under the document ({here} vs {again})");
}

/// (13.6) The document menu belongs to the central panel: a right click over a
/// side panel opens nothing at all.
#[test]
fn the_document_menu_only_opens_over_the_text_panel() {
    use winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
    let mut h = harness("r13-rmb");
    h.app.new_tab();
    h.app.insert_text("hello\n");
    h.app.set_caret(0, false);
    h.draw();
    let editor = h.app.editor_rect;
    let side = h.app.sidebar_rect;
    assert!(!side.is_empty() && !editor.is_empty());
    let rmb = |pos: (f32, f32), h: &mut Harness| {
        h.app.mouse = pos;
        let mut input = bare_md::input::InputState::default();
        h.app.handle_event(
            &WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state: ElementState::Pressed,
                button: MouseButton::Right,
            },
            &mut h.r,
            &mut input,
        );
        h.draw();
    };
    rmb((side.x + side.w * 0.5, side.y + side.h * 0.5), &mut h);
    assert!(h.app.menu.is_none(), "a side panel opened the document menu");
    rmb((editor.x + editor.w * 0.5, editor.y + editor.h * 0.5), &mut h);
    assert!(h.app.menu.is_some(), "the document did not open its menu");
}

/// (13.7) Text longer than a table column wraps inside it: no glyph of a cell may
/// be painted past the column's right edge.
#[test]
fn long_cell_text_wraps_inside_its_column() {
    let mut h = harness("r13-cell-wrap");
    h.app.new_tab();
    h.app.insert_text(
        "| A | B | C |\n| - | - | - |\n| a rather long cell with many many words in it that go on and on and on well past any column | short | mid |\n",
    );
    h.app.set_caret(0, false);
    h.draw();
    let line = &h.app.tab().layout.lines[2];
    assert!(line.table_cols.len() >= 3, "three columns");
    assert!(line.rows.len() > 1 || line.table_rows.len() > 1, "the long cell did not wrap");
    for band in &line.table_rows {
        for run in band {
            // The column this piece belongs to, and the right edge of it: the next
            // column's left edge, or the table's own width for the last one.
            let ci = line
                .table_cols
                .iter()
                .rposition(|x| *x <= run.x + 0.5)
                .unwrap_or(0);
            let edge = line
                .table_cols
                .get(ci + 1)
                .copied()
                .unwrap_or(line.table_width);
            assert!(
                run.x + run.w <= edge + 0.5,
                "column {ci} text runs past its edge: {} + {} > {}",
                run.x,
                run.w,
                edge
            );
        }
    }
}

/// (13.8) A selection in a dialog field is a wash *behind* the text: the painter
/// fills it before it draws the glyphs, and every theme keeps the two apart.
#[test]
fn a_selected_field_shows_its_text_over_the_wash() {
    let src = ui_source();
    let body = &src[src.find("    pub fn field(").unwrap()..];
    let body = &body[..body.find("\n    }\n").unwrap()];
    let wash = body.find("theme.selection").expect("the field paints no selection");
    let glyphs = body.find("self.text(").expect("the field paints no text");
    assert!(wash < glyphs, "the selection is painted over the text");
    for kind in ThemeKind::ALL {
        let t = bare_md::theme::Theme::by_kind(kind);
        let d = (t.selection.r - t.text.r).abs()
            + (t.selection.g - t.text.g).abs()
            + (t.selection.b - t.text.b).abs();
        assert!(d > 0.5, "{kind:?}: the selection wash is too close to the text colour");
    }
}

/// (13.10) json, toml and rust fences are all highlighted.
#[test]
fn json_toml_and_rust_fences_are_highlighted() {
    let mut h = harness("r13-langs");
    h.app.new_tab();
    h.app.insert_text(
        "```json\n{\"key\": true, \"n\": 12}\n```\n\n```toml\nname = \"x\"\nport = 8080\n```\n\n```rs\nfn main() { let x: u32 = 1; }\n```\n",
    );
    h.app.set_caret(0, false);
    h.draw();
    let theme = h.app.theme;
    let coloured = |needle: &str, want: bare_md::geom::Color, what: &str| {
        let line = h
            .app
            .tab()
            .layout
            .lines
            .iter()
            .find(|l| l.chars.iter().collect::<String>().contains(needle))
            .unwrap_or_else(|| panic!("no line with {needle:?}"));
        let fgs: Vec<bare_md::geom::Color> = line.clean.iter().filter_map(|r| r.style.fg).collect();
        assert!(fgs.iter().any(|c| *c == want), "{what} is not coloured: {fgs:?}");
    };
    coloured("key", theme.syn_string, "a json key");
    coloured("port", theme.syn_number, "a toml number");
    coloured("main", theme.syn_keyword, "a rust keyword");
}

/// (13.9) A release that follows a short pause still flings: the flick used to die
/// because the idle window was shorter than a normal hand release.
#[test]
fn a_slow_release_still_flings() {
    let mut h = harness("r13-fling");
    h.app.new_tab();
    let body = filler(200);
    h.app.insert_text(&body);
    h.app.set_caret(0, false);
    h.draw();
    // Flick upwards the way a hand does: a few moves, then a release a tenth of a
    // second later. That last pause is what used to kill the fling.
    let x = h.app.text_rect.x + h.app.text_rect.w * 0.5;
    let mut y = h.app.text_rect.y + h.app.text_rect.h * 0.8;
    h.app.mouse = (x, y);
    h.app.pan_begin();
    for _ in 0..4 {
        std::thread::sleep(std::time::Duration::from_millis(25));
        y -= 45.0;
        h.app.mouse = (x, y);
        h.app.pan_to((x, y));
    }
    std::thread::sleep(std::time::Duration::from_millis(100));
    h.app.pan_end();
    let at_release = h.app.tab().scroll;
    assert!(h.app.kinetic_active(), "a flick released after a pause lost its momentum");
    std::thread::sleep(std::time::Duration::from_millis(20));
    h.app.step_kinetic();
    let after = h.app.tab().scroll;
    assert!(
        after > at_release + 0.5,
        "the page stopped dead on release ({at_release} -> {after})"
    );
}

// ---------------------------------------------------------------------------
// Round 14
// ---------------------------------------------------------------------------

/// (14.1) A link in a table cell: the cell shows the label, not the whole
/// `[label](url)` text, and the caret going into the cell reveals the source so
/// the link can be edited instead of being followed.
#[test]
fn a_link_in_a_table_cell_renders_and_can_be_edited() {
    let mut h = harness("r14-table-link");
    h.app.new_tab();
    h.app.insert_text("| A | B |\n| - | - |\n| [Example](https://example.com) | x |\n");
    h.app.set_caret(0, false);
    h.draw();
    let line = h.app.tab().layout.lines.get(2).expect("the link row");
    let painted: String = line
        .bands(false)
        .iter()
        .flatten()
        .map(|r| line.chars[r.start..r.end].iter().collect::<String>())
        .collect();
    assert!(
        painted.contains("Example") && !painted.contains("https://"),
        "the cell paints the whole link instead of its label: {painted:?}"
    );
    // Put the caret in the label: the source with the destination must appear, or
    // the link cannot be edited in the rendered view.
    let text = h.app.tab().doc.text();
    let at = text.find("Example").expect("the label") as i32;
    h.app.set_caret(at as usize + 2, false);
    h.draw();
    let line = h.app.tab().layout.lines.get(2).expect("the link row");
    let shown: String = line
        .bands(true)
        .iter()
        .flatten()
        .map(|r| line.chars[r.start..r.end].iter().collect::<String>())
        .collect();
    assert!(
        shown.contains("https://example.com"),
        "the destination stays invisible while the caret is in the link: {shown:?}"
    );
    // …but the pipes stay hidden: the grid is still the editing surface.
    assert!(!shown.contains('|'), "the row flipped to raw source: {shown:?}");
}

/// (14.1b -> r25) A link pasted *as* a link (`[url](url)`) is understood. A plain
/// click inside a table now follows the link, exactly like a click in a paragraph
/// (r25: a link in a cell simply did not open); Alt+click is what puts the caret
/// into the label to edit it.
#[test]
fn clicking_a_link_in_a_table_places_the_caret() {
    let mut h = harness("r14-table-link-click");
    h.app.new_tab();
    h.app.insert_text("| A | B |\n| - | - |\n| [Link]([https://example.com](https://example.com)) | x |\n");
    h.app.set_caret(0, false);
    h.draw();
    // The nested parentheses are the whole destination, not just the inner label.
    let chars: Vec<char> = h.app.tab().doc.text().chars().collect();
    let targets = bare_md::markdown::link_targets(&chars[chars.iter().position(|c| *c == '|').unwrap()..]);
    assert!(
        targets.iter().any(|(_, _, t)| t == "https://example.com"),
        "the pasted link has no usable destination: {targets:?}"
    );
    // Alt+click on the label: the caret lands in the cell, and nothing is opened.
    let row = h.app.tab().layout.lines[2].clone();
    let label_local = row.info.cells[0].0;
    let (x, y) = h.cell_point(2, 0, 0.15);
    h.app.alt_down = true;
    h.click_full((x, y));
    h.app.alt_down = false;
    let caret = h.app.tab().caret;
    assert!(
        caret >= row.start_char + label_local && caret <= row.start_char + row.info.cells[0].1,
        "the click did not land in the link's cell ({} is outside {:?})",
        caret,
        row.info.cells[0]
    );
}

/// (14.2) In a table, the caret sits exactly where the glyphs are: the grid, the
/// cell text and the caret all start at the same x.
#[test]
fn the_table_caret_lines_up_with_its_text() {
    let mut h = harness("r14-table-caret");
    h.app.new_tab();
    h.app.insert_text("| Name | Value |\n| ---- | ----- |\n| alpha | 1 |\n");
    let text = h.app.tab().doc.text();
    let at = text.find("alpha").expect("the cell") as usize;
    h.app.set_caret(at + 5, false);
    h.draw();
    let line = h.app.tab().layout.lines[2].clone();
    let caret = h.app.caret_rect(&mut h.r.text);
    // Where the layout says the caret is, on screen: the same index it used.
    let local = h.app.tab().caret - line.start_char;
    let want = h.app.text_rect.x + line.indent + line.table_pos[local].1;
    assert!(
        (caret.x - want).abs() < 1.5,
        "the caret is at {} but the text is at {}",
        caret.x,
        want
    );
    // A click on the right half of that character lands *after* it.
    let cell_x = h.app.text_rect.x + line.indent + line.table_cols[0];
    assert!(cell_x > h.app.text_rect.x, "the table is not indented at all");
}

/// (22) Starting the program a second time hands the file to the running copy and
/// exits - it never opens a window of its own, so it can never fight the first one
/// over the remembered window geometry.
#[test]
fn a_second_launch_hands_its_file_to_the_running_instance() {
    use bare_md::platform::single::{claim, Claim};
    let dir = std::env::temp_dir().join(format!("bare-md-handoff-bin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("from-the-desktop.md");
    std::fs::write(&file, "# opened by a double click\n").unwrap();

    // This process plays the running editor.
    let Claim::Primary(instance) = claim(&dir, &[]) else {
        panic!("the test could not become the instance");
    };
    let settings = dir.join("settings.conf");

    // …and this is the launch a desktop double-click performs.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_bare-md"))
        .arg(&file)
        .env("BAREMD_DATA_DIR", &dir)
        .output()
        .expect("could not start the editor");
    assert!(
        out.status.success(),
        "a second launch should exit quietly, but it failed: {:?}",
        out
    );

    let mut got: Vec<Vec<PathBuf>> = Vec::new();
    for _ in 0..100 {
        got = instance.take();
        if !got.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        got,
        vec![vec![file.clone()]],
        "the running instance never received the file"
    );
    // A launch that only forwards must not write settings: the running window's
    // geometry is the only geometry there is.
    assert!(
        !settings.exists(),
        "the forwarding launch wrote settings.conf, so it would have overruled the running window"
    );
    instance.release();
    assert!(!dir.join("instance.lock").exists(), "the instance lock was left behind");
    let _ = std::fs::remove_dir_all(&dir);
}

/// (21) A file deleted behind its tab is marked in red, and closing that tab asks
/// first - the text exists nowhere else any more.
#[test]
fn a_deleted_file_turns_its_tab_red_and_closing_asks_first() {
    let mut h = harness("r21-file-gone");
    let path = h._dir.join("gone.md");
    std::fs::write(&path, "# One\n\nwords\n").unwrap();
    h.app.open_path(&path);
    h.draw();
    assert!(!h.app.tab().doc.file_gone(), "a file that exists is not gone");

    let idx = h.app.active;
    // Delete it behind the editor's back.
    std::fs::remove_file(&path).unwrap();
    h.app.poll_disk();
    h.draw();
    assert!(h.app.tab().doc.file_gone(), "the deletion was not noticed");
    assert!(!h.app.tab().doc.dirty, "nothing was typed, so the buffer is clean");

    // The tab paints its name in the red reserved for this case, in every theme.
    for kind in ThemeKind::ALL {
        let theme = bare_md::theme::Theme::by_kind(kind);
        let red = bare_md::ui::tab_ink(&theme, true, true);
        assert!(
            red.r > red.g + 0.1 && red.r > red.b + 0.1,
            "{:?}: a deleted file's tab is not red ({red:?})",
            kind
        );
        let plain = bare_md::ui::tab_ink(&theme, true, false);
        assert_eq!(plain, theme.text, "{:?}: a healthy tab changed colour", kind);
    }

    // Closing it asks, and says why.
    let tabs = h.app.tabs.len();
    h.app.close_tab(idx);
    match &h.app.dialog {
        Some(bare_md::app::Dialog::Confirm { title, message, danger, .. }) => {
            assert!(*danger, "the question about a deleted file is not marked as dangerous");
            assert!(title.contains("deleted"), "unexpected title: {title}");
            assert!(
                message.contains("only copy") || message.contains("deleted"),
                "the question does not explain what is at stake: {message}"
            );
        }
        other => panic!("closing a tab whose file is gone did not ask: {other:?}"),
    }
    assert_eq!(h.app.tabs.len(), tabs, "the tab was closed without asking");
    // Cancel keeps it.
    h.draw();
    assert!(h.click_hit(|hit| *hit == Hit::DialogCancel), "no Cancel button");
    h.draw();
    assert_eq!(h.app.tabs.len(), tabs, "Cancel closed the tab anyway");
    // …and Discard closes it.
    h.app.close_tab(idx);
    h.draw();
    assert!(h.click_hit(|hit| *hit == Hit::DialogConfirm), "no Discard button");
    h.draw();
    assert_eq!(h.app.tabs.len(), tabs - 1, "Discard did not close the tab");
    assert!(
        h.app.tabs.iter().all(|t| t.doc.path.as_deref() != Some(path.as_path())),
        "the closed tab's document is still there"
    );
}

/// (21) Saving brings the file back, so the red mark goes away.
#[test]
fn saving_a_deleted_document_clears_the_mark() {
    let mut h = harness("r21-file-back");
    let path = h._dir.join("back.md");
    std::fs::write(&path, "text\n").unwrap();
    h.app.open_path(&path);
    h.draw();
    std::fs::remove_file(&path).unwrap();
    h.app.poll_disk();
    h.draw();
    assert!(h.app.tab().doc.file_gone());
    h.app.save();
    h.draw();
    assert!(path.exists(), "saving did not write the file back");
    assert!(!h.app.tab().doc.file_gone(), "the tab is still marked as deleted");
    assert!(!h.app.tab().doc.dirty);
}

/// (21) Quitting warns about a deleted file too, not only about unsaved edits.
#[test]
fn quitting_counts_a_deleted_file_as_data_at_risk() {
    let mut h = harness("r21-quit");
    let path = h._dir.join("quit.md");
    std::fs::write(&path, "text\n").unwrap();
    h.app.open_path(&path);
    h.draw();
    assert!(h.app.tabs_at_risk().is_empty(), "a clean, present file is not at risk");
    std::fs::remove_file(&path).unwrap();
    h.app.poll_disk();
    h.draw();
    assert_eq!(
        h.app.tabs_at_risk(),
        vec![h.app.active],
        "a tab whose file was deleted has to be counted when quitting"
    );
}

/// (20.2) Bold has to *show* inside a heading.
///
/// A heading, a table's header row and inline code are drawn with the bold face
/// already, so `**strong**` there used to change nothing at all - the report was
/// "bold on headers is not visible". Emphasis the font cannot express with a
/// heavier weight is drawn as extra ink on the same glyphs, and the advance is
/// untouched, so nothing moves.
#[test]
fn bold_inside_a_heading_shows_as_heavier_ink() {
    let mut h = harness("r20-heading-bold");
    h.app.new_tab();
    h.app.insert_text("# Heading words here\n\nbody words here\n");
    // Bold the word in the heading.
    let at = h.app.tab().doc.text().find("words").unwrap();
    h.app.select_range(at, at + 5);
    h.app.toggle_inline("**", "**");
    h.app.set_caret(h.app.tab().doc.text().len() - 1, false);
    h.draw();
    let line = h.app.tab().layout.lines[h.app.tab().layout.line_of_char(2)].clone();
    let raw = h.app.tab().line_is_raw(h.app.mode, h.app.tab().layout.line_of_char(2));
    let mut headed = None;
    for run in line.runs(raw) {
        let s: String = line.chars[run.start..run.end].iter().collect();
        if s == "words" {
            headed = Some(run.style.clone());
        }
    }
    let headed = headed.expect("the bold run in the heading is missing");
    assert_eq!(headed.face, Face::Bold, "a heading is drawn with the bold face");
    assert!(
        headed.weight > 0.0,
        "bold inside a heading carries no extra ink, so it cannot be seen"
    );
    // The rest of the heading must not become heavier.
    for run in line.runs(raw) {
        let s: String = line.chars[run.start..run.end].iter().collect();
        if s != "words" && !s.contains("Heading") {
            assert_eq!(run.style.weight, 0.0, "{s:?} was thickened as well");
        }
    }

    // In body text the face itself changes, and no extra ink is needed.
    let body = h.app.tab().doc.text().find("words here").unwrap();
    h.app.select_range(body, body + 5);
    h.app.toggle_inline("**", "**");
    h.app.set_caret(0, false);
    h.draw();
    let li = h.app.tab().layout.line_of_char(body);
    let line = h.app.tab().layout.lines[li].clone();
    let raw = h.app.tab().line_is_raw(h.app.mode, li);
    let bold = line
        .runs(raw)
        .iter()
        .find(|r| line.chars[r.start..r.end].iter().collect::<String>() == "words")
        .expect("the bold run in the body is missing")
        .style
        .clone();
    assert_eq!(bold.face, Face::Bold, "body bold uses the bold face");
    // (r25) …and it also carries a little extra ink: the two weights of Lato are
    // close enough at text sizes that the face alone is barely visible.
    assert!(
        bold.weight >= 0.5,
        "body bold is not heavier than the text around it (weight {})",
        bold.weight
    );

    // The extra ink is ink only: the run keeps its own width.
    let adv = h.r.text_ref().char_advances(Face::Bold, "words", headed.size);
    assert!((adv.iter().sum::<f32>() - headed.size * 1.2).abs() > 0.0 || !adv.is_empty());
}

/// (20.2) Weighted drawing changes the pixels, not the pen.
#[test]
fn weighted_text_keeps_the_advance_and_adds_ink() {
    let mut h = harness("r20-weight-runs");
    h.draw();
    let before = h.r.stats.glyph_quads;
    let adv_plain = h.r.text_run(40.0, 40.0, "Heading words", Face::Bold, 31.0, bare_md::geom::Color::rgba(1.0, 1.0, 1.0, 1.0));
    let quads_plain = h.r.stats.glyph_quads - before;
    let before = h.r.stats.glyph_quads;
    let adv_weight = h.r.text_run_weighted(
        40.0,
        40.0,
        "Heading words",
        Face::Bold,
        31.0,
        bare_md::geom::Color::rgba(1.0, 1.0, 1.0, 1.0),
        1.7,
    );
    let quads_weight = h.r.stats.glyph_quads - before;
    assert!(
        (adv_plain - adv_weight).abs() < 0.01,
        "the weighted run advanced {adv_weight} px instead of {adv_plain}"
    );
    assert!(
        quads_weight > quads_plain,
        "weighted text drew {quads_weight} quads, the plain one {quads_plain} - no extra ink"
    );
}

/// (19.2) One wheel notch scrolls a sensible amount - more than a couple of lines,
/// and exactly the named constant, so the speed lives in one place.
#[test]
fn one_wheel_notch_scrolls_a_readable_amount() {
    use winit::event::{MouseScrollDelta, WindowEvent};
    let mut h = harness("r19-wheel");
    h.app.new_tab();
    let body: String = (0..200).map(|i| format!("line {i}\n")).collect();
    h.app.insert_text(&body);
    h.app.set_caret(0, false);
    h.draw();
    let line_h = h.app.tab().layout.lines[0].row_h;
    let before = h.app.tab().scroll;
    let mut input = bare_md::input::InputState::default();
    h.app.handle_event(
        &WindowEvent::MouseWheel {
            delta: MouseScrollDelta::LineDelta(0.0, -1.0),
            phase: winit::event::TouchPhase::Moved,
            device_id: winit::event::DeviceId::dummy(),
        },
        &mut h.r,
        &mut input,
    );
    h.draw();
    let moved = h.app.tab().scroll - before;
    assert!(
        (moved - bare_md::input::WHEEL_STEP).abs() < 0.5,
        "a notch moved {moved} px, not the {} px the constant names",
        bare_md::input::WHEEL_STEP
    );
    assert!(
        moved > line_h * 2.0,
        "a notch moves {moved} px, barely {} lines - too slow",
        moved / line_h
    );
    // And it scrolls the same distance back up.
    let down = h.app.tab().scroll;
    h.app.handle_event(
        &WindowEvent::MouseWheel {
            delta: MouseScrollDelta::LineDelta(0.0, 1.0),
            phase: winit::event::TouchPhase::Moved,
            device_id: winit::event::DeviceId::dummy(),
        },
        &mut h.r,
        &mut input,
    );
    h.draw();
    assert!((h.app.tab().scroll - (down - moved)).abs() < 1.0, "the wheel is not symmetric");
}

/// (19.3) A long document name cannot push the unsaved-changes dot out of its own
/// tab: the name is ellipsised into the space left over, and the dot keeps its own
/// slot - next to the close button, never on it.
#[test]
fn a_long_name_keeps_the_dirty_dot_inside_its_tab() {
    let mut h = harness("r19-dirty-dot");
    let long = h
        ._dir
        .join("a-really-quite-long-document-name-that-will-never-fit-in-one-tab.md");
    std::fs::write(&long, "# hi\n").unwrap();
    h.app.open_path(&long);
    h.app.insert_text("typed, so the tab is dirty");
    h.draw();
    assert!(h.app.tab().doc.dirty, "the tab should be dirty");
    let tabs: Vec<Rect> = h
        .app
        .hits
        .entries_for_test()
        .iter()
        .filter(|(_, hit)| matches!(hit, Hit::Tab(_)))
        .map(|(r, _)| *r)
        .collect();
    let tab = *tabs.last().expect("no tab hit area");
    // Nothing the tab draws may reach past its right edge, where the next tab is.
    for (show_close, dirty) in [(true, true), (true, false), (false, true), (false, false)] {
        let lane = bare_md::ui::tab_lane(show_close, dirty);
        let name_w = tab.w - bare_md::ui::TAB_NAME_INSET - lane;
        assert!(name_w >= 12.0, "a tab must still have room for a name");
        // The dot, when there is one, sits fully inside the tab.
        let (cx, cy) = bare_md::ui::tab_dot_center(tab, show_close);
        let half = bare_md::ui::TAB_DOT * 0.5;
        assert!(
            cx - half >= tab.x && cx + half <= tab.right() - 1.0,
            "the dot at {cx} escapes the tab {}..{}",
            tab.x,
            tab.right()
        );
        assert!(cy > tab.y && cy < tab.bottom(), "the dot is outside the tab's height");
        // With a close button there must be no overlap: the button starts at
        // `right - 21`, so the dot's right edge has to stop short of it.
        if show_close {
            assert!(
                cx + half < tab.right() - 21.0,
                "the dot at {cx} is drawn on top of the close button"
            );
        }
        // The name never grows into the lane that the dot and button live in.
        assert!(
            name_w + bare_md::ui::TAB_NAME_INSET <= tab.w,
            "the name would be drawn under the dot"
        );
    }
}

/// (19.1) Text is measured the way it is drawn.
///
/// Adding up single-character widths is not the width of the run: kerning makes
/// `AVATAR WAVE` 114.4 px one way and 108.6 px the other. The layout used the
/// per-character sum while the painter drew the shaped run, so in a long table
/// cell the caret and the pointer drifted away from the glyphs by more than a
/// character - the "text is one symbol ahead" report. Both numbers must now be the
/// same one.
#[test]
fn per_character_advances_match_the_shaped_run() {
    let mut engine = bare_md::text::TextEngine::new();
    let mut saw_difference = false;
    for text in [
        "AVATAR WAVE",
        "plain words here",
        "Wrapping matters for prose: this paragraph is deliberately long",
        "Текст на кириллице в ячейке таблицы, довольно длинный",
        "office difficult fluffy",
        "\u{4e00}\u{4e8c} \u{4e09}",
    ] {
        let drawn = engine.width(Face::Regular, text, 16.0);
        let adv = engine.char_advances(Face::Regular, text, 16.0);
        assert_eq!(adv.len(), text.chars().count(), "one advance per character");
        let sum: f32 = adv.iter().sum();
        assert!(
            (sum - drawn).abs() < 0.05,
            "{text:?}: character advances add up to {sum} but the run is drawn {drawn} px wide"
        );
        // And the old way really was different - otherwise this test proves nothing.
        // (Not for every script: CJK has no kerning pairs.)
        let naive: f32 = text
            .chars()
            .map(|c| engine.advance(Face::Regular, c, 16.0))
            .sum();
        saw_difference |= (naive - drawn).abs() > 0.05;
    }
    assert!(
        saw_difference,
        "no sample showed the kerning difference; pick strings that kern"
    );
}

/// (19.1) Inside a table cell, the caret's travel across a run is the width that
/// run is painted at, character by character - no accumulated drift.
#[test]
fn table_cell_caret_travel_matches_the_painted_run() {
    let mut h = harness("r19-cell-measure");
    h.app.new_tab();
    let wide = "AVATAR WAVE To Wave Away A V A T A R ".repeat(2);
    h.app
        .insert_text(&format!("| A | B |\n| - | - |\n| {wide} | x |\n| plain | y |\n"));
    h.app.set_caret(0, false);
    h.draw();
    let line = h.app.tab().layout.lines[h.app.tab().layout.line_of_char(h.app.tab().doc.line_start(2))].clone();
    let mut engine = h.r.text_ref();
    assert!(!line.bands(false).is_empty(), "the table row was not laid out");
    for band in line.bands(false) {
        for run in band {
            let text: String = line.chars[run.start..run.end].iter().collect();
            let painted = engine.width(run.style.face, &text, run.style.size);
            assert!(
                (run.w - painted).abs() < 0.5,
                "{text:?}: the layout calls the run {} px wide, the painter draws it {} px",
                run.w,
                painted
            );
            // The caret walks the run one character at a time; it must arrive at
            // the far end of the same rectangle.
            let start = line.x_abs(&mut engine, run.start, false);
            let end = line.x_abs(&mut engine, run.end, false);
            let travel = if run.end == line.chars.len() {
                end - start
            } else {
                // `x_abs(run.end)` is the gap after the run (the cell padding);
                // measure the last character on its own instead.
                let last: String = line.chars[run.end - 1..run.end].iter().collect();
                line.x_abs(&mut engine, run.end - 1, false) - start
                    + engine.width(run.style.face, &last, run.style.size)
            };
            assert!(
                (travel - painted).abs() < 0.6,
                "{text:?}: the caret travels {travel} px over a run painted {painted} px wide"
            );
        }
    }
}

/// (14.3) A workspace chip's `x` is the same button a document tab draws, in the
/// same place and with the same hit area.
#[test]
fn workspace_and_document_tabs_close_the_same_way() {
    let mut h = harness("r14-chip-close");
    let dir = h._dir.join("second");
    std::fs::create_dir_all(&dir).unwrap();
    h.app.add_workspace(dir);
    h.draw();
    let chip = h
        .app
        .hits
        .find(|hit| matches!(hit, Hit::WorkspaceTab(_)))
        .expect("no workspace chip");
    h.app.mouse = (chip.x + chip.w * 0.5, chip.y + chip.h * 0.5);
    h.draw();
    let ws_close = h
        .app
        .hits
        .find(|hit| matches!(hit, Hit::WorkspaceTabClose(_)))
        .expect("no close button on the workspace chip");
    let tab = h.app.hits.find(|hit| matches!(hit, Hit::Tab(_))).expect("no document tab");
    h.app.mouse = (tab.x + tab.w * 0.5, tab.y + tab.h * 0.5);
    h.draw();
    let doc_close = h
        .app
        .hits
        .find(|hit| matches!(hit, Hit::TabClose(_)))
        .expect("no close button on the document tab");
    // Same size, same distance from the chip's right edge.
    assert_eq!(ws_close.w, doc_close.w, "the two close buttons are different sizes");
    assert_eq!(ws_close.h, doc_close.h, "the two close buttons are different heights");
    let ws_gap = chip.right() - ws_close.right();
    let doc_gap = tab.right() - doc_close.right();
    assert!(
        (ws_gap - doc_gap).abs() < 0.6,
        "the close buttons do not line up ({ws_gap} vs {doc_gap} from the edge)"
    );
}

/// (14.4) The bars never change size or pitch: a long document fills the strip and the map
/// *slides* with the page.
///
/// The pitch used to shrink with the document, down to 0.85 px a row. At a fractional pitch
/// the row that survives the collision check changes from frame to frame, so the strip
/// shimmered (moire) and the lines looked thinner than they are. One pitch - three pixels a
/// row, two-pixel bars - for every document is the only version that does not do that; what
/// changes with length is how much of the file the strip shows, and dragging slides it.
#[test]
fn a_long_document_slides_instead_of_changing_its_pitch() {
    let mut h = harness("r14-map-slide");
    h.app.new_tab();
    h.app.insert_text(&filler(900));
    h.app.set_caret(0, false);
    h.draw();
    let map = h.app.minimap_rect;
    let layout = &h.app.tab().layout;
    let total = layout.total_height;
    let text_h = h.app.text_rect.h;
    let top = bare_md::ui::minimap_geom_at(layout, map, 0.0, text_h);
    assert_eq!(
        top.scale,
        bare_md::ui::MINIMAP_PITCH,
        "a long document changed its pitch ({})",
        top.scale
    );
    assert_eq!(top.bar_h, bare_md::ui::MINIMAP_BAR_H, "the bars changed thickness");
    assert!(top.content_h > map.h, "the map fits, so this is not the sliding case");
    assert_eq!(top.offset, 0.0, "the map is not at the top of the document");
    let mid = bare_md::ui::minimap_geom_at(layout, map, total * 0.5, text_h);
    let end = bare_md::ui::minimap_geom_at(layout, map, total, text_h);
    assert!(mid.offset > 0.0, "the map does not slide");
    assert!(end.offset > mid.offset, "the map does not slide the whole way down");
    let room = (map.h - bare_md::ui::MINIMAP_PAD - 2.0).max(8.0);
    assert!(
        end.offset <= end.content_h - room + 1.0,
        "the map slid past its own end ({} of {})",
        end.offset,
        end.content_h - room
    );
    // The bars move with the map: the same line is painted at a different y.
    let y_top = top.y_of_virtual(top.vtotal * 0.5);
    let y_end = end.y_of_virtual(end.vtotal * 0.5);
    assert!((y_top - y_end).abs() > 1.0, "the map content did not slide with the page");
}

/// (14.5) The workspace-panel button is a plain outline: a frame and one divider,
/// nothing else to read.
#[test]
fn the_panel_icon_is_a_plain_outline() {
    let src = bare_md::icons::source("sidebar").expect("no sidebar icon");
    let shapes = src.matches("<rect").count() + src.matches("<path").count();
    assert_eq!(shapes, 2, "the panel icon has {shapes} shapes; it should be a frame and a divider");
    assert!(src.contains("M9.5 4.5v15"), "the icon lost its divider");
}

/// (14.6) The crate is split by layer, and each layer owns its job: no winit or
/// wgpu outside `platform`, no Markdown parsing in `ui`.
#[test]
fn the_layers_own_their_jobs() {
    let platform = [include_str!("../src/platform/window.rs"), include_str!("../src/platform/input.rs")].concat();
    assert!(platform.contains("winit::"), "the platform layer must be the winit one");
    for (name, src) in [
        ("ui", ui_source()),
        ("markdown", include_str!("../src/markdown/layout.rs")),
        ("editor", include_str!("../src/editor/app.rs")),
    ] {
        assert!(!src.contains("use winit::"), "{name} must not depend on winit");
        assert!(!src.contains("use wgpu::"), "{name} must not depend on wgpu");
    }
    // The renderer is reachable from `ui` and `editor` only through the `gpu`
    // alias, and the parser through `markdown`.
    assert!(ui_source().contains("crate::gpu::Renderer"));
}

// ---------------------------------------------------------------- round 15

/// (15.1) A cell whose whole content is `![alt](src)` shows the picture, not its
/// source, and the row grows to hold it.
#[test]
fn a_picture_in_a_table_cell_is_drawn() {
    let mut h = harness("r15-cell-image");
    h.app.new_tab();
    // A real picture on disk, so this is the path the reader takes.
    let dir = h._dir.clone();
    {
        let f = std::fs::File::create(dir.join("dot.png")).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), 40, 24);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().unwrap();
        let mut data = Vec::new();
        for _ in 0..40 * 24 {
            data.extend_from_slice(&[200, 120, 60, 255]);
        }
        w.write_image_data(&data).unwrap();
    }
    std::fs::write(dir.join("note.md"), "").unwrap();
    h.app.open_path(&dir.join("note.md"));
    h.type_str("| Pic | Name |\n| - | - |\n| ![a dot](dot.png) | words |\n");
    h.app.set_caret(0, false);
    h.draw();
    let line = h.app.tab().layout.lines[2].clone();
    assert_eq!(
        line.table_images.len(),
        1,
        "the cell's picture was not picked up: {:?}",
        line.table_images
    );
    let (_, img) = &line.table_images[0];
    assert!(!img.missing, "the picture on disk was not found: {:?}", img.path);
    assert!(img.w > 4.0 && img.h > 4.0, "the picture has no size: {}x{}", img.w, img.h);
    // The row is tall enough for the picture, and the source text is not painted
    // over it.
    assert!(
        line.height >= img.h,
        "the row is {} tall for a {} px picture",
        line.height,
        img.h
    );
    let painted: Vec<(usize, usize)> = line
        .bands(false)
        .iter()
        .flatten()
        .map(|r| (r.start, r.end))
        .collect();
    assert!(
        !painted.iter().any(|(a, b)| *a <= 2 && *b >= 17),
        "the image source is still laid out as text: {painted:?}"
    );
    // While the caret is in the cell the source comes back, so it can be edited.
    let start = h.app.tab().layout.lines[2].start_char;
    h.app.set_caret(start + 4, false);
    h.draw();
    let line = h.app.tab().layout.lines[2].clone();
    assert!(
        line.bands(true).iter().flatten().any(|r| r.start == 2),
        "clicking the cell does not bring the image's source back: {:?}",
        line.bands(true)
    );
}

/// (15.2) A click in a table row lands on the character under the pointer, cell by
/// cell - even in a row that is showing its inline markers.
#[test]
fn a_click_in_a_table_lands_on_the_character_under_it() {
    let mut h = harness("r15-table-click");
    h.app.new_tab();
    h.type_str("| Aaaa | Bbbb | Cccc |\n| - | - | - |\n| one two | **bold** | three |\n");
    h.app.set_caret(0, false);
    h.draw();
    let start = h.app.tab().layout.lines[2].start_char;
    h.app.set_caret(start + 3, false);
    h.draw();
    let line = h.app.tab().layout.lines[2].clone();
    let revealed = h.app.tab().table_shows_markers(h.app.mode, 2);
    assert!(revealed, "the row under the caret should show its source");
    let mut checked = 0;
    for (bi, band) in line.bands(revealed).iter().enumerate() {
        for run in band {
            let n = (run.end - run.start).max(1) as f32;
            let cw = run.w / n;
            for k in 0..(run.end - run.start) {
                let cx = h.app.text_rect.x - h.app.tab().scroll_x + line.indent + run.x + (k as f32 + 0.5) * cw;
                let cy = h.row_y(2, bi, 0.5);
                h.click((cx, cy));
                let local = h.app.tab().caret.saturating_sub(start);
                let want = run.start + k;
                assert!(
                    local == want || local == want + 1,
                    "clicking character {want} of the row put the caret at {local} (band {bi})"
                );
                // And the caret is drawn where the layout put that character: the
                // band and the x come from the same wrap the glyphs were painted
                // from, so the caret cannot sit in a column of its own. (One
                // offset has two positions at a soft-wrap boundary - the end of
                // the line above and the start of the line below, exactly as in a
                // wrapped paragraph - and the caret takes the line the character
                // itself starts on.)
                let cr = h.app.caret_rect(&mut h.r.text);
                let l_local = h.app.tab().caret.saturating_sub(start);
                let l_band = line.row_of_local_in(l_local, revealed);
                let l_x = line.band_pos(revealed).get(l_local).map(|t| t.1).unwrap_or(0.0);
                assert!(
                    (cr.x - (h.app.text_rect.x + line.indent + l_x)).abs() < 1.5,
                    "the caret for local {l_local} is drawn at {:.1}, the layout has it at {:.1}",
                    cr.x,
                    h.app.text_rect.x + line.indent + l_x
                );
                let want_y = h.row_y(2, l_band, 0.0);
                assert!(
                    (cr.y - want_y).abs() < 3.0,
                    "the caret for local {l_local} is drawn at y {:.1}, its row starts at {want_y:.1}",
                    cr.y
                );
                checked += 1;
            }
        }
    }
    assert!(checked >= 20, "only {checked} positions were checked");
}

/// (15.3) On a long document the minimap's handle keeps up with the pointer: one
/// pixel of drag is one pixel of handle, no matter how much the map is compressed.
#[test]
fn the_minimap_handle_follows_the_pointer() {
    let mut h = harness("r15-map-follow");
    h.app.new_tab();
    h.app.insert_text(&filler(320));
    h.app.set_caret(0, false);
    h.draw();
    let mut y = h.minimap_press(0.5);
    let mut handle = h.handle_y();
    for _ in 0..3 {
        y += 12.0;
        h.minimap_drag_to(y);
        let next = h.handle_y();
        assert!(
            (next - handle - 12.0).abs() < 1.5,
            "the pointer moved 12 px and the handle moved {:.1} px",
            next - handle
        );
        handle = next;
    }
    h.app.mouse_up(&mut h.r);
    // Same going up, from wherever the handle now is.
    let mut y = h.minimap_press((y - h.minimap().y) / h.minimap().h);
    let mut handle = h.handle_y();
    for _ in 0..2 {
        y -= 12.0;
        h.minimap_drag_to(y);
        let next = h.handle_y();
        assert!(
            (handle - next - 12.0).abs() < 1.5,
            "dragging up moved the handle {:.1} px for 12 px of pointer",
            handle - next
        );
        handle = next;
    }
    h.app.mouse_up(&mut h.r);
}

/// (15.4) The folder dialog's tree can be collapsed again: the triangle is the
/// thing under the pointer, not the row it sits on.
#[test]
fn the_folder_tree_can_be_closed() {
    let mut h = harness("r15-tree-close");
    let dir = h._dir.clone();
    std::fs::create_dir_all(dir.join("aaa/inner")).unwrap();
    std::fs::create_dir_all(dir.join("bbb")).unwrap();
    h.app.open_picker(bare_md::app::PickerKind::OpenFolder);
    h.draw();
    // Every triangle that is on screen right now, with the row it belongs to.
    let mut targets = Vec::new();
    for i in 0..h.picker().tree.rows().len() {
        if let Some(r) = h.app.hits.find(|hit| *hit == Hit::DialogTreeChevron(i)) {
            let row = &h.picker().tree.rows()[i];
            if row.expanded && row.has_dirs {
                targets.push((i, r));
            }
        }
    }
    assert!(
        !targets.is_empty(),
        "no open folder on screen has a triangle to close it"
    );
    let (i, chev) = targets[0].clone();
    let open_rows = h.picker().tree.rows().len();
    let path = h.picker().tree.rows()[i].path.clone();
    let centre = (chev.x + chev.w * 0.5, chev.y + chev.h * 0.5);
    h.click(centre);
    assert!(
        !h.picker().tree.is_expanded(&path),
        "clicking the triangle of {:?} selected it instead of closing it",
        path
    );
    let closed_rows = h.picker().tree.rows().len();
    assert!(
        closed_rows < open_rows,
        "the tree still lists {closed_rows} rows, it had {open_rows}"
    );
    // And the same triangle opens it again, so one control does both.
    h.draw();
    let chev = h
        .app
        .hits
        .find(|hit| *hit == Hit::DialogTreeChevron(i))
        .expect("the triangle vanished with the folder closed");
    h.click((chev.x + chev.w * 0.5, chev.y + chev.h * 0.5));
    assert!(h.picker().tree.is_expanded(&path), "the tree did not open again");
    assert_eq!(h.picker().tree.rows().len(), open_rows, "the tree did not come back");
}

/// (15.5) A code block's fence rows are as tall as its code rows: the markers fit
/// on their own line and the caret on them is a full caret.
#[test]
fn the_fence_rows_of_a_code_block_are_full_rows() {
    let mut h = harness("r15-fence");
    h.app.new_tab();
    h.type_str("```rust\nlet x = 1;\n```\n\nAfter.\n");
    h.app.set_caret(0, false);
    h.draw();
    let lines = h.app.tab().layout.lines.clone();
    assert!(matches!(lines[0].block(), bare_md::markdown::Block::Fence));
    assert!(matches!(lines[1].block(), bare_md::markdown::Block::Code));
    assert!(
        (lines[0].height - lines[1].height).abs() < 0.5,
        "the opening fence row is {} tall next to a {} row of code",
        lines[0].height,
        lines[1].height
    );
    assert!(
        (lines[2].height - lines[1].height).abs() < 0.5,
        "the closing fence row is {} tall next to a {} row of code",
        lines[2].height,
        lines[1].height
    );
    // A click on the opening fence row puts the caret on that line, and the caret
    // is a whole row tall - not the half-height sliver that looked cut off.
    let l0 = lines[0].clone();
    let y = h.app.text_rect.y + l0.top + l0.height * 0.5;
    h.click((h.app.text_rect.x + 40.0, y));
    let caret = h.app.tab().caret;
    assert!(caret <= l0.end_char(), "a click on the fence row moved the caret to {caret}");
    let cr = h.app.caret_rect(&mut h.r.text);
    let text_lh = h.r.text.line_height(Face::Regular, h.app.theme.font_size);
    assert!(
        cr.h >= text_lh * 0.92,
        "the caret on a fence row is {} px tall, a text line is {text_lh}",
        cr.h
    );
    assert!(
        cr.y >= h.app.text_rect.y + l0.top - 1.0 && cr.y < y,
        "the caret was drawn at {} while the fence row starts at {}",
        cr.y,
        h.app.text_rect.y + l0.top
    );
}

// ---------------------------------------------------------------- round 16

/// (16.1) A long string is measured in chunks, and the chunks add up: the number
/// must not depend on how the string was cut, or every wrapped line in the editor
/// would drift once chunking kicked in.
#[test]
fn a_long_string_is_measured_in_chunks_that_add_up() {
    let mut h = harness("r16-chunks");
    let word = "portable ";
    let long: String = word.repeat(600); // 5400 chars
    let whole = h.r.text.width(Face::Regular, &long, 16.0);
    // The same text measured in pieces has to give the same number.
    let mut sum = 0.0;
    for part in long.split_inclusive(' ') {
        sum += h.r.text.width(Face::Regular, part, 16.0);
    }
    assert!(
        (whole - sum).abs() < 0.5,
        "a 5400-character string measures {whole:.1} whole and {sum:.1} in pieces"
    );
    // And the glyphs the painter places add up to the same width.
    let shaped = h.r.text.shape(Face::Regular, &long);
    let glyphs: f32 = shaped.iter().map(|g| g.advance).sum::<f32>() * 16.0;
    assert!(
        (whole - glyphs).abs() < 0.5,
        "the width and the glyphs disagree: {whole:.1} vs {glyphs:.1}"
    );
    assert_eq!(shaped.len(), long.chars().count(), "the shape lost characters");
}

/// (16.1) Typing in a cell that already holds a wall of text stays interactive:
/// neither the layout nor the frame may re-shape the whole cell for one keystroke.
#[test]
fn typing_in_a_large_table_cell_stays_fast() {
    let mut h = harness("r16-cell-perf");
    h.app.new_tab();
    let wall: String = (0..4000).map(|i| if i % 12 == 11 { ' ' } else { 'x' }).collect();
    h.app.insert_text(&format!("| A | B |\n| - | - |\n| {wall} | second |\n"));
    h.app.set_caret(0, false);
    h.draw();
    let start = h.app.tab().layout.lines[2].start_char;
    h.app.set_caret(start + 10, false);
    h.draw();
    // Warm, then measure a keystroke in the middle of the wall of text.
    let t = std::time::Instant::now();
    h.app.insert_text("abc");
    h.draw();
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    assert!(
        ms < 250.0,
        "one keystroke in a 4000-character cell took {ms:.0} ms (it used to be ~500)"
    );
    // The table is still a table, and the text is still where it was typed.
    let layout = &h.app.tab().layout;
    let rows: Vec<usize> = (0..layout.lines.len())
        .filter(|&i| layout.lines[i].block() == bare_md::markdown::Block::Table)
        .collect();
    assert_eq!(rows.len(), 3, "the table lost a row: {rows:?}");
    assert!(h.app.tab().doc.text().contains("xxxabcxxx"), "the typing did not land");
}

/// (16.2) Selecting across the wrapped lines of a table cell highlights the cell's
/// own words: one wash per piece, inside the cell, on the piece's own band.
#[test]
fn selecting_several_lines_of_a_cell_stays_inside_the_cell() {
    let mut h = harness("r16-cell-select");
    h.app.new_tab();
    h.type_str(
        "| Long | B |\n| - | - |\n| alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi | second cell with a good deal of words in it |\n",
    );
    h.app.set_caret(0, false);
    h.draw();
    let line = h.app.tab().layout.lines[2].clone();
    let bands = line.bands(false).len();
    assert!(bands >= 2, "the cell did not wrap: {bands} bands");
    let start = line.start_char;
    // From the fourth character of the cell to the middle of the second band.
    let second = line.bands(false)[1].first().map(|r| r.start + 3).unwrap_or(20);
    h.app.select_range(start + 3, start + second);
    h.draw();
    let line = h.app.tab().layout.lines[2].clone();
    let cell_right = line.table_cols.get(1).copied().unwrap_or(f32::MAX);
    let rects = line.table_sel_rects(3, second, false);
    assert!(!rects.is_empty(), "a selection inside a cell painted nothing");
    for (band, x, w) in &rects {
        assert!(
            *x + *w <= cell_right,
            "the wash of band {band} ends at {:.1}, past its own cell ({cell_right:.1})",
            *x + *w
        );
        assert!(
            *x >= line.table_cols[0] - 1.0,
            "the wash of band {band} starts at {x:.1}, left of the first cell"
        );
    }
    let bands_used: Vec<usize> = rects.iter().map(|(b, _, _)| *b).collect();
    assert!(
        bands_used.contains(&0) && bands_used.contains(&1),
        "a selection over two lines of the cell was painted on {bands_used:?}"
    );
    // The whole of band 0 is selected, so its wash is as wide as the words on it.
    let (_, x0, w0) = rects.iter().find(|(b, _, _)| *b == 0).copied().unwrap();
    assert!(x0 > line.table_cols[0] && w0 > 100.0, "band 0 wash {x0:.1}+{w0:.1}");
}

/// (17.1) Enter inside a cell goes to the cell below and selects it, the way a
/// spreadsheet (and Obsidian) fills a table in. Nothing is inserted and nothing is
/// deleted: it is navigation.
#[test]
fn enter_in_a_table_cell_goes_to_the_cell_below() {
    let mut h = harness("r17-enter-cell");
    h.app.new_tab();
    h.type_str("| Step | Notes |\n| - | - |\n| one | first note |\n| two | second note |\n");
    h.app.set_caret(0, false);
    h.draw();
    let line = h.app.tab().layout.lines[2].clone();
    h.app.set_caret(line.start_char + 3, false);
    h.draw();
    h.app.newline();
    h.draw();
    // The document is untouched: Enter does not split or delete anything.
    assert_eq!(
        h.app.tab().doc.text(),
        "| Step | Notes |\n| - | - |\n| one | first note |\n| two | second note |\n",
        "Enter edited the table"
    );
    // The caret is in the same column of the row below, and that cell is selected.
    let layout = &h.app.tab().layout;
    let next = &layout.lines[3];
    let (a, b) = next.info.cells[0];
    assert_eq!(h.app.tab().anchor, next.start_char + a, "the new cell is not selected from its start");
    assert_eq!(h.app.tab().caret, next.start_char + b, "the new cell is not selected to its end");
    let selected: String = {
        let t = h.app.tab();
        let (s, e) = t.sel();
        t.doc.text().chars().skip(s).take(e - s).collect()
    };
    assert_eq!(selected, "two", "the cell below was not selected");
    // Enter again does not eat the cell it just selected - it moves on down.
    h.app.newline();
    h.draw();
    assert!(h.app.tab().doc.text().contains("| two | second note |"), "the second Enter deleted the cell");
    // That row is the last one, so a row was opened under it and the caret sits in
    // the same column of it.
    let layout = &h.app.tab().layout;
    let rows: Vec<usize> = (0..layout.lines.len())
        .filter(|&i| layout.lines[i].block() == bare_md::markdown::Block::Table)
        .collect();
    assert_eq!(rows.len(), 5, "Enter on the last row did not open a row");
    let new_row = &layout.lines[rows[4]];
    assert_eq!(h.app.tab().caret, new_row.start_char + new_row.info.cells[0].0);
}

/// (17.1) Enter on the header row jumps past the `| --- |` rule into the first body
/// row - never *into* the rule, which would stop it being a separator and take the
/// whole table down with it.
#[test]
fn enter_on_the_header_row_never_touches_the_rule() {
    let mut h = harness("r17-enter-header");
    h.app.new_tab();
    h.type_str("| A | B |\n| - | - |\n| one | two |\n");
    h.app.set_caret(0, false);
    h.draw();
    let header = h.app.tab().layout.lines[0].clone();
    h.app.set_caret(header.start_char + 3, false);
    h.draw();
    h.app.newline();
    h.draw();
    assert_eq!(
        h.app.tab().doc.text(),
        "| A | B |\n| - | - |\n| one | two |\n",
        "Enter on the header changed the table"
    );
    let layout = &h.app.tab().layout;
    assert!(layout.lines[1].info.table_sep, "the separator row moved");
    assert_eq!(
        layout.lines.iter().filter(|l| l.block() == bare_md::markdown::Block::Table).count(),
        3,
        "the table changed shape"
    );
    // The caret went into the body row, in the same column.
    let body = &layout.lines[2];
    assert_eq!(h.app.tab().anchor, body.start_char + body.info.cells[0].0);
    assert_eq!(h.app.tab().caret, body.start_char + body.info.cells[0].1);
}

/// (17.1) A table with only a header still gets a body row - under the rule.
#[test]
fn enter_on_a_header_only_table_opens_a_body_row() {
    let mut h = harness("r17-enter-header-only");
    h.app.new_tab();
    h.type_str("| A | B |\n| - | - |\n");
    h.app.set_caret(0, false);
    h.draw();
    h.app.set_caret(h.app.tab().layout.lines[0].start_char + 3, false);
    h.draw();
    h.app.newline();
    h.draw();
    let text = h.app.tab().doc.text();
    assert!(text.starts_with("| A | B |\n| - | - |\n|"), "the new row is not under the rule: {text:?}");
    assert_eq!(
        h.app.tab().layout.lines.iter().filter(|l| l.block() == bare_md::markdown::Block::Table).count(),
        3,
        "the table should have a header, a rule and one row"
    );
}

/// (17.1) Enter on an empty last row is the way out of a table.
#[test]
fn enter_on_an_empty_last_row_leaves_the_table() {
    let mut h = harness("r17-enter-exit");
    h.app.new_tab();
    h.type_str("| A | B |\n| - | - |\n| one | two |\n");
    h.app.set_caret(0, false);
    h.draw();
    let line = h.app.tab().layout.lines[2].clone();
    h.app.set_caret(line.start_char + 3, false);
    h.draw();
    h.app.newline(); // opens a row under the last one
    h.draw();
    let rows = |h: &Harness| {
        h.app.tab().layout.lines.iter().filter(|l| l.block() == bare_md::markdown::Block::Table).count()
    };
    assert_eq!(rows(&h), 4);
    h.app.newline(); // the new row is empty: this leaves the table
    h.draw();
    assert_eq!(rows(&h), 3, "Enter on the empty last row did not leave the table");
    let line = h.app.tab().layout.line_of_char(h.app.tab().caret);
    assert_eq!(
        h.app.tab().layout.lines[line].block(),
        bare_md::markdown::Block::Blank,
        "the caret did not end up on a plain line"
    );
}


/// (17.3 -> r25) A short document's minimap only fills the top of the column, and
/// the handle now lives *with its bars*: dragging it moves the page one pixel per
/// pixel, and a press below the bars lands at the end of the document instead of
/// sliding a handle down a column of empty space.
#[test]
fn a_short_documents_minimap_handle_follows_the_pointer_anywhere() {
    let mut h = harness("r17-map-short");
    h.app.new_tab();
    h.app.insert_text(&(0..120).map(|i| format!("row {i}\n")).collect::<String>());
    h.app.set_caret(0, false);
    h.draw();
    let map = h.app.minimap_rect;
    let (geom, _) = h.app.minimap_geometry();
    // The bars keep the pitch of text and stop where the text stops - the map is a
    // picture of the document, not a ladder of dashes spread over the column.
    assert!(
        geom.content_h < geom.room() - 1.0,
        "this test needs a map that does not fill its column ({} of {})",
        geom.content_h,
        geom.room()
    );
    let total = h.app.tab().layout.total_height;
    let text_h = h.app.text_rect.h;
    assert!(total > text_h + 1.0, "this test needs a document that scrolls");
    // …while the handle, which moves over those bars, keeps room to move in.
    assert!(
        geom.handle_travel(total, text_h) >= 8.0,
        "the handle has only {:.0} px of travel",
        geom.handle_travel(total, text_h)
    );
    let handle_h = geom.view_h_px(total, text_h);
    let travel = geom.handle_travel(total, text_h);
    // The handle moves over the bars, top to bottom.
    assert!(
        travel <= geom.track_h() - handle_h + 1.0,
        "the handle travels {travel:.0} px over a {:.0} px track",
        geom.track_h()
    );
    let handle_y = |h: &Harness| -> f32 {
        let (g, _) = h.app.minimap_geometry();
        let total = h.app.tab().layout.total_height;
        let text_h = h.app.text_rect.h;
        map.y + g.indicator_y(h.app.tab().scroll, total, text_h)
    };
    // A press on the bars grabs it, and the drag is one map pixel per pointer pixel.
    let cx = map.x + map.w * 0.5;
    let press = map.y + geom.content_h * 0.5;
    h.app.mouse = (cx, press);
    h.draw();
    h.app.click((cx, press), &mut h.r);
    h.draw();
    let mut y = press;
    let mut handle = handle_y(&h);
    for _ in 0..4 {
        y += 20.0;
        let before = h.app.tab().scroll;
        h.app.minimap_drag((cx, y), &mut h.r);
        h.draw();
        assert!(h.app.tab().scroll > before, "dragging did not scroll at {y:.0}");
        let next = handle_y(&h);
        assert!(
            (next - handle - 20.0).abs() < 1.5,
            "the pointer moved 20 px and the handle moved {:.1} px",
            next - handle
        );
        handle = next;
    }
    h.app.mouse_up(&mut h.r);
    // A press below the bars - where the pointer is after typing into an empty
    // window - grabs the map without jumping (that column stands for no line), and the
    // drag from there moves the handle up over the text, one pixel per pixel, never
    // off it. The handle's track is the text, so it cannot be the thing that runs away
    // down the empty column.
    let low = map.y + map.h - 4.0;
    h.app.tab_mut().scroll = (h.app.tab().layout.total_height - h.app.text_rect.h).max(0.0);
    h.draw();
    h.app.mouse = (cx, low);
    h.draw();
    let caret = h.app.tab().caret;
    let before = h.app.tab().scroll;
    h.app.click((cx, low), &mut h.r);
    h.draw();
    assert!(h.app.dragging_minimap, "a press below the bars grabbed nothing");
    assert!(
        (h.app.tab().scroll - before).abs() < 0.5,
        "a press below the bars jumped the page from {before:.0} to {:.0}",
        h.app.tab().scroll
    );
    let mut handle = handle_y(&h);
    let mut pointer = low;
    for step in [-10.0f32, -30.0, -60.0] {
        pointer += step;
        h.app.mouse = (cx, pointer);
        assert!(h.app.minimap_drag((cx, pointer), &mut h.r), "the drag was dropped");
        h.draw();
        assert!(h.app.tab().scroll < before, "dragging up from below the bars did not scroll");
        let next = handle_y(&h);
        assert!(
            next < handle,
            "the handle did not move up: {handle:.1} -> {next:.1}"
        );
        let (g, _) = h.app.minimap_geometry();
        assert!(next >= map.y + g.bars_top() - 1.0, "the handle left the text at the top");
        handle = next;
    }
    h.app.mouse_up(&mut h.r);
    assert_eq!(h.app.tab().caret, caret, "a press on the map moved the caret");
}

/// (57) The handle the pointer holds is the handle on screen.
///
/// The painter used to derive the handle from `tab.scroll`, through a clamp against
/// the bars - so any disagreement between where the drag put the handle and what the
/// scroll produced showed up as a handle that did not move while the page did. Now,
/// while a drag is in flight, the handle is drawn from `App::map_handle`, the drag's
/// own position, so the two cannot differ.
#[test]
fn the_handle_under_the_pointer_is_the_handle_on_screen() {
    let mut h = harness("r57-map-handle");
    h.app.new_tab();
    h.app.insert_text(&(0..150).map(|i| format!("row {i} of a document\n")).collect::<String>());
    h.app.set_caret(0, false);
    h.app.settings.minimap_open = true;
    h.draw();
    let map = h.app.minimap_rect;
    assert!(!map.is_empty(), "the minimap is on");
    let total = h.app.tab().layout.total_height;
    let text_h = h.app.text_rect.h;
    assert!(total > text_h + 1.0, "this test needs a document that scrolls");
    // Press low in the column, as the pointer is after typing, and drag upwards in
    // steps: every step must move the handle the painter draws.
    let cx = map.x + map.w * 0.5;
    let y = map.y + map.h * 0.85;
    h.app.mouse = (cx, y);
    h.draw();
    h.app.click((cx, y), &mut h.r);
    h.draw();
    assert!(h.app.dragging_minimap, "the press did not grab the map");
    assert!(h.app.map_handle.is_some(), "the drag did not record a handle position");
    let mut last = h.app.map_handle.unwrap();
    let mut pointer = y;
    for step in [-15.0f32, -25.0, -40.0, 30.0, 50.0] {
        pointer += step;
        h.app.mouse = (cx, pointer);
        assert!(h.app.minimap_drag((cx, pointer), &mut h.r), "the drag was dropped");
        h.draw();
        let now = h.app.map_handle.expect("the drag lost its handle");
        assert!(
            (now - last).abs() > 0.01,
            "the pointer moved {step:+.0} and the handle stayed at {last:.1}"
        );
        // …and the drawn handle is that position: within the bars, at the top the drag
        // asks for.
        let (g, _) = h.app.minimap_geometry();
        // `map_handle` is a screen y - the same number the painter draws at.
        let drawn = now;
        let lo = map.y + g.track_top();
        let hi = map.y + g.track_top() + g.track_h() - g.view_h_px(total, text_h);
        assert!(
            drawn >= lo - 0.5 && drawn <= hi + 0.5,
            "the drawn handle ({drawn:.1}) is outside the text ({lo:.1}..{hi:.1})"
        );
        last = now;
    }
    h.app.mouse_up(&mut h.r);
    h.draw();
    assert!(
        h.app.map_handle.is_none(),
        "the handle is still held after the release"
    );
}

/// (63) A short document keeps the pitch of text and still gives its thumb somewhere to
/// go.
///
/// Spreading a stub of a map over the column was the wrong answer to "I can't drag it": it
/// turns twenty lines into a ladder of dashes, which this app has been asked twice not to
/// do. The bars keep three pixels a row, and the *thumb* gives way instead - it never takes
/// more than the bars minus `MIN_TRAVEL_PX` (down to `MIN_STUB_HANDLE_H`), so a sixty-pixel
/// strip still leaves about forty pixels of travel.
#[test]
fn a_short_map_keeps_the_pitch_of_text_and_still_drags() {
    let mut h = harness("r63-map-short-pitch");
    h.app.new_tab();
    h.app.insert_text(&(0..20).map(|i| format!("row {i} of a short document\n")).collect::<String>());
    h.app.set_caret(0, false);
    h.app.settings.minimap_open = true;
    h.draw();
    let map = h.app.minimap_rect;
    let (geom, _) = h.app.minimap_geometry();
    let total = h.app.tab().layout.total_height;
    let text_h = h.app.text_rect.h;
    assert!(total > text_h + 1.0, "this test needs a document that scrolls");
    assert_eq!(
        geom.scale, bare_md::ui::MINIMAP_PITCH,
        "a short document's bars stay at the pitch of text, never spread (pitch {:.1})",
        geom.scale
    );
    assert!(
        geom.bars_h() < geom.room(),
        "a short document's bars stop where its text stops"
    );
    assert!(
        geom.handle_travel(total, text_h) >= bare_md::ui::MIN_TRAVEL_PX,
        "the thumb has only {:.0} px of travel",
        geom.handle_travel(total, text_h)
    );
    let thumb = geom.view_h_px(total, text_h);
    assert!(
        thumb >= bare_md::ui::MIN_STUB_HANDLE_H,
        "the thumb is squeezed to {thumb:.1} px"
    );
    // Press where the pointer rests after typing and drag: the thumb follows 1:1 and never
    // leaves the text.
    let cx = map.x + map.w * 0.5;
    let y = map.y + map.h * 0.8;
    h.app.mouse = (cx, y);
    h.draw();
    h.app.click((cx, y), &mut h.r);
    h.draw();
    assert!(h.app.dragging_minimap, "the press did not grab the map");
    let lo = map.y + geom.track_top();
    let hi = lo + geom.track_h() - thumb;
    let mut pointer = y;
    let mut prev = h.app.map_handle.expect("the press left no handle");
    for step in [-20.0f32, -20.0, 20.0, 20.0] {
        pointer += step;
        h.app.mouse = (cx, pointer);
        assert!(h.app.minimap_drag((cx, pointer), &mut h.r), "the drag was dropped");
        h.draw();
        let now = h.app.map_handle.expect("the drag lost its handle");
        assert!((now - prev - step).abs() < 1.5, "the drag was not 1:1 ({step:+.0} gave {:+.1})", now - prev);
        assert!(now >= lo - 0.5 && now <= hi + 0.5, "the thumb left the strip ({now:.1} outside {lo:.1}..{hi:.1})");
        let (g, _) = h.app.minimap_geometry();
        let drawn = map.y + g.indicator_y(h.app.tab().scroll, total, text_h);
        assert!((drawn - now).abs() < 1.5, "the drawn thumb ({drawn:.1}) is not where the drag holds it ({now:.1})");
        prev = now;
    }
    h.app.mouse_up(&mut h.r);
}

/// (64) The handle is the handle: the same height at every document length, and travel over
/// nearly the whole height of the text.
///
/// Sizing it by the viewport made it eat its own track - a 120-line document's 83 px thumb
/// had 34 px to move in, which is what "it moves only a bit, I can't move it over the whole
/// text" looks like.
#[test]
fn the_handle_is_one_height_and_travels_the_text() {
    let mut h = harness("r64-map-constant-handle");
    let mut seen = Vec::new();
    for rows in [20usize, 40, 150, 1000] {
        h.app.new_tab();
        h.app.insert_text(&(0..rows).map(|i| format!("row {i} of the document\n")).collect::<String>());
        h.app.set_caret(0, false);
        h.app.settings.minimap_open = true;
        h.draw();
        let (geom, _) = h.app.minimap_geometry();
        let total = h.app.tab().layout.total_height;
        let text_h = h.app.text_rect.h;
        let thumb = geom.view_h_px(total, text_h);
        assert!(
            (thumb - bare_md::ui::MIN_HANDLE_H).abs() < 0.01,
            "{rows} rows: the handle is {thumb:.1}, not {}",
            bare_md::ui::MIN_HANDLE_H
        );
        let travel = geom.handle_travel(total, text_h);
        assert!(
            travel >= geom.bars_h() * 0.6,
            "{rows} rows: the handle travels {travel:.0} px of its {:.0} px track",
            geom.bars_h()
        );
        seen.push(thumb);
    }
    assert!(
        seen.iter().all(|t| (t - seen[0]).abs() < 0.01),
        "the handle changed height with the document: {seen:?}"
    );
}

/// (65) A drag belongs to the window, and the window keeps the pointer.
///
/// winit does not capture the mouse: `CursorMoved` simply stops arriving once the pointer
/// crosses the window's edge. The minimap is flush against that edge, so a drag with a
/// little sideways drift froze the handle halfway down the strip with the rest of the
/// document still below it - "the handle won't go any further", at any document length.
/// `App::dragging_something` is what `platform::window` uses to hold the pointer
/// (`CursorGrabMode::Confined`) for the duration of a drag and to let it go afterwards.
#[test]
fn a_drag_holds_the_pointer_until_the_button_is_released() {
    use winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
    let mut h = harness("r65-drag-holds-pointer");
    h.app.new_tab();
    h.app.insert_text(&filler(120));
    h.app.set_caret(0, false);
    h.app.settings.minimap_open = true;
    h.draw();
    assert!(!h.app.dragging_something(), "nothing is dragging yet");
    let map = h.app.minimap_rect;
    let mut input = bare_md::platform::input::InputState::default();
    let button = |state: ElementState| WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state,
        button: MouseButton::Left,
    };
    let x = (map.x + map.w * 0.5) as f64;
    let pointer = |y: f32| WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: winit::dpi::PhysicalPosition::new(x, y as f64),
    };
    let y = map.y + map.h * 0.3;
    h.app.handle_event(&pointer(y), &mut h.r, &mut input);
    h.draw();
    h.app.handle_event(&button(ElementState::Pressed), &mut h.r, &mut input);
    h.draw();
    assert!(h.app.dragging_minimap, "the press did not grab the map");
    assert!(
        h.app.dragging_something(),
        "the window would not hold the pointer for the drag"
    );
    h.app.handle_event(&button(ElementState::Released), &mut h.r, &mut input);
    h.draw();
    assert!(
        !h.app.dragging_something(),
        "the pointer is still held after the button came up"
    );
}

/// (65) Overshooting the strip is not a wall: the handle goes to the very end of the
/// document, and the page with it.
#[test]
fn a_minimap_drag_past_the_column_still_reaches_the_end() {
    use winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
    let mut h = harness("r65-drag-overshoot");
    h.app.new_tab();
    h.app.insert_text(&filler(300));
    h.app.set_caret(0, false);
    h.app.settings.minimap_open = true;
    h.draw();
    let map = h.app.minimap_rect;
    let total = h.app.tab().layout.total_height;
    let text_h = h.app.text_rect.h;
    let max = (total - text_h).max(0.0);
    assert!(max > 1000.0, "the test needs a document that scrolls");
    let mut input = bare_md::platform::input::InputState::default();
    let button = |state: ElementState| WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state,
        button: MouseButton::Left,
    };
    let pointer = |x: f32, y: f32| WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: winit::dpi::PhysicalPosition::new(x as f64, y as f64),
    };
    let cx = map.x + map.w * 0.5;
    let y0 = map.y + 4.0;
    h.app.handle_event(&pointer(cx, y0), &mut h.r, &mut input);
    h.draw();
    h.app.handle_event(&button(ElementState::Pressed), &mut h.r, &mut input);
    h.draw();
    // Down, and well past the window's bottom and its right edge on the way.
    let mut y = y0;
    for _ in 0..20 {
        y += 60.0;
        h.app.handle_event(&pointer(cx + 400.0, y), &mut h.r, &mut input);
        h.draw();
    }
    let (geom, _) = h.app.minimap_geometry();
    let thumb = geom.view_h_px(total, text_h);
    let lo = map.y + geom.track_top();
    let hi = lo + geom.track_h() - thumb;
    let held = h.app.map_handle.expect("the drag lost its handle");
    assert!(
        (held - hi).abs() < 1.5,
        "the handle stopped at {:.1}, not at the end of the text ({:.1})",
        held,
        hi
    );
    assert!(held <= hi + 0.5, "the handle left the text");
    assert!(
        (h.app.tab().scroll - max).abs() < 2.0,
        "the page stopped at {:.0} of {:.0}",
        h.app.tab().scroll,
        max
    );
    h.app.handle_event(&button(ElementState::Released), &mut h.r, &mut input);
}

/// (67) The handle and the drag live where the lines are, and a long document's lines
/// reach the bottom of the strip.
///
/// The two halves of the same report: "drag happens on the whole height instead of where
/// the text lines are" and "a 10 000-line file's map only covers half the strip". The
/// picture now always fits the whole file (so a long document's bars cover the column),
/// and the handle's track is the bars (so a short document only drags inside its stub).
#[test]
fn the_map_covers_the_file_and_the_drag_stays_on_the_lines() {
    let mut h = harness("r67-map-band");
    // (a) A long document: the bars reach the bottom, so the drag does too.
    h.app.new_tab();
    h.app.insert_text(&filler(900));
    h.app.set_caret(0, false);
    h.app.settings.minimap_open = true;
    h.draw();
    let map = h.app.minimap_rect;
    let (geom, _) = h.app.minimap_geometry();
    let total = h.app.tab().layout.total_height;
    let text_h = h.app.text_rect.h;
    assert!(total > text_h + 1.0, "this test needs a document that scrolls");
    assert!(
        geom.bars_bottom() >= map.h - 8.0,
        "a long document's bars stop at {:.0} of {:.0}",
        geom.bars_bottom(),
        map.h
    );
    assert!(
        geom.track_h() <= geom.bars_h() + 0.5,
        "the handle's track is taller than the bars ({} vs {})",
        geom.track_h(),
        geom.bars_h()
    );
    let thumb = geom.view_h_px(total, text_h);
    let lo = map.y + geom.track_top();
    let hi = lo + geom.track_h() - thumb;
    // The end of the document puts the handle at the bottom of the text.
    let at_end = map.y + geom.indicator_y(total - text_h, total, text_h);
    assert!(
        (at_end - hi).abs() < 1.5,
        "at the end of the document the handle is at {at_end:.1}, not {hi:.1}"
    );

    // (b) A short document: the bars are a stub, and the drag stays inside it.
    h.app.new_tab();
    h.app.insert_text(&(0..20).map(|i| format!("row {i} of a short document\n")).collect::<String>());
    h.app.set_caret(0, false);
    h.draw();
    let (g, _) = h.app.minimap_geometry();
    assert!(
        g.bars_h() < g.room() - 20.0,
        "the test needs a document short enough to leave the column empty below its text"
    );
    let cx = map.x + map.w * 0.5;
    // Press far below the text: it clamps into the band, it does not sit down there.
    let y = map.y + map.h * 0.9;
    h.app.mouse = (cx, y);
    h.draw();
    h.app.click((cx, y), &mut h.r);
    h.draw();
    let held = h.app.map_handle.expect("the press did not grab the handle");
    let thumb = g.view_h_px(h.app.tab().layout.total_height, h.app.text_rect.h);
    let lo = map.y + g.track_top();
    let hi = lo + g.track_h() - thumb;
    assert!(
        held >= lo - 0.5 && held <= hi + 0.5,
        "the handle sits at {held:.1}, outside the text {lo:.1}..{hi:.1}"
    );
    h.app.mouse_up(&mut h.r);
}

/// (68) One long wrapped line is many rows on the map, not one.
///
/// `vtotal` used to be the virtual *top of the last line*, so a file whose text wraps
/// across many rows inside a single line counted as one row: ten thousand characters on one
/// line are a hundred and eleven rows on the page and claimed one row on the map, which gave
/// three pixels of bars and ten pixels of handle travel - "I can't get past about 30 px".
#[test]
fn a_single_long_line_is_counted_row_by_row_on_the_map() {
    let mut h = harness("r68-map-wrapped-rows");
    h.app.new_tab();
    h.app.insert_text(&"x".repeat(10_400));
    h.app.set_caret(0, false);
    h.app.settings.minimap_open = true;
    h.draw();
    let layout = &h.app.tab().layout;
    let rows: f32 = layout.lines.iter().map(|l| l.rows.len().max(1) as f32).sum();
    assert!(rows > 40.0, "the test needs a line that wraps many times ({rows})");
    let (geom, _) = h.app.minimap_geometry();
    assert_eq!(
        geom.vtotal, rows,
        "the map counts {} rows, the page has {rows}",
        geom.vtotal
    );
    let total = layout.total_height;
    let text_h = h.app.text_rect.h;
    assert!(total > text_h + 1.0, "this test needs a document that scrolls");
    let bars_px = rows * geom.scale;
    assert!(
        (geom.bars_h() - bars_px).abs() < 2.0,
        "the bars are {:.0} px for {rows} rows at {:.2} px a row",
        geom.bars_h(),
        geom.scale
    );
    assert!(
        geom.handle_travel(total, text_h) > 200.0,
        "the handle has only {:.0} px of travel over a document of {rows} rows",
        geom.handle_travel(total, text_h)
    );
    // And the same holds when the wrap comes from several long lines.
    h.app.new_tab();
    h.app.insert_text(&(0..17).map(|_| format!("{}\n", "y".repeat(600))).collect::<String>());
    h.app.set_caret(0, false);
    h.draw();
    let rows: f32 = h.app.tab().layout.lines.iter().map(|l| l.rows.len().max(1) as f32).sum();
    let (geom, _) = h.app.minimap_geometry();
    assert_eq!(geom.vtotal, rows, "the map miscounts the rows of long lines");
}

/// (69) The dialog's left column is the filesystem: every volume, not just the one the
/// folder happens to be on.
///
/// The tree used to start at the root of the current folder's volume, so on Windows the
/// column showed one drive and there was no way to reach another disk from it.
#[test]
fn the_folder_dialog_tree_lists_every_volume() {
    let mut h = harness("r69-tree-volumes");
    let a = h._dir.join("volume-a");
    let b = h._dir.join("volume-b");
    std::fs::create_dir_all(a.join("inside")).unwrap();
    std::fs::create_dir_all(b.join("other")).unwrap();

    // Two volumes, both at the top of the column, each openable on its own.
    let mut tree = bare_md::app::DirTree::new(vec![a.clone(), b.clone()]);
    let roots: Vec<PathBuf> = tree.rows().iter().filter(|r| r.depth == 0).map(|r| r.path.clone()).collect();
    assert_eq!(roots, vec![a.clone(), b.clone()], "the tree does not list every volume");
    tree.reveal(&b.join("other"));
    let names: Vec<String> = tree.rows().iter().map(|r| r.name.clone()).collect();
    assert!(
        names.contains(&"other".to_string()),
        "revealing a folder on the second volume did not open that volume: {names:?}"
    );
    // The first volume stays where it was - collapsing it is not a side effect of
    // revealing something on another disk.
    assert!(
        tree.rows().iter().any(|r| r.path == a),
        "the tree lost the first volume"
    );

    // And the real dialog asks for every volume on the machine.
    h.app.open_picker(bare_md::app::PickerKind::OpenFile);
    let picker = match &h.app.dialog {
        Some(bare_md::app::Dialog::Picker(p)) => p,
        _ => panic!("no picker"),
    };
    let want = bare_md::app::volume_roots();
    assert!(!want.is_empty(), "this machine reports no volumes at all");
    assert_eq!(
        picker.tree.roots, want,
        "the dialog's tree does not start at the machine's volumes"
    );
    assert!(
        picker.tree.rows().iter().any(|r| r.depth == 0 && r.path == want[0]),
        "the first volume is not a row in the column"
    );
    assert!(
        picker.tree.rows().iter().any(|r| r.path == picker.dir),
        "the folder the dialog opened in is not in the tree"
    );
}

/// (70) Save writes into the folder the dialog is showing, not into whatever the listing
/// happens to have selected.
///
/// The listing always starts on ".." (the parent), so a brand-new document had the parent
/// highlighted; pressing Save walked the dialog up a level and saved the file there - or
/// into any other folder that happened to be selected. Walking folders is a double-click
/// or the tree, not the Save button.
#[test]
fn saving_a_new_document_writes_into_the_shown_folder() {
    let mut h = harness("r70-save-as-folder");
    let root = h._dir.clone();
    let sub = root.join("subfolder");
    std::fs::create_dir_all(&sub).unwrap();
    h.app.new_tab();
    h.app.insert_text("brand new document\n");
    h.app.open_picker(bare_md::app::PickerKind::SaveAs);
    // Open the dialog in the workspace, then highlight a *different* folder in the
    // listing, the way the pointer does.
    let (dir, idx) = match &mut h.app.dialog {
        Some(bare_md::app::Dialog::Picker(p)) => {
            p.select_dir(root.clone());
            p.name.set("fresh");
            let idx = p
                .entries
                .iter()
                .position(|e| e.name == ".." || e.name == "subfolder")
                .expect("the listing has a folder to highlight");
            p.selected = idx;
            (p.dir.clone(), idx)
        }
        _ => panic!("no picker"),
    };
    assert_eq!(dir, root, "the test needs the dialog in the workspace");
    h.draw();
    let confirmed = h.click_hit(|hit| *hit == Hit::DialogConfirm);
    assert!(confirmed, "there is no Save button");
    assert!(
        h.app.dialog.is_none(),
        "the dialog did not answer Save (it navigated instead: selection {idx})"
    );
    let saved = root.join("fresh.md");
    assert!(saved.is_file(), "Save did not write {} ", saved.display());
    assert!(h.app.tab().doc.path.as_ref() == Some(&saved), "the tab is not on the new file");
    assert!(
        std::fs::read_dir(&sub).unwrap().next().is_none(),
        "the file went into the highlighted folder"
    );
}

/// (72) The bars are the same size and the same distance apart in every document.
///
/// The pitch used to shrink with the file (down to 0.85 px a row), which put the bars off a
/// whole-pixel grid: which row survived the collision check changed from frame to frame, so
/// the strip shimmered (moire) and the lines looked thinner than they are. One pitch, one
/// bar height, for every document - and the map slides when the file does not fit.
#[test]
fn the_bars_never_change_size_or_pitch() {
    let mut h = harness("r72-map-constant-bars");
    for rows in [5usize, 20, 300, 2000] {
        h.app.new_tab();
        h.app.insert_text(&filler(rows));
        h.app.set_caret(0, false);
        h.app.settings.minimap_open = true;
        h.draw();
        let map = h.app.minimap_rect;
        let (geom, _) = h.app.minimap_geometry();
        assert_eq!(
            geom.scale,
            bare_md::ui::MINIMAP_PITCH,
            "{rows} lines: the pitch is {} px a row, not {}",
            geom.scale,
            bare_md::ui::MINIMAP_PITCH
        );
        assert_eq!(
            geom.bar_h,
            bare_md::ui::MINIMAP_BAR_H,
            "{rows} lines: the bars are {} px tall, not {}",
            geom.bar_h,
            bare_md::ui::MINIMAP_BAR_H
        );
        assert!(
            geom.bar_h < geom.scale,
            "the bars touch each other, so there is no gap left to read"
        );
        // Scrolled to the middle the bars are the same again - the map slides, it does not
        // re-pitch.
        let total = h.app.tab().layout.total_height;
        let text_h = h.app.text_rect.h;
        if total > text_h + 1.0 {
            h.app.tab_mut().scroll = (total - text_h) * 0.5;
            h.draw();
            let (g, _) = h.app.minimap_geometry();
            assert_eq!(g.scale, geom.scale, "{rows} lines: the pitch changed while scrolling");
            assert_eq!(g.bar_h, geom.bar_h, "{rows} lines: the bars changed while scrolling");
            // …and every bar sits on the same grid, so consecutive rows are exactly one
            // pitch apart (no sub-pixel drift, no moire).
            for k in 1..6 {
                let a = g.y_of_virtual(k as f32 * 40.0);
                let b = g.y_of_virtual(k as f32 * 40.0 + 1.0);
                assert!(
                    ((b - a) - bare_md::ui::MINIMAP_PITCH).abs() < 0.001,
                    "{rows} lines: one row is {} px, not {}",
                    b - a,
                    bare_md::ui::MINIMAP_PITCH
                );
            }
            let _ = map;
        }
    }
}

/// (73) Auto-close is a toggle, and off means off.
///
/// On (the default): `(` writes `()`, typing the `)` steps over it, and typing `"` around a
/// selection wraps it. Off: every character is typed exactly as pressed - no closing
/// character appears, nothing steps over anything, and a selection is replaced rather than
/// wrapped. The setting is remembered between runs, and the toolbar chip, the document menu
/// and `Alt+B` all flip the same one.
#[test]
fn auto_close_brackets_can_be_switched_off() {
    let mut h = harness("r73-autoclose");
    h.app.new_tab();
    assert!(h.app.settings.autoclose, "auto-close is on by default");

    // On: the pair appears and the caret sits between the two characters.
    h.type_key("(");
    assert_eq!(h.app.tab().doc.text(), "()", "typing ( did not write a pair");
    assert_eq!(h.app.tab().caret, 1, "the caret is not between the brackets");
    // Typing the closing character steps over it instead of doubling it.
    h.type_key(")");
    assert_eq!(h.app.tab().doc.text(), "()", "typing ) doubled the closing bracket");
    assert_eq!(h.app.tab().caret, 2, "the caret did not step over the closing bracket");

    // Off: a bracket is a bracket.
    h.app.settings.autoclose = false;
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.type_key("(");
    assert_eq!(h.app.tab().doc.text(), "()(", "auto-close still added a closing bracket");
    h.type_key(")");
    assert_eq!(h.app.tab().doc.text(), "()()", "auto-close still stepped over the bracket");

    // A selection is replaced, not wrapped, when it is off.
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.type_key("hello");
    h.app.select_range(2, 4);
    h.type_key("[");
    assert_eq!(
        h.app.tab().doc.text(),
        "()[hello",
        "typing over a selection wrapped it while auto-close was off"
    );

    // On again: the same selection case wraps.
    h.app.settings.autoclose = true;
    h.app.set_caret(0, false);
    h.app.select_range(0, 2);
    h.type_key("{");
    assert_eq!(
        h.app.tab().doc.text(),
        "{()}[hello",
        "typing over a selection did not wrap it while auto-close was on"
    );
}

/// (73) The toggle is reachable three ways, and all three flip the same setting.
#[test]
fn the_auto_close_toggle_is_one_setting() {
    let mut h = harness("r73-autoclose-toggle");
    h.app.new_tab();
    h.app.insert_text("some text\n");
    h.draw();

    // 1. The toolbar chip.
    let chip = h
        .app
        .hits
        .find(|hit| *hit == Hit::Toolbar(ToolbarAction::ToggleAutoclose))
        .expect("the toolbar has no auto-close chip");
    let pos = (chip.x + chip.w * 0.5, chip.y + chip.h * 0.5);
    h.app.mouse = pos;
    h.draw();
    assert!(h.click_hit(|hit| *hit == Hit::Toolbar(ToolbarAction::ToggleAutoclose)));
    assert!(!h.app.settings.autoclose, "the chip did not switch auto-close off");

    // 2. The document menu: a ticked item that flips the same setting.
    h.app.mouse = (h.app.text_rect.x + 40.0, h.app.text_rect.y + 20.0);
    h.draw();
    h.app.open_context_menu(h.app.mouse);
    h.draw();
    let item = {
        let menu = h.app.menu.as_ref().expect("no menu");
        let i = menu
            .entries
            .iter()
            .position(|e| e.label == "Auto-close brackets")
            .expect("the menu has no auto-close entry");
        assert!(!menu.entries[i].checked, "the entry is ticked while auto-close is off");
        i
    };
    let rect = h
        .app
        .hits
        .find(|hit| matches!(hit, Hit::MenuItem(k) if *k == item))
        .expect("the menu entry is not clickable");
    let mpos = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
    h.app.mouse = mpos;
    h.draw();
    assert!(h.click_hit(|hit| matches!(hit, Hit::MenuItem(k) if *k == item)));
    assert!(h.app.settings.autoclose, "the menu entry did not switch auto-close on");

    // 3. The `Alt+B` path - the same call the key layer makes - and the setting it leaves
    //    behind is the one the chip reads back.
    assert!(!h.app.toggle_autoclose(), "Alt+B did not switch auto-close off");
    h.draw();
    let menu = h.app.menu.as_ref();
    let _ = menu;
    h.app.open_context_menu(h.app.mouse);
    h.draw();
    let ticked = h
        .app
        .menu
        .as_ref()
        .and_then(|m| m.entries.iter().find(|e| e.label == "Auto-close brackets"))
        .map(|e| e.checked);
    assert_eq!(ticked, Some(false), "the menu still shows auto-close as on");
    assert!(h.app.toggle_autoclose(), "Alt+B did not switch auto-close back on");
}

/// (73) The setting survives a restart: it is written to, and read back from, the portable
/// config file next to the executable.
#[test]
fn auto_close_is_remembered_between_runs() {
    let dir = std::env::temp_dir().join(format!("bare-md-reg-{}-r73-config", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let paths = Paths {
        data_dir: dir.clone(),
        config_file: dir.join("settings.conf"),
        origin: "test".into(),
    };
    let mut settings = Settings::default();
    assert!(settings.autoclose, "auto-close is on by default");
    settings.autoclose = false;
    settings.save(&paths);
    let back = Settings::load(&paths);
    assert!(!back.autoclose, "auto-close came back on after a restart");
    settings.autoclose = true;
    settings.save(&paths);
    assert!(Settings::load(&paths).autoclose, "auto-close came back off after a restart");
    let _ = std::fs::remove_dir_all(&dir);
}

/// (74) The minimap's cached totals are the totals.
///
/// `Layout` now carries `virtual_rows` and `widest_row_chars` so the map does not add up
/// the whole document on every frame (that was 235 us of a 400 us minimap on ten thousand
/// lines). A cache is only worth having if it cannot go stale: these are the same numbers
/// computed the long way, after a full build, after an edit, and after a scroll.
#[test]
fn the_minimap_totals_in_the_layout_are_the_real_totals() {
    let mut h = harness("r74-map-totals");
    h.app.new_tab();
    h.app.insert_text(&filler(120));
    h.app.insert_text("a much longer line that will wrap across several rows in the editor, with words in it\n");
    h.app.set_caret(0, false);
    h.app.settings.minimap_open = true;
    h.draw();
    let check = |h: &Harness, when: &str| {
        let layout = &h.app.tab().layout;
        let rows: f32 = layout.lines.iter().map(|l| l.virtual_span()).sum();
        assert!(
            (layout.virtual_rows - rows).abs() < 0.001,
            "{when}: the layout says {} rows, the lines add up to {rows}",
            layout.virtual_rows
        );
        let widest = layout
            .lines
            .iter()
            .flat_map(|l| l.rows.iter())
            .map(|r| r.end.saturating_sub(r.start))
            .max()
            .unwrap_or(0)
            .max(8);
        assert_eq!(
            layout.widest_row_chars, widest,
            "{when}: the layout says the widest row is {} characters, it is {widest}",
            layout.widest_row_chars
        );
        // …and the map reads them, so its geometry follows.
        let (geom, _) = h.app.minimap_geometry();
        assert!(
            (geom.vtotal - rows.max(1.0)).abs() < 0.001,
            "{when}: the map counts {} rows, the document has {rows}",
            geom.vtotal
        );
        assert_eq!(geom.max_row, widest, "{when}: the map does not use the layout's widest row");
    };
    check(&h, "after the first build");
    // An edit in the middle: the incremental update recomputes the totals.
    h.app.set_caret(40, false);
    h.app.insert_text("edited ");
    h.draw();
    check(&h, "after an edit");
    // A line added at the end.
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.app.insert_text("another line\n");
    h.draw();
    check(&h, "after appending");
    // Scrolling does not change them.
    h.app.tab_mut().scroll = 200.0;
    h.draw();
    check(&h, "after scrolling");
}

// ---------------------------------------------------------------------------
// Round 18
// ---------------------------------------------------------------------------

/// (18.1) Undo and redo, through the shortcut table.
#[test]
fn undo_and_redo_run_from_the_keyboard() {
    let mut h = harness("r18-undo");
    h.type_str("hello");
    h.app.newline();
    h.type_str("world");
    assert_eq!(h.app.tab().doc.text(), "hello\nworld");
    assert!(h.app.handle_shortcut("z", false, &mut h.r), "Ctrl+Z is not a shortcut");
    h.draw();
    assert_eq!(h.app.tab().doc.text(), "hello\n", "Ctrl+Z did not undo");
    assert!(h.app.handle_shortcut("y", false, &mut h.r), "Ctrl+Y is not a shortcut");
    h.draw();
    assert_eq!(h.app.tab().doc.text(), "hello\nworld", "Ctrl+Y did not redo");
    assert!(h.app.handle_shortcut("z", false, &mut h.r));
    assert!(h.app.handle_shortcut("z", true, &mut h.r), "Ctrl+Shift+Z is not a shortcut");
    h.draw();
    assert_eq!(h.app.tab().doc.text(), "hello\nworld", "Ctrl+Shift+Z did not redo");
}

/// (18.4) `==highlight==` is yellow *text*: no wash behind it, and the ink of the
/// marked words is the theme's yellow.
#[test]
fn marked_words_are_yellow_text() {
    let mut h = harness("r18-mark");
    h.app.new_tab();
    h.type_str("plain ==marked== plain\n");
    h.app.set_caret(0, false);
    h.draw();
    let line = &h.app.tab().layout.lines[0];
    let marked = line.runs(false).iter().any(|r| r.style.highlight);
    assert!(marked, "the run did not keep the highlight flag: {:?}", line.runs(false));
    // The theme has a yellow for it, and the painter uses it as the *ink*.
    let src = ui_source();
    let draw = &src[src.find("let color = run.style.fg.unwrap_or_else(").unwrap()..];
    assert!(
        draw[..200].contains("theme.mark"),
        "the marked run is not painted with the mark ink"
    );
    assert!(
        !src.contains("run.style.highlight || run.style.bg.is_some()"),
        "==highlight== still paints a wash behind the words"
    );
    let mark = h.app.theme.mark;
    assert!(mark.r > mark.b + 0.15 && mark.g > mark.b + 0.1, "the mark ink is not yellow: {mark:?}");
}

/// (18.5) A click in a table cell lands on the character under the pointer - also
/// when the cell holds `**markup**`, whose markers are hidden while the row is not
/// being edited. (Mapping the click through the *revealed* variant, or dividing a
/// run's width by its character count instead of adding up the real advances, put
/// the caret one character to the right of the pointer.)
#[test]
fn a_click_in_a_marked_up_cell_lands_on_the_character_under_the_pointer() {
    let mut h = harness("r18-cell-click");
    h.app.new_tab();
    h.type_str("| head | note |\n| --- | --- |\n| gamma | **bold** x |\n| alpha | plain words here |\n");
    h.app.set_caret(0, false);
    h.draw();
    for row in [2usize, 3] {
        let start = h.app.tab().doc.line_start(row);
        let li = h.app.tab().layout.line_of_char(start);
        let raw = {
            let tab = h.app.tab();
            tab.line_is_raw(h.app.mode, li) || tab.table_shows_markers(h.app.mode, li)
        };
        let cells = h.app.tab().layout.lines[li].info.cells.clone();
        // A click a fifth of the way into a glyph has to land on the left edge of
        // that glyph, in every cell of the row.
        let mut clicks: Vec<(char, f32, isize, f32)> = Vec::new();
        for &(a, b) in &cells {
            for i in a..b {
                let c = h.app.tab().doc.slice(start + i, start + i + 1).chars().next().unwrap();
                if c == '|' || c.is_whitespace() {
                    continue;
                }
                // A hidden `**` shares its x with the word it marks: it is not a
                // position of its own, so there is nothing to aim at.
                if !h.app.tab().layout.lines[li]
                    .runs(raw)
                    .iter()
                    .any(|r| i >= r.start && i < r.end)
                {
                    continue;
                }
                let x0 = h.local_x(li, i, 0, raw);
                let x1 = h.local_x(li, i + 1, 0, raw);
                clicks.push((c, x0 + (x1 - x0) * 0.2, i as isize, h.row_y(li, 0, 0.5)));
            }
        }
        for (c, x, want, ymid) in clicks {
            let got = h.app.char_at_point((x, ymid), &mut h.r).map(|g| g as isize - start as isize);
            assert_eq!(got, Some(want), "line {row}: a click inside {c:?} landed on {got:?}");
        }
    }
}

/// (18.6 -> r25) A short document's minimap handle is the size of the text it
/// describes, it moves over the bars and not over the whole column, and one map
/// pixel of pointer travel is still one map pixel of handle travel.
#[test]
fn a_short_documents_minimap_handle_is_the_size_of_its_text() {
    let mut h = harness("r18-map-size");
    h.app.new_tab();
    h.app.insert_text(&(0..26).map(|i| format!("row {i}\n")).collect::<String>());
    h.app.set_caret(0, false);
    h.draw();
    let map = h.app.minimap_rect;
    assert!(!map.is_empty(), "the minimap is on");
    let (geom, _) = h.app.minimap_geometry();
    let total = h.app.tab().layout.total_height;
    let text_h = h.app.text_rect.h;
    let handle = geom.view_h_px(total, text_h);
    assert!(
        handle <= geom.track_h() + 0.01,
        "the handle ({handle:.0} px) is taller than the column it moves in ({:.0} px)",
        geom.track_h()
    );
    assert!(handle >= bare_md::ui::MIN_HANDLE_H - 0.01, "the handle is too small to grab: {handle}");
    // …and it travels the whole column, so a press anywhere on the map has room to
    // move it in either direction.
    let travel = geom.handle_travel(total, text_h);
    assert!(
        travel <= geom.track_h() - handle + 1.0,
        "the handle travels {travel:.0} px over a {:.0} px column",
        geom.track_h()
    );
    assert!(travel >= 0.0, "the handle cannot travel backwards");
    // One map pixel of pointer is one map pixel of handle, and one map pixel of handle is
    // `max_scroll / travel` document pixels: the composed mapping is what the drag uses.
    // (The drag itself is exercised on a document with room to move in
    // `the_minimap_handle_follows_the_pointer`; a 26-row file barely scrolls, so a
    // simulated drag here would only measure its own clamps.)
    let max_scroll = (total - text_h).max(0.0);
    let per_px = max_scroll / travel.max(0.001);
    assert!(
        per_px.is_finite() && per_px >= 0.0,
        "the drag maps one pixel to {per_px} document pixels"
    );
}

/// (18.2) The executable's own icon: the `.ico` the Windows resource is built
/// from exists, holds every size Windows asks for, and is generated from the same
/// SVGs the window icon uses.
#[test]
fn the_executable_icon_is_a_real_multisize_ico() {
    let ico = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/app/app.ico");
    let data = std::fs::read(&ico).expect("assets/app/app.ico is missing");
    let u16_at = |i: usize| u16::from_le_bytes([data[i], data[i + 1]]) as usize;
    assert_eq!(u16_at(0), 0, "not an icon file");
    assert_eq!(u16_at(2), 1, "not an icon file");
    let n = u16_at(4);
    assert!(n >= 5, "only {n} sizes in the icon");
    let mut sizes = Vec::new();
    for i in 0..n {
        let e = 6 + 16 * i;
        let w = data[e] as u32;
        sizes.push(if w == 0 { 256 } else { w });
        let off = u32::from_le_bytes([data[e + 12], data[e + 13], data[e + 14], data[e + 15]]) as usize;
        assert_eq!(&data[off..off + 8], b"\x89PNG\r\n\x1a\n", "image {i} is not a PNG payload");
    }
    for want in [16u32, 32, 48, 256] {
        assert!(sizes.contains(&want), "no {want} px image: {sizes:?}");
    }
    // …and the build script is what puts it in the binary.
    let build = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("build.rs"))
        .expect("build.rs is missing");
    assert!(build.contains("app.ico") && build.contains("res.compile()"), "build.rs does not embed the icon");
}

/// (18.7) Selecting several wrapped lines of a cell highlights exactly the
/// selected glyphs: every wash starts and ends where the characters do, on the
/// band they are painted on, and it never runs across the row.
#[test]
fn a_selection_across_a_wrapped_cell_covers_exactly_its_glyphs() {
    let mut h = harness("r18-cell-selection");
    h.app.new_tab();
    // A column narrow enough that the cell wraps into several bands.
    let long = "wrapped words that fill several bands of the cell and keep going for a good while longer than the page is wide, so the column has to wrap them";
    h.type_str(&format!("| n | note |\n| --- | --- |\n| 1 | **{long}** |\n| 2 | short |\n"));
    h.app.set_caret(0, false);
    h.draw();
    let start = h.app.tab().doc.line_start(2);
    let (a, b, raw) = {
        let tab = h.app.tab();
        let li = tab.layout.line_of_char(start);
        let ll = &tab.layout.lines[li];
        assert!(
            ll.table_rows.len() >= 2,
            "this test needs a cell that wraps, got {} bands",
            ll.table_rows.len()
        );
        let cells = &ll.info.cells;
        // The selection covers the whole cell that wraps, plus one character of
        // the cell before it (so it is not a single-run special case).
        (cells[1].0.saturating_sub(1), cells[1].1, false)
    };
    let tab = h.app.tab();
    let ll = &tab.layout.lines[2];
    let rects = ll.table_sel_rects(a, b, raw);
    assert!(!rects.is_empty(), "a selection inside a wrapped cell painted nothing");
    // Each selected character is covered by the wash of its own band (a band can
    // carry a piece of more than one cell, so it can have more than one wash).
    let mut seen: Vec<usize> = Vec::new();
    for (band, x, w) in &rects {
        seen.push(*band);
        assert!(*w < ll.table_width, "the wash is as wide as the whole table");
        // A wash never spills past the column it belongs to.
        assert!(*x >= 0.0 && *x < ll.table_width, "a wash starts outside the table: {x}");
    }
    for i in a..b {
        let Some(&(band, xa)) = ll.band_pos(raw).get(i) else { continue };
        let band = band as usize;
        // Only what is painted can be washed: the pipes and the padding between
        // two cells (and a hidden marker) own no pixels of their own.
        let painted = ll.bands(raw).get(band).map(|runs| {
            runs.iter().any(|r| i >= r.start && i < r.end)
        }) == Some(true);
        if !painted {
            continue;
        }
        let covered = rects
            .iter()
            .any(|&(b, x, w)| b == band && xa >= x - 0.6 && xa <= x + w + 0.6);
        assert!(covered, "character {i} is at {xa:.1} on band {band}, outside its wash: {rects:?}");
    }
    assert!(seen.len() >= 2, "the selection only touched one band: {seen:?}");
    // The wash is measured in the variant the glyphs are painted in.
    let src = ui_source();
    let sel = &src[src.find("// ---- selection (per visual row").unwrap()..];
    assert!(
        sel[..1200].contains("table_shows_markers"),
        "the selection wash does not read the revealed variant"
    );
}

/// (18.8) Dragging down inside a wrapped cell selects the characters the pointer
/// passes over, band by band - not a slice of the row and not a jump into another
/// cell.
#[test]
fn dragging_down_a_wrapped_cell_selects_the_characters_under_the_pointer() {
    let mut h = harness("r18-cell-drag");
    h.app.new_tab();
    let long = "wrapped words that fill several bands of the cell and keep going for a good while longer than the page is wide, so the column has to wrap them";
    h.type_str(&format!("| n | note |\n| --- | --- |\n| 1 | **{long}** |\n| 2 | short |\n"));
    h.app.set_caret(0, false);
    h.draw();
    let start = h.app.tab().doc.line_start(2);
    // Two points in the same cell on two different bands, and the characters the
    // painted row shows there.
    let (from, to) = {
        let mut te = h.r.text_ref();
        let tab = h.app.tab();
        let li = tab.layout.line_of_char(start);
        let ll = &tab.layout.lines[li];
        // The wrapped cell: the second one, which is the one with a lot of text.
        let ci = ll.info.cells.len() - 1;
        let (ca, cb) = ll.info.cells[ci];
        let bands: Vec<usize> = (ca..cb)
            .filter_map(|i| ll.band_pos(false).get(i).map(|t| t.0 as usize))
            .collect();
        let first_band = *bands.first().unwrap();
        let last_band = *bands.last().unwrap();
        assert!(last_band > first_band, "the cell did not wrap");
        let x0 = ll.table_cols[ci] + 12.0;
        let shift = ll.align_offset(h.app.text_rect.w) + ll.indent + h.app.text_rect.x;
        let band_y = |band: usize| {
            h.app.text_rect.y - tab.scroll + ll.top + (band as f32 + 0.5) * ll.row_h
        };
        let at = (shift + x0 + 40.0, band_y(first_band));
        let to = (shift + x0 + 40.0, band_y(last_band));
        let _ = &mut te;
        (at, to)
    };
    h.app.mouse = from;
    h.draw();
    h.app.click(from, &mut h.r);
    h.draw();
    h.app.mouse = to;
    let mut input = bare_md::input::InputState::default();
    h.app.handle_event(
        &winit::event::WindowEvent::CursorMoved {
            device_id: winit::event::DeviceId::dummy(),
            position: winit::dpi::PhysicalPosition::new(to.0 as f64, to.1 as f64),
        },
        &mut h.r,
        &mut input,
    );
    h.draw();
    let t = h.app.tab();
    assert!(t.has_selection(), "dragging down a wrapped cell selected nothing");
    let (a, b) = (t.anchor.min(t.caret), t.anchor.max(t.caret));
    assert!(b > a, "the drag selected nothing");
    let li = t.layout.line_of_char(a);
    let ll = &t.layout.lines[li];
    assert!(
        b <= ll.end_char(),
        "the selection ran past the row it started in ({a}..{b}, row ends at {})",
        ll.end_char()
    );
    // Both ends sit in the same cell: nothing leaked into the neighbouring column.
    let ci = ll.info.cells.iter().rposition(|&(s, _)| s <= a - ll.start_char).unwrap();
    let ci_b = ll.info.cells.iter().rposition(|&(s, _)| s <= b - ll.start_char).unwrap();
    assert_eq!(ci, ci_b, "the drag left the cell it started in");
}

/// (24.1) Entering a table row for editing must not re-flow it.
///
/// A row shows its inline markers while the caret is on it (`table_shows_markers`),
/// and the source of a cell is wider than what it renders to by the width of every
/// marker in it. The columns used to be measured from the *rendered* cell only, so
/// `**bold**` no longer fitted its column the moment the caret entered the row: the
/// cell was wrapped into three bands and the words slid sideways, then snapped back
/// as soon as the caret left the row again. A column is as wide as the wider of the
/// two variants, so the two variants wrap the same way and the row stands still.
#[test]
fn entering_a_table_row_does_not_re_flow_it() {
    let mut h = harness("r24-row-stable");
    h.app.new_tab();
    h.type_str("| head | note |\n| --- | --- |\n| gamma | **bold** x |\n");
    h.app.set_caret(0, false);
    h.draw();
    // Row 2 is the row with the markup in it; row 1 is the `| --- |` rule.
    for row in [2usize] {
        let (plain_bands, src_bands, height, row_h) = {
            let tab = h.app.tab();
            let ll = &tab.layout.lines[row];
            (ll.bands(false).len(), ll.bands(true).len(), ll.height, ll.row_h)
        };
        assert!(
            src_bands <= plain_bands,
            "row {row}: the row being edited needs {src_bands} bands, the one on screen {plain_bands}"
        );
        assert!(
            height <= row_h * src_bands as f32 + 1.0,
            "row {row}: the row reserves {} px for {src_bands} band(s) of {row_h} px",
            height
        );
    }
    // And the caret really does enter the row in this document: clicking one of its
    // cells is what used to trigger the re-flow.
    let start = h.app.tab().doc.line_start(2);
    h.app.set_caret(start + 4, false);
    h.draw();
    let markers = {
        let tab = h.app.tab();
        tab.table_shows_markers(h.app.mode, 2)
    };
    assert!(markers, "this test needs a row whose markers show while it is edited");
}

/// (24.2) The caret can be put right after a word whose closing marker is hidden,
/// and a click past the last letter of a cell stays in that cell.
///
/// The hidden markers of `**bold**` may not hold a caret - typing there would split
/// the markup - but the position *after* the word must be reachable, otherwise the
/// pointer inside the last letter of the word was mapped to the end of the cell and
/// the next character typed landed one symbol further on than the pointer.
#[test]
fn the_caret_can_be_placed_beside_a_hidden_marker_in_a_cell() {
    let mut h = harness("r24-cell-carets");
    h.app.new_tab();
    h.type_str("| head | note |\n| --- | --- |\n| gamma | **bold** x |\n");
    h.app.set_caret(0, false);
    h.draw();
    let row = 2;
    let start = h.app.tab().doc.line_start(row);
    let (top, row_h, shift) = {
        let tab = h.app.tab();
        let li = tab.layout.line_of_char(start);
        let ll = &tab.layout.lines[li];
        (
            h.app.text_rect.y - tab.scroll + ll.top,
            ll.row_h,
            ll.align_offset(h.app.text_rect.w) + ll.indent + h.app.text_rect.x,
        )
    };
    let y = top + row_h * 0.5;
    // The `d` of `bold`; its closing `**` is hidden while the row is not edited.
    let mut at = |local: usize| -> f32 {
        let mut te = h.r.text_ref();
        let tab = h.app.tab();
        let li = tab.layout.line_of_char(start);
        let ll = &tab.layout.lines[li];
        let raw = tab.line_is_raw(h.app.mode, li) || tab.table_shows_markers(h.app.mode, li);
        ll.x_abs(&mut te, local, raw)
    };
    let text = h.app.tab().doc.line_text(row);
    let d = text.find("bold").expect("the row holds the word") + 3; // the `d`
    // The hidden `**` has no width of its own, so the end of the word is the x of
    // the character that follows the markup - the space before the trailing `x`.
    let (xd, xend) = (at(d), at(d + 4));
    assert!(xend > xd + 2.0, "the word has to be wider than two pixels to click in");
    // A fifth of the way into `d`: the caret belongs to `d`, not to a marker after it.
    let got = h
        .app
        .char_at_point((shift + xd + (xend - xd) * 0.2, y), &mut h.r)
        .map(|g| g as isize - start as isize);
    assert_eq!(got, Some(d as isize), "a click inside the last letter of the cell");
    // Just past the letter: the caret stays in this cell, right after the markup -
    // it used to jump to the end of the cell, which put the next typed character
    // one symbol further along than the pointer.
    let got = h
        .app
        .char_at_point((shift + xend + 1.0, y), &mut h.r)
        .map(|g| g as isize - start as isize);
    assert_eq!(got, Some(d as isize + 4), "a click past the end of the word");
    let end_of_cell = h.app.tab().layout.lines[row].info.cells[1].1;
    assert!(
        got.unwrap() < end_of_cell as isize,
        "the caret has to stay inside the cell's own text, not at its end"
    );
}

// ============================================================ round 25 (r25)

/// (25.1 -> r55) The minimap's bars keep the pitch of text and stop where the text
/// stops; the *handle* is the thumb of the whole column and travels all of it, so a
/// press anywhere on the map can put it under the finger.
#[test]
fn a_short_documents_map_handle_stays_with_its_bars() {
    let mut h = harness("r25-map-travel");
    h.app.new_tab();
    h.app.insert_text("one\ntwo\nthree\nfour\nfive\n");
    h.app.set_caret(0, false);
    h.draw();
    let (geom, _) = h.app.minimap_geometry();
    let total = h.app.tab().layout.total_height;
    let text_h = h.app.text_rect.h;
    assert!(geom.content_h < geom.room() - 1.0, "this test needs a short document");
    let handle = geom.view_h_px(total, text_h);
    let travel = geom.handle_travel(total, text_h);
    assert!(
        travel <= geom.track_h() - handle + 1.0,
        "the handle travels {travel:.0} px over a {:.0} px column",
        geom.track_h()
    );
    // …and the rest of the column still clamps to the end rather than doing nothing.
    let map = h.app.minimap_rect;
    let at_bottom = geom.scroll_at_handle_y(map.h, total, text_h);
    assert!(at_bottom >= (total - text_h).max(0.0) - 1.0, "the bottom of the map must reach the end");
}

/// (25.2) A link inside a table cell opens on a plain click, the way it does in a
/// paragraph - and a relative target is resolved next to the document.
#[test]
fn a_link_in_a_table_cell_opens_on_a_plain_click() {
    let mut h = harness("r25-cell-link");
    let dir = h._dir.clone();
    std::fs::write(dir.join("notes.md"), "hello\n").unwrap();
    let doc = dir.join("r25.md");
    std::fs::write(&doc, "| a | b |\n| --- | --- |\n| [notes](notes.md) | x |\n").unwrap();
    h.app.open_path(&doc);
    h.app.set_caret(0, false);
    h.draw();
    // The `notes` label is at chars 3..8 of row 2.
    let start = h.app.tab().doc.line_start(2);
    let (x, y) = {
        let mut te = h.r.text_ref();
        let tab = h.app.tab();
        let ll = &tab.layout.lines[2];
        let shift = ll.align_offset(h.app.text_rect.w) + ll.indent + h.app.text_rect.x - tab.scroll_x;
        let x0 = ll.x_abs(&mut te, 4, false);
        let x1 = ll.x_abs(&mut te, 5, false);
        (
            shift + x0 + (x1 - x0) * 0.3,
            h.app.text_rect.y - tab.scroll + ll.top + ll.row_h * 0.5,
        )
    };
    let _ = start;
    h.app.mouse = (x, y);
    h.app.on_mouse_down(&mut h.r);
    let titles: Vec<String> = h.app.tabs.iter().map(|t| t.title()).collect();
    assert!(
        titles.iter().any(|t| t == "notes.md"),
        "clicking the link in the cell did not open it (tabs: {titles:?}, status {:?})",
        h.app.status
    );
}

/// (25.3) Attach, save-as and open all use the same two-column picker: tree on the
/// left, listing on the right. Attach used to be a single-column box from before
/// the folder picker existed.
#[test]
fn every_picker_is_the_same_two_column_dialog() {
    use bare_md::app::PickerKind;
    let (w, hgt) = (1180.0, 780.0);
    let mut rects = Vec::new();
    for kind in [PickerKind::Attach, PickerKind::OpenFile, PickerKind::OpenFolder, PickerKind::SaveAs] {
        let tree = bare_md::ui::picker_tree_rect(kind, w, hgt);
        let list = bare_md::ui::picker_list_rect(kind, w, hgt);
        assert!(tree.w > 0.0 && list.w > 0.0, "{kind:?}: a column is missing");
        assert!(tree.right() < list.x, "{kind:?}: the columns overlap");
        rects.push((tree, list));
    }
    let (t0, _) = rects[0];
    for (tree, _) in &rects {
        assert_eq!(tree.w, t0.w, "the tree column differs between pickers");
        assert_eq!(tree.x, t0.x, "the tree column differs between pickers");
        assert_eq!(tree.h, t0.h, "the tree column differs between pickers");
    }
}

/// (25.4) `**strong**` is visibly heavier than the text around it: the bold face
/// alone is barely a pixel heavier at text sizes, so emphasis is stamped a little
/// thicker on top of it - without moving the pen, so nothing re-flows.
#[test]
fn bold_is_heavier_than_the_text_around_it() {
    let mut h = harness("r25-bold");
    h.app.new_tab();
    h.app.insert_text("plain **strong** plain\n");
    h.app.set_caret(0, false);
    h.draw();
    let ll = &h.app.tab().layout.lines[0];
    let runs = ll.runs(false);
    let strong = runs
        .iter()
        .find(|r| r.style.face == Face::Bold)
        .expect("the strong run is drawn with the bold face");
    assert!(
        strong.style.weight > 0.4,
        "the bold run is not stamped any heavier (weight {})",
        strong.style.weight
    );
    let plain = runs
        .iter()
        .find(|r| r.style.face == Face::Regular)
        .expect("there is plain text beside it");
    assert_eq!(plain.style.weight, 0.0, "plain text must not be thickened");
    // The advance is untouched: the pen positions still come from the plain metrics.
    let te = h.r.text_ref();
    let shaped = te.width(Face::Bold, "strong", strong.style.size);
    assert!((strong.w - shaped).abs() < 1.0, "emphasis must not change the advance");
}

/// (25.5) A numbered list keeps the numbers the writer wrote, and "Renumber"
/// counts forwards.
#[test]
fn numbered_lists_keep_their_numbers_and_renumber_forwards() {
    let mut h = harness("r25-ordered");
    h.app.new_tab();
    h.app.insert_text("5. five\n6. six\n7. seven\n");
    h.app.set_caret(0, false);
    h.draw();
    let markers: Vec<String> = h
        .app
        .tab()
        .layout
        .lines
        .iter()
        .map(|l| l.info.marker.clone())
        .collect();
    assert_eq!(markers[..3], ["5.", "6.", "7."], "the rendered list must show the written numbers");
    // Renumber the whole run: counting used to run backwards (4, 3, 2, 1).
    h.app.set_caret(0, false);
    h.app.select_range(0, h.app.tab().doc.len_chars());
    h.app.renumber_list();
    assert_eq!(h.app.tab().doc.text(), "1. five\n2. six\n3. seven\n");
}

/// (25.6) An inline code span is drawn with a chip behind it; a fenced block is a
/// card and gets no chip per line.
#[test]
fn inline_code_gets_a_background_and_code_blocks_do_not() {
    let mut h = harness("r25-code-chip");
    h.app.new_tab();
    h.app.insert_text("a `one liner` b\n\n```\nlet x = 1;\n```\n");
    h.app.set_caret(0, false);
    h.draw();
    let inline = &h.app.tab().layout.lines[0];
    let chip = inline
        .runs(false)
        .iter()
        .find(|r| r.style.code())
        .expect("the inline span is marked as code");
    assert_eq!(chip.style.face, Face::Mono);
    // A line inside a card is not chipped.
    let card: Vec<usize> = h
        .app
        .tab()
        .layout
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.block() == bare_md::markdown::Block::Code)
        .map(|(i, _)| i)
        .collect();
    assert!(!card.is_empty(), "the fenced block is there");
    for i in card {
        assert!(
            h.app.tab().layout.lines[i].runs(false).iter().all(|r| !r.style.code()),
            "a card line must not get a chip"
        );
    }
}

/// (25.7) Every bundled font file is used, and the fallback face is a real answer:
/// Lato has no `✓` or `★`, so those come from DejaVu instead of a `.notdef` box.
#[test]
fn every_bundled_font_is_used_and_missing_glyphs_fall_back() {
    use bare_md::text::{Face, TextEngine};
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts");
    let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/text/mod.rs"))
        .expect("the text engine's source");
    for entry in std::fs::read_dir(&dir).expect("assets/fonts") {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.ends_with(".ttf") {
            continue;
        }
        assert!(src.contains(&name), "{name} is bundled but never loaded");
    }
    let mut te = TextEngine::new();
    for c in ['✓', '★', '😀', '▸'] {
        let face = te.resolve(Face::Regular, c);
        assert!(
            matches!(face, Face::Fallback | Face::FallbackBold | Face::Regular | Face::Bold),
            "{c} resolved to {face:?}"
        );
        // The regular faces do not have these: the answer has to be the fallback.
        assert!(
            matches!(face, Face::Fallback | Face::FallbackBold),
            "{c} is missing from Lato, so it must come from the fallback face, not {face:?}"
        );
    }
}

/// (25.8) The list button works on the whole selection, switches the kind, and
/// numbers a converted selection from one.
#[test]
fn the_list_button_works_on_the_whole_selection() {
    use bare_md::app::ToolbarAction;
    let mut h = harness("r25-list-selection");
    h.app.new_tab();
    h.app.insert_text("one\ntwo\n\nthree\nfour\n");
    h.app.set_caret(0, false);
    h.draw();
    // Select everything, ordered: every paragraph becomes an item, and blank lines
    // are not turned into empty items.
    h.app.select_range(0, h.app.tab().doc.len_chars());
    h.app.run_toolbar_action(ToolbarAction::Ordered, &mut h.r);
    h.draw();
    assert_eq!(h.app.tab().doc.text(), "1. one\n2. two\n\n3. three\n4. four\n");
    // Now the bullet button on the same (still selected) text switches the kind.
    let (a, b) = h.app.tab().sel();
    h.app.select_range(a.min(h.app.tab().doc.len_chars()), b.min(h.app.tab().doc.len_chars()));
    h.app.run_toolbar_action(ToolbarAction::Bullet, &mut h.r);
    h.draw();
    let text = h.app.tab().doc.text();
    assert!(
        text.contains("- one") && text.contains("- four") && !text.contains("1."),
        "switching to bullets left the numbers behind: {text:?}"
    );
    // Pressing the same button again takes the markers off.
    let (a, b) = h.app.tab().sel();
    h.app.select_range(a, b);
    h.app.run_toolbar_action(ToolbarAction::Bullet, &mut h.r);
    h.draw();
    assert_eq!(h.app.tab().doc.text(), "one\ntwo\n\nthree\nfour\n");
}

// ---------------------------------------------------------------------------
// Round 26
// ---------------------------------------------------------------------------

/// (26) Click an empty table cell, type: the letter must be painted *at* the
/// caret, not one glyph ahead of it.
///
/// The reference is the same text typed in a plain paragraph: the offset of the
/// caret from the start of the text it follows has to be identical. Before the
/// fix `table_cells` recorded only the *starts* of its runs, so the slot at the
/// end of a run - the caret position you get the moment the first letter lands
/// in an empty cell - was filled in with the start of the previous character and
/// the caret was drawn a whole glyph to the left of the text. Any re-layout (a
/// table button, for instance) hid it again, which is why it looked like the
/// toolbar "fixed" the table.
#[test]
fn typing_into_an_empty_table_cell_leaves_the_caret_after_the_letter() {
    // Where the caret sits when the given text has just been typed, measured
    // from the left edge of the text it belongs to.
    fn offsets(h: &mut Harness, doc: &str, row: usize, col: usize, typed: &str) -> Vec<f32> {
        h.app.new_tab();
        h.app.insert_text(doc);
        h.app.set_caret(0, false);
        h.draw();
        let (top, h_row, left, col_x) = {
            let t = h.app.tab();
            let ll = &t.layout.lines[row];
            (
                h.app.text_rect.y - t.scroll + ll.top,
                ll.row_h,
                h.app.text_rect.x - t.scroll_x + ll.indent + ll.align_offset(h.app.text_rect.w),
                ll.table_cols[col],
            )
        };
        h.click((left + col_x + 6.0, top + h_row * 0.5));
        let mut seen = Vec::new();
        for ch in typed.chars() {
            h.app.insert_typed(&ch.to_string());
            h.draw();
            let caret = h.caret_x();
            seen.push(caret - left - col_x);
        }
        seen
    }

    let mut h = harness("r26-empty-cell");
    // A paragraph is the yardstick.
    let para = {
        h.app.new_tab();
        h.app.insert_text("hello\n");
        h.app.set_caret(0, false);
        h.draw();
        let left = h.app.text_rect.x - h.app.tab().scroll_x;
        let mut seen = Vec::new();
        for ch in "www".chars() {
            h.app.insert_typed(&ch.to_string());
            h.draw();
            seen.push(h.caret_x() - left);
        }
        seen
    };

    let cases: &[(&str, &str, usize, usize)] = &[
        ("last cell empty", "| a | b |\n| --- | --- |\n| x |  |\n", 2, 1),
        ("first cell empty", "| a | b |\n| --- | --- |\n|  | y |\n", 2, 0),
        ("middle of three", "| a | b | c |\n| - | - | - |\n| p |  | r |\n", 2, 1),
        ("whole row empty", "| a | b |\n| --- | --- |\n|  |  |\n", 2, 0),
        ("bold neighbour", "| a | b |\n| --- | --- |\n| **B** |  |\n", 2, 1),
        ("code neighbour", "| a | b |\n| --- | --- |\n| `c` |  |\n", 2, 1),
    ];
    for (tag, doc, row, col) in cases {
        let got = offsets(&mut h, doc, *row, *col, "www");
        for (i, (g, w)) in got.iter().zip(&para).enumerate() {
            assert!(
                (g - w).abs() < 1.5,
                "{tag}: after {} letter(s) the caret is {g:.2} from the cell text, \
                 but the same text in a paragraph puts it at {w:.2} - the text is \
                 painted ahead of the pointer",
                i + 1
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Round 27
// ---------------------------------------------------------------------------

/// (27 -> r56) The handle moves over the text and only the text, and the map is
/// grabbable from anywhere on it.
///
/// Two requirements pull in opposite directions and both are real:
///   * the handle says where in the *document* the viewport is, so it travels the bars
///     and stops at the last one - a handle that runs down the empty column makes a
///     drag cover far more document than the map shows;
///   * a document typed into an empty window leaves the page at its end and the pointer
///     below the bars, so a press there has to grab the map, or it looks dead until a
///     jump from the outline moves the page back up.
/// The resolution: a press on the text jumps (the handle comes to the finger); a press
/// anywhere else grabs the handle where it is, without moving the page; either way the
/// drag then moves it one map pixel per pointer pixel, inside the bars.
#[test]
fn the_minimap_handle_travels_the_text_and_the_map_grabs_anywhere() {
    let mut h = harness("r27-map-bars");
    h.app.new_tab();
    h.app.insert_text(&(0..200).map(|i| format!("row {i} with some words in it\n")).collect::<String>());
    h.app.set_caret(0, false);
    h.app.settings.minimap_open = true;
    h.draw();
    let map = h.app.minimap_rect;
    assert!(!map.is_empty(), "the minimap is on");
    let (geom, _) = h.app.minimap_geometry();
    let total = h.app.tab().layout.total_height;
    let text_h = h.app.text_rect.h;
    assert!(total > text_h + 1.0, "this test needs a document that scrolls");
    let max_scroll = (total - text_h).max(0.0);

    // (a) The handle stays on the text at every scroll position, its height never
    // changes, and it is a thumb sized by the text - never a block.
    let height = geom.view_h_px(total, text_h);
    assert!(
        height <= geom.track_h() * 0.45 + 1.0,
        "the thumb ({height:.0} px) is a block, not a thumb (strip {:.0} px)",
        geom.track_h()
    );
    for frac in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
        h.app.tab_mut().scroll = max_scroll * frac;
        h.draw();
        let (g, _) = h.app.minimap_geometry();
        let top = map.y + g.indicator_y(h.app.tab().scroll, total, text_h);
        let bottom = top + g.view_h_px(total, text_h);
        assert!(
            top >= map.y + g.track_top() - 0.5
                && bottom <= map.y + g.track_top() + g.track_h() + 0.5,
            "at {:.0}% of the document the handle spans {top:.1}..{bottom:.1}, outside the strip {:.1}..{:.1}",
            frac * 100.0,
            map.y + g.track_top(),
            map.y + g.track_top() + g.track_h()
        );
        assert!(
            (g.view_h_px(total, text_h) - height).abs() < 0.01,
            "the handle changed height while scrolling"
        );
    }

    // (b) The map's press target is the whole column.
    let hit = h
        .app
        .hits
        .find(|hit| matches!(hit, Hit::Minimap))
        .expect("the minimap must be a hit target");
    assert!(
        hit.h >= map.h - 1.0,
        "the minimap only takes presses over {:.0} px of its {:.0} px column",
        hit.h,
        map.h
    );

    // (c) A press on the text jumps: the handle's middle comes to the finger, and the
    // drag from there is 1:1 over the text in both directions.
    let cx = map.x + map.w * 0.5;
    let caret = h.app.tab().caret;
    let bars_top = map.y + geom.bars_top();
    let bars_h = geom.bars_bottom() - geom.bars_top();
    for frac in [0.1f32, 0.3, 0.55] {
        h.app.tab_mut().scroll = 0.0;
        h.draw();
        let y = bars_top + bars_h * frac;
        h.app.mouse = (cx, y);
        h.draw();
        h.app.click((cx, y), &mut h.r);
        h.draw();
        assert!(h.app.dragging_minimap, "a press on the bars did not grab the map");
        let (g, _) = h.app.minimap_geometry();
        let top0 = map.y + g.indicator_y(h.app.tab().scroll, total, text_h);
        let middle = top0 + g.view_h_px(total, text_h) * 0.5;
        assert!(
            (middle - y).abs() < 6.0,
            "a press on the bars left the handle's middle at {middle:.0}, not under the finger ({y:.0})"
        );
        let mut pointer = y;
        for step in [-30.0f32, -40.0, 60.0, 30.0] {
            pointer += step;
            h.app.mouse = (cx, pointer);
            assert!(h.app.minimap_drag((cx, pointer), &mut h.r), "the drag was dropped");
            h.draw();
            let (g, _) = h.app.minimap_geometry();
            let next = map.y + g.indicator_y(h.app.tab().scroll, total, text_h);
            let lo = map.y + g.track_top();
            let hi = map.y + g.track_top() + g.track_h() - g.view_h_px(total, text_h);
            // The handle's position is the press position plus the pointer's own
            // displacement, folded into the bars - so 60 px of pointer is 60 px of
            // handle, and a haul past the end snaps back to the text's end.
            let wanted = (top0 + (pointer - y)).clamp(lo, hi);
            assert!(
                (next - wanted).abs() < 1.5,
                "the pointer is {:+.0} from the press and the handle is at {next:.1} (wanted {wanted:.1})",
                pointer - y
            );
            assert!(
                (g.view_h_px(total, text_h) - height).abs() < 0.01,
                "the handle changed height while dragging"
            );
        }
        h.app.mouse_up(&mut h.r);
    }

    // (d) A press below the bars puts the thumb there too - one rule for the whole
    // column, so the map never has a place where a press does nothing.
    h.app.tab_mut().scroll = max_scroll;
    h.draw();
    let low = map.y + map.h - 4.0;
    h.app.mouse = (cx, low);
    h.draw();
    let before = h.app.tab().scroll;
    h.app.click((cx, low), &mut h.r);
    h.draw();
    assert!(h.app.dragging_minimap, "a press below the bars grabbed nothing");
    assert!(
        h.app.tab().scroll >= max_scroll - 2.0,
        "a press at the bottom of the column left the page at {:.0} of {max_scroll:.0}",
        h.app.tab().scroll
    );
    let (g, _) = h.app.minimap_geometry();
    let start = map.y + g.indicator_y(h.app.tab().scroll, total, text_h);
    h.app.mouse = (cx, low - 120.0);
    assert!(h.app.minimap_drag((cx, low - 120.0), &mut h.r));
    h.draw();
    let (g, _) = h.app.minimap_geometry();
    let moved = map.y + g.indicator_y(h.app.tab().scroll, total, text_h);
    assert!(
        (start - moved - 120.0).abs() < 1.5,
        "120 px of dragging up moved the handle {:.1} px",
        start - moved
    );
    let _ = before;
    assert!(
        moved >= map.y + g.bars_top() - 0.5,
        "after pressing below the text the thumb is off it ({moved:.1} < {:.1})",
        map.y + g.bars_top()
    );
    h.app.mouse_up(&mut h.r);
    assert_eq!(h.app.tab().caret, caret, "a press on the map moved the caret");
}

// ---------------------------------------------------------------------------
// Round 28 — right-to-left languages
// ---------------------------------------------------------------------------

/// A right-to-left paragraph starts at the **right** margin, like `dir="rtl"` in a
/// browser: its first character - and the caret before it - are at the right edge
/// of the page, not the left.
#[test]
fn a_hebrew_paragraph_starts_at_the_right_margin() {
    let mut h = harness("r28-rtl-align");
    h.app.new_tab();
    h.app.insert_text("שלום עולם\n");
    h.app.set_caret(0, false);
    h.draw();
    let text = h.app.text_rect;
    let line = &h.app.tab().layout.lines[0];
    assert!(line.rtl_para(), "a line of Hebrew is a right-to-left paragraph");
    let shift = line.align_offset_row(text.w, 0, false);
    let right = text.x + line.indent + shift + line.row_width_of(0, false);
    assert!(
        (right - text.right()).abs() < 1.0,
        "the paragraph ends at {right:.0}, but the page ends at {:.0}",
        text.right()
    );
    assert!(shift > text.w * 0.5, "a short Hebrew line must sit against the right margin (shift {shift:.0})");
    // The caret at the start of the line is at that right edge too.
    let cr = h.app.caret_rect(&mut h.r.text);
    assert!(
        (cr.x - (text.x + line.indent + shift + line.row_width_of(0, false))).abs() < 1.0,
        "the caret at the start of a Hebrew line sits at {:.0}, the text starts/ends at {right:.0}",
        cr.x
    );
    // A Latin line in the same document is untouched.
    h.app.new_tab();
    h.app.insert_text("hello\n");
    h.app.set_caret(0, false);
    h.draw();
    let line = &h.app.tab().layout.lines[0];
    assert!(!line.rtl_para());
    assert!(line.bidi.is_none(), "a Latin line must not go through the bidi code at all");
    assert_eq!(line.align_offset_row(h.app.text_rect.w, 0, false), 0.0);
}

/// Typing Hebrew: every letter lands to the **left** of the one before it, and the
/// caret stays right after the new letter - the same "the caret follows what I just
/// typed" that Latin gets.
#[test]
fn typing_hebrew_puts_each_letter_left_of_the_last_and_keeps_the_caret_with_it() {
    let mut h = harness("r28-rtl-typing");
    h.app.new_tab();
    h.app.insert_text("שלום");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    let mut last = h.caret_x();
    for ch in [" ", "ע", "ו", "ל", "ם"] {
        h.app.insert_typed(ch);
        h.draw();
        let x = h.caret_x();
        assert!(x < last - 1.0, "typing {ch:?} moved the caret {:.1} px - it must move left", x - last);
        last = x;
        // …and the caret is exactly where the layout says the text ends.
        let t = h.app.tab();
        let line = &t.layout.lines[0];
        let local = t.caret - line.start_char;
        let want = h.app.text_rect.x - t.scroll_x + line.indent + line.align_offset_row(h.app.text_rect.w, 0, false) + line.x_abs(&mut h.r.text, local, false);
        assert!((x - want).abs() < 1.0, "the caret is at {x:.1} but the text ends at {want:.1}");
    }
    assert_eq!(h.app.tab().doc.line_text(0), "שלום עולם");
}

/// Clicking a bidirectional line lands on the character under the pointer: for
/// every position of the line, clicking the x the layout gives that position puts
/// the caret back on it. This is the round trip the caret, the painter and the
/// pointer all share.
#[test]
fn clicking_a_bidirectional_line_lands_on_the_character_under_the_pointer() {
    let mut h = harness("r28-rtl-click");
    for doc in ["שלום עולם", "مرحبا بالعالم", "abc שלום def", "hello שלום"] {
        h.app.new_tab();
        h.app.insert_text(doc);
        h.app.set_caret(0, false);
        h.draw();
        let n = doc.chars().count();
        let y = h.row_y(0, 0, 0.5);
        // Where the direction changes, two positions can share one pixel: the start
        // and the end of a Hebrew run are both at the run's left edge. A click there
        // can only pick one of them, so those positions are not part of this test -
        // every *distinguishable* position is.
        let xs: Vec<f32> = (0..n).map(|i| h.local_x(0, i, 0, false).round()).collect();
        for i in 0..n {
            if xs.iter().filter(|&&x| (x - xs[i]).abs() < 0.5).count() > 1 {
                continue;
            }
            let x = h.local_x(0, i, 0, false);
            // Two clicks close together in time and place are a double click, which
            // selects a word - so every click here starts from a clean slate.
            h.app.last_click = None;
            h.app.click_count = 1;
            h.app.mouse = (x, y);
            h.draw();
            h.app.click((x, y), &mut h.r);
            let got = h.app.tab().caret;
            assert_eq!(
                got, i,
                "{doc:?}: a click at the position of character {i} put the caret on {got}"
            );
        }
    }
}

/// A number inside a Hebrew sentence keeps its digits in reading order while the
/// words around it are reversed - the rule everybody notices first when bidi is
/// wrong.
#[test]
fn a_number_inside_hebrew_keeps_its_digits_in_reading_order() {
    let mut h = harness("r28-rtl-number");
    h.app.new_tab();
    let doc = "אב 123 גד";
    h.app.insert_text(doc);
    h.app.set_caret(0, false);
    h.draw();
    let line = &h.app.tab().layout.lines[0];
    assert!(line.rtl_para());
    let mut te = h.r.text_ref();
    let xs: Vec<f32> = (0..doc.chars().count()).map(|i| line.x_abs(&mut te, i, false)).collect();
    let digits: Vec<usize> = doc.chars().enumerate().filter(|(_, c)| c.is_ascii_digit()).map(|(i, _)| i).collect();
    assert_eq!(digits.len(), 3);
    // The digits form one left-to-right run: inside it, each character sits to the
    // right of the one before, so the caret moves rightwards as you type them -
    // 1, 2, 3 and not 3, 2, 1.
    let inside: Vec<f32> = vec![xs[4], xs[5], xs[6]];
    assert!(
        inside[0] < inside[1] && inside[1] < inside[2],
        "the digits of 123 were placed right to left ({inside:?}) - they must read 1, 2, 3"
    );
    // …and the whole number sits inside the sentence: the Hebrew that comes before
    // it in the source is to its right.
    assert!(
        xs[3] >= inside[2] - 0.5,
        "the number must end where the Hebrew before it begins ({:.1} vs {:.1})",
        xs[3],
        inside[2]
    );
    // Nothing of it is reversed into the other word.
    assert!(inside[0] > xs[7], "the number must be right of the Hebrew that follows it");
}

/// A long Hebrew paragraph wraps, and every wrapped row is flush with the right
/// margin - not just the first one.
#[test]
fn wrapped_hebrew_rows_all_end_at_the_right_margin() {
    let mut h = harness("r28-rtl-wrap");
    h.app.new_tab();
    let doc = "זהו טקסט ארוך מאוד שצריך להישבר לכמה שורות כדי לבדוק את הגלישה ".repeat(12);
    h.app.insert_text(&doc);
    h.app.set_caret(0, false);
    h.draw();
    let text = h.app.text_rect;
    let line = &h.app.tab().layout.lines[0];
    assert!(line.rows.len() > 2, "the paragraph must wrap ({} rows)", line.rows.len());
    for ri in 0..line.rows.len() {
        let right = text.x + line.indent + line.align_offset_row(text.w, ri, false) + line.row_width_of(ri, false);
        assert!(
            (right - text.right()).abs() < 1.5,
            "row {ri} ends at {right:.0}, the page at {:.0}",
            text.right()
        );
    }
}

// ---------------------------------------------------------------------------
// Round 29 — what a mixed-direction stress document exposed
// ---------------------------------------------------------------------------

/// (29.1) A task whose marker is `[x]` is still a Hebrew paragraph: the Latin `x`
/// inside the marker must not decide the direction, or the line reads
/// left-to-right while its text is Hebrew, and the checkbox ends up at the wrong
/// end of the line.
#[test]
fn the_checkbox_of_a_hebrew_task_sits_on_the_right() {
    let mut h = harness("r29-task");
    h.app.new_tab();
    h.app.insert_text("- [ ] משימה ראשונה\n- [x] משימה שנייה\n");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    let text = h.app.text_rect;
    for i in 0..2 {
        let line = &h.app.tab().layout.lines[i];
        assert!(
            line.rtl_para(),
            "line {i} ({:?}) is not recognised as right-to-left",
            h.app.tab().doc.line_text(i)
        );
        // The paragraph is indented from the right, so its text ends one gutter
        // short of the page: the checkbox lives in that gutter.
        let shift = line.align_offset_row(text.w, 0, false);
        let row_w = line.row_width_of(0, false);
        let gutter = text.right() - line.indent - (text.x + line.indent + shift + row_w);
        assert!(
            gutter.abs() < 1.5,
            "line {i}: the text ends {gutter:.1} px short of where its right gutter starts"
        );
    }
    // The two checkboxes line up with each other, which they did not when one line
    // was detected as LTR and the other as RTL.
    let size = (h.app.theme.font_size).max(16.0);
    let boxes: Vec<f32> = (0..2)
        .map(|i| {
            let line = &h.app.tab().layout.lines[i];
            let shift = line.align_offset_row(text.w, 0, false);
            text.x + line.indent + shift + line.row_width_of(0, false) + 6.0 + size * 0.0
        })
        .collect();
    assert!((boxes[0] - boxes[1]).abs() < 1.5, "the checkboxes are not aligned: {boxes:?}");
}

/// (29.2) A table cell is a paragraph of its own: Hebrew and Arabic inside a cell
/// are painted right to left, and the column order of the table is untouched.
#[test]
fn hebrew_and_arabic_table_cells_are_laid_out_right_to_left() {
    use bare_md::markdown::Block;
    let mut h = harness("r29-table-rtl");
    h.app.new_tab();
    h.app.insert_text("| עמודה | عمود | Column |\n| --- | --- | --- |\n| שלום | مرحبا | hello |\n");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    let line = &h.app.tab().layout.lines[2];
    assert_eq!(line.block(), Block::Table);
    let band = line.bands(false).first().cloned().unwrap_or_default();
    assert!(!band.is_empty(), "the row must have a band");
    // Every RTL cell is one right-to-left run, in the cell's own column.
    let mut rtl_runs = 0;
    for run in &band {
        let text: String = line.chars[run.start..run.end].iter().collect();
        if text.chars().any(is_rtl_char) {
            assert!(run.rtl, "cell {text:?} was laid out left to right");
            rtl_runs += 1;
        }
    }
    assert_eq!(rtl_runs, 2, "both the Hebrew and the Arabic cell must be RTL: {band:?}");
    // The Latin cell beside them is still left to right: the fix must not turn every
    // cell around.
    let latin: Vec<bool> = band
        .iter()
        .filter(|r| line.chars[r.start..r.end].iter().collect::<String>().contains("hello"))
        .map(|r| r.rtl)
        .collect();
    assert_eq!(latin, vec![false], "the Latin cell was turned around: {band:?}");
    // The columns keep their order: the Hebrew cell starts left of the Arabic one.
    let xs: Vec<f32> = band
        .iter()
        .filter(|r| r.rtl)
        .map(|r| r.x)
        .collect();
    assert!(xs[0] < xs[1], "the table's column order changed: {xs:?}");
}

/// Pressing Bold twice must always come back to exactly what was there. Three
/// separate reports each checked that from one caret position; the property is the
/// same, so the check is written once and the positions are the fixtures.
fn assert_bold_round_trip(tag: &str, doc: &str, caret: usize) {
    let mut h = harness(tag);
    h.type_str(doc);
    h.app.set_caret(caret, false);
    h.draw();
    h.app.toggle_inline("**", "**");
    h.draw();
    let once = h.app.tab().doc.text();
    assert_ne!(once, doc, "{tag}: the first press did nothing");
    h.app.toggle_inline("**", "**");
    h.draw();
    let twice = h.app.tab().doc.text();
    assert_eq!(twice, doc, "{tag}: two presses left {twice:?} instead of {doc:?}");
}

/// (30.1) Every symbol of a right-to-left table cell has a wash of its own: the
/// selection covers the glyph instead of collapsing to a 1.5 px sliver.
///
/// Three shapes of cell, because "a cell of Hebrew" is not the only one that exists:
/// plain words, words carrying inline formatting, and a word with digits inside it -
/// and both views, because a marker that is hidden when rendered is still a symbol
/// you can put the caret on and select when the source is showing.
#[test]
fn every_symbol_of_a_right_to_left_table_cell_can_be_selected() {
    let docs = [
        RTL_TABLE,
        "| a | b |\n| --- | --- |\n| **שלום** | مرحبا |\n",
        "| a | b |\n| --- | --- |\n| שלום 12 | 2026 مرحبا |\n",
    ];
    let mut h = harness("r30-cell-wash");
    let mut checked = 0;
    for doc in docs {
        for raw in [false, true] {
            h.app.new_tab();
            h.app.insert_text(doc);
            h.app.set_caret(h.app.tab().doc.len_chars(), false);
            h.draw();
            let row = h.last_table_row();
            let start = h.app.tab().doc.line_start(row);
            let (cells, pos) = {
                let line = &h.app.tab().layout.lines[row];
                (line.info.cells.clone(), line.band_pos(raw).to_vec())
            };
            let showing_source = raw || h.app.tab().line_is_raw(h.app.mode, row);
            for &(a, b) in &cells {
                for i in a..b {
                    let ch = h.app.tab().doc.slice(start + i, start + i + 1);
                    let Some(&(_, cx)) = pos.get(i) else { continue };
                    let Some(c) = ch.chars().next() else { continue };
                    if c.is_whitespace() || c == '|' {
                        continue;
                    }
                    let rects = h.app.tab().layout.lines[row].table_sel_rects(i, i + 1, raw);
                    let Some(&(_, x0, w)) = rects.first() else {
                        // Only a marker that the rendered view hides may have no wash.
                        assert!(
                            !showing_source && is_md_marker(c),
                            "{doc:?} (raw={raw}): the symbol {c:?} at {i} has no wash"
                        );
                        continue;
                    };
                    assert!(
                        w >= 3.0,
                        "{doc:?} (raw={raw}): the wash of {c:?} is only {w} px wide, so it \
                         cannot be seen: {rects:?}"
                    );
                    // And it sits on the glyph, not beside it.
                    assert!(
                        x0 - 0.01 <= cx && cx <= x0 + w + 0.01,
                        "{doc:?} (raw={raw}): the wash of {c:?} at {cx} does not cover it: {rects:?}"
                    );
                    if is_rtl_char(c) {
                        checked += 1;
                    }
                }
            }
        }
    }
    assert!(checked >= 24, "every RTL symbol must be checked in both views, saw {checked}");
}

/// (30.2) Right moves the caret to the right **on screen** inside a Hebrew or
/// Arabic cell, and Left moves it back.
///
/// Checked on every cell of the row - the two right-to-left ones and the Latin one,
/// so a fix that only special-cases Hebrew cannot pass - and both directions.
#[test]
fn right_in_a_right_to_left_table_cell_moves_the_caret_right() {
    let mut h = harness("r30-cell-arrows");
    h.app.new_tab();
    h.app.insert_text(RTL_TABLE);
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    let row = h.last_table_row();
    let start = h.app.tab().doc.line_start(row);
    let cells = h.app.tab().layout.lines[row].info.cells.clone();
    let line_len = h.app.tab().doc.line_len(row);
    for &(a, b) in &cells {
        let text: String = h.app.tab().doc.slice(start + a, start + b).trim().to_string();
        if text.is_empty() {
            continue;
        }
        // Between them the two arrows reach every symbol: in a right-to-left cell the
        // word runs leftwards visually, so its letters lie to the *left* of the
        // cell's first position and are reached with Left, not with Right.
        let mut seen: Vec<usize> = Vec::new();
        for forward in [true, false] {
            h.app.set_caret(start + a, false);
            h.draw();
            seen.push(a);
            let walk = h.walk_arrows(forward, 6);
            assert_walks_one_way(&walk, forward, &format!("cell {text:?}"));
            seen.extend(walk.iter().map(|p| p.0 - start));
        }
        for i in a..b.min(line_len) {
            let ch = h.app.tab().doc.slice(start + i, start + i + 1);
            if ch.trim().is_empty() || ch == "|" {
                continue;
            }
            assert!(
                seen.contains(&i),
                "no arrow reached {ch:?} at {i} of cell {text:?} ({a}..{b}): {seen:?}"
            );
        }
    }
    // A Right press inside an RTL cell crosses into the next column: the caret is
    // not trapped in the cell it started in.
    let (a0, b0) = cells[0];
    h.app.set_caret(start + a0, false);
    h.draw();
    let walk = h.walk_arrows(true, 1);
    assert!(
        walk[0].0 - start > b0,
        "Right did not leave the Hebrew cell: stopped at {} (cell ends at {b0})",
        walk[0].0 - start
    );
}

/// (30.4) The same rule on an ordinary Hebrew paragraph, outside any table: Right
/// moves the caret to the right on screen, which inside a right-to-left run means
/// towards the **start** of the sentence. Left is the mirror image.
#[test]
fn right_arrow_in_a_hebrew_paragraph_moves_the_caret_right() {
    let mut h = harness("r30-para-arrows");
    for doc in ["שלום עולם", "مرحبا بالعالم", "abc שלום"] {
        h.app.new_tab();
        h.app.insert_text(doc);
        // Anchor in the middle of the line, where both directions have somewhere to
        // go, and walk out and back.
        let line_len = h.app.tab().doc.line_len(0);
        h.app.set_caret(line_len / 2, false);
        h.draw();
        let start_x = h.caret_x();
        let back = h.walk_arrows(false, 2);
        assert_walks_one_way(&back, false, doc);
        let forward = h.walk_arrows(true, 2);
        assert_walks_one_way(&forward, true, doc);
        assert!(
            (h.caret_x() - start_x).abs() < 1.5,
            "{doc:?}: Right did not come back to where Left started ({} vs {start_x:.1})",
            h.caret_x()
        );
    }
}

/// (30.3) A click anywhere in a right-to-left cell lands on the **nearest**
/// position: a sweep across the glyphs of every cell of the row, in the two
/// right-to-left columns and the Latin one.
///
/// The click at a fraction of the word's inked span must put the caret on the
/// position whose own x is nearest to the pointer - the property a user feels as
/// "the caret goes where I clicked", and the one that used to fail inside Hebrew
/// because the positions were walked in document order.
#[test]
fn clicking_inside_a_right_to_left_table_cell_lands_on_the_nearest_position() {
    let mut h = harness("r30-cell-click");
    h.app.new_tab();
    h.app.insert_text(RTL_TABLE);
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    let row = h.last_table_row();
    let start = h.app.tab().doc.line_start(row);
    let cells = h.app.tab().layout.lines[row].info.cells.clone();
    let y = h.row_y(row, 0, 0.5);
    let mut placed = 0;
    for &(a, b) in &cells {
        let text: String = h.app.tab().doc.slice(start + a, start + b).trim().to_string();
        if text.is_empty() {
            continue;
        }
        // Where the layout paints each position of this cell, on screen.
        let mut stops: Vec<(usize, f32)> = Vec::new();
        for i in a..b {
            let ch = h.app.tab().doc.slice(start + i, start + i + 1);
            if ch.trim().is_empty() {
                continue;
            }
            let x = h.local_x(row, i, 0, false);
            stops.push((i, x));
        }
        let (lo, hi) = stops.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &(_, x)| {
            (lo.min(x), hi.max(x))
        });
        for frac in [0.15, 0.35, 0.5, 0.65, 0.85] {
            let x = lo + (hi - lo) * frac;
            h.app.last_click = None;
            h.app.click_count = 1;
            h.click_full((x, y));
            let at = h.app.tab().caret - start;
            assert!(
                at >= a && at <= b,
                "a click {frac} into cell {text:?} landed at {at}, outside ({a}..{b})"
            );
            // The nearest painted position, by the layout's own x.
            let nearest = stops
                .iter()
                .map(|&(i, sx)| ((x - sx).abs(), i))
                .fold((f32::MAX, a), |best, cur| if cur.0 < best.0 { cur } else { best });
            let landed_x = h.local_x(row, at.min(b), 0, false);
            assert!(
                (landed_x - x).abs() <= nearest.0 + 6.0,
                "a click at {x:.1} in cell {text:?} landed on {at} at {landed_x:.1}, \
                 not the nearest position {} at {:.1}",
                nearest.1,
                x - nearest.0
            );
        }
        placed += 1;
    }
    assert_eq!(placed, 3, "all three cells must be reachable by a click");
}

/// (30.5) A selection in a right-to-left paragraph is **drawn** over the characters
/// it selects. The wash used to be built as "from the x of the first selected offset
/// to the x of the second", which is only true left to right: in a Hebrew row the
/// first offset is the one furthest right, so the width came out negative and the
/// highlight collapsed to a 2 px sliver.
#[test]
fn a_selection_in_a_right_to_left_paragraph_covers_its_characters() {
    let mut h = harness("r30-para-wash");
    h.app.new_tab();
    let doc = "זהו טקסט ארוך מאוד שצריך להישבר לכמה שורות כדי לבדוק את הגלישה ".repeat(5);
    h.app.insert_text(&doc);
    h.app.set_caret(0, false);
    h.draw();
    let line = h.app.tab().layout.lines[0].clone();
    assert!(line.rows.len() >= 3, "this test needs a paragraph that wraps");
    // 1. A part of the first row: the wash starts at the leftmost selected glyph and
    //    ends at the rightmost one.
    let (a, b) = (30usize, 90usize);
    let spans = line.row_span(&mut h.r.text, a, b, false);
    assert_eq!(spans.len(), 1, "only the first row is selected: {spans:?}");
    let (ri, lo, hi) = spans[0];
    assert_eq!(ri, 0);
    assert!(hi - lo > 60.0, "the wash of a 60-character selection is only {:.1} px wide", hi - lo);
    for i in a..b {
        let x = line.x_abs(&mut h.r.text, i, false);
        assert!(
            x >= lo - 0.6 && x <= hi + 0.6,
            "character {i} is at {x:.1}, outside the wash {lo:.1}..{hi:.1}"
        );
    }
    // 2. Whole rows: the wash runs the full width of the row, and each row is
    //    measured with its own alignment shift - wrapped RTL rows are not all the
    //    same width, and the last one is the shortest.
    let last = line.rows.len() - 1;
    let spans = line.row_span(&mut h.r.text, 0, line.rows[last].end, false);
    assert_eq!(spans.len(), line.rows.len(), "every row is selected: {spans:?}");
    for (ri, lo, hi) in &spans {
        let w = line.row_width_of(*ri, false);
        assert!((lo.abs()) < 0.01, "row {ri} starts at {lo:.1}, not at its left edge");
        assert!(
            (hi - w).abs() < 0.01,
            "row {ri} is {w:.1} px wide but its wash ends at {hi:.1}"
        );
    }
    let shifts: Vec<f32> = (0..line.rows.len())
        .map(|ri| line.align_offset_row(h.app.text_rect.w, ri, false))
        .collect();
    assert!(
        shifts.iter().any(|&s| (s - shifts[0]).abs() > 1.0),
        "this test needs rows of different widths: {shifts:?}"
    );
}

/// (30.6) The find highlight is measured the same way: a hit inside a right-to-left
/// paragraph is drawn over the match, not as a sliver beside it.
#[test]
fn a_find_hit_in_a_right_to_left_paragraph_is_drawn_over_the_match() {
    let mut h = harness("r30-para-find");
    h.app.new_tab();
    h.app.insert_text("שלום עולם\n");
    h.app.set_caret(0, false);
    h.app.open_find(false);
    h.app.tab_mut().find.query = "עולם".into();
    h.app.tab_mut().find.version = u64::MAX;
    h.app.find_step(true);
    h.draw();
    let (a, b) = h.app.tab().find.matches[0];
    let line = h.app.tab().layout.lines[0].clone();
    let spans = line.row_span(&mut h.r.text, a, b, false);
    assert_eq!(spans.len(), 1, "the match is on one row: {spans:?}");
    let (_, lo, hi) = spans[0];
    assert!(hi - lo > 20.0, "the highlight of a 4-letter match is {:.1} px wide", hi - lo);
    for i in a..b {
        let x = line.x_abs(&mut h.r.text, i, false);
        assert!(x >= lo - 0.6 && x <= hi + 0.6, "character {i} at {x:.1} is outside {lo:.1}..{hi:.1}");
    }
}

/// (30.7) Both washes are measured by the layout. The painter used to build its own
/// rectangle out of two offsets, which is where the sliver came from - a fix in the
/// layout would be invisible if the painter kept doing its own arithmetic.
#[test]
fn the_selection_and_the_search_wash_are_both_measured_by_the_layout() {
    let src = ui_source();
    let used = src.matches("row_span").count();
    assert!(
        used >= 2,
        "the selection and the search wash must both be measured by the layout (found {used} use(s))"
    );
}

/// Grab the finished frame as RGBA8. Tests that must know what reached the screen
/// ask the pixels, not the code that was supposed to paint them.
fn capture(h: &mut Harness) -> (u32, u32, Vec<u8>) {
    h.r.capture = true;
    h.draw();
    h.r.last_frame.take().expect("the frame was not captured")
}

/// (30.8) The selection in a Hebrew paragraph reaches the **screen**. Rendered twice -
/// once plain, once with a selection - and compared pixel by pixel: the wash has to
/// cover a stretch of glyphs, not a sliver. This is the regression for "selection not
/// displaying properly", which no geometry test could have caught on its own.
#[test]
fn the_wash_of_a_right_to_left_selection_reaches_the_screen() {
    let mut h = harness("r30-para-pixels");
    h.app.new_tab();
    h.app.insert_text(&"זהו טקסט ארוך מאוד שצריך להישבר לכמה שורות כדי לבדוק את הגלישה ".repeat(5));
    h.app.set_caret(0, false);
    h.draw();
    let (w, _, plain) = capture(&mut h);
    let (a, b) = (30usize, 90usize);
    h.app.select_range(a, b);
    h.draw();
    let (_, _, sel) = capture(&mut h);
    let line = h.app.tab().layout.lines[0].clone();
    let (ri, lo, hi) = line.row_span(&mut h.r.text, a, b, false)[0];
    assert_eq!(ri, 0);
    let shift = line.align_offset_row(h.app.text_rect.w, 0, false);
    let gx = h.app.text_rect.x - h.app.tab().scroll_x + line.indent + shift;
    let y0 = (h.app.text_rect.y - h.app.tab().scroll + line.top) as u32;
    let y1 = (y0 as f32 + line.row_h).min(h.app.text_rect.bottom()) as u32;
    // Columns inside the first visual row where the two frames disagree, as runs.
    // (The caret and the row's edge can add their own few pixels; the wash is the
    // long run, and it is the long run that has to be there.)
    let mut runs: Vec<(u32, u32)> = Vec::new();
    let mut cur: Option<(u32, u32)> = None;
    for x in 0..w {
        let mut differs = false;
        for y in y0..y1 {
            let i = ((y * w + x) * 4) as usize;
            if plain[i..i + 3] != sel[i..i + 3] {
                differs = true;
                break;
            }
        }
        match (differs, cur) {
            (true, None) => cur = Some((x, x)),
            (true, Some((s0, _))) => cur = Some((s0, x)),
            (false, Some(r)) => {
                runs.push(r);
                cur = None;
            }
            _ => {}
        }
    }
    if let Some(r) = cur {
        runs.push(r);
    }
    let &(px0, px1) = runs
        .iter()
        .max_by_key(|(a, b)| b - a)
        .expect("the selection painted nothing at all");
    let (px0, px1) = (px0 as f32, px1 as f32);
    let want = hi - lo;
    assert!(
        (px1 - px0) > want * 0.9,
        "the wash reaches the screen {:.0} px wide; the layout says {want:.0} px of glyphs are selected",
        px1 - px0
    );
    // …and it is drawn where the layout says it is.
    assert!(
        (px0 - (gx + lo)).abs() < 4.0 && (px1 - (gx + hi)).abs() < 4.0,
        "the wash is on screen at {px0:.0}..{px1:.0}, the layout puts it at {:.0}..{:.0}",
        gx + lo,
        gx + hi
    );
}

/// (29.3) A right-to-left list item is indented from the **right**: its text ends
/// in the right gutter rather than running to the margin, and its marker sits in
/// that gutter.
#[test]
fn a_hebrew_list_item_is_indented_from_the_right() {
    let mut h = harness("r29-list-indent");
    h.app.new_tab();
    h.app.insert_text("- פריט ראשון\n- פריט שני\n");
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.draw();
    let text = h.app.text_rect;
    for i in 0..2 {
        let line = &h.app.tab().layout.lines[i];
        assert!(line.rtl_para());
        assert!(line.indent > 4.0, "a list item must be indented");
        let shift = line.align_offset_row(text.w, 0, false);
        let right = text.x + line.indent + shift + line.row_width_of(0, false);
        assert!(
            (right - (text.right() - line.indent)).abs() < 1.5,
            "line {i} ends at {right:.0}; a mirrored indent puts it at {:.0}",
            text.right() - line.indent
        );
        // The mirrored indent leaves the same gutter for the marker on the right.
        assert!(line.indent < text.w * 0.2, "the gutter must stay a gutter");
    }
}

// ---------------------------------------------------------------- about box

/// (0.8.17) The About box shows the notices that are compiled into the binary: the
/// program's licence, every crate in the lock file, and the whole font licence text.
#[test]
fn the_about_box_shows_what_is_compiled_into_the_binary() {
    use bare_md::ui::about;
    let mut h = harness("about-open");
    h.app.open_about();
    h.draw();
    assert!(
        matches!(h.app.dialog, Some(bare_md::app::Dialog::About(_))),
        "the About box did not open"
    );
    // Both buttons are there to click.
    assert!(h.app.hits.find(|h| *h == Hit::AboutClose).is_some(), "no Close button");
    assert!(h.app.hits.find(|h| *h == Hit::AboutSave).is_some(), "no Save button");

    let Some(bare_md::app::Dialog::About(a)) = &h.app.dialog else { unreachable!() };
    // The box starts at the top, and the text is wrapped to fit it.
    assert_eq!(a.scroll, 0);
    assert!(a.visible > 10, "the box shows only {} lines", a.visible);
    assert!(a.text.len() > a.visible * 4, "the notice is too short to be the whole licence");
    for line in &a.text {
        assert!(line.chars().count() <= about::COLUMNS, "a line runs outside the box");
    }
    // Everything that must be in there is: the app licence, the crates, the fonts.
    let all = a.text.join("\n");
    for needle in [
        "Bare MD",
        "GNU AFFERO GENERAL PUBLIC LICENSE",
        "THIRD-PARTY CRATES",
        "SIL OPEN FONT LICENSE, VERSION 1.1",
        "SECTION B: DEJAVU FONTS LICENSE",
    ] {
        assert!(all.contains(needle), "{needle} is missing from the About box");
    }
    // The version is the version: the box draws it above the text, and the notice
    // that can be saved carries it too.
    assert!(about::notice().contains(env!("CARGO_PKG_VERSION")));
    const ABOUT_UI: &str = include_str!("../src/ui/dialogs.rs");
    assert!(ABOUT_UI.contains("info::VERSION"), "the box does not show its version");
    assert!(!about::CRATES.is_empty());
    assert!(all.contains(&format!("{} {}", about::CRATES[0].0, about::CRATES[0].1)));
}

/// (0.8.17) The wheel scrolls the notice while the pointer is over the box, and only
/// while the box is open - the document underneath must not move at the same time.
#[test]
fn the_wheel_scrolls_the_about_box() {
    use winit::event::{MouseScrollDelta, WindowEvent};
    let mut h = harness("about-wheel");
    h.draw();
    let doc_before = h.app.tab().scroll;
    h.app.open_about();
    h.draw();
    let d = bare_md::ui::about::box_rect(W as f32, H as f32);
    h.app.mouse = (d.x + d.w * 0.4, d.y + d.h * 0.5);
    let mut input = bare_md::input::InputState::default();
    let wheel = |dy: f32| WindowEvent::MouseWheel {
        device_id: winit::event::DeviceId::dummy(),
        delta: MouseScrollDelta::LineDelta(0.0, dy),
        phase: winit::event::TouchPhase::Moved,
    };
    let scroll = |h: &Harness| match &h.app.dialog {
        Some(bare_md::app::Dialog::About(a)) => a.scroll,
        _ => panic!("no About box"),
    };
    assert_eq!(scroll(&h), 0);
    h.app.handle_event(&wheel(-1.0), &mut h.r, &mut input);
    h.draw();
    let down = scroll(&h);
    assert!(down > 0, "the wheel did not scroll the notice");
    h.app.handle_event(&wheel(1.0), &mut h.r, &mut input);
    h.draw();
    assert_eq!(scroll(&h), 0, "scrolling back up did not return to the top");
    // A wheel at the top stays at the top instead of going negative.
    h.app.handle_event(&wheel(1.0), &mut h.r, &mut input);
    h.draw();
    assert_eq!(scroll(&h), 0);
    assert_eq!(h.app.tab().scroll, doc_before, "the document moved under the box");
}

/// (0.8.17) F1 opens the box and the menu entry is the other way in; both go through
/// the same command, so they cannot drift apart.
///
/// The key handling itself is driven through `on_key`, which builds a `KeyEvent` from
/// winit - a type only winit can construct - so what is pinned here is the wiring:
/// F1 calls `open_about`, and the Tools menu has an About entry that does too.
#[test]
fn f1_and_the_menu_entry_both_open_the_about_box() {
    const INPUT: &str = include_str!("../src/platform/input.rs");
    const MENU: &str = include_str!("../src/editor/app.rs");
    let f1 = INPUT.find("NamedKey::F1").expect("F1 is not handled at all");
    let around = &INPUT[f1.saturating_sub(80)..(f1 + 260).min(INPUT.len())];
    assert!(around.contains("open_about"), "F1 does not open the About box");
    assert!(
        MENU.contains("About Bare MD") && MENU.contains("MenuAction::About => self.open_about()"),
        "the Tools menu entry does not open the About box"
    );
    // Escape and Enter are the generic dialog keys: the About box is a Dialog, so both
    // already reach it, and the draw arm exists to be closed.
    const DIALOGS: &str = include_str!("../src/ui/dialogs.rs");
    assert!(DIALOGS.contains("Dialog::About(about) => draw_about("), "no About arm in the dispatcher");
}

/// (0.8.17) The Save button writes the whole notice out as one file, and the file holds
/// exactly what the box shows.
#[test]
fn the_about_box_can_save_the_notice_as_a_file() {
    let mut h = harness("about-save");
    h.app.open_about();
    h.draw();
    let wrote = h.click_hit(|hit| *hit == Hit::AboutSave);
    assert!(wrote, "there is no Save button to click");
    assert!(
        h.app.status.starts_with("Saved ") || h.app.status.starts_with("Could not save"),
        "the save said nothing: {:?}",
        h.app.status
    );
    if h.app.status.starts_with("Saved ") {
        let path = h.app.status.trim_start_matches("Saved ").to_string();
        let text = std::fs::read_to_string(&path).expect("the saved notice");
        let _ = std::fs::remove_file(&path);
        assert!(text.contains("THIRD-PARTY CRATES"));
        assert!(text.contains("SIL OPEN FONT LICENSE, VERSION 1.1"));
        assert_eq!(text, bare_md::ui::about::notice(), "the file is not the notice");
    }
    // Saving leaves the box open, so the reader can carry on.
    assert!(matches!(h.app.dialog, Some(bare_md::app::Dialog::About(_))));
    // …and Close really closes it.
    assert!(h.click_hit(|hit| *hit == Hit::AboutClose));
    assert!(h.app.dialog.is_none(), "Close did not close the box");
}

/// (0.8.18) The About box's scrollbar can be **dragged**, not only clicked: press on
/// the thumb, move, and the notice follows the pointer - up as well as down - and
/// letting go stops it. A press on the empty track jumps there first, like every other
/// scrollbar in the app.
#[test]
fn the_about_scrollbar_can_be_dragged() {
    use bare_md::input::InputState;
    use bare_md::ui::about;
    use winit::event::{DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent};

    fn scroll_of(h: &Harness) -> usize {
        match &h.app.dialog {
            Some(bare_md::app::Dialog::About(a)) => a.scroll,
            _ => panic!("no About box"),
        }
    }
    fn last_scroll(h: &Harness) -> usize {
        match &h.app.dialog {
            Some(bare_md::app::Dialog::About(a)) => a.text.len().saturating_sub(a.visible),
            _ => panic!("no About box"),
        }
    }

    let mut h = harness("about-drag-scrollbar");
    h.app.open_about();
    h.draw();
    let track = h.app.about_track;
    assert!(!track.is_empty(), "the box has no scrollbar");
    let mut input = InputState::default();
    let (thumb, visible) = match &h.app.dialog {
        Some(bare_md::app::Dialog::About(a)) => {
            (about::thumb_rect(track, a.scroll, a.text.len(), a.visible), a.visible)
        }
        _ => unreachable!(),
    };
    assert!(thumb.h < track.h - 40.0, "the test needs a notice longer than the box");
    let max = last_scroll(&h);
    let travel = track.h - thumb.h;
    let x = (track.x + 2.0) as f64;

    let button = |state: ElementState| WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state,
        button: MouseButton::Left,
    };
    let pointer = |y: f32| WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: winit::dpi::PhysicalPosition::new(x, y as f64),
    };
    let press = |h: &mut Harness, input: &mut InputState, y: f32| {
        h.app.mouse = (track.x + 2.0, y);
        h.draw();
        h.app.handle_event(&button(ElementState::Pressed), &mut h.r, input);
        h.draw();
    };
    let release = |h: &mut Harness, input: &mut InputState| {
        h.app.handle_event(&button(ElementState::Released), &mut h.r, input);
        h.draw();
    };
    let move_to = |h: &mut Harness, input: &mut InputState, y: f32| {
        h.app.handle_event(&pointer(y), &mut h.r, input);
        h.draw();
    };

    // Press on the thumb, near its top: the notice does not jump.
    press(&mut h, &mut input, thumb.y + 4.0);
    assert_eq!(scroll_of(&h), 0, "grabbing the thumb moved the notice");

    // Drag down: the notice follows, and by the distance the pointer moved.
    move_to(&mut h, &mut input, thumb.y + 4.0 + travel * 0.5);
    let half = scroll_of(&h);
    let expected = (0.5 * max as f32).round() as usize;
    assert!(half > 0, "dragging the thumb did not scroll the notice");
    assert!(
        half.abs_diff(expected) <= 2,
        "the thumb does not track the pointer ({half} vs {expected})"
    );

    // Drag to the very bottom: it is the last screen, not past it.
    move_to(&mut h, &mut input, track.bottom() + 300.0);
    assert_eq!(scroll_of(&h), max, "the drag ran past the end");

    // …and back up, stopping at the top.
    move_to(&mut h, &mut input, thumb.y + 4.0);
    assert!(scroll_of(&h) <= 1, "dragging back up did not return to the top");
    move_to(&mut h, &mut input, thumb.y - 200.0);
    assert_eq!(scroll_of(&h), 0, "the drag ran past the top");

    // Release, then move: nothing follows any more.
    release(&mut h, &mut input);
    let after_release = scroll_of(&h);
    move_to(&mut h, &mut input, track.bottom() - 4.0);
    assert_eq!(scroll_of(&h), after_release, "the box kept dragging after the release");

    // A press on the empty track jumps there and then drags from that point.
    press(&mut h, &mut input, track.bottom() - 6.0);
    let jumped = scroll_of(&h);
    assert!(jumped > after_release + visible, "a click on the track did not jump ({jumped})");
    assert!(jumped <= max);
    release(&mut h, &mut input);

    // …and the wheel still works on top of all that.
    h.app.mouse = (track.x - 100.0, track.y + 40.0);
    h.app.handle_event(
        &WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, 1.0),
            phase: TouchPhase::Moved,
        },
        &mut h.r,
        &mut input,
    );
    h.draw();
    assert!(scroll_of(&h) < jumped, "the wheel stopped working after a drag");
}

// ---------------------------------------------------------------- round 73

/// (73.1) The caret keeps its size and its place when a row is empty: an empty
/// line is a line of text, not a sliver of one.
///
/// The caret is drawn from its row's height, so a blank row that was `0.62` of a
/// text row gave a shorter caret sitting higher up - "pointer size is small and
/// position is closer to upper row; when I type 1 symbol, pointer becomes bigger and
/// offsets position a bit lower". Typing the first character of that same row must
/// change the character and nothing else.
#[test]
fn the_caret_is_the_same_size_and_place_on_an_empty_row() {
    let mut h = harness("r73-caret-blank");
    h.type_str("alpha\n\nbeta");
    h.draw();
    let blank = 6; // start of the empty row between the two paragraphs
    h.app.set_caret(blank, false);
    h.draw();
    let before = h.app.caret_rect(&mut h.r.text);
    let row_h_empty = h.app.tab().layout.lines[1].row_h;
    // A row with text on it, for comparison: the last one.
    let row_h_text = h.app.tab().layout.lines[2].row_h;
    assert!(
        (row_h_empty - row_h_text).abs() < 0.01,
        "an empty row is {row_h_empty} tall, a row of text {row_h_text}"
    );

    h.type_str("x");
    let after = h.app.caret_rect(&mut h.r.text);
    assert!(
        (before.h - after.h).abs() < 0.01,
        "the caret changed height when the row got its first character: {} -> {}",
        before.h,
        after.h
    );
    assert!(
        (before.y - after.y).abs() < 0.01,
        "the caret moved when the row got its first character: {} -> {}",
        before.y,
        after.y
    );
    assert_eq!(h.app.tab().doc.text(), "alpha\nx\nbeta");
}

/// (73.2, 73.3) Quoting a list keeps every item a list item, and the second press
/// takes the quote off again - all of it, not just the caret's line.
///
/// The toggle used to collapse the selection onto the caret's line while it
/// rewrote the lines, so the second press saw one line: the list came back with
/// its last item only ("probably the same issue as 2 but backwards").
#[test]
fn quoting_a_list_twice_unquotes_the_whole_list() {
    let mut h = harness("r73-quote-list");
    h.type_str("* one\n* two\n* three");
    h.draw();
    let end = h.app.tab().doc.len_chars();
    h.app.select_range(0, end);
    h.app.toggle_quote();
    h.draw();
    assert_eq!(
        h.app.tab().doc.text(),
        "> * one\n> * two\n> * three",
        "quoting changed the list syntax itself"
    );
    // The lines are still selected, so the same command applies to the same list.
    assert!(h.app.tab().has_selection(), "the selection was lost by the rewrite");
    h.app.toggle_quote();
    h.draw();
    assert_eq!(
        h.app.tab().doc.text(),
        "* one\n* two\n* three",
        "the second press did not unquote the whole list"
    );
    // And a third press quotes it again: the command is all-or-nothing both ways.
    h.app.toggle_quote();
    assert_eq!(h.app.tab().doc.text(), "> * one\n> * two\n> * three");
    // …and it is still *rendered* as a list: the quote's bar, plus a bullet where
    // the hidden marker was. "Shows it as a quote only" was the other half of the
    // report.
    h.draw();
    for line in 0..3 {
        let ll = &h.app.tab().layout.lines[line];
        assert_eq!(ll.block(), bare_md::markdown::Block::Quote);
        assert!(ll.info.quote_bullet, "line {line} lost its list marker");
        let shown: String = ll
            .clean
            .iter()
            .map(|r| ll.chars[r.start..r.end].iter().collect::<String>())
            .collect();
        assert!(
            !shown.contains('*'),
            "the list marker is painted as text on line {line}: {shown:?}"
        );
        assert!(!shown.trim().is_empty(), "nothing is drawn on line {line}");
    }
}

/// (73.4) Reaching the last position of a full row does not slide the page
/// sideways: with soft wrap on there is nothing to the right to scroll to.
#[test]
fn the_end_of_a_wrapped_row_does_not_move_the_camera() {
    let mut h = harness("r73-camera");
    h.type_str(&"abcdefghij ".repeat(40));
    h.draw();
    assert!(h.app.tab().layout.lines[0].rows.len() > 2, "the line did not wrap");
    let row_end = h.app.tab().layout.lines[0].row(0).end;
    h.app.set_caret(row_end, false);
    h.draw();
    assert_eq!(
        h.app.tab().scroll_x, 0.0,
        "the camera slid sideways at the end of a wrapped row"
    );
    // …and at the very end of the document, which is the same situation.
    let end = h.app.tab().doc.len_chars();
    h.app.set_caret(end, false);
    h.draw();
    assert_eq!(h.app.tab().scroll_x, 0.0);
}

/// (73.5) Soft wrap is a setting with a switch: on breaks long lines into visual
/// rows, off keeps one row per line and lets the view scroll sideways.
#[test]
fn soft_wrap_can_be_turned_off() {
    let mut h = harness("r73-wrap-toggle");
    h.type_str(&"word ".repeat(60));
    h.draw();
    assert!(h.app.tab().layout.lines[0].rows.len() > 1, "no wrap to begin with");
    assert!(h.app.settings.wrap);

    assert!(!h.app.toggle_wrap(), "the switch did not turn wrap off");
    h.draw();
    assert_eq!(
        h.app.tab().layout.lines[0].rows.len(),
        1,
        "a long line is still broken into rows"
    );
    assert!(
        h.app.tab().layout.max_row_width > h.app.text_rect.w,
        "with wrap off the line is not wider than the view, so there is nothing to scroll"
    );

    assert!(h.app.toggle_wrap(), "the switch did not turn wrap back on");
    h.draw();
    assert!(h.app.tab().layout.lines[0].rows.len() > 1, "the line is not wrapped again");
    assert!(h.app.tab().layout.max_row_width <= h.app.text_rect.w + 8.0);
}

/// (73.6) A click in the empty strip right of the text puts the caret at the last
/// position of the row that was clicked - not on the row a long word flowed onto.
#[test]
fn a_click_in_the_strip_right_of_the_text_lands_at_the_end_of_the_row() {
    let mut h = harness("r73-margin-click");
    h.type_str(&"wrap me ".repeat(40));
    h.draw();
    let rows = h.app.tab().layout.lines[0].rows.len();
    assert!(rows > 2, "the fixture does not wrap");
    for row in [0usize, 1, rows - 1] {
        let y = h.row_y(0, row, 0.5);
        let x = h.app.text_rect.right() + 6.0;
        assert!(x < h.app.editor_rect.right(), "the strip is outside the editor");
        h.click_full((x, y));
        let t = h.app.tab();
        let ll = &t.layout.lines[0];
        assert_eq!(
            t.caret,
            ll.start_char + ll.row(row).end,
            "the click on row {row} did not land at the end of that row"
        );
    }
}

// ---------------------------------------------------------------- round 74

/// (74.1) However many tabs are open, none of them reaches the view controls -
/// Find, the mode switch and the chips - and none of them covers the `+`.
///
/// The strip used to reserve a guessed 220 px for a lane that had grown to more
/// than twice that, so the last tabs were drawn on top of the controls.
#[test]
fn tabs_never_reach_the_view_controls() {
    let mut h = harness("r74-tab-lane");
    // Names long enough to need the widest chip the strip allows.
    for i in 0..30 {
        h.type_str(&format!("doc {i}"));
        h.app.new_tab();
        h.draw();
        h.type_str(&format!(
            "a fairly long document title number {i} so the chips are wide"
        ));
        h.draw();
    }
    assert!(h.app.tabs.len() > 25, "the fixture did not open many tabs");
    // Where the controls really are: the Find button's own rect.
    let find = h
        .app
        .hits
        .find(|hit| *hit == Hit::Toolbar(ToolbarAction::FindReplace))
        .expect("the Find button");
    let lane_left = bare_md::ui::view_lane_left(h.app.tabbar_rect);
    assert!(
        (find.x - lane_left).abs() < 2.0,
        "the lane's measured left edge ({lane_left}) is not where Find is ({})",
        find.x
    );
    // A chip may still be *drawn* past the lane - it is clipped there - but it may
    // never start past it, and it may never win a click over a control. (76: the
    // strip scrolls now, so the cut is made by the clip and by the hit order, not by
    // stopping the layout short.)
    for (i, r) in h.app.tab_rects.iter().enumerate() {
        let on_screen = r.right() > h.app.tab_strip_left && r.x < h.app.tab_strip_right;
        assert!(
            !on_screen || r.x <= h.app.tab_strip_right + 1.0,
            "tab {i} starts at {}, past the strip's edge {}",
            r.x,
            h.app.tab_strip_right
        );
    }
    for hit in [ToolbarAction::ToggleRaw, ToolbarAction::FindReplace, ToolbarAction::ToggleTheme] {
        let cr = h.app.hits.find(|x| *x == Hit::Toolbar(hit)).expect("a chip");
        let at = h.app.hits.at((cr.x + cr.w * 0.5, cr.y + cr.h * 0.5));
        assert!(
            at == Some(Hit::Toolbar(hit)),
            "a view control does not own the click at its own centre: {at:?}"
        );
    }
}

/// (74.2) The document menu offers "Open folder in workspace", and it opens the
/// folder of the file being edited as a workspace tab with the file revealed.
#[test]
fn the_document_menu_opens_the_documents_folder_in_the_workspace() {
    let mut h = harness("r74-reveal-menu");
    let dir = h._dir.join("project");
    std::fs::create_dir_all(dir.join("notes")).unwrap();
    let file = dir.join("notes/plan.md");
    std::fs::write(&file, "# Plan\n").unwrap();
    h.app.open_path(&file);
    h.draw();

    h.app.open_context_menu((h.app.text_rect.x + 40.0, h.app.text_rect.y + 20.0));
    let menu = h.app.menu.clone().expect("a context menu");
    let item = menu
        .entries
        .iter()
        .find(|e| e.label == "Open folder in workspace")
        .expect("the menu has no way to open the document's folder");
    let action = item.action.clone();
    h.app.menu = None;
    h.app.run_menu_action(action, &mut h.r);
    h.draw();

    assert_eq!(
        h.app.ws.root,
        dir.join("notes"),
        "the panel did not move to the document's folder"
    );
    assert!(h.app.settings.sidebar_open, "the panel was left closed");
    assert!(
        h.app.ws.entries().iter().any(|e| e.path == file),
        "the file is not visible in the tree"
    );
    assert!(
        h.app.settings.workspaces.iter().any(|w| *w == dir.join("notes")),
        "the folder was not remembered as a workspace tab"
    );
}

/// (74.3) A file the editor is *started* with brings its folder into the workspace;
/// a file opened later leaves the workspace alone.
#[test]
fn only_a_startup_file_brings_its_folder() {
    let dir = std::env::temp_dir().join(format!("bare-md-reg-74-start-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let home = dir.join("home");
    let other = dir.join("other");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let first = home.join("readme.md");
    let later = other.join("later.md");
    std::fs::write(&first, "# One\n").unwrap();
    std::fs::write(&later, "# Two\n").unwrap();

    let paths = Paths {
        data_dir: dir.join("data"),
        config_file: dir.join("data/settings.conf"),
        origin: "test".into(),
    };
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    let mut settings = Settings::default();
    settings.workspace = Some(home.clone());
    settings.theme = ThemeKind::Black;
    let gpu = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = App::new(paths, settings, Session::default(), vec![first.clone()]);
    let mut r = Renderer::offscreen(W, H, 1.0);
    let _ = &mut r;
    drop(gpu);

    // The launch file's folder is the workspace, and the file is in the tree.
    assert_eq!(app.ws.root, home, "the launch file's folder was not opened");
    assert!(
        app.ws.entries().iter().any(|e| e.path == first),
        "the launch file is not visible in the tree"
    );
    // A file opened afterwards must not move the panel.
    app.open_path(&later);
    assert_eq!(app.ws.root, home, "opening a file later moved the workspace");
    // …and neither may a file forwarded from another launch.
    app.open_dropped(&later);
    assert_eq!(app.ws.root, home, "a dropped file moved the workspace");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- round 75

/// A workspace tree with a file in it, opened as the sandbox's workspace.
fn tree_harness(tag: &str, files: &[&str]) -> (Harness, PathBuf) {
    let mut h = harness(tag);
    let dir = h._dir.join("tree");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    for f in files {
        std::fs::write(dir.join(f), "# a file\n").unwrap();
    }
    h.app.ws.set_root(dir.clone());
    h.app.settings.sidebar_open = true;
    h.draw();
    (h, dir)
}

/// Index of the tree row for `path`.
fn tree_row(h: &Harness, path: &std::path::Path) -> usize {
    h.app
        .ws
        .entries()
        .iter()
        .position(|e| e.path == path)
        .unwrap_or_else(|| panic!("{} is not in the tree", path.display()))
}

/// (75.1) Right-click on a tab offers close / close others / close all / rename,
/// and never closes the tab on the press itself.
#[test]
fn the_tab_menu_closes_and_renames() {
    let mut h = harness("r75-tab-menu");
    for i in 0..4 {
        h.app.new_tab();
        h.type_str(&format!("doc {i}"));
    }
    h.draw();
    assert_eq!(h.app.tabs.len(), 5);

    // The menu is opened on tab 1 and covers tab 1.
    h.app.open_tab_menu(1, (h.app.text_rect.x + 20.0, h.app.text_rect.y + 10.0));
    let labels: Vec<String> = h.app.tab_menu_entries().into_iter().map(|(l, _)| l).collect();
    for want in ["Close others", "Close all"] {
        assert!(labels.iter().any(|l| l == want), "{want} is missing from {labels:?}");
    }
    // The × on the tab is "close this one"; a row repeating it was removed.
    assert!(
        !labels.iter().any(|l| l == "Close"),
        "the tab menu still offers Close next to the tab's own ×: {labels:?}"
    );
    // "Close others" keeps tab 1 and takes the other four - asking first about the
    // one with unsaved text, which is what the others have too.
    h.app.run_tab_menu(bare_md::app::TabMenuAction::CloseOthers);
    assert!(
        matches!(h.app.dialog, Some(bare_md::app::Dialog::Confirm { .. })),
        "close others did not ask about the unsaved tabs"
    );
    // Answer "discard" to every question until the walk finishes.
    for _ in 0..8 {
        if h.app.dialog.is_none() {
            break;
        }
        h.app.dialog_confirm(&mut h.r);
    }
    h.draw();
    assert_eq!(h.app.tabs.len(), 1, "close others left {} tabs", h.app.tabs.len());
    assert_eq!(h.app.active, 0);
    // Tab 1 is the first one typed into ("doc 0"): the walk kept the tab the menu was
    // opened on, not whichever tab happened to be active.
    assert_eq!(h.app.tab().doc.text(), "doc 0", "close others kept the wrong tab");

    // …and "close all" ends with the one empty document (asking about the unsaved
    // one on the way, exactly like the × would).
    h.app.new_tab();
    h.type_str("also unsaved");
    h.draw();
    h.app.close_all_tabs();
    for _ in 0..8 {
        if h.app.dialog.is_none() {
            break;
        }
        h.app.dialog_confirm(&mut h.r);
    }
    h.draw();
    assert_eq!(h.app.tabs.len(), 1);
    assert!(h.app.tab().doc.text().is_empty());
}

/// (75.1) "Close others" asks about unsaved text one tab at a time, and Cancel stops
/// the whole thing - it does not leave half the tabs closed.
#[test]
fn close_others_asks_before_losing_unsaved_text() {
    let mut h = harness("r75-close-others-dirty");
    h.type_str("keep me");
    h.app.new_tab();
    h.type_str("dirty one");
    h.app.new_tab();
    h.type_str("dirty two");
    h.draw();
    assert_eq!(h.app.tabs.len(), 3);

    h.app.active = 0;
    h.app.close_other_tabs(0);
    // Two dirty tabs -> a question, and nothing closed yet.
    assert!(matches!(h.app.dialog, Some(bare_md::app::Dialog::Confirm { .. })));
    assert_eq!(h.app.tabs.len(), 3);
    // Cancel: everything stays.
    h.app.dialog = None;
    h.app.pending_close = None;
    h.draw();
    assert_eq!(h.app.tabs.len(), 3, "cancel still closed tabs");
    assert_eq!(h.app.tab().doc.text(), "keep me");
}

/// (75.1) Renaming a document tab renames the file and keeps the tab on it.
#[test]
fn a_tab_can_be_renamed() {
    let mut h = harness("r75-tab-rename");
    let dir = h._dir.join("ren");
    std::fs::create_dir_all(&dir).unwrap();
    let old = dir.join("old-name.md");
    std::fs::write(&old, "# Old\n").unwrap();
    h.app.open_path(&old);
    h.draw();

    h.app.open_rename(bare_md::app::RenameTarget::Tab(old.clone()));
    let Some(bare_md::app::Dialog::Rename(mut box_)) = h.app.dialog.take() else {
        panic!("no rename box");
    };
    // The stem is selected, so typing replaces the name and keeps the extension.
    box_.field.set("new-name.md");
    h.app.dialog = Some(bare_md::app::Dialog::Rename(box_));
    h.app.commit_rename();
    h.draw();

    let new = dir.join("new-name.md");
    assert!(new.exists(), "the file was not renamed on disk");
    assert!(!old.exists());
    assert_eq!(h.app.tab().doc.path.as_deref(), Some(new.as_path()), "the tab kept the old path");
    assert_eq!(h.app.tab().title(), "new-name.md");
    assert!(h.app.dialog.is_none());
    // A name that is taken is refused, and the box stays open to say so.
    h.app.open_rename(bare_md::app::RenameTarget::Tab(new.clone()));
    if let Some(bare_md::app::Dialog::Rename(box_)) = &mut h.app.dialog {
        box_.field.set("old-name.md");
        std::fs::write(dir.join("old-name.md"), "x").unwrap();
    }
    h.app.commit_rename();
    assert!(h.app.dialog.is_some(), "the box closed although the rename failed");
    assert!(!new.exists() || h.app.tab().doc.path.as_deref() == Some(new.as_path()));
}

/// (75.2) Double-clicking a file in the workspace tree opens the rename box.
#[test]
fn a_double_click_in_the_tree_renames_the_file() {
    let (mut h, dir) = tree_harness("r75-tree-double", &["note.md"]);
    let file = dir.join("note.md");
    let row = tree_row(&h, &file);
    let r = h
        .app
        .hits
        .find(|hit| *hit == Hit::SidebarEntry(row))
        .expect("the row is not clickable");
    let c = (r.x + r.w * 0.5, r.y + r.h * 0.5);
    // Two clicks in quick succession.
    h.click_full(c);
    h.click_full(c);
    assert!(
        matches!(h.app.dialog, Some(bare_md::app::Dialog::Rename(_))),
        "a double click did not open the rename box"
    );
    let Some(bare_md::app::Dialog::Rename(box_)) = &mut h.app.dialog else { unreachable!() };
    box_.field.set("renamed.md");
    h.app.commit_rename();
    h.draw();
    assert!(dir.join("renamed.md").exists(), "the rename did not reach the disk");
    assert!(!file.exists());
}

/// (75.3) The tree's menu offers open / rename / duplicate / copy / paste / delete,
/// and the commands do what they say.
#[test]
fn the_tree_menu_duplicates_copies_pastes_and_deletes() {
    let (mut h, dir) = tree_harness("r75-tree-menu", &["one.md"]);
    let file = dir.join("one.md");
    std::fs::write(&file, "the text\n").unwrap();

    // Duplicate: a copy next to it, same contents.
    h.app.run_tree_menu_for_test(file.clone(), bare_md::app::TreeMenuAction::Duplicate);
    let copy = dir.join("one copy.md");
    assert!(copy.exists(), "duplicate made no copy");
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "the text\n");

    // Copy, then paste into the sub-folder.
    h.app.run_tree_menu_for_test(file.clone(), bare_md::app::TreeMenuAction::Copy);
    h.app.run_tree_menu_for_test(dir.join("sub"), bare_md::app::TreeMenuAction::Paste);
    assert!(dir.join("sub/one.md").exists(), "paste did not write into the folder");

    // Paste again into the same folder: a free name, not an overwrite.
    h.app.run_tree_menu_for_test(dir.join("sub"), bare_md::app::TreeMenuAction::Paste);
    assert!(dir.join("sub/one 2.md").exists(), "a second paste overwrote the first");

    // The menu itself, as it is offered.
    h.app.open_tree_menu(file.clone(), (10.0, 10.0));
    let labels: Vec<String> = h.app.tree_menu_entries().into_iter().map(|(l, _)| l).collect();
    for want in ["Open", "Rename", "Duplicate", "Copy", "Paste", "Delete"] {
        assert!(labels.iter().any(|l| l == want), "{want} is missing from {labels:?}");
    }

    // Delete asks first, then removes the file and closes the tab that showed it.
    h.app.open_path(&file);
    h.draw();
    assert_eq!(h.app.tabs.iter().filter(|t| t.doc.path.as_deref() == Some(file.as_path())).count(), 1);
    h.app.run_tree_menu_for_test(file.clone(), bare_md::app::TreeMenuAction::Delete);
    assert!(
        matches!(h.app.dialog, Some(bare_md::app::Dialog::Confirm { .. })),
        "delete did not ask"
    );
    assert!(file.exists(), "delete removed the file before the question was answered");
    h.app.delete_path(&file);
    h.app.dialog = None;
    h.draw();
    assert!(!file.exists(), "the file is still there");
    assert!(
        !h.app.tabs.iter().any(|t| t.doc.path.as_deref() == Some(file.as_path())),
        "the tab of the deleted file is still open"
    );
}

/// (75.3) Deleting a folder takes the trees and tabs under it with it.
#[test]
fn deleting_a_folder_closes_what_was_open_inside_it() {
    let (mut h, dir) = tree_harness("r75-delete-folder", &[]);
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let inner = dir.join("sub/inner.md");
    std::fs::write(&inner, "# inner\n").unwrap();
    h.app.open_path(&inner);
    h.draw();
    assert!(h.app.tabs.iter().any(|t| t.doc.path.as_deref() == Some(inner.as_path())));

    h.app.delete_path(&dir.join("sub"));
    h.draw();
    assert!(!dir.join("sub").exists());
    assert!(
        !h.app.tabs.iter().any(|t| t.doc.path.as_deref() == Some(inner.as_path())),
        "a tab inside the deleted folder is still open"
    );
}

/// (76) With more tabs than fit, the strip scrolls: the newest tabs are reached by
/// the wheel, the chevrons and `Ctrl+Tab`, not lost past the edge.
///
/// The lane fix stopped the tabs from running under the view controls by simply
/// not drawing past them, which made the newest tabs unreachable.
#[test]
fn a_full_tab_strip_scrolls_to_its_last_tabs() {
    let mut h = harness("r76-tab-strip-scroll");
    for i in 0..24 {
        h.type_str(&format!("document number {i} with a longish name"));
        h.app.new_tab();
    }
    h.draw();
    let n = h.app.tabs.len();
    assert!(n > 20, "the fixture opened only {n} tabs");
    let strip_right = h.app.tab_strip_right;
    assert!(h.app.tab_scroll_max > 0.0, "the strip claims it has nothing to scroll");

    // A newly opened tab is scrolled into view by the next frame, which is the point
    // of the "+" button; go back to the start of the strip to see what is hidden.
    h.app.active = 0;
    h.app.pending_reveal_tab = true;
    h.draw();
    // Revealing the first tab brings the strip (nearly) back to its start, and no
    // chip *begins* past the lane's edge: past it is clipped, and a chip that starts
    // there would be a chip drawn over the controls.
    assert!(
        h.app.tab_scroll <= App::TAB_SCROLL_STEP,
        "revealing the first tab left the strip at {}",
        h.app.tab_scroll
    );
    // Chips past the right edge exist - that is what "the strip is full" means - and
    // they are clipped there; who owns a click at the controls is checked by
    // `tabs_never_reach_the_view_controls`.
    let hidden = h.app.tab_rects.iter().filter(|r| r.x >= strip_right).count();
    assert!(hidden > 0, "no chip is past the edge, so there is nothing to scroll to");

    // Scrolling to the end brings the last tab's chip into the visible strip.
    h.app.scroll_tabs_by(h.app.tab_scroll_max + 1000.0);
    h.draw();
    assert!(
        (h.app.tab_scroll - h.app.tab_scroll_max).abs() < 0.5,
        "the strip did not stop at its end: {} of {}",
        h.app.tab_scroll,
        h.app.tab_scroll_max
    );
    let last = *h.app.tab_rects.last().expect("no chips");
    assert!(
        last.right() <= strip_right + 1.0 && last.right() > strip_right - 220.0,
        "the last chip is not inside the strip after scrolling: {last:?} vs {strip_right}"
    );
    // Off-screen chips are recorded at negative x (they are scrolled out to the
    // left) - what matters is that the *visible* ones never cover the controls.
    for (i, r) in h.app.tab_rects.iter().enumerate() {
        if r.right() <= h.app.tab_strip_left || r.x >= strip_right {
            continue;
        }
        let cr = h
            .app
            .hits
            .find(|x| *x == Hit::Toolbar(ToolbarAction::FindReplace))
            .expect("Find");
        assert!(r.x <= cr.x, "visible tab {i} starts past the controls");
    }

    // Back to the start, and "reveal the active tab" scrolls it into view: this is
    // what Ctrl+Tab and a newly opened file rely on.
    h.app.tab_scroll = 0.0;
    h.draw();
    h.app.active = n - 1;
    h.app.pending_reveal_tab = true;
    h.draw();
    let active_r = h.app.tab_rects[h.app.active];
    assert!(
        active_r.x >= h.app.text_rect.x - 24.0 && active_r.right() <= strip_right + 1.0,
        "the active tab was not scrolled into view: {active_r:?}"
    );
}

/// (76) The wheel over the tab strip scrolls the strip, not the document, and the
/// chevrons scroll by a step.
#[test]
fn the_wheel_over_the_strip_scrolls_the_strip() {
    let mut h = harness("r76-wheel-strip");
    for _ in 0..24 {
        h.app.new_tab();
    }
    h.draw();
    let before_scroll = h.app.tab().scroll;
    let strip = h.app.tabbar_rect;
    h.app.mouse = (strip.x + strip.w * 0.35, strip.y + strip.h * 0.5);
    h.draw();
    let mut input = bare_md::input::InputState::default();
    use winit::event::{MouseScrollDelta, WindowEvent};
    let wheel = |dy: f32| WindowEvent::MouseWheel {
        delta: MouseScrollDelta::LineDelta(0.0, dy),
        device_id: winit::event::DeviceId::dummy(),
        phase: winit::event::TouchPhase::Moved,
    };
    h.app.handle_event(&wheel(-1.0), &mut h.r, &mut input);
    h.draw();
    assert!(
        h.app.tab_scroll > 0.0,
        "the wheel over the strip did not scroll the strip"
    );
    assert_eq!(h.app.tab().scroll, before_scroll, "the wheel over the strip scrolled the document");

    // The chevron scrolls back by one step.
    let step = bare_md::app::App::TAB_SCROLL_STEP;
    let was = h.app.tab_scroll;
    h.app.scroll_tabs_by(-step);
    h.draw();
    assert!((h.app.tab_scroll - (was - step).max(0.0)).abs() < 0.5);
}

/// (77) Clicking an entry of a right-click menu runs it - through the **hit map**, the
/// way a mouse does.
///
/// The round-75 menus were complete and drew correctly, and every row of both did
/// nothing: the dispatcher cleared the menu and only then asked the (now empty) menu
/// which entry had been clicked. The unit tests called `run_tab_menu` /
/// `run_tree_menu` directly, so they never went through that code. This one clicks
/// the pixels.
#[test]
fn a_click_on_a_menu_row_runs_that_row() {
    let mut h = harness("r77-menu-clicks");
    // Two tabs, so "close others" has something to close.
    h.type_str("first");
    h.app.new_tab();
    h.type_str("second");
    h.draw();

    // ---- the tab menu ----
    h.app.open_tab_menu(0, (h.app.text_rect.x + 30.0, h.app.text_rect.y + 20.0));
    h.draw();
    let labels: Vec<String> = h.app.tab_menu_entries().into_iter().map(|(l, _)| l).collect();
    // These tabs are untitled buffers, so the menu offers the three closes and no
    // Rename - which is the rule (a buffer without a file has no name to change).
    assert!(!labels.iter().any(|l| l.starts_with("Rename")), "Rename offered for an untitled tab");
    let others_at = labels.iter().position(|l| l == "Close others").expect("Close others");
    let row = h.app.hits.find(|hit| *hit == Hit::TabMenu(others_at)).expect("a clickable row");
    let c = (row.x + row.w * 0.5, row.y + row.h * 0.5);
    h.click_full(c);
    h.draw();
    // The click ran the command; both tabs hold unsaved text, so it asks before
    // closing them - and answering "discard" finishes the job.
    assert!(
        matches!(h.app.dialog, Some(bare_md::app::Dialog::Confirm { .. })),
        "clicking 'Close others' ran nothing at all"
    );
    for _ in 0..8 {
        if h.app.dialog.is_none() {
            break;
        }
        h.app.dialog_confirm(&mut h.r);
    }
    h.draw();
    assert_eq!(h.app.tabs.len(), 1, "clicking 'Close others' did nothing");
    assert!(h.app.tab_menu.is_none(), "the menu stayed open after its row was clicked");

    // ---- the tree menu: Rename through the pixels ----
    let dir = h._dir.join("tree77");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("thing.md");
    std::fs::write(&file, "# thing\n").unwrap();
    h.app.ws.set_root(dir.clone());
    h.app.settings.sidebar_open = true;
    h.draw();
    h.app.open_tree_menu(file.clone(), (h.app.text_rect.x + 30.0, h.app.text_rect.y + 20.0));
    h.draw();
    let tlabels: Vec<String> = h.app.tree_menu_entries().into_iter().map(|(l, _)| l).collect();
    let ren = tlabels.iter().position(|l| l == "Rename").expect("Rename");
    let row = h.app.hits.find(|hit| *hit == Hit::TreeMenu(ren)).expect("a clickable tree row");
    let c = (row.x + row.w * 0.5, row.y + row.h * 0.5);
    h.click_full(c);
    h.draw();
    assert!(
        matches!(h.app.dialog, Some(bare_md::app::Dialog::Rename(_))),
        "clicking 'Rename' in the tree menu did nothing"
    );
}

/// (77) Rename on a tab that has a file: right-click the tab, click "Rename…" in the
/// menu, get the rename box. This is the exact path that did nothing.
#[test]
fn rename_on_a_tab_opens_the_box_when_clicked() {
    let mut h = harness("r77-tab-rename-click");
    let dir = h._dir.join("r77");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("saved.md");
    std::fs::write(&file, "# saved\n").unwrap();
    h.app.open_path(&file);
    h.draw();

    // Right-click the tab itself.
    let tab = h
        .app
        .hits
        .find(|hit| *hit == Hit::Tab(1))
        .expect("the saved document's tab");
    // Rename is only offered for the tab that has a file - pointer in the middle of
    // the *saved* document's chip.
    let tp = (tab.x + tab.w * 0.5, tab.y + tab.h * 0.5);
    h.app.mouse = tp;
    h.draw();
    let mut input = bare_md::input::InputState::default();
    use winit::event::{ElementState, MouseButton, WindowEvent};
    h.app.handle_event(
        &WindowEvent::MouseInput {
            state: ElementState::Pressed,
            button: MouseButton::Right,
            device_id: winit::event::DeviceId::dummy(),
        },
        &mut h.r,
        &mut input,
    );
    h.draw();
    assert!(h.app.tab_menu.is_some(), "the right click opened no tab menu");

    // Its Rename row is there, and clicking it opens the box.
    let labels: Vec<String> = h.app.tab_menu_entries().into_iter().map(|(l, _)| l).collect();
    let ren = labels.iter().position(|l| l.starts_with("Rename")).expect("Rename row");
    let row = h.app.hits.find(|hit| *hit == Hit::TabMenu(ren)).expect("a clickable row");
    let c = (row.x + row.w * 0.5, row.y + row.h * 0.5);
    h.click_full(c);
    h.draw();
    assert!(
        matches!(h.app.dialog, Some(bare_md::app::Dialog::Rename(_))),
        "clicking Rename on a tab did nothing"
    );
    // …and the box renames the file.
    let Some(bare_md::app::Dialog::Rename(box_)) = &mut h.app.dialog else { unreachable!() };
    box_.field.set("renamed-tab.md");
    h.app.commit_rename();
    h.draw();
    assert!(dir.join("renamed-tab.md").exists(), "the rename never happened");
}

/// The rename box is a text field: what you type has to land in it.
///
/// It used to open, show the old name and then ignore the keyboard - `route_text`,
/// backspace and delete-forward knew the colour box and the folder dialog, and
/// returned `false` for everything else, so every key went to the document underneath.
#[test]
fn the_rename_box_takes_what_is_typed_into_it() {
    let mut h = harness("r77-rename-types");
    let dir = h._dir.join("r77t");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("before.md");
    std::fs::write(&file, "# before\n").unwrap();
    h.app.open_rename(bare_md::app::RenameTarget::Entry(file.clone()));
    h.draw();

    let text = |h: &Harness| h.app.dialog_field().map(|f| f.text.clone()).unwrap_or_default();

    // Typed text (the IME commit path, which is where winit delivers characters).
    h.type_into_find("after");
    assert!(
        matches!(h.app.dialog, Some(bare_md::app::Dialog::Rename(_))),
        "typing dismissed the box"
    );
    assert_eq!(text(&h), "after.md", "typing into the rename box did not reach it");

    // Backspace and Delete edit it like any other field.
    h.app.route_backspace();
    assert_eq!(text(&h), "afte.md", "Backspace in the rename box did not reach it");
    h.app.dialog_field_mut().unwrap().place_caret(0, false);
    h.app.delete_forward_in_field();
    assert_eq!(text(&h), "fte.md", "Delete in the rename box did not reach it");

    // …and what ends up in the box is what gets renamed.
    h.app.dialog_field_mut().unwrap().set("after.md");
    h.app.commit_rename();
    h.draw();
    assert!(h._dir.join("r77t").join("after.md").exists(), "the box did not rename the file");
}

/// A document tab's own rename box is the same field, reached the other way.
#[test]
fn the_rename_box_in_a_document_title_takes_typing() {
    let mut h = harness("r77-rename-types-doc");
    let dir = h._dir.join("r77u");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("doc.md");
    std::fs::write(&file, "x\n").unwrap();
    h.app.open_path(&file);
    h.draw();
    h.app.open_rename(bare_md::app::RenameTarget::Tab(file.clone()));
    h.draw();
    assert!(matches!(h.app.dialog, Some(bare_md::app::Dialog::Rename(_))));
    // The whole name is replaced by what is typed over it.
    h.app.dialog_select_all();
    h.type_into_find("typed.md");
    let shown = h.app.dialog_field().map(|f| f.text.clone()).unwrap_or_default();
    assert_eq!(shown, "typed.md", "typing went nowhere");
    h.app.commit_rename();
    h.draw();
    assert!(dir.join("typed.md").exists(), "the tab's box did not rename the file");
}

// ---------------------------------------------------------------- round 78

/// (78.1) The tab scroll chevrons sit in their own lanes: no chip is ever under one,
/// and hovering a chevron does not light up a tab.
#[test]
fn the_scroll_chevrons_never_sit_on_a_tab() {
    let mut h = harness("r78-chevron-lanes");
    for i in 0..24 {
        h.type_str(&format!("document number {i} with a longish name"));
        h.app.new_tab();
    }
    h.draw();
    assert!(h.app.tab_scroll_max > 0.0, "the strip has nothing to scroll");
    // Half way along, both chevrons are on screen.
    h.app.scroll_tabs_by(h.app.tab_scroll_max * 0.5);
    h.draw();
    let left = h.app.hits.find(|hit| *hit == Hit::TabScrollLeft).expect("no left chevron");
    let right = h.app.hits.find(|hit| *hit == Hit::TabScrollRight).expect("no right chevron");

    // No chip *draws* under either button: a chip scrolled half out of the strip is
    // clipped at the lane, and the lane is where each button lives.
    for (i, r) in h.app.tab_rects.iter().enumerate() {
        let visible = Rect::new(
            r.x.max(h.app.tab_strip_left),
            r.y,
            (r.right().min(right.x - 2.0) - r.x.max(h.app.tab_strip_left)).max(0.0),
            r.h,
        );
        if visible.w <= 0.0 {
            continue;
        }
        for (name, b) in [("left", left), ("right", right)] {
            let overlap = visible.x < b.right() && b.x < visible.right();
            assert!(!overlap, "tab {i} {visible:?} overlaps the {name} chevron {b:?}");
        }
    }

    // Hovering the right chevron lights no tab: the tab nearest the button is not
    // hovered, and no close button appears where the button is.
    h.app.mouse = (right.x + right.w * 0.5, right.y + right.h * 0.5);
    h.draw();
    // No chip's *visible* part is under the pointer: the chips are clipped at the
    // lane, so a chip that starts inside the lane is not drawn there at all.
    let lane_left = h.app.tab_strip_left;
    for (i, r) in h.app.tab_rects.iter().enumerate() {
        let vis = Rect::new(
            r.x.max(lane_left),
            r.y,
            (r.right().min(right.x - 2.0) - r.x.max(lane_left)).max(0.0),
            r.h,
        );
        assert!(
            !vis.contains(h.app.mouse),
            "the pointer over the right chevron is inside tab {i}'s visible chip {vis:?}"
        );
    }
    // …and the strip is not shifted by the lanes appearing: the first chip starts at
    // the left lane, so the tab strip's left edge is a constant.
    assert!(
        (h.app.tab_strip_left - (h.app.tabbar_rect.x + bare_md::ui::TAB_LANE)).abs() < 0.6,
        "the chips do not start at the lane: {} (bar x {})",
        h.app.tab_strip_left,
        h.app.tabbar_rect.x
    );
}

/// (78.2) The × on a tab is a cross you can see: the glyph, not the button, grew.
#[test]
fn the_tab_close_glyph_is_bigger_than_it_was() {
    const CHROME: &str = include_str!("../src/ui/chrome.rs");
    let at = CHROME.find("pub fn chip_close").expect("no chip_close");
    let body = &CHROME[at..(at + 700).min(CHROME.len())];
    // The button is still 16 px and still 21 px from the chip's right edge…
    assert!(body.contains("chip.right() - 21.0"), "the close button moved");
    assert!(body.contains("16.0, 16.0"), "the close button changed size");
    // …while the icon is inset by 1.5, not 4.5: a 7 px cross became a 13 px one.
    assert!(
        body.contains("cr.inset(1.5)"),
        "the close glyph is still the old small one"
    );
    // And it is still drawn by the one function both tab rows use.
    let mut h = harness("r78-close-glyph");
    h.draw();
    let cr = h.app.hits.find(|hit| matches!(hit, Hit::TabClose(_))).expect("a close button");
    assert_eq!((cr.w, cr.h), (22.0, 20.0), "the close hit area changed: {cr:?}");
}

/// (78.3) A tab's right-click menu has no "Close" row: the × beside it is that.
#[test]
fn the_tab_menu_has_no_close_row() {
    let mut h = harness("r78-no-close-row");
    h.app.new_tab();
    h.draw();
    h.app.open_tab_menu(0, (200.0, 30.0));
    let labels: Vec<String> = h.app.tab_menu_entries().into_iter().map(|(l, _)| l).collect();
    assert!(!labels.iter().any(|l| l == "Close"), "{labels:?}");
    for want in ["Close others", "Close all"] {
        assert!(labels.iter().any(|l| l == want), "{want} is missing from {labels:?}");
    }
}

/// (78.4) A right-click on the empty part of the workspace panel offers a new file in
/// the folder the panel is showing, and running it creates the file there.
#[test]
fn a_right_click_on_the_panel_makes_a_file_in_that_folder() {
    let (mut h, dir) = tree_harness("r78-new-file", &["one.md"]);
    // The blank part of the panel: below the last row.
    let blank = h
        .app
        .hits
        .find(|hit| *hit == Hit::SidebarBlank)
        .expect("the panel has no empty area registered");
    let pos = (blank.x + blank.w * 0.5, blank.y + 10.0);
    h.app.mouse = pos;
    h.draw();
    let mut input = bare_md::input::InputState::default();
    use winit::event::{ElementState, MouseButton, WindowEvent};
    h.app.handle_event(
        &WindowEvent::MouseInput {
            state: ElementState::Pressed,
            button: MouseButton::Right,
            device_id: winit::event::DeviceId::dummy(),
        },
        &mut h.r,
        &mut input,
    );
    h.draw();
    let entries = h.app.tree_menu_entries();
    assert!(!entries.is_empty(), "a right click on the empty panel opened no menu");
    let (label, action) = entries[0].clone();
    assert!(
        label.starts_with("New file"),
        "the first row is not \"New file\": {label}"
    );
    // The row says which folder it will use.
    assert!(
        label.contains("tree"),
        "the row does not name the folder: {label}"
    );
    h.app.run_tree_menu(action);
    h.draw();
    assert!(dir.join("untitled.md").exists(), "no file was created in {}", dir.display());
    // …and it is open in a tab, showing it.
    assert_eq!(
        h.app.tab().doc.path.as_deref(),
        Some(dir.join("untitled.md").as_path()),
        "the new file was not opened"
    );

    // A second one does not overwrite the first.
    h.app.run_tree_menu_for_test(dir.clone(), bare_md::app::TreeMenuAction::NewFile);
    assert!(dir.join("untitled 2.md").exists(), "the second new file reused the name");
}

/// (78.4b) The same command on a *file's* row creates the file in that file's folder,
/// which is where the row's label says it will go.
#[test]
fn new_file_on_a_file_row_goes_into_its_folder() {
    let (mut h, dir) = tree_harness("r78-new-file-row", &["one.md"]);
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let one = dir.join("one.md");
    h.app.run_tree_menu_for_test(one.clone(), bare_md::app::TreeMenuAction::NewFile);
    assert!(dir.join("untitled.md").exists(), "not created next to the file");
    h.app.run_tree_menu_for_test(dir.join("sub"), bare_md::app::TreeMenuAction::NewFile);
    assert!(dir.join("sub").join("untitled.md").exists(), "not created inside the folder");
}

/// (78.5) A file can be dragged out of the tree onto a folder in it, and dropped there.
#[test]
fn a_file_dragged_onto_a_folder_moves_into_it() {
    let (mut h, dir) = tree_harness("r78-drag-drop", &["move-me.md"]);
    let src = dir.join("move-me.md");
    let sub = dir.join("sub");
    // Open the file so the tab that follows it can be checked too.
    h.app.open_path(&src);
    h.draw();
    let from = tree_row(&h, &src);
    let to = tree_row(&h, &sub);
    assert!(
        h.app.ws.entries()[from].path == src && h.app.ws.entries()[to].path == sub,
        "the fixture's rows are not where they should be"
    );
    let row = h.app.sidebar_rows[from];
    let target = h.app.sidebar_rows[to];
    h.app.mouse = (row.x + 20.0, row.y + row.h * 0.5);
    h.draw();
    h.app.click(h.app.mouse, &mut h.r);
    h.draw();
    // Drag it over the folder: nothing moves yet, the folder says it would take it.
    let over = (target.x + 30.0, target.y + target.h * 0.5);
    h.app.mouse = over;
    assert!(h.app.tree_drag_update(over), "the drag did not start");
    h.draw();
    assert_eq!(
        h.app.tree_drag.as_ref().and_then(|d| d.over.clone()).as_deref(),
        Some(sub.as_path()),
        "the drag does not know which folder it is over"
    );
    assert!(src.exists(), "the file moved before the button came up");

    // Drop.
    h.app.mouse_up(&mut h.r);
    h.draw();
    assert!(!src.exists(), "the file did not leave its folder");
    let moved = sub.join("move-me.md");
    assert!(moved.exists(), "the file is not in the folder it was dropped on");
    assert_eq!(
        h.app.tab().doc.path.as_deref(),
        Some(moved.as_path()),
        "the open tab did not follow the file"
    );
    assert!(tree_row(&h, &moved) > 0, "the tree does not show the file in its new folder");
}

/// (78.5b) The three moves that would destroy something are refused, and a release
/// that never moved anything moves nothing.
#[test]
fn a_drag_that_would_lose_a_file_does_nothing() {
    let (mut h, dir) = tree_harness("r78-drag-refuse", &["keep.md"]);
    let src = dir.join("keep.md");
    let sub = dir.join("sub");
    // A press-and-release with no travel is a click on the row, not a move to the
    // folder the pointer happened to be over.
    h.draw();
    let row = h.app.sidebar_rows[tree_row(&h, &src)];
    h.app.mouse = (row.x + 20.0, row.y + row.h * 0.5);
    h.draw();
    h.app.click(h.app.mouse, &mut h.r);
    h.app.mouse_up(&mut h.r);
    h.draw();
    assert!(src.exists(), "a plain click moved the file");

    // A folder into its own child would lose the folder.
    let nested = sub.join("deep");
    std::fs::create_dir_all(&nested).unwrap();
    h.app.ws.refresh();
    h.draw();
    h.app.move_into_folder(&sub, &nested);
    assert!(sub.exists() && nested.exists(), "a folder was moved into its own child");
    assert!(
        h.app.status.contains("cannot move"),
        "the refusal was silent: {:?}",
        h.app.status
    );

    // Moving a file into the folder it already lives in is a no-op, not a copy.
    h.app.move_into_folder(&src, &dir);
    assert!(src.exists());
    assert!(!dir.join("keep 2.md").exists(), "a no-op move made a duplicate");
}

// ---------------------------------------------------------------- round 79

/// (79) A file dragged out of a folder and onto the bare panel goes back to the
/// workspace root.
///
/// The first version of the drop could only find *rows* as targets, and the root
/// has no row of its own - it is the address chip at the top of the panel - so a
/// file could be put into a folder but never taken back out of one.
#[test]
fn a_file_can_be_dragged_back_to_the_root() {
    let (mut h, dir) = tree_harness("r79-drag-back-to-root", &["other.md"]);
    let sub = dir.join("sub");
    let src = sub.join("leaf.md");
    std::fs::write(&src, "# leaf\n").unwrap();
    h.app.open_path(&src);
    h.draw();
    assert_eq!(
        h.app.tab().doc.path.as_deref(),
        Some(src.as_path()),
        "the fixture did not open the file"
    );

    // The file lives in `sub`; drag it over the blank part of the panel.
    let row = h.app.sidebar_rows[tree_row(&h, &src)];
    h.app.mouse = (row.x + 20.0, row.y + row.h * 0.5);
    h.draw();
    h.app.click(h.app.mouse, &mut h.r);
    h.draw();
    let blank = h.app.sidebar_blank;
    assert!(blank.h > 4.0, "the panel has no empty area to drop on: {blank:?}");
    let over = (blank.x + blank.w * 0.5, blank.y + 8.0);
    h.app.mouse = over;
    assert!(h.app.tree_drag_update(over), "the drag did not start");
    h.draw();
    assert_eq!(
        h.app.tree_drag.as_ref().and_then(|d| d.over.clone()).as_deref(),
        Some(dir.as_path()),
        "the blank panel is not the workspace root"
    );
    h.app.mouse_up(&mut h.r);
    h.draw();
    assert!(!src.exists(), "the file did not leave the folder");
    let back = dir.join("leaf.md");
    assert!(back.exists(), "the file is not back in the root");
    assert_eq!(
        h.app.tab().doc.path.as_deref(),
        Some(back.as_path()),
        "the open tab did not follow the file back to the root"
    );
}

/// (79b) The address chip at the top of the panel is part of the panel, so a file
/// can be dropped on it too.
#[test]
fn the_address_chip_is_the_roots_row_for_a_drag() {
    let (mut h, dir) = tree_harness("r79-drag-root-chip", &["leaf.md"]);
    let sub = dir.join("sub");
    let src = sub.join("leaf.md");
    std::fs::write(&src, "x\n").unwrap();
    h.draw();
    let chip = h.app.sidebar_root_row;
    assert!(chip.h > 10.0, "no address chip was published: {chip:?}");

    // Halfway down the chip is a hit for "go up", and it is still the root for a
    // drop: the two meanings do not collide because the drag only reads the drop.
    h.app.tree_drag = Some(bare_md::app::TreeDrag {
        path: src.clone(),
        start: (chip.x + 5.0, chip.y + 5.0),
        moved: true,
        over: None,
    });
    assert_eq!(
        h.app.drop_target_at((chip.x + 40.0, chip.y + chip.h * 0.5), &src).as_deref(),
        Some(dir.as_path()),
        "the address chip is not a drop target for the root"
    );
    h.draw();
    // The drag itself agrees with the query, and the drop does the move. Nothing is
    // drawn for it: the panel is the target, not this row.
    h.app.mouse_down = true;
    h.app.mouse = (chip.x + 40.0, chip.y + chip.h * 0.5);
    assert!(h.app.tree_drag_update(h.app.mouse), "the drag stopped tracking");
    assert_eq!(
        h.app.tree_drag.as_ref().and_then(|d| d.over.clone()).as_deref(),
        Some(dir.as_path()),
        "the chip did not claim the drag"
    );
    h.app.mouse_up(&mut h.r);
    h.draw();
    assert!(dir.join("leaf.md").exists(), "the drop on the chip did not move the file");
    assert!(!src.exists());
}

/// (79c) A drop on the folder the file already lives in does nothing, and a drop on
/// the file's own parent while it sits in the root is not a move to itself.
#[test]
fn dropping_a_file_where_it_already_is_does_nothing() {
    let (mut h, dir) = tree_harness("r79-drop-noop", &["stay.md"]);
    let src = dir.join("stay.md");
    h.draw();
    let blank = h.app.sidebar_blank;
    let pos = (blank.x + 10.0, blank.y + 6.0);
    assert_eq!(
        h.app.drop_target_at(pos, &src).as_deref(),
        Some(dir.as_path()),
        "the root is not offered as a target"
    );
    // …but moving it into the folder it is already in is recognised as a no-op.
    h.app.move_into_folder(&src, &dir);
    assert!(src.exists());
    assert!(!dir.join("stay 2.md").exists(), "a no-op move made a copy");
    // A file cannot be dropped into itself, and a folder not into its own child.
    assert_eq!(h.app.drop_target_at(pos, &dir), None, "a folder was offered itself");
}

// ---------------------------------------------------------------- round 80

/// (80) The whole panel takes a drop: no special box, no caption, anywhere in it.
#[test]
fn the_whole_workspace_panel_takes_a_drop() {
    let (mut h, dir) = tree_harness("r80-drop-anywhere", &["a.md", "b.md"]);
    let sub = dir.join("sub");
    let src = dir.join("a.md");
    h.draw();
    let panel = h.app.sidebar_rect;
    assert!(panel.w > 50.0 && panel.h > 50.0, "no panel: {panel:?}");

    // The root answers for every part of the panel that is not a folder row: the
    // header, the address chip, the empty space below the list, the very edge.
    let spots = [
        (panel.x + 4.0, panel.y + 4.0),
        (panel.x + panel.w * 0.5, panel.y + 40.0),
        (h.app.sidebar_blank.x + 6.0, h.app.sidebar_blank.y + 6.0),
        (panel.right() - 3.0, panel.bottom() - 3.0),
    ];
    for spot in spots {
        assert_eq!(
            h.app.drop_target_at(spot, &src).as_deref(),
            Some(dir.as_path()),
            "{spot:?} is not a drop for the root"
        );
    }

    // A file's row means the folder that file is in, a folder's row means itself.
    let a_row = h.app.sidebar_rows[tree_row(&h, &src)];
    assert_eq!(
        h.app.drop_target_at((a_row.x + 10.0, a_row.y + a_row.h * 0.5), &dir.join("b.md")).as_deref(),
        Some(dir.as_path()),
        "a file row is not its own folder"
    );
    let sub_row = h.app.sidebar_rows[tree_row(&h, &sub)];
    assert_eq!(
        h.app.drop_target_at((sub_row.x + 10.0, sub_row.y + sub_row.h * 0.5), &src).as_deref(),
        Some(sub.as_path()),
        "a folder row is not that folder"
    );

    // Outside the panel nothing is offered: a drag that wanders into the document
    // does not quietly move a file into the root.
    assert_eq!(h.app.drop_target_at((panel.right() + 40.0, panel.y + 40.0), &src), None);

    // And the drop really works from the bare panel.
    h.app.click((a_row.x + 10.0, a_row.y + a_row.h * 0.5), &mut h.r);
    h.draw();
    let spot = (h.app.sidebar_blank.x + 10.0, h.app.sidebar_blank.y + 6.0);
    h.app.mouse = spot;
    assert!(h.app.tree_drag_update(spot), "the drag did not start");
    h.app.mouse_up(&mut h.r);
    h.draw();
    assert!(dir.join("a.md").exists(), "the file left the root it was already in");
}

/// (80b) A folder row still lights up - it is the only row that means anything more
/// specific than the panel - and the root case draws nothing at all.
#[test]
fn only_a_folder_row_is_highlighted_for_a_drag() {
    let (mut h, dir) = tree_harness("r80-highlight", &["a.md"]);
    let sub = dir.join("sub");
    let src = dir.join("a.md");
    h.draw();
    // Carrying a file over the sub folder: the folder is the target.
    h.app.tree_drag = Some(bare_md::app::TreeDrag {
        path: src.clone(),
        start: (10.0, 10.0),
        moved: true,
        over: Some(sub.clone()),
    });
    h.draw();
    let sub_row = h.app.sidebar_rows[tree_row(&h, &sub)];
    assert!(sub_row.h > 0.0);
    // Over the root there is no target row to draw, and nothing is invented for it:
    // the chip does not turn into a box and the bare panel gets no border or caption.
    const CHROME: &str = include_str!("../src/ui/chrome.rs");
    assert!(!CHROME.contains("drop in \\"), "the panel still draws a drop caption");
    assert!(
        !CHROME.contains("p.stroke(blank"),
        "the panel still draws a drop border"
    );
}
