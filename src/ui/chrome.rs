//! The chrome: everything that frames the document.
//!
//! Tab strip, floating toolbar, menus, status bar, workspace panel and outline.
//! None of it knows where it is on screen: every function takes the rectangle it
//! may use, and the frame in `mod.rs` is the only place that decides those.

use super::*;

pub fn draw_menu(menu: &crate::app::Menu, p: &mut Painter, w: f32, h: f32) {
    if menu.split > 0 {
        draw_menu_two_columns(menu, p, w, h);
        return;
    }
    let row_h = 24.0;
    let pad = 5.0;
    let mut width = 150.0f32;
    for e in &menu.entries {
        let w = p.width(&e.label, Face::Regular, p.theme.ui_size())
            + p.width(&e.hint, Face::Regular, p.theme.ui_small())
            + 54.0;
        width = width.max(w);
    }
    let width = width.min(320.0);
    let height: f32 = menu
        .entries
        .iter()
        .map(|e| if e.divider { row_h + 7.0 } else { row_h })
        .sum::<f32>()
        + pad * 2.0;
    let x = menu.x.min(w - width - 8.0).max(8.0);
    let y = menu.y.min(h - height - 8.0).max(8.0);
    let panel = Rect::new(x, y, width, height);
    p.round(panel.translate(0.0, 4.0), 10.0, Color::rgba(0.0, 0.0, 0.0, 0.35));
    p.round(panel, 10.0, p.theme.bg_elevated);
    p.stroke(panel, 10.0, 1.0, p.theme.border_strong);

    // The panel itself is registered first: a click on its background only
    // dismisses the menu, while the entry rows (drawn next) stay clickable.
    p.hit(panel, Hit::MenuButton(MenuKind::Context));

    let mut cy = panel.y + pad;
    for (i, entry) in menu.entries.iter().enumerate() {
        if entry.divider {
            p.hline(panel.x + 8.0, cy + 3.0, panel.w - 16.0, p.theme.border);
            cy += 7.0;
        }
        let r = Rect::new(panel.x + 3.0, cy, panel.w - 6.0, row_h);
        if p.hovered(r) {
            p.round(r, 6.0, p.theme.bg_hover);
        }
        let size = p.theme.ui_size();
        let b = p.baseline_in(r, size);
        let mut tx = r.x + 8.0;
        if let Some(swatch) = entry.swatch {
            let sw = Rect::new(tx, r.y + (row_h - 12.0) * 0.5, 12.0, 12.0);
            p.round(sw, 3.0, swatch);
            p.stroke(sw, 3.0, 1.0, p.theme.border_strong);
            tx += 18.0;
        } else if entry.checked {
            p.text(tx, b, "\u{2713}", Face::Bold, size, p.theme.text);
            tx += 18.0;
        }
        p.text(tx, b, &entry.label, Face::Regular, size, p.theme.text);
        if !entry.hint.is_empty() {
            p.label_right(
                Rect::new(r.x, r.y, r.w - 8.0, r.h),
                0.0,
                &entry.hint,
                Face::Regular,
                p.theme.ui_small(),
                p.theme.text_faint,
            );
        }
        p.hit(r, Hit::MenuItem(i));
        cy += row_h;
    }
}

/// A small right-click menu for a tab or a workspace row.
///
/// One painter for both, because they are the same thing: a list of labels at a
/// point, with the row under the pointer filled in. `row_hit` and `panel_hit` differ
/// only in which `Hit` variant the row registers.
fn draw_item_menu<A>(
    p: &mut Painter,
    w: f32,
    h: f32,
    at: (f32, f32),
    labels: &[(String, A)],
    row_hit: &dyn Fn(usize) -> Hit,
    panel_hit: Hit,
) {
    let row_h = 24.0;
    let pad = 5.0;
    let width = labels
        .iter()
        .map(|(l, _)| p.width(l, Face::Regular, p.theme.ui_size()) + 44.0)
        .fold(140.0f32, f32::max)
        .min(280.0);
    let height = labels.len() as f32 * row_h + pad * 2.0;
    let x = at.0.min(w - width - 8.0).max(8.0);
    let y = at.1.min(h - height - 8.0).max(8.0);
    let panel = Rect::new(x, y, width, height);
    p.round(panel.translate(0.0, 4.0), 10.0, Color::rgba(0.0, 0.0, 0.0, 0.35));
    p.round(panel, 10.0, p.theme.bg_elevated);
    p.stroke(panel, 10.0, 1.0, p.theme.border_strong);
    // The panel first: a click on its own background only dismisses it.
    p.hit(panel, panel_hit);
    for (i, (label, _)) in labels.iter().enumerate() {
        let r = Rect::new(panel.x + 3.0, panel.y + pad + i as f32 * row_h, panel.w - 6.0, row_h);
        if p.hovered(r) {
            p.round(r, 6.0, p.theme.bg_hover);
        }
        let size = p.theme.ui_size();
        let b = p.baseline_in(r, size);
        // "Delete" is the one entry that destroys something.
        let ink = if label.starts_with("Delete") { p.theme.danger } else { p.theme.text };
        p.text(r.x + 10.0, b, label, Face::Regular, size, ink);
        p.hit(r, row_hit(i));
    }
}

/// The tab strip's menu, if it is open.
pub fn draw_tab_menu(app: &App, p: &mut Painter, w: f32, h: f32) {
    let Some(menu) = &app.tab_menu else { return };
    draw_item_menu(
        p,
        w,
        h,
        menu.at,
        &menu.entries,
        &|i| Hit::TabMenu(i),
        Hit::MenuButton(MenuKind::Context),
    );
}

/// The workspace tree's menu, if it is open.
pub fn draw_tree_menu(app: &App, p: &mut Painter, w: f32, h: f32) {
    let Some(menu) = &app.tree_menu else { return };
    draw_item_menu(
        p,
        w,
        h,
        menu.at,
        &menu.entries,
        &|i| Hit::TreeMenu(i),
        Hit::MenuButton(MenuKind::Context),
    );
}

