//! Dialogs and the find bar: the floating panels that are not the document.

use super::*;

pub fn draw_find_bar(app: &App, p: &mut Painter, editor: Rect) {
    let tab = app.tab();
    let with_replace = tab.find.show_replace;
    let w = 460.0_f32.min(editor.w - 24.0);
    let h = if with_replace { 76.0 } else { 40.0 };
    // Sit under the floating toolbar instead of colliding with it.
    let bar = Rect::new(
        editor.right() - w - 16.0,
        editor.y + 12.0 + Theme::FLOATING_TOOLBAR_H + 8.0,
        w,
        h,
    );
    p.panel(bar, 8.0, p.theme.bg_elevated, 1.0, p.theme.border_strong);

    let row1 = Rect::new(bar.x + 8.0, bar.y + 7.0, bar.w - 16.0, 26.0);
    // query field
    let qf = Rect::new(row1.x, row1.y, row1.w - 214.0, row1.h);
    let focused = app.focus == Focus::FindQuery;
    let sel = tab.find.selection(crate::search::FindFocus::Query);
    let caret = tab.find.caret(crate::search::FindFocus::Query).0;
    p.field(
        qf,
        tab.find.field_str(crate::search::FindFocus::Query),
        "Find",
        sel,
        Some(caret),
        focused,
        Hit::FindQuery,
    );

    // count
    let count_text = if tab.find.query.is_empty() {
        String::new()
    } else if tab.find.matches.is_empty() {
        "0".to_string()
    } else {
        format!(
            "{} / {}",
            tab.find.current.map(|c| c + 1).unwrap_or(1),
            tab.find.matches.len()
        )
    };
    let cr = Rect::new(qf.right() + 6.0, row1.y, 54.0, row1.h);
    p.label_center(cr, &count_text, Face::Regular, p.theme.ui_small(), p.theme.text_faint);

    let mut bx = cr.right() + 4.0;
    for (label, hit, active) in [
        ("Aa", Hit::FindToggleCase, tab.find.case_sensitive),
        ("W", Hit::FindToggleWord, tab.find.whole_word),
    ] {
        let r = Rect::new(bx, row1.y, 26.0, row1.h);
        p.button(r, label, active, hit);
        bx += 30.0;
    }
    let prev = Rect::new(bx, row1.y, 28.0, row1.h);
    p.button(prev, "up", false, Hit::FindPrev);
    bx += 32.0;
    let next = Rect::new(bx, row1.y, 28.0, row1.h);
    p.button(next, "dn", false, Hit::FindNext);
    let _ = next.right();

    // close
    let close = Rect::new(bar.right() - 26.0, bar.y + 9.0, 18.0, 18.0);
    if p.hovered(close) {
        p.round(close, 4.0, p.theme.bg_hover);
    }
    p.icon("close", close.inset(4.0), p.theme.text_dim);
    p.hit(close, Hit::FindClose);

    if with_replace {
        let row2 = Rect::new(bar.x + 8.0, row1.bottom() + 6.0, bar.w - 16.0, 26.0);
        let rf = Rect::new(row2.x, row2.y, row2.w - 180.0, row2.h);
        let focused = app.focus == Focus::FindReplace;
        let sel = tab.find.selection(crate::search::FindFocus::Replace);
        let caret = tab.find.caret(crate::search::FindFocus::Replace).0;
        p.field(
            rf,
            tab.find.field_str(crate::search::FindFocus::Replace),
            "Replace with",
            sel,
            Some(caret),
            focused,
            Hit::FindReplace,
        );

        let one = Rect::new(rf.right() + 6.0, row2.y, 76.0, row2.h);
        p.button(one, "Replace", false, Hit::FindReplaceOne);
        let all = Rect::new(one.right() + 4.0, row2.y, 84.0, row2.h);
        p.button(all, "Replace all", false, Hit::FindReplaceAll);
    }
}

// ---------------------------------------------------------------- dialogs

