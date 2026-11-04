//! Headless UI tests: build the real app, draw the real UI into an offscreen
//! wgpu texture and write PNGs to `target/snapshots/` for visual inspection.
//!
//! These run without a window manager (software Vulkan/lavapipe is enough).

use std::path::{Path, PathBuf};

use bare_md::app::{App, Dialog, PickerKind};
use bare_md::config::{Paths, Session, Settings};
use bare_md::geom::Rect;
use bare_md::gpu::Renderer;
use bare_md::search::FindFocus;
use bare_md::theme::ThemeKind;
use bare_md::ui;

const W: u32 = 1180;
const H: u32 = 780;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bare-md-ui-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn snapshot_dir() -> PathBuf {
    let dir = Path::new("target/snapshots").to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sample_doc() -> String {
    let long = "Wrapping matters for prose: this paragraph is deliberately long so that the \
soft-wrap path in the layout engine has to break it across several visual rows, and the caret, \
selection and click hit-testing all have to agree about where each visual row starts and ends.";
    format!(
        "# Bare MD\n\nA **portable** Markdown editor with *live* rendering.\n\n## Long prose\n\n{long} {long}\n\n## Lists\n\n- first item\n- second item with `inline code`\n- [x] a finished task\n- [ ] an open task\n\n### Quote\n\n> Quoted text is dimmed and gets a bar.\n\n### Code\n\n```rust\nfn main() {{\n    println!(\"hello\");\n}}\n```\n\n### Table\n\n| Column | Column |\n| ------ | ------ |\n| Cell   | Cell   |\n\n---\n\nThe end.\n"
    )
}

struct Harness {
    app: App,
    r: Renderer,
    shots: PathBuf,
    step: usize,
}

impl Harness {
    fn new(tag: &str) -> Harness {
        let dir = temp_dir(tag);
        let paths = Paths {
            data_dir: dir.clone(),
            config_file: dir.join("settings.conf"),
            origin: "test".into(),
        };
        // A tiny workspace so the sidebar has something to show.
        let ws = dir.join("notes");
        std::fs::create_dir_all(ws.join("archive")).unwrap();
        std::fs::write(ws.join("demo.md"), sample_doc()).unwrap();
        std::fs::write(ws.join("todo.md"), "# Todo\n\n- [ ] write docs\n").unwrap();
        std::fs::write(ws.join("archive/old.md"), "# Old\n").unwrap();

        let mut settings = Settings::default();
        settings.workspace = Some(ws.clone());
        settings.theme = ThemeKind::Black;
        let mut app = App::new(paths, settings, Session::default(), vec![ws.join("demo.md")]);
        app.mouse = (-100.0, -100.0);
        let r = Renderer::offscreen(W, H, 1.0);
        Harness { app, r, shots: snapshot_dir(), step: 0 }
    }

    fn draw(&mut self) {
        ui::draw(&mut self.app, &mut self.r);
        self.r.end();
    }

    fn shot(&mut self, name: &str) -> PathBuf {
        self.step += 1;
        let path = self.shots.join(format!("{:02}-{name}.png", self.step));
        self.r.screenshot = Some(path.clone());
        self.r.screenshot_frames = 1;
        self.draw();
        path
    }

}

#[test]
fn snapshot_full_ui() {
    let mut h = Harness::new("full");
    h.shot("default-black");
    assert_eq!(h.app.tabs.len(), 1);
    assert!(h.app.tab().doc.path.is_some());
}

#[test]
fn snapshot_light_theme() {
    let mut h = Harness::new("light");
    h.app.toggle_theme();
    h.shot("light");
}

#[test]
fn snapshot_find_bar() {
    let mut h = Harness::new("find");
    h.app.open_find(true);
    h.app.focus = bare_md::app::Focus::FindReplace;
    h.app.tab_mut().find.focus = FindFocus::Replace;
    h.app.tab_mut().find.query = "the".into();
    h.app.tab_mut().find.replace = "THE".into();
    h.app.tab_mut().find.version = u64::MAX;
    h.app.find_step(true);
    h.shot("find-replace");
    let matches = h.app.tab().find.matches.len();
    assert!(matches >= 3, "expected several matches, got {matches}");
    assert!(h.app.tab().has_selection());
}

#[test]
fn snapshot_outline_and_minimap() {
    let mut h = Harness::new("outline");
    h.app.settings.outline_open = true;
    h.app.settings.minimap_open = true;
    h.shot("outline");
    let headings = h.app.tab().layout.headings.len();
    assert_eq!(headings, 6, "outline should list every heading");
}

#[test]
fn snapshot_sidebar_collapsed() {
    let mut h = Harness::new("nosidebar");
    h.app.settings.sidebar_open = false;
    h.shot("no-sidebar");
    let entries = h.app.ws.entries().len();
    assert!(entries >= 3, "workspace should list the sample files");
}

#[test]
fn snapshot_raw_line_reveal() {
    let mut h = Harness::new("reveal");
    // Put the caret inside the second heading so that line shows its markers.
    let text = h.app.tab().doc.text();
    let pos = text.find("## Long prose").unwrap();
    let char_pos = text[..pos].chars().count();
    h.app.set_caret(char_pos + 4, false);
    h.shot("caret-reveals-markers");
}

#[test]
fn snapshot_picker_dialog() {
    let mut h = Harness::new("picker");
    h.app.open_picker(PickerKind::OpenFile);
    h.shot("open-dialog");
    match &h.app.dialog {
        Some(Dialog::Picker(p)) => assert!(!p.entries.is_empty()),
        _ => panic!("picker should be open"),
    }
}

#[test]
fn snapshot_multiple_tabs() {
    let mut h = Harness::new("tabs");
    let ws = h.app.ws.root.clone();
    h.app.open_path(&ws.join("todo.md"));
    h.app.active = 0;
    h.app.new_tab();
    h.shot("three-tabs");
    assert_eq!(h.app.tabs.len(), 3);
    assert_eq!(h.app.tabs[0].title(), "demo.md");
    assert_eq!(h.app.tabs[1].title(), "todo.md");
}

#[test]
fn editor_rect_is_reported_for_input() {
    let mut h = Harness::new("rects");
    h.draw();
    assert!(h.app.text_rect.w > 400.0, "text rect: {:?}", h.app.text_rect);
    assert!(h.app.editor_rect.contains((h.app.text_rect.x + 5.0, h.app.text_rect.y + 5.0)));
    // The chrome must not overlap the status bar.
    let c = ui::chrome(&h.app, W as f32, H as f32);
    assert!(c.status.y >= c.editor.bottom() - 1.0);
    assert!((c.status.h - bare_md::theme::Theme::STATUS_H).abs() < 0.5);
    assert!(c.sidebar.x == 0.0 && c.sidebar.h >= c.doc.h, "sidebar must run full height");
}

#[test]
fn long_paragraph_wraps_and_height_grows() {
    let mut h = Harness::new("wrap");
    h.draw();
    let layout = &h.app.tab().layout;
    let wrapped = layout.lines.iter().filter(|l| l.row_count() > 1).count();
    assert!(wrapped >= 1, "expected at least one wrapped line");
    let total: f32 = layout.lines.iter().map(|l| l.height + l.space_before).sum();
    assert!((total - layout.total_height + 130.0).abs() < 200.0);
    let _ = Rect::ZERO;
}

#[test]
fn snapshot_scrolled_code_and_table() {
    let mut h = Harness::new("scrolled");
    h.draw();
    let total = h.app.tab().layout.total_height;
    h.app.tab_mut().scroll = total - H as f32 + 40.0;
    h.shot("code-and-table");
}

#[test]
fn snapshot_selection_and_caret() {
    let mut h = Harness::new("selection");
    let text = h.app.tab().doc.text();
    let a = text[..text.find("Wrapping matters").unwrap()].chars().count();
    let b = a + 120;
    h.app.select_range(a, b);
    h.draw();
    h.app.ensure_caret_visible();
    h.shot("selection");
    assert!(h.app.tab().has_selection());
}

#[test]
fn snapshot_workspace_folder_expanded() {
    let mut h = Harness::new("folders");
    let archive = h.app.ws.root.join("archive");
    h.app.ws.toggle(&archive);
    h.shot("sidebar-expanded");
    assert!(h.app.ws.is_expanded(&archive));
}

#[test]
fn snapshot_empty_state() {
    let mut h = Harness::new("empty");
    h.app.new_tab();
    h.app.tabs.last_mut().unwrap().doc = bare_md::doc::Doc::empty();
    h.shot("empty-state");
    assert_eq!(h.app.tab().doc.len_chars(), 0);
}

#[test]
fn snapshot_floating_toolbar_and_tabs() {
    let mut h = Harness::new("toolbar");
    let ws = h.app.ws.root.clone();
    h.app.open_path(&ws.join("todo.md"));
    h.app.active = 0;
    h.app.new_tab();
    h.app.insert_text("# Typing a heading");
    h.shot("typing-heading");
    // The caret must sit right after the heading text, at heading height.
    let cr = h.app.caret_rect(&mut h.r.text);
    let heading_lh = h.r.text.line_height(bare_md::text::Face::Bold, h.app.theme.heading_size(1));
    assert!(cr.h > heading_lh * 0.8, "caret {} should be heading height", cr.h);
}

#[test]
fn snapshot_raw_mode() {
    let mut h = Harness::new("raw");
    h.app.set_view_mode(bare_md::layout::ViewMode::Raw);
    let end = h.app.tab().doc.len_chars();
    h.app.set_caret(end, false);
    h.shot("raw-mode");
    assert!(h.app.mode == bare_md::layout::ViewMode::Raw);
}

#[test]
fn snapshot_task_checkbox() {
    let mut h = Harness::new("tasks");
    let text = h.app.tab().doc.text();
    let pos = text.find("- [x]").unwrap();
    let at = text[..pos].chars().count();
    h.app.set_caret(at + 6, false);
    h.shot("checkbox");
}

#[test]
fn snapshot_code_blocks() {
    let mut h = Harness::new("code");
    h.app.new_tab();
    h.app.insert_text(
        "# Code blocks\n\n```rust\nfn main() {\n    println!(\"hi\");\n\n    // an empty line above\n}\n```\n\nA one-liner: ```let x = 1;```\n\nAnd an empty fence:\n\n```\n```\n",
    );
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.shot("code-blocks");
}

#[test]
fn snapshot_table_grid() {
    let mut h = Harness::new("table");
    h.app.new_tab();
    h.app.insert_text(
        "| Name | Value | Notes |\n| ---- | ----- | ----- |\n| alpha | 1 | first row |\n| beta | 22 | second |\n\nAfter the table.\n",
    );
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.shot("table");
}

#[test]
fn snapshot_report_blocks() {
    let mut h = Harness::new("report");
    h.app.new_tab();
    h.app.insert_text(
        "# Blocks\n\nA paragraph before a table that is long enough to show spacing.\n\n| Name | Value | Notes |\n| ---- | ----- | ----- |\n| alpha | 1 | first row |\n| beta | 22 | second |\n\nA paragraph after the table.\n\n> Quoted line one\n>\n> > nested quote inside\n> back to one\n\nText between.\n\n```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n\nText after code.\n\n```let x = 1;```\n\nA [link](https://example.com) and an empty pair: ``\n",
    );
    h.app.set_caret(h.app.tab().doc.len_chars(), false);
    h.shot("report-blocks");
}

#[test]
fn snapshot_report_tail() {
    let mut h = Harness::new("report-tail");
    h.app.new_tab();
    h.app.insert_text(
        "# One-liners\n\n```let x = 1;```\n\nAn empty pair: ``\n\nAn empty fenced block:\n\n```\n```\n\nA [pale blue link](https://example.com) sits here, with *italic* and __bold__ words.\n",
    );
    h.app.set_caret(0, false);
    h.shot("report-tail");
}

#[test]
fn snapshot_round4() {
    let mut h = Harness::new("round4");
    h.app.new_tab();
    h.app.insert_text(
        "# Round 4\n\n> first quoted row\n>\n> > nested inside\n> last quoted row\n\n| Name | Value |\n| ---- | ----- |\n| alpha | 1 |\n| beta | 22 |\n\n```rust\nfn main() {}\n```\n\n    1. Item 3a\n\n    2. Item 3b\n\n- [ ] a task\n- [x] a done task\n\nTail paragraph.\n",
    );
    // Caret inside the table: the row must stay rendered (no raw pipes).
    let text = h.app.tab().doc.text().to_string();
    let at = text.find("beta").unwrap();
    h.app.set_caret(text[..at].chars().count() + 1, false);
    h.shot("round4-live");

    // Caret inside the code block: the card must stay unbroken.
    let at = text.find("fn main").unwrap();
    h.app.set_caret(text[..at].chars().count() + 2, false);
    h.shot("round4-code-caret");

    // Live+ shows the markers and keeps the formatting.
    h.app.set_view_mode(bare_md::layout::ViewMode::LivePlus);
    h.app.set_caret(0, false);
    h.shot("round4-live-plus");

    // Raw is plain source.
    h.app.set_view_mode(bare_md::layout::ViewMode::Raw);
    h.shot("round4-raw");
}

#[test]
fn snapshot_black_theme() {
    let mut h = Harness::new("black");
    h.app.set_theme(ThemeKind::Black);
    h.shot("theme-black");
    assert_eq!(h.app.theme.kind, ThemeKind::Black);
}

#[test]
fn snapshot_white_theme() {
    let mut h = Harness::new("white");
    h.app.set_theme(ThemeKind::White);
    h.shot("theme-white");
    assert_eq!(h.app.theme.kind, ThemeKind::White);
}

#[test]
fn theme_cycle_reaches_every_theme_and_keeps_font_size() {
    let mut h = Harness::new("cycle");
    h.app.zoom(3.0);
    let size = h.app.theme.font_size;
    // Black -> Grey -> White -> Black, and the zoom survives every step.
    for expected in [
        ThemeKind::Grey,
        ThemeKind::White,
        ThemeKind::Black,
    ] {
        h.app.toggle_theme();
        assert_eq!(h.app.theme.kind, expected);
        assert_eq!(h.app.theme.font_size, size, "zoom must survive a theme switch");
        h.draw();
    }
}

// ---------------------------------------------------------------------------
// Round 5: the new chrome
// ---------------------------------------------------------------------------

/// The toolbar is compact: every button fits inside the pill, in fixed groups,
/// and the pill never grows past a sane width.
#[test]
fn toolbar_is_compact_and_fits() {
    let mut h = Harness::new("r5-toolbar");
    h.draw();
    let editor = h.app.text_rect;
    let pill = ui::floating_toolbar_rect(&h.app, editor);
    assert!(pill.w <= 820.0, "toolbar pill is {} px wide", pill.w);
    assert!(pill.w < editor.w, "the pill must not span the page");
    // The right-hand tail (attach, table, tools) is always present.
    for want in [
        bare_md::app::Hit::MenuButton(bare_md::app::MenuKind::Heading),
        bare_md::app::Hit::MenuButton(bare_md::app::MenuKind::Color),
        bare_md::app::Hit::MenuButton(bare_md::app::MenuKind::Table),
        bare_md::app::Hit::MenuButton(bare_md::app::MenuKind::Tools),
    ] {
        assert!(
            h.app.hits.find(|hit| *hit == want).is_some(),
            "missing {want:?} on the toolbar"
        );
    }
    h.shot("r5-toolbar");
}

#[test]
fn snapshot_r5_menus() {
    let mut h = Harness::new("r5-menus");
    h.app.new_tab();
    h.app.insert_text("# Menus\n\nSome **words** to colour.\n");
    let at = h.app.tab().doc.text().find("words").unwrap();
    h.app.set_caret(at, false);
    h.draw();
    let editor = h.app.text_rect;
    let toolbar = ui::floating_toolbar_rect(&h.app, editor);
    for (kind, name) in [
        (bare_md::app::MenuKind::Heading, "r5-menu-heading"),
        (bare_md::app::MenuKind::Color, "r5-menu-color"),
        (bare_md::app::MenuKind::Align, "r5-menu-align"),
        (bare_md::app::MenuKind::Table, "r5-menu-table"),
        (bare_md::app::MenuKind::Tools, "r5-menu-tools"),
        (bare_md::app::MenuKind::SupSub, "r5-menu-supsub"),
    ] {
        h.app.open_menu(kind, toolbar);
        assert!(h.app.menu.is_some(), "{kind:?} menu did not open");
        h.shot(name);
        h.app.close_menu();
    }
}

#[test]
fn snapshot_r5_context_menu() {
    let mut h = Harness::new("r5-context");
    h.app.new_tab();
    h.app.insert_text("| Name | Value |\n| ---- | ----- |\n| alpha | 1 |\n| beta | 22 |\n");
    let at = h.app.tab().doc.text().find("alpha").unwrap();
    h.app.set_caret(at, false);
    h.draw();
    h.app.open_context_menu((h.app.text_rect.x + 80.0, h.app.text_rect.y + 120.0));
    h.shot("r5-context-menu");
}

/// Hovering a table draws the `+` row / `+` column buttons and the grips, and
/// they are clickable.
#[test]
fn snapshot_r5_table_hover_widgets() {
    let mut h = Harness::new("r5-table-hover");
    h.app.new_tab();
    h.app.insert_text(
        "| Name | Value |\n| ---- | ----- |\n| alpha | 1 |\n| beta | 22 |\n\nAfter.\n",
    );
    h.app.set_caret(0, false);
    h.draw();
    // Park the pointer over the table so the chrome appears.
    let tab = h.app.tab();
    let line = tab
        .layout
        .lines
        .iter()
        .find(|l| l.block() == bare_md::markdown::Block::Table)
        .expect("table row");
    let y = h.app.text_rect.y + line.top + line.height * 0.5 - tab.scroll;
    h.app.mouse = (h.app.text_rect.x + 40.0, y);
    h.shot("r5-table-hover");
    assert!(
        h.app.hits.find(|h| matches!(h, bare_md::app::Hit::TableZone(_, _))).is_some(),
        "no + button on a hovered table"
    );
    assert!(
        h.app.hits.find(|h| matches!(h, bare_md::app::Hit::RowGrip(_, _))).is_some(),
        "no row grips on a hovered table"
    );
}

/// Zoomed out, list items keep an even rhythm (no uneven gaps after the text).
#[test]
fn snapshot_r5_zoomed_out_lists() {
    let mut h = Harness::new("r5-zoom");
    h.app.new_tab();
    h.app.insert_text(
        "# Zoom\n\n* Item 3a\n\n* Item 3b\n\n* Item 3c\n\n```\nfn main() {}\n```\n\nTail.\n",
    );
    h.app.set_caret(0, false);
    h.app.zoom(-7.0);
    h.app.invalidate_layouts();
    h.shot("r5-zoomed-out");
    assert!(h.app.theme.font_size < 12.0);
}

/// A whole-row selection is a band over the text only, not to the page edge.
#[test]
fn snapshot_r5_whole_row_selection() {
    let mut h = Harness::new("r5-rowsel");
    h.app.new_tab();
    h.app.insert_text(
        "A short line.\n\nA much longer line of text that keeps going for a while so the row is wide.\n\n```\nfn main() {}\n```\n",
    );
    let text = h.app.tab().doc.text();
    let a = text[..text.find("A short").unwrap()].chars().count();
    let b = text[..text.find("A much").unwrap()].chars().count() + 8;
    h.app.select_range(a, b);
    h.shot("r5-row-selection");
}

/// (6.5) A picture in the document: real PNG pixels on the page, with the
/// source revealed only on the caret line.
#[test]
fn snapshot_r6_image() {
    let ws = temp_dir("r6-image");
    // A small checkerboard PNG written into the workspace.
    let path = ws.join("shot.png");
    let (w, h) = (120u32, 80u32);
    let mut bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut bytes, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().unwrap();
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let on = ((x / 15) + (y / 15)) % 2 == 0;
                data.extend_from_slice(if on { &[235, 235, 235, 255] } else { &[60, 60, 60, 255] });
            }
        }
        writer.write_image_data(&data).unwrap();
    }
    std::fs::write(&path, &bytes).unwrap();
    let doc = ws.join("pic.md");
    std::fs::write(&doc, "").unwrap();

    let mut h = Harness::new("r6-image-doc");
    h.app.open_path(&doc);
    h.app.insert_text("# Picture\n\n![](shot.png)\n\nAfter the picture.\n\n![](missing.png)\n");
    h.app.set_caret(0, false);
    h.shot("r6-image");
    let has = h.app.tab().layout.lines.iter().filter(|l| l.image.is_some()).count();
    assert_eq!(has, 2, "both the real and the missing image keep their box");
}