/// A menu laid out in two columns (the colour tool: text on the left, background
/// on the right), each with its own heading.
pub fn draw_menu_two_columns(menu: &crate::app::Menu, p: &mut Painter, w: f32, h: f32) {
    let row_h = 24.0;
    let pad = 6.0;
    let head_h = 18.0;
    let gap = 14.0;
    let split = menu.split.min(menu.entries.len());

    // Width of each column from its own labels.
    let mut col_w = |entries: &[crate::app::MenuEntry]| -> f32 {
        entries
            .iter()
            .map(|e| p.width(&e.label, Face::Regular, p.theme.ui_size()) + 40.0)
            .fold(96.0f32, f32::max)
    };
    let left_w = col_w(&menu.entries[..split]);
    let right_w = col_w(&menu.entries[split..]);
    let width = (left_w + right_w + pad * 2.0 + gap).min(420.0);
    let rows = |entries: &[crate::app::MenuEntry]| -> f32 {
        entries
            .iter()
            .map(|e| if e.divider { row_h + 7.0 } else { row_h })
            .sum::<f32>()
    };
    let body = rows(&menu.entries[..split]).max(rows(&menu.entries[split..]));
    let height = body + pad * 2.0 + head_h;
    let x = menu.x.min(w - width - 8.0).max(8.0);
    let y = menu.y.min(h - height - 8.0).max(8.0);
    let panel = Rect::new(x, y, width, height);
    p.round(panel.translate(0.0, 4.0), 10.0, Color::rgba(0.0, 0.0, 0.0, 0.35));
    p.round(panel, 10.0, p.theme.bg_elevated);
    p.stroke(panel, 10.0, 1.0, p.theme.border_strong);
    p.hit(panel, Hit::MenuButton(MenuKind::Context));

    let half = (width - pad * 2.0 - gap) * 0.5;
    for (col, (from, to, head)) in [
        (0usize, (0usize, split, menu.heads.0)),
        (1usize, (split, menu.entries.len(), menu.heads.1)),
    ] {
        let cx = panel.x + pad + col as f32 * (half + gap);
        p.label(
            Rect::new(cx + 8.0, panel.y + pad, half, head_h),
            0.0,
            head,
            Face::Bold,
            p.theme.ui_small(),
            p.theme.text_faint,
        );
        let mut cy = panel.y + pad + head_h;
        for i in from..to {
            let entry = &menu.entries[i];
            if entry.divider {
                p.hline(cx + 8.0, cy + 3.0, half - 16.0, p.theme.border);
                cy += 7.0;
            }
            let r = Rect::new(cx + 3.0, cy, half - 6.0, row_h);
            if p.hovered(r) {
                p.round(r, 6.0, p.theme.bg_hover);
            }
            let size = p.theme.ui_size();
            let b = p.baseline_in(r, size);
            let mut tx = r.x + 8.0;
            if let Some(swatch) = entry.swatch {
                let sw = Rect::new(tx, r.y + (row_h - 12.0) * 0.5, 12.0, 12.0);
                p.round(sw, 3.0, swatch);
                p.stroke(sw, 3.0, 1.0, p.theme.border_strong);
                tx += 18.0;
            }
            p.text(tx, b, &entry.label, Face::Regular, size, p.theme.text);
            p.hit(r, Hit::MenuItem(i));
            cy += row_h;
        }
    }
    // A hairline between the columns.
    let edge = panel.x + pad + half + gap * 0.5;
    p.vline(edge, panel.y + pad + head_h - 4.0, height - pad * 2.0 - head_h, p.theme.border);
}

// ---------------------------------------------------------------- toolbar

/// A compact row of controls that hovers over the top of the document.
///
/// Groups that do not fit are dropped as the window narrows; the pill never
/// grows into a toolbar band.
pub fn toolbar_groups(editor: Rect) -> (f32, bool, bool, bool) {
    let pad = 5.0;
    let sep_w = 11.0;
    let slot = 26.0;
    let avail = (editor.w - 24.0).min(820.0);

    // history(2) | inline(8) | meta(4 dropdowns) | lists(2) | blocks(5) and the
    // always-visible tail (attach, table, tools) on the right.
    let history = 2.0 * slot;
    let inline = 8.0 * slot + sep_w;
    let meta = 4.0 * slot + sep_w;
    let lists = 2.0 * slot + sep_w;
    let blocks = 5.0 * slot + sep_w;
    let tail = 3.0 * slot + sep_w;
    let base = pad * 2.0 + history + inline + tail;

    let (show_meta, show_lists, show_blocks) = if avail >= base + meta + lists + blocks {
        (true, true, true)
    } else if avail >= base + meta + lists {
        (true, true, false)
    } else if avail >= base + meta {
        (true, false, false)
    } else {
        (false, false, false)
    };
    let mut width = base;
    if show_meta {
        width += meta;
    }
    if show_lists {
        width += lists;
    }
    if show_blocks {
        width += blocks;
    }
    (width.min(editor.w - 8.0), show_meta, show_lists, show_blocks)
}

/// Rectangle the floating toolbar occupies over the top of the document.
pub fn floating_toolbar_rect(_app: &App, editor: Rect) -> Rect {
    let (width, _, _, _) = toolbar_groups(editor);
    Rect::new(editor.x + (editor.w - width) * 0.5, editor.y + 12.0, width, Theme::FLOATING_TOOLBAR_H)
}



/// It is drawn last so it sits above the text, and it shrinks (dropping the
/// less-used groups) when the window is narrow.
/// The colour the toolbar swatch shows before anything has been coloured: a warm
/// red, chosen because it is the one hue that reads as "colour" at 16 px.
pub const DEFAULT_COLOUR: u32 = 0xe0603c;