pub fn draw_dialog(app: &App, p: &mut Painter, dialog: &Dialog, w: f32, h: f32) {
    // scrim
    p.rect(Rect::new(0.0, 0.0, w, h), Color::rgba(0.0, 0.0, 0.0, 0.45));
    match dialog {
        Dialog::Confirm { title, message, danger, .. } => {
            // One line per line of text: a question about a deleted file needs a
            // second sentence, and a box sized for one line clipped it.
            let lines: Vec<&str> = message.split('\n').collect();
            let line_h = p.theme.ui_size() + 6.0;
            let body_h = lines.len() as f32 * line_h;
            let box_h = (86.0 + body_h + 46.0).max(150.0);
            let d = Rect::new((w - 460.0) * 0.5, (h - box_h) * 0.5, 460.0, box_h);
            p.panel(d, 10.0, p.theme.bg_elevated, 1.0, p.theme.border_strong);
            // "This tab is the only copy left" is the one place the monochrome
            // chrome turns red.
            let head = if *danger { p.theme.danger } else { p.theme.text };
            p.label(Rect::new(d.x, d.y + 20.0, d.w, 24.0), 24.0, title, Face::Bold, 15.0, head);
            for (i, line) in lines.iter().enumerate() {
                p.label(
                    Rect::new(d.x, d.y + 54.0 + i as f32 * line_h, d.w, line_h),
                    24.0,
                    line,
                    Face::Regular,
                    p.theme.ui_size(),
                    p.theme.text_dim,
                );
            }
            let b1 = Rect::new(d.right() - 200.0, d.bottom() - 52.0, 84.0, 30.0);
            p.button(b1, "Discard", false, Hit::DialogConfirm);
            let b2 = Rect::new(d.right() - 108.0, d.bottom() - 52.0, 84.0, 30.0);
            p.button(b2, "Cancel", true, Hit::DialogCancel);
        }
        Dialog::Picker(picker) => draw_picker(app, p, picker, w, h),
        Dialog::Colour(cp) => draw_colour_picker(p, cp, w, h),
        Dialog::External(ec) => draw_external_dialog(p, ec, w, h),
        Dialog::About(about) => draw_about(p, about, w, h),
    }
}