/// (6.3) A short document keeps its minimap bars close together instead of being
/// stretched over the whole column.
#[test]
fn snapshot_r6_minimap_spacing() {
    let mut h = Harness::new("r6-minimap");
    h.app.new_tab();
    h.app.insert_text("# Short\n\none line\n\ntwo lines\n\nthree\n");
    h.app.set_caret(0, false);
    h.shot("r6-minimap-short");
    let layout = &h.app.tab().layout;
    let tops = ui::minimap_virtual_tops(layout);
    let total = tops.last().copied().unwrap_or(1.0) + 1.0;
    // With a fixed pitch the map must not fill the column.
    assert!(
        total * ui::MINIMAP_PITCH < h.app.minimap_rect.h,
        "a 7-line document should not stretch to fill the column ({total} rows)"
    );
}

/// (6.4) The outline rows carry drag handles, and dragging one reorders sections.
#[test]
fn snapshot_r6_outline_handles() {
    let mut h = Harness::new("r6-outline");
    h.app.settings.outline_open = true;
    h.app.new_tab();
    h.app.insert_text("# Intro\n\ntext\n\n## Detail\n\nmore\n\n# Second\n\nbody\n");
    h.app.set_caret(0, false);
    h.draw();
    let grip = h.app.hits.find(|hit| matches!(hit, bare_md::app::Hit::OutlineGrip(0)));
    assert!(grip.is_some(), "the first outline row needs a handle");
    // Hover it so the handle lights up in the screenshot.
    let g = grip.unwrap();
    h.app.mouse = (g.x + g.w * 0.5, g.y + g.h * 0.5);
    h.shot("r6-outline-handles");
    // Drag the LAST section's handle onto the first row: it moves to the top,
    // taking its body with it. (Dragging a parent onto its own child is a no-op,
    // which is what you want: the child already lives there.)
    let first = h
        .app
        .hits
        .find(|hit| matches!(hit, bare_md::app::Hit::OutlineGrip(0)))
        .expect("first handle");
    let last = h
        .app
        .hits
        .find(|hit| matches!(hit, bare_md::app::Hit::OutlineGrip(2)))
        .expect("third handle");
    assert!(h.app.drag_begin(&bare_md::app::Hit::OutlineGrip(2), (last.x, last.y)));
    h.app.drag_update((last.x, first.y + 2.0));
    assert!(h.app.drag_end(&mut h.r));
    let text = h.app.tab().doc.text();
    assert_eq!(text, "# Second\n\nbody\n\n# Intro\n\ntext\n\n## Detail\n\nmore\n", "section moved: {text:?}");
}

