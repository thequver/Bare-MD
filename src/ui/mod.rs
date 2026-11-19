//! All drawing: the chrome around the document, the document itself and the
//! dialogs.
//!
//! Immediate mode, in one place: every frame rebuilds a hit map that the input
//! layer consumes in the same frame, so geometry is defined exactly once. The
//! module is split by *what* is drawn, not by widget:
//!
//! ```text
//! mod.rs       the frame: allocate the columns, draw the sheet, then the chrome
//! painter.rs   Painter: the only way to put a quad or a glyph on screen
//! chrome.rs    tab strip, toolbar, menus, status bar, workspace panel, outline
//! document.rs  the document: text, tables, minimap, pictures, mermaid diagrams
//! dialogs.rs   find bar, folder dialog, colour picker, "changed on disk"
//! ```
//!
//! Everything below the `use` block is re-exported, so `crate::ui::draw` and the
//! helpers the input layer and the tests share keep one flat namespace.

mod chrome;
mod dialogs;
mod document;
mod painter;

pub use chrome::*;
pub use dialogs::*;
pub use document::*;
pub use painter::*;

// ---- the icons, the field widget, the themes, the app icon and the about box ----
pub mod about;
pub mod third_party;
pub mod appicon;
pub mod field;
pub mod icons;
pub mod svg;
pub mod theme;

pub(crate) use crate::app::{
    ink_color, App, Dialog, Focus, Hit, MenuKind, Picker, PickerKind, ToolbarAction,
};
pub(crate) use crate::geom::{Color, Rect};
pub(crate) use crate::gpu::Renderer;
pub(crate) use crate::layout::{
    Inks, Layout, LineLayout, ViewMode, CARD_PAD_Y, CODE_CHIP_PAD, CODE_PAD_X, QUOTE_INDENT,
    TABLE_CELL_PAD,
};
pub(crate) use crate::markdown::Block;
pub(crate) use crate::text::Face;
pub(crate) use crate::theme::Theme;

pub struct Chrome {
    pub tabbar: Rect,
    pub status: Rect,
    pub sidebar: Rect,
    pub outline: Rect,
    pub doc: Rect,
    pub minimap: Rect,
    pub editor: Rect,
    pub text: Rect,
}

/// Layout of the window:
///
///   +-----------+--------------------------------------+
///   |           |  tabs (same colour as the document)  |
///   | workspace +--------------------------------------+
///   |  sidebar  |  editor (floating toolbar hovers here)|
///   +-----------+--------------------------------------+
///   |                    status bar                     |
pub fn chrome(app: &App, w: f32, h: f32) -> Chrome {
    let root = Rect::new(0.0, 0.0, w, h);
    let (body, status) = root.split_top((h - Theme::STATUS_H).max(0.0));
    let (sidebar, right) = if app.settings.sidebar_open {
        let width = Theme::SIDEBAR_W.min(body.w * 0.42);
        body.split_left(width)
    } else {
        (Rect::ZERO, body)
    };
    // `split_right` returns (right strip, remaining) - the outline is the strip.
    let (outline, doc) = if app.settings.outline_open && right.w > 620.0 {
        let width = Theme::OUTLINE_W.min(right.w * 0.3);
        right.split_right(width)
    } else {
        (Rect::ZERO, right)
    };
    let (tabbar, doc_body) = doc.split_top(Theme::TABBAR_H);
    let (minimap, editor) = if app.settings.minimap_open && doc_body.w > 420.0 {
        doc_body.split_right(Theme::MINIMAP_W)
    } else {
        (Rect::ZERO, doc_body)
    };
    let text = Rect::new(
        editor.x + Theme::EDITOR_PAD_X,
        editor.y + Theme::EDITOR_PAD_TOP,
        (editor.w - Theme::EDITOR_PAD_X * 2.0 - 10.0).max(80.0),
        (editor.h - Theme::EDITOR_PAD_TOP - 10.0).max(40.0),
    );
    Chrome { tabbar, status, sidebar, outline, doc, minimap, editor, text }
}

// ---------------------------------------------------------------- entry point