/// The custom colour picker, in **two columns**: text colour on the left,
/// background colour on the right.
///
/// Each column owns a preview, six presets, a hue strip, a saturation/value
/// square, a hex field and a Clear switch, so a colour and a highlight can be set
/// in one visit. The squares are painted as grids of solid quads (this renderer
/// has no gradients) — about 800 of them, which is nothing next to a page of
/// text.
pub fn draw_colour_picker(p: &mut Painter, cp: &crate::app::ColourPicker, w: f32, h: f32) {
    use crate::app::{hsv_to_rgb, COLOUR_PRESETS};
    // Two columns of controls, and nothing else: the dialog hugs them (a fixed
    // tall box just left a band of empty panel under the hex fields).
    let d = Rect::new((w - 580.0) * 0.5, (h - 360.0) * 0.5, 580.0, 360.0);
    p.panel(d, 10.0, p.theme.bg_elevated, 1.0, p.theme.border_strong);
    p.label(
        Rect::new(d.x, d.y + 14.0, d.w, 22.0),
        20.0,
        "Text and background colour",
        Face::Bold,
        15.0,
        p.theme.text,
    );

    let col_w = (d.w - 40.0 - 18.0) * 0.5;
    for (bg, x) in [(false, d.x + 20.0), (true, d.x + 20.0 + col_w + 18.0)] {
        let slot = cp.slot(bg);
        let active = cp.active_bg == bg;
        let head = Rect::new(x, d.y + 44.0, col_w, 18.0);
        p.label(head, 0.0, if bg { "BACKGROUND" } else { "TEXT" }, Face::Bold, p.theme.ui_small(), p.theme.text_faint);
        if active {
            // A hairline under the active column's title: the hex field and the
            // keyboard follow it.
            p.rect(Rect::new(x, head.bottom() + 1.0, col_w * 0.5, 1.0), p.theme.accent);
        }

        // ---- preview + hex ----
        let prev = Rect::new(x, head.bottom() + 8.0, col_w, 26.0);

        if slot.on {
            p.round(prev, 5.0, crate::layout::rgb_color(slot.rgb()));
        } else {
            // "No colour": a checkerboard-free hatch reads as empty without
            // pulling a colour into a monochrome theme.
            p.round(prev, 5.0, p.theme.bg);
            for k in 0..8 {
                let sx = prev.x + 6.0 + k as f32 * 22.0;
                p.rect(Rect::new(sx, prev.y + 13.0, 12.0, 1.0), p.theme.border_strong);
            }
        }
        p.stroke(prev, 5.0, 1.0, if active { p.theme.accent } else { p.theme.border_strong });
        p.hit(prev, Hit::ColourClear(bg));
        let size = p.theme.ui_small();
        let b = p.baseline_in(prev, size);
        let ink = if slot.on && slot.val > 0.6 && slot.sat < 0.75 { p.theme.bg } else { p.theme.text };
        p.text(prev.x + 8.0, b, &format!("#{:06x}", slot.rgb()), Face::Mono, size, ink);
        let state = if slot.on { "on" } else { "off" };
        let state_w = p.width(state, Face::Bold, size);
        p.text(prev.right() - state_w - 8.0, b, state, Face::Bold, size, ink.with_alpha(0.7));

        // ---- presets ----
        let sw = (col_w - 5.0 * 6.0) / 6.0;
        for (i, value) in COLOUR_PRESETS.iter().enumerate() {
            let r = Rect::new(x + i as f32 * (sw + 6.0), prev.bottom() + 8.0, sw, 16.0);
            p.round(r, 4.0, crate::layout::rgb_color(*value));
            if slot.rgb() == *value && slot.on {
                p.stroke(r, 4.0, 2.0, p.theme.text);
            }
            p.hit(r, Hit::ColourPreset(bg, i));
        }

        // ---- saturation / value square + hue strip ----
        let sq_w = col_w - 30.0;
        let sq = Rect::new(x, prev.bottom() + 32.0, sq_w, 132.0);
        p.round(sq, 6.0, p.theme.bg);
        let cols = 24u32;
        let rows = 14u32;
        for j in 0..rows {
            for i in 0..cols {
                let sat = i as f32 / (cols - 1) as f32;
                let val = 1.0 - j as f32 / (rows - 1) as f32;
                let cw = sq.w / cols as f32;
                let chh = sq.h / rows as f32;
                p.rect(
                    Rect::new(sq.x + i as f32 * cw, sq.y + j as f32 * chh, cw + 0.6, chh + 0.6),
                    crate::layout::rgb_color(hsv_to_rgb(slot.hue, sat, val)),
                );
            }
        }
        p.stroke(sq, 6.0, 1.0, p.theme.border_strong);
        p.hit(sq, Hit::ColourSv(bg));
        let mx = sq.x + slot.sat.clamp(0.0, 1.0) * sq.w;
        let my = sq.y + (1.0 - slot.val.clamp(0.0, 1.0)) * sq.h;
        let ring = Rect::new(mx - 5.0, my - 5.0, 10.0, 10.0);
        p.stroke(ring, 5.0, 1.6, if slot.val > 0.55 { p.theme.bg } else { p.theme.text });
        p.stroke(ring.inset_xy(1.2, 1.2), 4.0, 1.0, if slot.val > 0.55 { p.theme.text } else { p.theme.bg });

        let hue = Rect::new(sq.right() + 8.0, sq.y, 20.0, sq.h);
        let steps = 32u32;
        for i in 0..steps {
            let h0 = i as f32 / steps as f32 * 360.0;
            let hh = hue.h / steps as f32;
            p.rect(
                Rect::new(hue.x, hue.y + i as f32 * hh, hue.w, hh + 0.6),
                crate::layout::rgb_color(hsv_to_rgb(h0, 1.0, 1.0)),
            );
        }
        p.stroke(hue, 5.0, 1.0, p.theme.border_strong);
        p.hit(hue, Hit::ColourHue(bg));
        let hy = hue.y + (slot.hue / 360.0) * hue.h;
        p.rect(Rect::new(hue.x - 3.0, hy - 1.0, hue.w + 6.0, 2.0), p.theme.text);

        // ---- hex field ----
        let field = Rect::new(x, sq.bottom() + 12.0, col_w * 0.55, 26.0);
        // The hex box is a text field like any other: its own caret and selection,
        // drawn from its own string.
        let hex_focused = cp.active_bg == bg;
        p.field(
            field,
            &slot.hex.text,
            "rrggbb",
            slot.hex.selection(),
            Some(slot.hex.caret),
            hex_focused,
            Hit::ColourHex(bg),
        );
    }

    // ---- buttons ----
    let ok = Rect::new(d.right() - 20.0 - 96.0, d.bottom() - 46.0, 96.0, 30.0);
    p.button(ok, "Apply", true, Hit::ColourOk);
    let cancel = Rect::new(ok.x - 8.0 - 88.0, ok.y, 88.0, 30.0);
    p.button(cancel, "Cancel", false, Hit::ColourCancel);
    p.label(
        Rect::new(d.x + 20.0, d.bottom() - 42.0, d.w - 240.0, 24.0),
        0.0,
        "Click the swatch to switch a column between colour and none",
        Face::Regular,
        p.theme.ui_small(),
        p.theme.text_faint,
    );
}