// ---------------------------------------------------------------------------
// Round 7
// ---------------------------------------------------------------------------

/// The third theme, the wider minimap and the new tab-strip buttons.
#[test]
fn snapshot_r7_grey_theme() {
    let mut h = Harness::new("r7-grey");
    h.app.set_theme(ThemeKind::Grey);
    h.shot("r7-theme-grey");
    assert_eq!(h.app.theme.kind, ThemeKind::Grey);
    // The sheet, the panels and the floating pill must be three distinct greys.
    let t = h.app.theme;
    assert!(t.bg.r < t.bg_panel.r && t.bg_panel.r < t.bg_elevated.r, "surface steps");
}

/// The tab strip carries every panel toggle as a labelled chip, right-aligned
/// next to the document they affect.
#[test]
fn snapshot_r7_tab_strip_chips() {
    let mut h = Harness::new("r7-chips");
    h.app.settings.sidebar_open = true;
    h.app.settings.outline_open = true;
    h.app.settings.minimap_open = true;
    // Hover the workspace chip so the screenshot also shows its hint.
    h.draw();
    let ws = h
        .app
        .hits
        .find(|hit| *hit == bare_md::app::Hit::Toolbar(bare_md::app::ToolbarAction::ToggleSidebar))
        .expect("workspace chip");
    h.app.mouse = (ws.x + ws.w * 0.5, ws.y + ws.h * 0.5);
    h.shot("r7-tab-strip");
    // Every toggle is a hit, including the workspace one.
    for want in [
        bare_md::app::Hit::Toolbar(bare_md::app::ToolbarAction::ToggleSidebar),
        bare_md::app::Hit::Toolbar(bare_md::app::ToolbarAction::ToggleOutline),
        bare_md::app::Hit::Toolbar(bare_md::app::ToolbarAction::ToggleMinimap),
        bare_md::app::Hit::Toolbar(bare_md::app::ToolbarAction::ToggleTheme),
        bare_md::app::Hit::Toolbar(bare_md::app::ToolbarAction::ReloadFile),
    ] {
        assert!(h.app.hits.find(|hit| *hit == want).is_some(), "missing {want:?}");
    }
    // …and the chips sit on the right-hand side of the strip (round 8 moved them
    // there so they are next to the document rather than next to the sidebar).
    let ws = h
        .app
        .hits
        .find(|hit| *hit == bare_md::app::Hit::Toolbar(bare_md::app::ToolbarAction::ToggleSidebar))
        .unwrap();
    assert!(
        ws.x > h.r.logical_size().0 * 0.5,
        "the workspace button belongs on the right: x = {}",
        ws.x
    );
}