pub fn draw_floating_toolbar(app: &App, p: &mut Painter, editor: Rect) {
    let theme = p.theme;
    let bh = 28.0;
    let pad = 5.0;
    let gap = 2.0;
    let bw = 24.0;

    let (_, show_meta, show_lists, show_blocks) = toolbar_groups(editor);
    let rect = floating_toolbar_rect(app, editor);

    // Shadow + pill.
    p.round(rect.translate(0.0, 3.0), 13.0, Color::rgba(0.0, 0.0, 0.0, if theme.kind.is_light() { 0.10 } else { 0.45 }));
    p.round(rect, 12.0, theme.bg_elevated);
    p.stroke(rect, 12.0, 1.0, theme.border_strong);

    struct Pen {
        x: f32,
        y: f32,
        h: f32,
        gap: f32,
    }
    impl Pen {
        fn slot(&mut self, w: f32) -> Rect {
            let r = Rect::new(self.x, self.y, w, self.h);
            self.x += w + self.gap;
            r
        }
        fn sep(&mut self, p: &mut Painter) {
            p.vline(self.x + 4.0, self.y + 6.0, self.h - 12.0, p.theme.border_strong);
            self.x += 11.0;
        }
    }
    let mut pen = Pen { x: rect.x + pad, y: rect.y + pad, h: bh, gap };

    let icon_button = |p: &mut Painter, r: Rect, hit: Hit, active: bool, draw: &dyn Fn(&mut Painter, Rect, Color)| {
        let hovered = p.hovered(r);
        if active {
            p.round(r, 6.0, p.theme.bg_active);
        } else if hovered {
            p.round(r, 6.0, p.theme.bg_hover);
        }
        draw(p, r.inset(4.0), if active { p.theme.text } else { p.theme.text });
        p.hit(r, hit);
    };
    // A button that opens a dropdown: same slot plus a chevron.
    let drop_button = |p: &mut Painter, r: Rect, kind: MenuKind, active: bool, draw: &dyn Fn(&mut Painter, Rect, Color)| {
        let hovered = p.hovered(r);
        if active || hovered {
            p.round(r, 6.0, if active { p.theme.bg_active } else { p.theme.bg_hover });
        }
        draw(p, Rect::new(r.x + 3.0, r.y + 6.0, r.w - 11.0, r.h - 12.0), p.theme.text);
        p.icon(
            if active { "chevron-up" } else { "chevron" },
            Rect::new(r.right() - 9.0, r.y + r.h * 0.5 - 4.0, 8.0, 8.0),
            if active { p.theme.text } else { p.theme.text_faint },
        );
        p.hit(r, Hit::MenuButton(kind));
    };

    // ---- history ----
    let r = pen.slot(bw);
    icon_button(p, r, Hit::Toolbar(ToolbarAction::Undo), false, &|p, r, c| { p.icon("undo", r, c); });
    let r = pen.slot(bw);
    icon_button(p, r, Hit::Toolbar(ToolbarAction::Redo), false, &|p, r, c| { p.icon("redo", r, c); });

    // ---- inline formatting ----
    pen.sep(p);
    let r = pen.slot(bw);
    icon_button(p, r, Hit::Toolbar(ToolbarAction::Bold), false, &|p, r, c| { p.icon("bold", r, c); });
    let r = pen.slot(bw);
    icon_button(p, r, Hit::Toolbar(ToolbarAction::Italic), false, &|p, r, c| { p.icon("italic", r, c); });
    let r = pen.slot(bw);
    icon_button(p, r, Hit::Toolbar(ToolbarAction::Underline), false, &|p, r, c| { p.icon("underline", r, c); });
    let r = pen.slot(bw);
    icon_button(p, r, Hit::Toolbar(ToolbarAction::Strike), false, &|p, r, c| { p.icon("strike", r, c); });
    let r = pen.slot(bw);
    icon_button(p, r, Hit::Toolbar(ToolbarAction::Highlight), false, &|p, r, c| { p.icon("highlight", r, c); });
    let r = pen.slot(bw);
    icon_button(p, r, Hit::Toolbar(ToolbarAction::Code), false, &|p, r, c| { p.icon("code", r, c); });
    let r = pen.slot(bw);
    icon_button(p, r, Hit::Toolbar(ToolbarAction::Link), false, &|p, r, c| { p.icon("link", r, c); });
    let r = pen.slot(bw);
    icon_button(p, r, Hit::Toolbar(ToolbarAction::ClearFormat), false, &|p, r, c| { p.icon("clear", r, c); });

    if show_meta {
        pen.sep(p);
        let active = app.menu.as_ref().map(|m| m.kind) == Some(MenuKind::Heading);
        let r = pen.slot(bw);
        drop_button(p, r, MenuKind::Heading, active, &|p, r, c| { p.icon("heading", r, c); });

        let active = app.menu.as_ref().map(|m| m.kind) == Some(MenuKind::SupSub);
        let r = pen.slot(bw);
        drop_button(p, r, MenuKind::SupSub, active, &|p, r, c| { p.icon("sup", r, c); });

        let active = app.menu.as_ref().map(|m| m.kind) == Some(MenuKind::Align);
        let r = pen.slot(bw);
        drop_button(p, r, MenuKind::Align, active, &|p, r, c| { p.icon("align", r, c); });

        let active = app.menu.as_ref().map(|m| m.kind) == Some(MenuKind::Color);
        let r = pen.slot(bw);
        drop_button(p, r, MenuKind::Color, active, &|p, r, c| { p.icon("color", r, c); });
        // A swatch of the last colour used under the "A": the button is the only
        // one in the pill that mixes in colour, which is exactly how it is found.
        {
            let swatch = app.last_colour.unwrap_or(DEFAULT_COLOUR);
            let icon = Rect::new(r.x + 3.0, r.y + 6.0, r.w - 11.0, r.h - 12.0);
            p.round(
                Rect::new(icon.x + icon.w * 4.0 / 24.0, icon.y + icon.h * 18.6 / 24.0, icon.w * 16.0 / 24.0, icon.h * 3.2 / 24.0),
                1.2,
                crate::layout::rgb_color(swatch),
            );
        }
    }

    if show_lists {
        pen.sep(p);
        let r = pen.slot(bw);
        icon_button(p, r, Hit::Toolbar(ToolbarAction::Bullet), false, &|p, r, c| { p.icon("bullet", r, c); });
        let r = pen.slot(bw);
        icon_button(p, r, Hit::Toolbar(ToolbarAction::Ordered), false, &|p, r, c| { p.icon("ordered", r, c); });
    }

    if show_blocks {
        pen.sep(p);
        let r = pen.slot(bw);
        icon_button(p, r, Hit::Toolbar(ToolbarAction::Task), false, &|p, r, c| { p.icon("task", r, c); });
        let r = pen.slot(bw);
        icon_button(p, r, Hit::Toolbar(ToolbarAction::Quote), false, &|p, r, c| { p.icon("quote", r, c); });
        let r = pen.slot(bw);
        icon_button(p, r, Hit::Toolbar(ToolbarAction::Renumber), false, &|p, r, c| { p.icon("renumber", r, c); });
        let r = pen.slot(bw);
        icon_button(p, r, Hit::Toolbar(ToolbarAction::MoveUp), false, &|p, r, c| { p.icon("moveup", r, c); });
        let r = pen.slot(bw);
        icon_button(p, r, Hit::Toolbar(ToolbarAction::MoveDown), false, &|p, r, c| { p.icon("movedown", r, c); });
    }

    // The table / attach / tools cluster is always the last thing in the pill.
    let mut rx = rect.right() - pad - bw;
    let tail = |p: &mut Painter, rx: &mut f32, action: ToolbarAction, draw: &dyn Fn(&mut Painter, Rect, Color)| {
        let r = Rect::new(*rx, rect.y + pad, bw, bh);
        *rx -= bw + gap;
        let hovered = p.hovered(r);
        if hovered {
            p.round(r, 6.0, p.theme.bg_hover);
        }
        draw(p, r.inset(4.0), p.theme.text);
        p.hit(r, Hit::Toolbar(action));
    };
    tail(p, &mut rx, ToolbarAction::AttachFile, &|p, r, c| { p.icon("attach", r, c); });
    // The table button is a dropdown: insert a table, or act on the one the
    // caret is in.
    {
        let r = Rect::new(rx, rect.y + pad, bw, bh);
        rx -= bw + gap;
        let active = app.menu.as_ref().map(|m| m.kind) == Some(MenuKind::Table);
        if active || p.hovered(r) {
            p.round(r, 6.0, if active { p.theme.bg_active } else { p.theme.bg_hover });
        }
        p.icon("table", r.inset(4.0), p.theme.text);
        p.hit(r, Hit::MenuButton(MenuKind::Table));
    }
    {
        let r = Rect::new(rx, rect.y + pad, bw, bh);
        let active = app.menu.as_ref().map(|m| m.kind) == Some(MenuKind::Tools);
        if active || p.hovered(r) {
            p.round(r, 6.0, if active { p.theme.bg_active } else { p.theme.bg_hover });
        }
        p.icon("tools", r.inset(4.0), p.theme.text);
        p.hit(r, Hit::MenuButton(MenuKind::Tools));
    }

    // The pen is advanced by the groups above; the tail grows from the right, so
    // a very narrow window never overlaps the two.
    let _ = pen;
}

/// A floppy-disk glyph for the save button.

// ---------------------------------------------------------------- tabs

/// The tab strip and the editor share one background colour so the document
/// reads as a single surface with its tabs folded into it.
pub fn draw_doc_column(app: &mut App, p: &mut Painter, tabbar: Rect, editor: Rect, text: Rect) {
    p.rect(Rect::new(tabbar.x, tabbar.y, tabbar.w, tabbar.h + editor.h), p.theme.bg);
    draw_tabs(app, p, tabbar);
    p.hline(tabbar.x, tabbar.bottom() - 1.0, tabbar.w, p.theme.border);
    draw_editor(app, p, editor, text);
}