/// The file / folder dialog.
///
/// Opening a folder gets **two columns**, the way an OS folder selector has them:
/// the filesystem tree on the left (expand a folder, click one to select it) and
/// that folder's contents on the right, with the address bar above it. Save-as
/// and attach keep one column, because what they need is a file *name*.
/// Rect of the folder dialog's left column (the tree).
///
/// Shared with the input layer, so "the wheel is over the tree" is decided by the
/// same rectangle that was painted.
pub fn picker_tree_rect(_kind: PickerKind, w: f32, h: f32) -> Rect {
    // Every picker is the same dialog: a tree on the left, a listing on the right,
    // an address bar over it. Attaching a file and saving one used to be a
    // single-column box from before the folder picker existed, so the one dialog a
    // writer uses most often was the one with the fewest ways through the
    // filesystem.
    let (dw, dh) = PICKER_SIZE;
    let d = Rect::new((w - dw) * 0.5, (h - dh) * 0.5, dw, dh);
    let tree_top = d.y + 52.0;
    let body_bottom = d.bottom() - 58.0;
    Rect::new(d.x + 20.0, tree_top, 280.0, body_bottom - tree_top)
}

/// The size of every picker dialog, in logical pixels.
pub const PICKER_SIZE: (f32, f32) = (900.0, 560.0);

/// Rect of the folder dialog's listing column.
pub fn picker_list_rect(kind: PickerKind, w: f32, h: f32) -> Rect {
    let (dw, dh) = PICKER_SIZE;
    let d = Rect::new((w - dw) * 0.5, (h - dh) * 0.5, dw, dh);
    let tree_w = 280.0;
    let list_x = d.x + 20.0 + tree_w + 16.0;
    let list_w = d.right() - 20.0 - list_x;
    let body_top = d.y + 82.0;
    // The save-as dialog carries a file-name box under the listing; the listing
    // must not run underneath it.
    let body_bottom = match kind {
        PickerKind::SaveAs => d.bottom() - 104.0,
        _ => d.bottom() - 58.0,
    };
    Rect::new(list_x, body_top, list_w, body_bottom - body_top)
}