/// A nested list renders as nested list items, with the markers still visible and
/// the indent growing per level.
#[test]
fn snapshot_r7_nested_list() {
    let mut h = Harness::new("r7-nested");
    h.app.new_tab();
    h.app.insert_text("- top\n  - nested\n    - deeper\n    - deeper two\n  - nested two\n- top two\n");
    h.app.set_caret(0, false);
    h.shot("r7-nested-list");
    let depths: Vec<usize> = h.app.tab().layout.lines.iter().take(6).map(|l| l.info.depth).collect();
    assert_eq!(depths, vec![0, 1, 2, 2, 1, 0], "nesting levels");
}

/// Hovering a code block shows its copy button.
#[test]
fn snapshot_r7_code_copy_button() {
    let mut h = Harness::new("r7-copy");
    h.app.new_tab();
    h.app.insert_text("```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n");
    h.app.set_caret(0, false);
    h.draw();
    let code = h
        .app
        .tab()
        .layout
        .lines
        .iter()
        .find(|l| l.block() == bare_md::markdown::Block::Code)
        .expect("code");
    let y = h.app.text_rect.y + code.top + code.height * 0.5;
    h.app.mouse = (h.app.text_rect.x + 60.0, y);
    h.shot("r7-code-copy");
    assert!(
        h.app.hits.find(|hit| matches!(hit, bare_md::app::Hit::CopyCode(..))).is_some(),
        "the code card needs a copy button"
    );
}