/// The close button of a tab-like chip, drawn the same way for a document tab and
/// a workspace tab.
///
/// One function, one geometry: the `×` of a workspace chip used to be smaller,
/// thinner and further from the edge than the `×` of a document tab, so the two
/// rows above each other did not line up. The returned rectangle is where the hit
/// belongs; the caller registers it (after the chip, so it wins the hit test).
pub fn chip_close(p: &mut Painter, chip: Rect) -> Rect {
    let cr = Rect::new(chip.right() - 21.0, chip.y + (chip.h - 16.0) * 0.5, 16.0, 16.0);
    let hot = p.hovered(cr);
    if hot {
        p.round(cr, 4.0, p.theme.bg_active);
    }
    // The glyph fills the button. A 7 px cross inside a 16 px button read as a speck;
    // the button itself is the size it should be (it is the same in the workspace
    // tabs and the tests pin the two against each other), so only the ink grew.
    p.icon("close", cr.inset(1.5), if hot { p.theme.text } else { p.theme.text_faint });
    cr
}

/// Ink for a tab's name.
///
/// A document whose file has been deleted is the one case where the monochrome
/// palette gives way: the name and its dot turn red, because the text in that tab
/// now exists nowhere else and the user has to see it before closing anything.
pub fn tab_ink(theme: &Theme, active: bool, gone: bool) -> Color {
    if gone {
        theme.danger
    } else if active {
        theme.text
    } else {
        theme.text_dim
    }
}

/// Left inset of a tab's name.
pub const TAB_NAME_INSET: f32 = 11.0;
/// Diameter of the "this tab has unsaved changes" dot.
pub const TAB_DOT: f32 = 5.0;

/// The right-hand lane of a tab: the room kept free for the close button and for
/// the unsaved-changes dot.
///
/// The name is ellipsised into `r.w - TAB_NAME_INSET - tab_lane(..)`, so however
/// long a document is called, nothing the tab draws can reach past its own right
/// edge and land on the tab next to it. The dot and the button have separate
/// slots in that lane, so a dirty tab whose name is long enough to need the whole
/// name area still shows both.
pub fn tab_lane(show_close: bool, dirty: bool) -> f32 {
    let close = if show_close { 26.0 } else { 0.0 };
    let dot = if dirty { 10.0 } else { 0.0 };
    let idle: f32 = if show_close || dirty { 0.0 } else { 15.0 };
    (close + dot + idle).max(15.0)
}

/// Centre of the unsaved-changes dot, inside the tab's own right-hand lane and
/// never on top of the close button.
pub fn tab_dot_center(r: Rect, show_close: bool) -> (f32, f32) {
    let cx = if show_close {
        // Left of the close button (which spans `right-21 .. right-5`).
        r.right() - 21.0 - 8.0
    } else {
        r.right() - 8.5
    };
    (cx, r.y + r.h * 0.5)
}

/// Height of the view chips in the tab strip.
pub const CHIP: f32 = 28.0;
/// Gap the chips put between themselves.
pub const CHIP_GAP: f32 = 4.0;
/// The `Live / Live+ / Raw` switch.
const MODE_W: f32 = 54.0;
/// The `Find` button.
const FIND_W: f32 = 40.0;

/// How much of the tab strip's right-hand end belongs to the view controls.
///
/// The tabs walk rightwards and the controls walk leftwards into the same strip,
/// so the strip needs one number for where the controls begin - and it has to be
/// the **real** width of the lane, not a guess. It was a guessed 220 px, which was
/// right when the lane held two buttons; with six view chips plus the mode switch
/// and Find it is more than twice that, and every tab past the guess was drawn on
/// top of the controls ("when many tabs are open, tabs overlap with find, mode, etc
/// buttons").
/// The lane, left to right: Find, the mode switch, then the seven view chips.
pub const CHIP_COUNT: f32 = 7.0;

pub fn view_lane_w() -> f32 {
    FIND_W
        + 6.0
        + MODE_W
        + 4.0
        + CHIP_COUNT * CHIP
        + (CHIP_COUNT - 1.0) * CHIP_GAP
        + 10.0
}

/// Width of the constant lane at each end of the tab strip that the scroll
/// chevrons live in. Always reserved, so a chip is never under a button and the
/// strip does not shift when one appears.
pub const TAB_LANE: f32 = 24.0;

/// Is the pointer over one of the tab strip's scroll chevrons?
///
/// The chips cannot ask the hit map (it is being built while they draw), so the
/// answer is derived from the same numbers the chevrons are drawn with.
fn over_chevron(rect: Rect, strip_right: f32, sx: f32, max: f32, mouse: (f32, f32)) -> bool {
    if max <= 0.5 {
        return false;
    }
    let chips_right = (strip_right - TAB_LANE).max(rect.x + TAB_LANE);
    if sx > 0.5 && Rect::new(rect.x + 6.0, rect.y + 6.0, 16.0, rect.h - 12.0).contains(mouse) {
        return true;
    }
    if sx < max - 0.5
        && Rect::new(chips_right + 2.0, rect.y + 6.0, 16.0, rect.h - 12.0).contains(mouse)
    {
        return true;
    }
    false
}

/// Left edge of the view controls: the tabs may not draw past this.
pub fn view_lane_left(rect: Rect) -> f32 {
    (rect.right() - view_lane_w()).max(rect.x + 60.0)
}