pub fn draw_picker(app: &App, p: &mut Painter, picker: &Picker, w: f32, h: f32) {
    let two_columns = true;
    let (dw, dh) = PICKER_SIZE;
    let d = Rect::new((w - dw) * 0.5, (h - dh) * 0.5, dw, dh);
    p.panel(d, 10.0, p.theme.bg_elevated, 1.0, p.theme.border_strong);
    let title = match picker.kind {
        PickerKind::Attach => "Attach a file",
        PickerKind::OpenFile => "Open file",
        PickerKind::OpenFolder => "Open folder",
        PickerKind::SaveAs => "Save as",
    };
    p.label(Rect::new(d.x, d.y + 16.0, d.w, 24.0), 20.0, title, Face::Bold, 15.0, p.theme.text);

    let tree_w = 280.0;
    let list_x = if two_columns { d.x + 20.0 + tree_w + 16.0 } else { d.x + 20.0 };
    let list_w = d.right() - 20.0 - list_x;
    // The tree spans the full height of the body; the listing sits under the
    // address bar, which belongs to the right-hand column.
    let tree_top = d.y + 52.0;
    let body_top = d.y + 82.0;
    let body_bottom = match picker.kind {
        PickerKind::SaveAs => d.bottom() - 104.0,
        _ => d.bottom() - 58.0,
    };

    // ---- left column: the filesystem tree ----
    if two_columns {
        let tree = Rect::new(d.x + 20.0, tree_top, tree_w, body_bottom - tree_top);
        p.round(tree, 6.0, p.theme.bg);
        p.r.push_clip(tree);
        let row_h = 22.0;
        let rows = picker.tree.rows();
        let visible = ((tree.h - 8.0) / row_h).max(1.0) as usize;
        // The column has its own scroll offset (the wheel over it moves this, not
        // the listing), and it keeps the selected folder in view when navigation
        // moves the selection off screen.
        let max_start = rows.len().saturating_sub(visible);
        let selected_row = rows.iter().position(|r| r.path == picker.dir).unwrap_or(0);
        let mut start = picker.tree_scroll.min(max_start);
        if selected_row < start || selected_row >= start + visible {
            // Navigation moved the selection off screen: centre it, the way the
            // column always has, instead of pinning it to the edge.
            start = selected_row.saturating_sub(visible / 2).min(max_start);
        }
        for (vi, (i, row)) in rows.iter().enumerate().skip(start).take(visible).enumerate() {
            let r = Rect::new(tree.x + 4.0, tree.y + 4.0 + vi as f32 * row_h, tree.w - 8.0, row_h);
            let is_selected = row.path == picker.dir;
            if is_selected {
                p.round(r, 5.0, p.theme.accent.with_alpha(0.22));
            } else if p.hovered(r) {
                p.round(r, 5.0, p.theme.bg_hover);
            }
            let indent = 6.0 + row.depth as f32 * 12.0;
            let chev = Rect::new(r.x + indent, r.y + (row_h - 12.0) * 0.5, 12.0, 12.0);
            if row.has_dirs {
                p.icon(
                    if row.expanded { "chevron-up" } else { "chevron" },
                    chev,
                    p.theme.text_faint,
                );
            }
            p.icon(
                "folder",
                Rect::new(chev.right() + 2.0, r.y + (row_h - 13.0) * 0.5, 13.0, 13.0),
                if is_selected { p.theme.accent } else { p.theme.text_dim },
            );
            let name = ellipsize(p, &row.name, Face::Regular, p.theme.ui_size(), r.w - indent - 40.0);
            let b = p.baseline_in(r, p.theme.ui_size());
            p.text(chev.right() + 18.0, b, &name, Face::Regular, p.theme.ui_size(), if is_selected { p.theme.text } else { p.theme.text_dim });
            p.hit(r, Hit::DialogTreeItem(i));
            // The triangle is registered *after* the row so it wins the hit test
            // (the map keeps the last match). The other way round, clicking the
            // chevron selected the folder and never collapsed it - which is why a
            // tree could be opened but not closed.
            if row.has_dirs {
                p.hit(chev.inset_xy(-2.0, -3.0), Hit::DialogTreeChevron(i));
            }
        }
        p.r.pop_clip();
    }

    // ---- address bar over the right-hand column ----
    let dir_rect = Rect::new(list_x, d.y + 48.0, list_w - 50.0, 26.0);
    p.round(dir_rect, 5.0, p.theme.bg);
    let path_focused = app.focus == Focus::Dialog && picker.field == crate::app::PickField::Path;
    p.stroke(dir_rect, 5.0, 1.0, if path_focused { p.theme.accent } else { p.theme.border });
    p.field(
        dir_rect,
        &picker.path.text,
        "/path/to/folder",
        picker.path.selection(),
        Some(picker.path.caret),
        path_focused,
        Hit::DialogPathField,
    );
    // No "Go" button: the address bar navigates on Enter, the way every other
    // file dialog works, and the arrow beside it steps one level up.
    let up = Rect::new(dir_rect.right() + 6.0, dir_rect.y, 44.0, 26.0);
    p.button(up, "up", false, Hit::DialogUp);

    // ---- right column: the folder's contents ----
    let list = Rect::new(list_x, body_top, list_w, body_bottom - body_top);
    p.round(list, 6.0, p.theme.bg);
    p.r.push_clip(list);
    let row_h = 24.0;
    let visible = (list.h / row_h) as usize;
    // The listing has its own scroll offset, and only the selection that the
    // keyboard moved is chased; the wheel scrolls the list and nothing else.
    let start = {
        let sel = picker.selected;
        let mut s = picker.list_scroll.min(picker.entries.len().saturating_sub(visible));
        if sel < s {
            s = sel;
        } else if sel >= s + visible {
            s = sel + 1 - visible.max(1);
        }
        s
    };
    for (vi, (i, entry)) in picker.entries.iter().enumerate().skip(start).take(visible + 1).enumerate() {
        let y = list.y + 4.0 + vi as f32 * row_h;
        let r = Rect::new(list.x + 4.0, y, list.w - 8.0, row_h);
        if i == picker.selected {
            p.round(r, 4.0, p.theme.accent.with_alpha(0.25));
        } else if p.hovered(r) {
            p.round(r, 4.0, p.theme.bg_hover);
        }
        if entry.is_dir {
            p.icon("folder", Rect::new(r.x + 8.0, r.y + 5.0, 14.0, 14.0), p.theme.accent);
        } else {
            p.icon("file", Rect::new(r.x + 8.0, r.y + 5.0, 13.0, 14.0), p.theme.text_faint);
        }
        let color = if entry.is_dir { p.theme.text } else { p.theme.text_dim };
        p.label(Rect::new(r.x + 28.0, r.y, r.w - 32.0, r.h), 0.0, &entry.name, Face::Regular, p.theme.ui_size(), color);
        p.hit(r, Hit::DialogItem(i));
    }
    p.r.pop_clip();

    // ---- name field (save as only) ----
    if picker.kind == PickerKind::SaveAs {
        let nf = Rect::new(list_x, d.bottom() - 98.0, list_w, 28.0);
        p.round(nf, 5.0, p.theme.bg);
        let focused = app.focus == Focus::Dialog && picker.field == crate::app::PickField::Name;
        p.stroke(nf, 5.0, 1.0, if focused { p.theme.accent } else { p.theme.border });
        p.field(
            nf,
            &picker.name.text,
            "file name",
            picker.name.selection(),
            Some(picker.name.caret),
            focused,
            Hit::DialogNameField,
        );
    }

    // ---- buttons ----
    let confirm_label = match picker.kind {
        PickerKind::Attach => "Insert link",
        PickerKind::OpenFile => "Open",
        PickerKind::OpenFolder => "Open folder",
        PickerKind::SaveAs => "Save",
    };
    let b1 = Rect::new(d.right() - 20.0 - 100.0, d.bottom() - 46.0, 100.0, 30.0);
    p.button(b1, confirm_label, true, Hit::DialogConfirm);
    let b2 = Rect::new(b1.x - 8.0 - 90.0, d.bottom() - 46.0, 90.0, 30.0);
    p.button(b2, "Cancel", false, Hit::DialogCancel);
    if let Some(err) = &picker.error {
        p.label(Rect::new(d.x + 20.0, d.bottom() - 44.0, d.w - 240.0, 26.0), 0.0, err, Face::Regular, p.theme.ui_small(), p.theme.warn);
    }
}