/// (8.1) The colour picker: hue strip, saturation/value square, presets and a
/// hex field, sized so it never covers the whole page.
#[test]
fn snapshot_r8_colour_picker() {
    let mut h = Harness::new("r8-colour-picker");
    h.app.new_tab();
    h.app.insert_text("# Colour\n\nA [#e05c5c]coloured[/] word and a [bg:#4f8fd0]highlighted[/] one.\n");
    h.app.set_caret(0, false);
    h.app.run_menu_action(bare_md::app::MenuAction::ColourPicker(false), &mut h.r);
    assert!(matches!(h.app.dialog, Some(Dialog::Colour(_))), "no colour dialog");
    h.draw();
    // Park the pointer in the middle of the saturation square so the marker shows.
    let sv = h
        .app
        .hits
        .find(|hit| matches!(hit, bare_md::app::Hit::ColourSv(_)))
        .expect("sv square");
    h.app.mouse = (sv.x + sv.w * 0.7, sv.y + sv.h * 0.3);
    h.shot("r8-colour-picker");
    for want in [
        bare_md::app::Hit::ColourSv(false),
        bare_md::app::Hit::ColourHue(false),
        bare_md::app::Hit::ColourHex(false),
        bare_md::app::Hit::ColourOk,
        bare_md::app::Hit::ColourCancel,
    ] {
        assert!(h.app.hits.find(|hit| *hit == want).is_some(), "missing {want:?}");
    }
    // Applying from the dialog colours the caret's line.
    let ok = h.app.hits.find(|hit| *hit == bare_md::app::Hit::ColourOk).expect("apply");
    h.app.mouse = (ok.x + ok.w * 0.5, ok.y + ok.h * 0.5);
    h.draw();
    h.app.click(h.app.mouse, &mut h.r);
    h.draw();
    assert!(h.app.dialog.is_none(), "the dialog stayed open");
    let text = h.app.tab().doc.text();
    assert!(text.contains("[#"), "nothing was coloured: {text}");
}