pub fn draw_tabs(app: &mut App, p: &mut Painter, rect: Rect) {
    // No panel colour here: the strip is part of the document surface.
    let size = p.theme.ui_size();
    // Where the tabs have to stop. Published so the drag and the tests agree with
    // what is drawn.
    let lane_left = view_lane_left(rect);
    let strip_right = (lane_left - 8.0).max(rect.x + 24.0);
    app.tab_strip_right = strip_right;
    let mut tab_rects: Vec<Rect> = Vec::with_capacity(app.tabs.len());

    // ---- measure every chip, then decide what has to scroll ----
    // The strip used to simply stop at the lane's edge, which made the view chips
    // safe but put the newest tabs out of reach. It scrolls now: all the chips are
    // measured here, and the ones that do not fit are reached by scrolling (wheel
    // over the strip, `Shift`/`Alt`+wheel, a drag on the strip, the chevrons, or
    // `Ctrl+Tab` bringing the active tab into view).
    let widths: Vec<f32> = app
        .tabs
        .iter()
        .map(|t| (p.width(&t.title(), Face::Regular, size) + 46.0).clamp(84.0, 200.0))
        .collect();
    let gap = 2.0;
    // The chips live between two constant lanes: `TAB_LANE` at the left for the
    // "scroll back" chevron and at the right for "scroll on". The chevrons used to
    // be pushed in over the chips, so a button sat on top of a tab and the tab under
    // it lit up on hover; a lane that is always there - whether or not the chevron is
    // drawn this frame - means a tab can never be underneath one, and the strip does
    // not jump by 18 px the first time a chevron appears.
    let chips_left = rect.x + TAB_LANE;
    let chips_right = (strip_right - TAB_LANE).max(chips_left);
    let content: f32 = widths.iter().sum::<f32>() + gap * widths.len().max(1) as f32 + 24.0;
    let room = (chips_right - chips_left).max(0.0);
    app.tab_scroll_max = (content - room).max(0.0);
    app.tab_strip_left = chips_left;
    app.tab_scroll = app.tab_scroll.clamp(0.0, app.tab_scroll_max);
    // Whole chips only: a strip stopped mid-chip reads as a broken layout, and the
    // chips stay on whole pixels so the glyphs do not blur.
    app.tab_scroll_drawn = app.tab_scroll.round();
    let sx = app.tab_scroll_drawn;

    // A chip can be 200 px wide, so stopping *after* the lane is not enough: the
    // strip is clipped as well, so nothing a tab draws can reach the controls.
    // Popped again before this function returns, because the tooltips are painted
    // later (from `ui::draw`) and would be clipped away with the strip.
    p.r.push_clip(Rect::new(chips_left, rect.y, (chips_right - chips_left).max(0.0), rect.h));
    for (i, tab) in app.tabs.iter().enumerate() {
        let title = tab.title();
        let tw = widths[i];
        // Chips left of the scroll offset are off the strip, not squeezed into it.
        let r = Rect::new(
            chips_left + (widths[..i].iter().sum::<f32>() + gap * i as f32) - sx,
            rect.y + 6.0,
            tw,
            rect.h - 12.0,
        );
        // Every chip is recorded in the tab's *own* slot, on screen or not: `rects[i]`
        // has to mean tab `i`, or "scroll the active tab into view" and a drag would
        // both be reading a shifted list as soon as the strip is scrolled.
        tab_rects.push(r);
        if r.right() < chips_left || r.x > chips_right {
            continue;
        }
        let active = i == app.active;
        let dragging = app.tab_drag.map(|d| d.moved && d.index == i).unwrap_or(false);
        // Which tab is hot. A tab under a chevron is not hot even though the pointer
        // is over its rect, and while a tab is being dragged only the one the drag is
        // about stays lit: the chips are sliding under the pointer, and lighting up
        // whatever happens to be under it makes the strip flicker through the drag.
        let hovered = match app.tab_drag {
            Some(d) if d.moved => d.hovered == Some(i),
            _ => p.hovered(r) && !over_chevron(rect, strip_right, sx, app.tab_scroll_max, p.mouse),
        };
        if active || dragging {
            if dragging {
                p.round(r.translate(0.0, 1.5), 7.0, Color::rgba(0.0, 0.0, 0.0, 0.25));
            }
            p.round(r, 7.0, p.theme.bg_active);
        } else if hovered {
            p.round(r, 7.0, p.theme.bg_hover.with_alpha(0.5));
        }
        let gone = tab.doc.file_gone();
        let fg = tab_ink(&p.theme, active, gone);
        // Close button: only shown for the active or hovered tab, which is what
        // keeps the strip quiet; the hit area is pushed *after* the tab so that
        // it wins the hit test (the hit map keeps the last match).
        let show_close = active || hovered;
        // The name is ellipsised into the room that is actually left, and the
        // "this tab has unsaved changes" dot is a *fixed* piece of the chip rather
        // than a character appended to the name. Appending it meant a long name
        // drew its dot past the end of the chip, on top of the next tab; the tab
        // has one right-hand lane (the close button, or the dot when there is no
        // button) and the name never grows into it.
        let dirty = tab.doc.dirty || tab.doc.file_gone();
        let lane = tab_lane(show_close, dirty);
        let name_w = (r.w - TAB_NAME_INSET - lane).max(12.0);
        let shown = ellipsize(p, &title, Face::Regular, size, name_w);
        p.label(Rect::new(r.x, r.y, name_w, r.h), TAB_NAME_INSET, &shown, Face::Regular, size, fg);
        // A deleted file gets a dot even when the buffer is clean: what is on
        // screen is no longer what is on disk, in the strongest possible way.
        if dirty || gone {
            let (cx, cy) = tab_dot_center(r, show_close);
            p.round(Rect::new(cx - TAB_DOT * 0.5, cy - TAB_DOT * 0.5, TAB_DOT, TAB_DOT), TAB_DOT * 0.5, fg.with_alpha(0.85));
        }

        p.hit(r, Hit::Tab(i));
        if show_close {
            let cr = chip_close(p, r);
            p.hit(cr.inset_xy(-3.0, -2.0), Hit::TabClose(i));
        }
    }
    p.r.pop_clip();
    // Published so the input layer can tell which tab a dragged tab is over.
    app.tab_rects = tab_rects;

    // new tab button - at the end of the *content*, so it scrolls with the chips
    // instead of sitting under the controls.
    let plus_x = chips_left + widths.iter().sum::<f32>() + gap * widths.len() as f32 - sx;
    if plus_x + 22.0 > chips_left && plus_x <= chips_right {
        let nr = Rect::new(plus_x + 2.0, rect.y + 8.0, 22.0, rect.h - 16.0);
        if p.hovered(nr) {
            p.round(nr, 5.0, p.theme.bg_hover);
        }
        let b = p.baseline_in(nr, size + 4.0);
        p.text(nr.x + 7.0, b, "+", Face::Regular, size + 4.0, p.theme.text_dim);
        p.hit(nr, Hit::NewTab);
    }

    // A hairline where the tabs end and the view controls begin, so the two lanes
    // read as two lanes.
    p.rect(
        Rect::new(lane_left - 4.0, rect.y + 9.0, 1.0, rect.h - 18.0),
        p.theme.border,
    );

    // ---- scroll affordances ----
    // Two small chevrons at the ends that have more tabs behind them. **No
    // scrollbar**: a bar under the chips was noise in a strip that is only one row
    // tall, and the chevrons say the same thing without a second horizontal line in
    // the chrome. Each chevron points the way it scrolls - the `chevron` glyph is a
    // *down* arrow, and using it for "right" is what made both buttons look like
    // "scroll to the bottom".
    if app.tab_scroll_max > 0.5 {
        // One button per side, in that side's lane, and only while that side has
        // something to show. The lane itself stays empty (and unclickable) when the
        // chevron is away, so nothing sits on a chip either way.
        if sx > 0.5 {
            let l = Rect::new(rect.x + 6.0, rect.y + 6.0, 16.0, rect.h - 12.0);
            if p.hovered(l) {
                p.round(l, 5.0, p.theme.bg_active);
            }
            p.icon(
                "chevron-left",
                Rect::new(l.x + 4.0, l.y + (l.h - 14.0) * 0.5, 8.0, 14.0),
                if p.hovered(l) { p.theme.text } else { p.theme.text_dim },
            );
            p.hit(l, Hit::TabScrollLeft);
        }
        if sx < app.tab_scroll_max - 0.5 {
            let rr = Rect::new(chips_right + 2.0, rect.y + 6.0, 16.0, rect.h - 12.0);
            if p.hovered(rr) {
                p.round(rr, 5.0, p.theme.bg_active);
            }
            p.icon(
                "chevron-right",
                Rect::new(rr.x + 4.0, rr.y + (rr.h - 14.0) * 0.5, 8.0, 14.0),
                if p.hovered(rr) { p.theme.text } else { p.theme.text_dim },
            );
            p.hit(rr, Hit::TabScrollRight);
        }
    }

    let bh = rect.h - 14.0;
    let by = rect.y + 7.0;
    // Laid out **left to right from the measured edge of the lane**, so where the
    // controls sit and where the tabs were told to stop are the same number by
    // construction (`view_lane_left`). Building the lane right to left and guessing
    // its width separately is what let the two drift apart.
    let mut rx = lane_left;

    // Find
    let find_r = Rect::new(rx, by, FIND_W, bh);
    if p.hovered(find_r) {
        p.round(find_r, 6.0, p.theme.bg_hover);
    }
    p.label_center(find_r, "Find", Face::Regular, p.theme.ui_size(), p.theme.text_dim);
    p.hit(find_r, Hit::Toolbar(ToolbarAction::FindReplace));
    rx = find_r.right() + 6.0;

    // Live -> Live+ -> Raw
    let mode_r = Rect::new(rx, by, MODE_W, bh);
    let mode = app.mode;
    p.button(mode_r, mode.label(), mode != ViewMode::Live, Hit::Toolbar(ToolbarAction::ToggleRaw));
    rx = mode_r.right() + CHIP_GAP;

    let mut chipped = |p: &mut Painter, active: bool, hit: Hit, hint: &str, draw: &dyn Fn(&mut Painter, Rect, Color)| {
        let r = Rect::new(rx, by, CHIP, bh);
        rx += CHIP + CHIP_GAP;
        let hovered = p.hovered(r);
        if active || hovered {
            p.round(r, 6.0, if active { p.theme.bg_active } else { p.theme.bg_hover });
        }
        // The glyph fills most of the chip: a 14 px icon in a 28 px button was
        // hard to recognise.
        draw(p, r.inset_xy(5.0, 2.5), if active { p.theme.text } else { p.theme.text_dim });
        p.hit(r, hit);
        if hovered {
            p.tooltip(r, hint);
        }
    };
    chipped(p, false, Hit::Toolbar(ToolbarAction::ReloadFile), "Reload from disk", &|p, r, c| { p.icon("reload", r, c); });
    chipped(p, app.settings.sidebar_open, Hit::Toolbar(ToolbarAction::ToggleSidebar), "Workspace panel  (Ctrl+Shift+E)", &|p, r, c| { p.icon("sidebar", r, c); });
    chipped(p, app.settings.outline_open, Hit::Toolbar(ToolbarAction::ToggleOutline), "Outline  (Ctrl+Shift+O)", &|p, r, c| { p.icon("outline", r, c); });
    chipped(p, app.settings.wrap, Hit::Toolbar(ToolbarAction::ToggleWrap), "Soft wrap  (Alt+W)", &|p, r, c| { p.icon("wrap", r, c); });
    chipped(p, app.settings.autoclose, Hit::Toolbar(ToolbarAction::ToggleAutoclose), "Auto-close brackets  (Alt+B)", &|p, r, c| { p.icon("brackets", r, c); });
    chipped(p, app.settings.minimap_open, Hit::Toolbar(ToolbarAction::ToggleMinimap), "Minimap  (Alt+M)", &|p, r, c| { p.icon("minimap", r, c); });
    chipped(p, app.settings.theme.is_light(), Hit::Toolbar(ToolbarAction::ToggleTheme), "Theme  (Alt+T)", &|p, r, c| { p.icon("theme", r, c); });
}