/// "This file changed on disk": three answers, each spelled out, because the
/// wrong one costs work.
pub fn draw_external_dialog(p: &mut Painter, ec: &crate::app::ExternalChange, w: f32, h: f32) {
    let d = Rect::new((w - 520.0) * 0.5, (h - 210.0) * 0.5, 520.0, 210.0);
    p.panel(d, 10.0, p.theme.bg_elevated, 1.0, p.theme.border_strong);
    p.label(Rect::new(d.x, d.y + 18.0, d.w, 24.0), 24.0, "Changed on disk", Face::Bold, 15.0, p.theme.text);
    let name = ec
        .path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| ec.path.display().to_string());
    p.label(
        Rect::new(d.x, d.y + 50.0, d.w, 22.0),
        24.0,
        &format!("{} was changed by another program.", name),
        Face::Regular,
        p.theme.ui_size(),
        p.theme.text_dim,
    );
    if ec.dirty {
        p.label(
            Rect::new(d.x, d.y + 74.0, d.w, 22.0),
            24.0,
            "Reloading throws away the edits you have not saved.",
            Face::Regular,
            p.theme.ui_small(),
            p.theme.warn,
        );
    }
    let b1 = Rect::new(d.right() - 24.0 - 120.0, d.bottom() - 46.0, 120.0, 30.0);
    p.button(b1, "Reload", true, Hit::ExternalReload);
    let b2 = Rect::new(b1.x - 8.0 - 100.0, b1.y, 100.0, 30.0);
    p.button(b2, "Keep mine", false, Hit::ExternalKeep);
    let b3 = Rect::new(d.x + 24.0, b1.y, 150.0, 30.0);
    p.button(b3, "Auto-reload", false, Hit::ExternalAuto);
    p.label(
        Rect::new(d.x + 24.0, b1.y - 30.0, d.w - 48.0, 20.0),
        0.0,
        "Auto-reload lasts for this workspace session only.",
        Face::Regular,
        p.theme.ui_small(),
        p.theme.text_faint,
    );
}