/// (8.2) Coloured text and backgrounds paint on screen, and a coloured span is
/// still coloured after a page has been typed around it.
#[test]
fn snapshot_r8_coloured_text() {
    let mut h = Harness::new("r8-colour-text");
    h.app.new_tab();
    h.app.insert_text(
        "# Colour\n\nA [#e05c5c]red[/] word, a [#4f8fd0]blue[/] one and a [bg:#e0a83c]highlighted[/] one.\n",
    );
    h.app.set_caret(0, false);
    h.shot("r8-coloured-text");
    let line = &h.app.tab().layout.lines[2];
    let inks: Vec<Option<bare_md::geom::Color>> = line.runs(false).iter().map(|r| r.style.fg).collect();
    let want_red = bare_md::layout::rgb_color(0xe05c5c);
    let want_blue = bare_md::layout::rgb_color(0x4f8fd0);
    assert!(inks.iter().any(|c| *c == Some(want_red)), "no red run: {inks:?}");
    assert!(inks.iter().any(|c| *c == Some(want_blue)), "no blue run: {inks:?}");
}

/// (8.9) The workspace header carries an "open folder" chip of its own.
#[test]
fn snapshot_r8_workspace_open_folder_chip() {
    let mut h = Harness::new("r8-open-folder");
    h.app.settings.sidebar_open = true;
    h.draw();
    let chip = h
        .app
        .hits
        .find(|hit| *hit == bare_md::app::Hit::SidebarOpenFolder)
        .expect("no open-folder chip in the workspace header");
    h.app.mouse = (chip.x + chip.w * 0.5, chip.y + chip.h * 0.5);
    h.shot("r8-open-folder");
    assert!(chip.x < h.app.text_rect.x, "the chip belongs inside the workspace panel");
}