// ---------------------------------------------------------------- status bar

pub fn draw_status(app: &App, p: &mut Painter, rect: Rect) {
    p.rect(rect, p.theme.bg_panel);
    p.hline(rect.x, rect.y, rect.w, p.theme.border);
    let tab = app.tab();
    let size = p.theme.ui_small();
    // Only the file path is shown here: a placeholder for an unnamed buffer, the
    // theme name and the portability tag were pure noise.
    if let Some(path) = &tab.doc.path {
        let full = path.display().to_string();
        let max_w = (rect.w * 0.55).max(120.0);
        let shown = ellipsize(p, &full, Face::Regular, size, max_w);
        let w = (p.width(&shown, Face::Regular, size) + 12.0).min(max_w + 12.0);
        let chip = Rect::new(rect.x + 4.0, rect.y + 2.0, w, rect.h - 4.0);
        let hot = p.hovered(chip);
        if hot {
            p.round(chip, 4.0, p.theme.bg_hover);
        }
        p.label(chip, 6.0, &shown, Face::Regular, size, if hot { p.theme.text } else { p.theme.text_dim });
        // Clicking the path puts it on the clipboard - the shortest way to get a
        // file's location into another window. Hovering says nothing: the path is
        // already on screen here, so a hover popup repeating it was the same
        // information twice. The only feedback is the notice after the click.
        p.hit(chip, Hit::StatusPath);
        if app.status == "Copied!" && app.status_at.elapsed().as_millis() < 1400 {
            let tw = (p.width("Copied!", Face::Regular, size) + 18.0).max(56.0);
            let toast = Rect::new(chip.x, chip.y - 26.0, tw, 22.0);
            p.round(toast, 5.0, p.theme.bg_elevated);
            p.stroke(toast, 5.0, 1.0, p.theme.border_strong);
            p.label(toast, 9.0, "Copied!", Face::Regular, size, p.theme.text);
        }
    }

    // cursor position
    let (line, col) = tab.doc.line_and_col(tab.caret);
    let pos = format!("Ln {}, Col {}", line + 1, col + 1);
    let words = format!("{} words", app.word_count());
    let right = format!("{}    {}", pos, words);
    p.label_right(rect, 10.0, &right, Face::Regular, size, p.theme.text_faint);

}

// ---------------------------------------------------------------- sidebar