/// The About box: the program, its version, and the notices that are compiled into
/// it - scrolled through with the wheel or the arrow keys.
pub fn draw_about(p: &mut Painter, about: &crate::ui::about::About, w: f32, h: f32) {
    use crate::ui::about as info;
    let d = info::box_rect(w, h);
    p.panel(d, 12.0, p.theme.bg_elevated, 1.0, p.theme.border_strong);

    p.label(Rect::new(d.x, d.y + 16.0, d.w, 26.0), 24.0, "Bare MD", Face::Bold, 19.0, p.theme.text);
    p.label_right(
        Rect::new(d.x, d.y + 20.0, d.w, 20.0),
        24.0,
        &format!("version {}", info::VERSION),
        Face::Regular,
        p.theme.ui_size(),
        p.theme.text_dim,
    );
    p.label(
        Rect::new(d.x, d.y + 42.0, d.w, 20.0),
        24.0,
        info::TAGLINE,
        Face::Regular,
        p.theme.ui_small(),
        p.theme.text_dim,
    );
    p.label(
        Rect::new(d.x, d.y + 60.0, d.w, 18.0),
        24.0,
        "program licence, crate list and the bundled font licences - drag the scrollbar to read it",
        Face::Regular,
        p.theme.ui_small(),
        p.theme.text_faint,
    );

    // The text pane.
    let body = info::body_rect(d);
    p.round(body, 6.0, p.theme.bg);
    p.r.push_clip(body);
    let visible = info::visible_lines(d.h);
    let mut y = body.y + 6.0;
    for line in about.text.iter().skip(about.scroll).take(visible) {
        if !line.is_empty() {
            p.text(
                body.x + 10.0,
                y + info::LINE_H - 4.0,
                line,
                Face::Mono,
                info::TEXT_SIZE,
                p.theme.text_dim,
            );
        }
        y += info::LINE_H;
    }
    p.r.pop_clip();

    // A scrollbar, so the box says there is more to read - and so the notice can be
    // dragged through, which is what a scrollbar is for. The geometry comes from
    // `about`, the same functions the drag itself uses: a thumb drawn in one place and
    // grabbed in another is a scrollbar that fights the pointer.
    let track = info::track_rect(d);
    p.round(track, 2.0, p.theme.border);
    let thumb = info::thumb_rect(track, about.scroll, about.text.len(), visible);
    p.round(thumb, 2.0, p.theme.text_faint);
    // Wider than it looks: a 4 px column is a hard thing to hit.
    p.hit(track.inset_xy(-5.0, 0.0), Hit::AboutScroll);

    // The two buttons.
    let close = Rect::new(d.right() - 24.0 - 100.0, d.bottom() - 44.0, 100.0, 30.0);
    p.button(close, "Close", true, Hit::AboutClose);
    let save = Rect::new(close.x - 8.0 - 132.0, close.y, 132.0, 30.0);
    p.button(save, "Save licences…", false, Hit::AboutSave);
    p.label(
        Rect::new(d.x + 24.0, close.y + 7.0, d.w - 300.0, 20.0),
        0.0,
        "Everything above is inside this executable.",
        Face::Regular,
        p.theme.ui_small(),
        p.theme.text_faint,
    );
}

/// Shorten a path from the **left** (`…/projects/notes`), which is what a file
/// manager's address bar does: the leaf is the part worth reading.
pub fn ellipsize_left(p: &mut Painter, text: &str, face: Face, size: f32, max_w: f32) -> String {
    if p.width(text, face, size) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    for i in (0..chars.len()).rev() {
        let mut cand = String::from("…");
        cand.extend(&chars[i..]);
        if p.width(&cand, face, size) > max_w {
            break;
        }
        out = cand;
    }
    if out.is_empty() {
        "…".to_string()
    } else {
        out
    }
}

pub fn ellipsize(p: &mut Painter, text: &str, face: Face, size: f32, max_w: f32) -> String {
    if p.width(text, face, size) <= max_w {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let mut cand = out.clone();
        cand.push(ch);
        cand.push('…');
        if p.width(&cand, face, size) > max_w {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