/// (9.11) A ```` ```mermaid ```` block is drawn as a flowchart, not as code.
#[test]
fn snapshot_r9_mermaid_diagram() {
    let mut h = Harness::new("r9-mermaid");
    h.app.new_tab();
    h.app.insert_text(
        "# Diagram\n\nA flowchart inside a note:\n\n```mermaid\nflowchart TD\n  A[Start] --> B{Ready?}\n  B -->|yes| C[Ship it]\n  B -->|no| D[(Fix)]\n  D -.-> A\n  C --> E([Done])\n```\n\nAnd the text carries on below it.\n",
    );
    h.app.set_caret(0, false);
    h.draw();
    let diagram = h.app.tab().layout.lines.iter().find_map(|l| l.diagram.clone()).expect("a diagram");
    assert!(diagram.nodes.len() == 5, "got {} nodes", diagram.nodes.len());
    h.shot("r9-mermaid-diagram");

    // (9.12) The caret walks into the block and the source comes back for editing.
    let open = h.app.tab().layout.mermaid[0].0;
    h.app.set_caret(h.app.tab().doc.line_start(open + 1) + 2, false);
    h.draw();
    assert!(h.app.tab().layout.lines[open].diagram.is_none(), "the picture stayed up");
    h.shot("r9-mermaid-source");
}