pub fn draw_sidebar(app: &mut App, p: &mut Painter, rect: Rect) {
    // Same panel colour as the outline: both side columns belong to the chrome,
    // the sheet between them belongs to the document.
    p.rect(rect, p.theme.bg_panel);
    p.vline(rect.right() - 1.0, rect.y, rect.h, p.theme.border);

    let head = Rect::new(rect.x + 4.0, rect.y + 8.0, rect.w - 8.0, 24.0);
    p.label(head, 12.0, "WORKSPACE", Face::Bold, p.theme.ui_small(), p.theme.text_faint);
    // Open another folder as the workspace without going through the menus.
    let open_chip = Rect::new(head.right() - 30.0, head.y + 2.0, 22.0, 20.0);
    if p.hovered(open_chip) {
        p.round(open_chip, 5.0, p.theme.bg_hover);
    }
    p.icon(
        "folder-open",
        Rect::new(open_chip.x + 4.0, open_chip.y + 3.0, 14.0, 14.0),
        if p.hovered(open_chip) { p.theme.text } else { p.theme.text_dim },
    );
    p.hit(open_chip, Hit::SidebarOpenFolder);
    if p.hovered(open_chip) {
        p.tooltip(open_chip, "Open folder…");
    }

    // ---- workspace tabs ----
    // One chip per open workspace, in strip order, the active one filled in.
    // Chips can be dragged to reorder them and each has its own close button; the
    // header's folder button is the only "open another folder" control (a "+"
    // chip next to the tabs was a second way to do the same thing).
    {
        let tabs_y = head.bottom() + 2.0;
        let left = rect.x + 6.0;
        let right = rect.right() - 6.0;
        // Chips share the row: they shrink together so every open workspace keeps
        // a visible tab instead of the last ones silently falling off the edge.
        let n = app.workspaces.len().max(1) as f32;
        let avail = (right - left).max(60.0);
        let gap = if n > 4.0 { 2.0 } else { 4.0 };
        let tw = ((avail - gap * (n - 1.0)) / n).clamp(30.0, 130.0);
        let mut x = left;
        let mut rects: Vec<Rect> = Vec::with_capacity(app.workspaces.len());
        let dragging = app.ws_drag.filter(|d| d.moved);
        for (i, ws) in app.workspaces.iter().enumerate() {
            let name = ws
                .root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| ws.root.display().to_string());
            if x + tw > right + 0.5 && i > 0 {
                break;
            }
            let r = Rect::new(x, tabs_y, tw, 22.0);
            rects.push(r);
            let active = i == app.ws_active;
            let hot = p.hovered(r);
            let lifted = dragging.map(|d| d.index == i).unwrap_or(false);
            if lifted {
                p.round(r.translate(0.0, 1.5), 5.0, Color::rgba(0.0, 0.0, 0.0, 0.25));
                p.round(r, 5.0, p.theme.bg_active);
            } else if active {
                p.round(r, 5.0, p.theme.bg_active);
            } else if hot {
                p.round(r, 5.0, p.theme.bg_hover);
            }
            // Room for the close button, which is only drawn when it is useful.
            let show_close = (active || hot) && app.workspaces.len() > 1;
            let text_w = r.w - 12.0 - if show_close { 22.0 } else { 0.0 };
            let label = ellipsize(p, &name, Face::Regular, p.theme.ui_small(), text_w.max(12.0));
            p.label(
                r,
                6.0,
                &label,
                if active { Face::Bold } else { Face::Regular },
                p.theme.ui_small(),
                if active { p.theme.text } else { p.theme.text_dim },
            );
            // One hit for the whole chip…
            p.hit(r, Hit::WorkspaceTab(i));
            if show_close {
                // …and one for the close button, pushed afterwards so it wins. It is
                // the same button a document tab draws, in the same place.
                let cr = chip_close(p, r);
                p.hit(cr.inset_xy(-3.0, -2.0), Hit::WorkspaceTabClose(i));
            }
            if hot && !show_close {
                p.tooltip(r, &ws.root.display().to_string());
            }
            x += tw + gap;
        }
        // Published for the drag: the input layer never re-derives chip widths.
        app.ws_tab_rects = rects;
    }

    // The panel's address: the folder being browsed, shown the way a file
    // manager shows it (shortened from the *left*, so the leaf stays readable).
    // Clicking the row steps one level up; no dialog in between.
    let root_path = app.ws.root.display().to_string();
    let rr = Rect::new(rect.x + 6.0, head.bottom() + 28.0, rect.w - 12.0, 26.0);
    app.sidebar_root_row = rr;
    let root_hot = p.hovered(rr);
    // Nothing special is drawn here for a drag: the whole panel takes a drop, so a
    // box around the address row said "this is the target" when the target was in
    // fact the panel. Only a folder row lights up, because only a folder row means
    // something more specific than "somewhere in the workspace".
    if root_hot {
        p.round(rr, 6.0, p.theme.bg_hover);
    }
    p.icon(
        if root_hot { "chevron-up" } else { "folder-open" },
        Rect::new(rr.x + 6.0, rr.y + 6.0, 14.0, 14.0),
        if root_hot { p.theme.text } else { p.theme.text_dim },
    );
    let root_label = ellipsize_left(p, &root_path, Face::Bold, p.theme.ui_size(), rr.w - 36.0);
    p.label(Rect::new(rr.x + 26.0, rr.y, rr.w - 30.0, rr.h), 0.0, &root_label, Face::Bold, p.theme.ui_size(), p.theme.text);
    p.hit(rr, Hit::SidebarUp);
    if root_hot {
        p.tooltip(rr, &format!("{root_path}  ·  click to go up"));
    }

    let mut y = rr.bottom() + 6.0;
    let row_h = 23.0;
    p.r.push_clip(Rect::new(rect.x, y, rect.w, rect.bottom() - y - 6.0));
    let active_path = app.tab().doc.path.clone();
    // The folder a file being dragged out of this tree would land in, so the row
    // says "drop it here" before the button comes up.
    let drop_target = app.tree_drag.as_ref().and_then(|d| {
        if d.moved {
            d.over.clone()
        } else {
            None
        }
    });
    let mut rows: Vec<Rect> = Vec::with_capacity(app.ws.entries().len());
    for (i, entry) in app.ws.entries().iter().enumerate() {
        if y > rect.bottom() {
            break;
        }
        let indent = 8.0 + entry.depth as f32 * 14.0;
        let r = Rect::new(rect.x + 6.0, y, rect.w - 12.0, row_h);
        rows.push(r);
        let is_active = active_path.as_deref() == Some(entry.path.as_path());
        // Only a folder row lights up: it is the one row that means something more
        // specific than the panel's default target.
        let is_drop = entry.is_dir && drop_target.as_deref() == Some(entry.path.as_path());
        // The file being carried is dimmed where it sits, the folder that would take
        // it is outlined: the drag says what it will do before it does it.
        let carried = app
            .tree_drag
            .as_ref()
            .filter(|d| d.moved)
            .map(|d| d.path.as_path() == entry.path.as_path())
            .unwrap_or(false);
        if is_drop {
            p.round(r, 6.0, p.theme.bg_active);
            p.stroke(r, 6.0, 1.0, p.theme.text);
        } else if is_active {
            p.round(r, 6.0, p.theme.bg_hover);
            p.round(Rect::new(r.x, r.y + 4.0, 2.0, r.h - 8.0), 1.0, p.theme.text);
        } else if p.hovered(r) && !carried {
            p.round(r, 6.0, p.theme.bg_hover.with_alpha(0.55));
        }
        if entry.is_dir {
            p.icon(if entry.expanded { "chevron-up" } else { "chevron" }, Rect::new(r.x + indent, r.y + 6.0, 12.0, 12.0), p.theme.text_faint);
            p.icon(if entry.expanded { "folder-open" } else { "folder" }, Rect::new(r.x + indent + 14.0, r.y + 6.0, 13.0, 13.0), p.theme.text_dim);
        } else {
            p.icon("file", Rect::new(r.x + indent + 14.0, r.y + 6.0, 12.0, 13.0), p.theme.text_faint);
        }
        let color = if carried {
            p.theme.text_faint
        } else if is_active || entry.is_dir {
            p.theme.text
        } else {
            p.theme.text_dim
        };
        let avail = (r.right() - (r.x + indent + 32.0) - 6.0).max(20.0);
        let name = ellipsize(p, &entry.name, Face::Regular, p.theme.ui_size(), avail);
        let b = p.baseline_in(r, p.theme.ui_size());
        p.text(r.x + indent + 32.0, b, &name, Face::Regular, p.theme.ui_size(), color);
        p.hit(r, Hit::SidebarEntry(i));
        y += row_h;
    }
    if app.ws.truncated {
        p.text(rect.x + 12.0, y + 14.0, "(listing truncated)", Face::Regular, p.theme.ui_small(), p.theme.text_faint);
    }
    p.r.pop_clip();
    // The rows are published for the drag (a drop reads them, it does not re-derive
    // the layout), and the empty part of the panel below them is a target of its own:
    // a right-click there makes a new file in the folder the panel is showing.
    app.sidebar_rows = rows;
    let blank_y = y + if app.ws.truncated { 26.0 } else { 4.0 };
    let blank = Rect::new(rect.x, blank_y, rect.w, (rect.bottom() - blank_y).max(0.0));
    app.sidebar_blank = if blank.h > 0.0 { blank } else { Rect::ZERO };
    if blank.h > 0.0 {
        // No box and no caption here: the empty part of the panel is simply part of
        // the panel, and dropping on it means the workspace root like everything
        // else that is not a folder row.
        p.hit(blank, Hit::SidebarBlank);
    }
}