pub fn draw(app: &mut App, r: &mut Renderer) {
    let (w, h) = r.logical_size();
    let ch = chrome(app, w, h);
    app.editor_rect = ch.editor;
    app.text_rect = ch.text;
    app.minimap_rect = ch.minimap;
    match &app.dialog {
        Some(Dialog::Picker(pk)) => {
            app.dialog_tree_rect = picker_tree_rect(pk.kind, w, h);
            app.dialog_list_rect = picker_list_rect(pk.kind, w, h);
        }
        _ => {
            app.dialog_tree_rect = Rect::ZERO;
            app.dialog_list_rect = Rect::ZERO;
        }
    }
    app.tabbar_rect = ch.tabbar;
    app.sidebar_rect = ch.sidebar;

    // ---- prepare: look for outside edits, refresh the minimap's follow state ----
    app.poll_disk_if_due();

    // ---- prepare: layout, scroll clamping, search refresh ----
    let theme = app.theme;
    let width = ch.text.w;
    let idx = app.active.min(app.tabs.len().saturating_sub(1));
    app.active = idx;
    let te = &mut r.text;
    app.tabs[idx].ensure_layout(te, &theme, width, app.mode, app.settings.wrap);
    {
        let t = &mut app.tabs[idx];
        if t.find.open {
            let doc = &t.doc;
            t.find.ensure_matches(doc);
        }
    }
    {
        let tab = &mut app.tabs[idx];
        let total = tab.layout.total_height;
        let max_scroll = (total - ch.text.h).max(0.0);
        tab.scroll = tab.scroll.clamp(0.0, max_scroll);
        let max_scroll_x = (tab.layout.max_row_width + 8.0 - ch.text.w).max(0.0);
        tab.scroll_x = tab.scroll_x.clamp(0.0, max_scroll_x);
    }

    r.begin(theme.bg);
    let mut p = Painter::new(r, theme, app.mouse);

    if app.settings.sidebar_open {
        draw_sidebar(app, &mut p, ch.sidebar);
    }
    app.outline_rect = ch.outline;
    if app.settings.outline_open {
        draw_outline(app, &mut p, ch.outline);
    }
    // A tab that was just opened (or switched to with `Ctrl+Tab`) is scrolled into
    // view **before** the strip is painted, using the chip rects of the last frame:
    // the scroll is a delta against the geometry that was on screen, so applying it
    // here lands the chip exactly at the edge on this frame - doing it afterwards
    // always drew one frame of the old offset.
    if std::mem::take(&mut app.pending_reveal_tab) {
        app.reveal_active_tab();
    }
    draw_doc_column(app, &mut p, ch.tabbar, ch.editor, ch.text);
    if app.settings.minimap_open && !ch.minimap.is_empty() {
        draw_minimap(app, &mut p, ch.minimap, ch.text);
    }
    draw_status(app, &mut p, ch.status);
    // The toolbar floats above the document, so it is drawn (and hit-tested) last.
    app.toolbar_rect = floating_toolbar_rect(app, ch.editor);
    draw_floating_toolbar(app, &mut p, ch.editor);

    if app.tabs[idx].find.open {
        draw_find_bar(app, &mut p, ch.editor);
    }
    flush_tooltips(&mut p);
    // Menus sit above everything except a modal dialog.
    if let Some(menu) = &app.menu {
        draw_menu(menu, &mut p, w, h);
    }
    // …and so do the two right-click menus (a tab, a row of the tree).
    draw_tab_menu(app, &mut p, w, h);
    draw_tree_menu(app, &mut p, w, h);
    // The About box scrolls, so it has to know how many lines it can show before the
    // text is drawn - and a window that got shorter must not leave it scrolled past
    // the end.
    if let Some(Dialog::About(about)) = &mut app.dialog {
        let d = about::box_rect(w, h);
        about.clamp(about::visible_lines(d.h));
        // The drag maps the pointer through this, so it is recorded with the frame
        // that painted it.
        app.about_track = about::track_rect(d);
    }
    if let Some(dialog) = &app.dialog {
        draw_dialog(app, &mut p, dialog, w, h);
    }

    app.hits = crate::app::Hits::new(p.hits);
}