/// (11.3) Opening a folder asks with a **two-column** dialog: the filesystem tree
/// on the left, that folder's contents on the right.
#[test]
fn snapshot_r11_folder_dialog_two_columns() {
    let mut h = Harness::new("r11-folder-dialog");
    let dir = std::env::temp_dir().join(format!("bare-md-ui-r11-pick-{}", std::process::id()));
    let _ = std::fs::create_dir_all(dir.join("notes"));
    let _ = std::fs::write(dir.join("README.md"), "# notes");
    h.app.open_picker(PickerKind::OpenFolder);
    if let Some(Dialog::Picker(p)) = &mut h.app.dialog {
        p.select_dir(dir.clone());
    }
    h.draw();
    h.shot("r11-folder-dialog");
    let rows = {
        let picker = match &h.app.dialog {
            Some(Dialog::Picker(p)) => p,
            _ => panic!("no picker"),
        };
        assert!(picker.tree.rows().len() > 1, "the tree has no rows");
        picker.entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>()
    };
    assert!(rows.iter().any(|n| n == "notes"), "the folder is not listed: {rows:?}");
    assert!(
        h.app.hits.find(|hit| matches!(hit, bare_md::app::Hit::DialogTreeItem(_))).is_some(),
        "the tree column is not clickable"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// (10.7) The colour tool: text on the left, background on the right.
#[test]
fn snapshot_r10_colour_menu_two_columns() {
    let mut h = Harness::new("r10-colour-menu");
    h.app.new_tab();
    h.app.insert_text("A coloured sentence.\n");
    h.app.set_caret(0, false);
    h.draw();
    h.app.open_menu(bare_md::app::MenuKind::Color, Rect::new(560.0, 52.0, 22.0, 22.0));
    h.app.mouse = (700.0, 200.0);
    h.draw();
    h.shot("r10-colour-menu");
    assert!(
        h.app.hits.find(|hit| matches!(hit, bare_md::app::Hit::MenuItem(_))).is_some(),
        "the colour menu drew no rows"
    );
}

/// (11.9) A fenced block with a language is syntax highlighted.
#[test]
fn snapshot_r11_syntax_highlight() {
    let mut h = Harness::new("r11-highlight");
    h.app.new_tab();
    h.app.insert_text(
        "# Highlighting\n\n```rust\npub fn total(values: &[u32]) -> u32 {\n    // sum them up\n    let mut sum = 0;\n    for v in values {\n        sum += *v;\n    }\n    sum\n}\n```\n\n```python\ndef load(path: str = \"notes.md\") -> str:\n    return open(path).read()\n```\n",
    );
    h.app.set_caret(0, false);
    h.draw();
    h.shot("r11-syntax-highlight");
    let coloured = h
        .app
        .tab()
        .layout
        .lines
        .iter()
        .filter(|l| l.clean.iter().any(|r| r.style.fg.is_some()))
        .count();
    assert!(coloured >= 6, "only {coloured} lines were highlighted");
}

/// (11.2) Workspaces are tabs in the panel header.
#[test]
fn snapshot_r11_workspace_tabs() {
    let mut h = Harness::new("r11-workspace-tabs");
    let base = h.app.ws.root.clone();
    let a = base.join("alpha");
    let b = base.join("beta");
    let _ = std::fs::create_dir_all(&a);
    let _ = std::fs::create_dir_all(&b);
    h.app.add_workspace(a);
    h.app.add_workspace(b);
    h.app.switch_workspace(1);
    h.draw();
    h.shot("r11-workspace-tabs");
    assert!(h.app.workspaces.len() >= 3, "the workspaces did not stack up");
}

/// (11.1) The "changed on disk" dialog.
#[test]
fn snapshot_r11_external_change() {
    let mut h = Harness::new("r11-external");
    let path = h.app.ws.root.join("note.md");
    let _ = std::fs::write(&path, "# note\n");
    h.app.open_path(&path);
    h.app.dialog = Some(Dialog::External(bare_md::app::ExternalChange {
        tab: h.app.active,
        path,
        dirty: true,
    }));
    h.draw();
    h.shot("r11-external-change");
    assert!(h.app.hits.find(|hit| *hit == bare_md::app::Hit::ExternalReload).is_some());
    assert!(h.app.hits.find(|hit| *hit == bare_md::app::Hit::ExternalKeep).is_some());
    assert!(h.app.hits.find(|hit| *hit == bare_md::app::Hit::ExternalAuto).is_some());
}

/// (0.8.17) The About box: the version, the program licence, the crate list and the
/// font licences, all of it inside the binary. Two frames - the top, and the end of
/// the font licences after scrolling - so the box's own scrolling is visible too.
#[test]
fn snapshot_about_box() {
    let mut h = Harness::new("about");
    h.app.open_about();
    h.draw();
    h.shot("about-top");
    assert!(h.app.hits.find(|hit| *hit == bare_md::app::Hit::AboutSave).is_some());
    assert!(h.app.hits.find(|hit| *hit == bare_md::app::Hit::AboutClose).is_some());

    // The text really is painted: count the lit pixels inside the box's middle band,
    // which is inside the text pane whichever way the frame is read back.
    let top = h.shot("about-top-lines");
    let lit = inked(&top);
    assert!(lit > 2000, "the About box looks empty ({lit} lit pixels)");

    // Scrolled to the end, the box shows the tail of the DejaVu/Arev licence.
    if let Some(Dialog::About(a)) = &mut h.app.dialog {
        let last = a.text.len().saturating_sub(a.visible);
        a.scroll = last;
    }
    let end = h.shot("about-end");
    assert!(inked(&end) > 2000, "the end of the notice looks empty");
}

/// Lit pixels in the middle of the frame, x 30–70 %, y 35–65 %.
///
/// A crude "there is writing there" measure: it does not need to know the font, only
/// that the document behind the scrim is darker than the text in the box.
fn inked(path: &Path) -> usize {
    let file = std::fs::File::open(path).expect("the snapshot");
    let mut dec = png::Decoder::new(file);
    dec.set_transformations(png::Transformations::EXPAND);
    let mut reader = dec.read_info().expect("png header");
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("png pixels");
    let (w, h) = (info.width as usize, info.height as usize);
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        _ => panic!("unexpected snapshot format {:?}", info.color_type),
    };
    let (x0, x1) = (w * 3 / 10, w * 7 / 10);
    let (y0, y1) = (h * 35 / 100, h * 65 / 100);
    let mut lit = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * w + x) * channels;
            if buf[i] > 90 && buf[i + 1] > 90 && buf[i + 2] > 90 {
                lit += 1;
            }
        }
    }
    lit
}