// ---------------------------------------------------------------- outline

/// Height of the outline panel's title row.
pub const OUTLINE_HEAD_H: f32 = 28.0;
/// Height of one outline row (kept public: a drag maps y back to rows with it).
pub const OUTLINE_ROW_H: f32 = 22.0;

pub fn draw_outline(app: &App, p: &mut Painter, rect: Rect) {
    p.rect(rect, p.theme.bg_panel);
    p.vline(rect.x, rect.y, rect.h, p.theme.border);
    let head = Rect::new(rect.x, rect.y, rect.w, OUTLINE_HEAD_H);
    p.label(head, 14.0, "OUTLINE", Face::Bold, p.theme.ui_small(), p.theme.text_faint);

    let layout = &app.tab().layout;
    let active = layout.active_heading(app.tab().caret);
    let mut y = head.bottom() + 2.0;
    let row_h = OUTLINE_ROW_H;
    let sections = app.outline_sections();
    let dragging = app.drag.filter(|d| d.kind == crate::app::DragKind::Outline);
    p.r.push_clip(Rect::new(rect.x, y, rect.w, rect.bottom() - y));
    if layout.headings.is_empty() {
        p.label(Rect::new(rect.x + 14.0, y, rect.w, 22.0), 0.0, "No headings", Face::Regular, p.theme.ui_small(), p.theme.text_faint);
    }
    for (i, (_, level, text)) in layout.headings.iter().enumerate() {
        if y > rect.bottom() {
            break;
        }
        let row = Rect::new(rect.x + 4.0, y, rect.w - 8.0, row_h);
        let indent = 14.0 + (*level as f32 - 1.0) * 10.0;
        let r = Rect::new(rect.x + 4.0, y, rect.w - 8.0, row_h);
        if dragging.map(|d| d.from) == Some(i) {
            p.round(r, 4.0, p.theme.bg_active);
        } else if active == Some(i) {
            p.round(r, 4.0, p.theme.accent.with_alpha(0.16));
        } else if p.hovered(r) {
            p.round(r, 4.0, p.theme.bg_hover);
        }
        // A drop line where the section would land, so a drag is predictable.
        if let Some(d) = dragging {
            if d.moved && d.to == i && d.from != i {
                let ty = if d.to > d.from { row.bottom() } else { row.y };
                p.rect(Rect::new(rect.x + 8.0, ty - 1.0, rect.w - 16.0, 2.0), p.theme.accent);
            }
        }
        let color = if active == Some(i) { p.theme.accent } else { p.theme.text_dim };
        let face = if *level <= 2 { Face::Bold } else { Face::Regular };
        // The grip on the right belongs to the row too: long headings used to run
        // straight under it. The label is measured against the room that is left
        // once the grip's column has been taken out.
        let avail = (r.right() - 18.0 - (rect.x + indent)).max(20.0);
        let name = ellipsize(p, text, face, p.theme.ui_small(), avail);
        let b = p.baseline_in(r, p.theme.ui_small());
        p.text(rect.x + indent, b, &name, face, p.theme.ui_small(), color);
        p.hit(r, Hit::OutlineItem(i));
        // The handle: drag it to move the whole section (its heading plus
        // everything under it).
        let grip = Rect::new(rect.right() - 16.0, y + (row_h - 14.0) * 0.5, 12.0, 14.0);
        let hot = p.hovered(grip) || dragging.map(|d| d.from) == Some(i) && dragging.map(|d| d.moved).unwrap_or(false);
        if hot {
            p.round(grip, 3.0, p.theme.bg_elevated);
        }
        let gc = if hot { p.theme.text } else { p.theme.text_faint.with_alpha(0.7) };
        p.icon("grip", grip, gc);
        p.hit(grip, Hit::OutlineGrip(i));
        let _ = &sections;
        y += row_h;
    }
    p.r.pop_clip();
}

// ---------------------------------------------------------------- editor

/// First line of the card run `line` belongs to.
///
/// Code blocks, tables and quotes are painted as one card, so the painter has to
/// start at the run's *first* line even when that line has been scrolled out of
/// sight. Starting at the first visible line instead made a long code card shrink
/// to the width of whatever happened to be on screen, and made a long table lose
/// its grid the moment its header row scrolled away.
pub fn card_run_start(layout: &Layout, line: usize) -> usize {
    if layout.lines.is_empty() {
        return 0;
    }
    let mut i = line.min(layout.lines.len() - 1);
    while i > 0
        && !layout.lines[i].folded
        && !layout.lines[i - 1].folded
        && matches!(
            layout.lines[i - 1].block(),
            Block::Code | Block::Fence | Block::Table | Block::Quote
        )
        && layout.lines[i - 1].block() == layout.lines[i].block()
    {
        i -= 1;
    }
    i
}


pub fn flush_tooltips(p: &mut Painter) {
    let queued = std::mem::take(&mut p.tooltips);
    for (anchor, text) in queued {
        let size = p.theme.ui_small();
        let w = p.width(&text, Face::Regular, size) + 16.0;
        let h = 20.0;
        let (vw, _) = p.r.logical_size();
        let x = (anchor.x + anchor.w * 0.5 - w * 0.5).clamp(4.0, (vw - w - 4.0).max(4.0));
        let mut y = anchor.bottom() + 5.0;
        // Near the bottom edge the hint flips above the button instead of
        // falling off the window.
        let (_, vh) = p.r.logical_size();
        if y + h > vh - 4.0 {
            y = anchor.y - h - 5.0;
        }
        let r = Rect::new(x, y, w, h);
        p.round(r, 5.0, p.theme.bg_elevated);
        p.stroke(r, 5.0, 1.0, p.theme.border_strong);
        let b = p.baseline_in(r, size);
        p.text(r.x + 8.0, b, &text, Face::Regular, size, p.theme.text_dim);
    }
}

// ---------------------------------------------------------------- find bar
