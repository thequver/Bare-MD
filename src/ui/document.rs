//! The document surface: text, tables, the minimap, pictures and diagrams.
//!
//! The text loop is the heart of the editor: it walks the layout's visual rows and
//! paints the runs the layout produced, with the washes (selection, search hits)
//! underneath them. Nothing here decides where a character is - `markdown::layout`
//! already did, and both the caret and the mouse read the same answer.

use super::*;

/// Paint a laid-out Mermaid flowchart.
///
/// Only the primitives the renderer already has are used: rounded rectangles,
/// one-pixel runs for the edge segments and a stepped triangle for the
/// arrowheads. Node shapes are approximated with those same rects, which at a
/// 17 px base font is indistinguishable from a true outline.
pub fn draw_diagram(p: &mut Painter, box_rect: Rect, d: &crate::mermaid::Diagram) {
    use crate::mermaid::{Line, Shape};
    let theme = p.theme;
    let radius = 10.0;
    let font = d.font;
    p.round(box_rect, radius, theme.bg_elevated);
    p.stroke(box_rect, radius, 1.0, theme.border);

    // ---- edges first: nodes are painted over their endpoints ----
    // The diagram is laid out around its own origin, so every point moves by the
    // box's top-left corner before it reaches the renderer.
    for e in &d.edges {
        let c = theme.text_faint;
        let thickness = if e.line == Line::Thick { 1.8 } else { 1.2 };
        let pts: Vec<(f32, f32)> = e
            .points
            .iter()
            .map(|p| (box_rect.x + p.0, box_rect.y + p.1))
            .collect();
        for seg in pts.windows(2) {
            let ((x0, y0), (x1, y1)) = (seg[0], seg[1]);
            if e.line == Line::Dotted {
                dashed(p, x0, y0, x1, y1, thickness, c);
            } else if (y0 - y1).abs() < 0.5 {
                p.rect(Rect::new(x0.min(x1), y0 - thickness * 0.5, (x1 - x0).abs(), thickness), c);
            } else {
                p.rect(Rect::new(x0 - thickness * 0.5, y0.min(y1), thickness, (y1 - y0).abs()), c);
            }
        }
        if let Some(t) = &e.label {
            let at = (box_rect.x + e.label_at.0, box_rect.y + e.label_at.1);
            let size = font * 0.92;
            let b = at.1 + size;
            // A patch of the card's own colour, so the rule does not cut the text.
            p.rect(
                Rect::new(at.0, at.1 - 1.0, e.label_w, size * 1.4),
                theme.bg_elevated,
            );
            p.text(at.0 + 4.0, b, t, Face::Regular, size, theme.text_dim);
        }
        if e.arrow {
            if let Some(last) = arrow_dir(&pts) {
                arrow_head(p, pts[pts.len() - 1], last, c);
            }
        }
        if e.start_arrow {
            if let Some(first) = arrow_dir(&pts) {
                arrow_head(p, pts[0], (-first.0, -first.1), c);
            }
        }
    }

    // ---- nodes ----
    for node in &d.nodes {
        let r = Rect::new(node.x, node.y, node.w, node.h);
        let r = Rect::new(box_rect.x + r.x, box_rect.y + r.y, r.w, r.h);
        let fill = theme.bg;
        let line = theme.border_strong;
        match node.shape {
            Shape::Rect => {
                p.rect(r, fill);
                p.stroke(r, 0.0, 1.2, line);
            }
            Shape::Round => {
                p.round(r, 6.0, fill);
                p.stroke(r, 6.0, 1.2, line);
            }
            Shape::Stadium => {
                p.round(r, r.h * 0.5, fill);
                p.stroke(r, r.h * 0.5, 1.2, line);
            }
            Shape::Circle => {
                p.round(r, r.h * 0.5, fill);
                p.stroke(r, r.h * 0.5, 1.2, line);
            }
            Shape::Cylinder => {
                p.round(r, 8.0, fill);
                p.stroke(r, 8.0, 1.2, line);
                // The lid: an arc drawn as a shallow stepped line.
                let y = r.y + 8.0;
                let inset = r.w * 0.12;
                p.rect(Rect::new(r.x + inset, y, r.w - inset * 2.0, 1.0), line);
            }
            Shape::Subroutine => {
                p.rect(r, fill);
                p.stroke(r, 0.0, 1.2, line);
                p.rect(Rect::new(r.x + 7.0, r.y, 1.0, r.h), line);
                p.rect(Rect::new(r.right() - 8.0, r.y, 1.0, r.h), line);
            }
            Shape::Hexagon | Shape::Diamond => {
                // Stepped outline: rows of shrinking width read as a slanted
                // edge, which is all a diamond or a hexagon needs at this size.
                stepped_shape(p, r, node.shape, fill, line);
            }
        }
        // Label, centred in the box.
        let size = font;
        let w = p.width(&node.label, Face::Regular, size);
        let baseline = p.baseline_in(r, size);
        p.text(r.x + (r.w - w) * 0.5, baseline, &node.label, Face::Regular, size, theme.text);
    }
}

/// Direction of the last segment of a polyline (unit-ish vector, axis aligned).
pub fn arrow_dir(points: &[(f32, f32)]) -> Option<(f32, f32)> {
    let a = points.get(points.len().wrapping_sub(2))?;
    let b = points.last()?;
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.01 {
        None
    } else {
        Some((dx / len, dy / len))
    }
}

/// A filled triangle pointing along `dir`, drawn as a stack of thin rects.
pub fn arrow_head(p: &mut Painter, tip: (f32, f32), dir: (f32, f32), c: Color) {
    let size = 6.0;
    let steps = 6;
    for k in 0..steps {
        let t = k as f32 / steps as f32;
        // From the tip backwards the triangle widens.
        let along = size * (1.0 - t);
        let half = size * 0.55 * t;
        let width = (half * 2.0).max(1.0);
        let cx = tip.0 - dir.0 * along;
        let cy = tip.1 - dir.1 * along;
        if dir.1.abs() > dir.0.abs() {
            p.rect(Rect::new(cx - width * 0.5, cy - 1.0, width, 2.0), c);
        } else {
            p.rect(Rect::new(cx - 1.0, cy - width * 0.5, 2.0, width), c);
        }
    }
}

/// A dashed axis-aligned segment.
pub fn dashed(p: &mut Painter, x0: f32, y0: f32, x1: f32, y1: f32, thickness: f32, c: Color) {
    let len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
    if len < 0.5 {
        return;
    }
    let dash = 5.0;
    let gap = 4.0;
    let mut t = 0.0;
    let (ux, uy) = ((x1 - x0) / len, (y1 - y0) / len);
    while t < len {
        let end = (t + dash).min(len);
        let ax = x0 + ux * t;
        let ay = y0 + uy * t;
        let bx = x0 + ux * end;
        let by = y0 + uy * end;
        if (y1 - y0).abs() < 0.5 {
            p.rect(Rect::new(ax.min(bx), ay - thickness * 0.5, (bx - ax).abs(), thickness), c);
        } else {
            p.rect(Rect::new(ax - thickness * 0.5, ay.min(by), thickness, (by - ay).abs()), c);
        }
        t = end + gap;
    }
}

/// Diamond / hexagon outline built from stacked rows.
pub fn stepped_shape(p: &mut Painter, r: Rect, shape: crate::mermaid::Shape, fill: Color, line: Color) {
    use crate::mermaid::Shape;
    let rows = (r.h / 2.0).round().max(6.0) as usize;
    for k in 0..rows {
        let t = k as f32 / (rows - 1) as f32;
        // 0 at the top, 1 at the bottom; the profile widens to the middle.
        let mut taper = 1.0 - (2.0 * t - 1.0).abs();
        if shape == Shape::Hexagon {
            taper = (taper * 2.2).min(1.0);
        }
        let half = r.w * 0.5 * taper;
        let cx = r.x + r.w * 0.5;
        let y = r.y + r.h * t;
        let thickness = r.h / rows as f32 + 0.5;
        p.rect(Rect::new(cx - half, y, half * 2.0, thickness), fill);
        // Outline: only the slanted ends of the row.
        p.rect(Rect::new(cx - half, y, 1.4, thickness), line);
        p.rect(Rect::new(cx + half - 1.4, y, 1.4, thickness), line);
    }
    // Flat top and bottom caps for the hexagon.
    if shape == Shape::Hexagon {
        p.rect(Rect::new(r.x + r.w * 0.2, r.y, r.w * 0.6, 1.2), line);
        p.rect(Rect::new(r.x + r.w * 0.2, r.bottom() - 1.2, r.w * 0.6, 1.2), line);
    }
}

pub fn draw_editor(app: &App, p: &mut Painter, editor: Rect, text: Rect) {
    let ch_w = editor.w - Theme::EDITOR_PAD_X * 2.0 - 10.0;
    let theme = p.theme;
    p.rect(editor, theme.bg);
    let tab = app.tab();
    let layout = &tab.layout;
    let scroll = tab.scroll;
    let scroll_x = tab.scroll_x;
    let (sel_a, sel_b) = tab.sel();
    let has_sel = tab.has_selection();
    let mode = app.mode;
    // Decorations (cards, bars, rules, grids) belong to the rendered view only.
    let decorated = mode == ViewMode::Live;

    p.r.push_clip(text.inset_xy(-18.0, -2.0));

    let (first, last) = layout.visible_range(scroll, text.h);
    let origin_y = text.y - scroll;
    let origin_x = text.x - scroll_x;

    // Hits that must sit *above* the editor surface (checkboxes, the table's
    // hover chrome): collected here, registered after `Hit::Editor`.
    let mut late_hits: Vec<(Rect, Hit)> = Vec::new();

    // ---- block decorations ----
    // Every card is drawn inside the vertical space the layout reserved for it
    // (`space_before` / `space_after`), so it can never sit on top of the text
    // above or below.
    if decorated {
        // A run of card lines (code, table, quote) is painted from its *first*
        // line, which may be scrolled above the viewport: starting at the first
        // visible line made a long code card shrink to the width of what happened
        // to be on screen and made a long table lose its grid the moment its
        // first row scrolled out. Painting above the clip costs nothing.
        let mut i = card_run_start(layout, first);
        while i <= last.min(layout.lines.len().saturating_sub(1)) {
            let line = &layout.lines[i];
            match line.block() {
                Block::Code | Block::Fence => {
                    // One rounded card for the whole run, hugging the code. The
                    // card is painted even while one of its lines is being edited:
                    // a hole in the background is a bug, not a feature.
                    if line.folded {
                        // A folded mermaid block is a picture, painted below.
                        i += 1;
                        continue;
                    }
                    let mut j = i;
                    let mut w = 0.0f32;
                    // The fences are usually hidden and take less room than a
                    // line of code, so the card hugs the *body*; a block with no
                    // body falls back to its markers.
                    let mut body_bottom: Option<f32> = None;
                    while j < layout.lines.len()
                        && !layout.lines[j].folded
                        && matches!(layout.lines[j].block(), Block::Code | Block::Fence)
                    {
                        let l = &layout.lines[j];
                        w = w.max(l.indent + l.width);
                        if l.block() == Block::Code {
                            body_bottom = Some(l.top + l.height);
                        }
                        j += 1;
                    }
                    let first_line = &layout.lines[i];
                    let last_line = &layout.lines[j - 1];
                    // The card hugs the code at any zoom: same scale factor the
                    // layout used for the block padding.
                    let vs = crate::layout::vertical_scale(p.theme.font_size);
                    let card_pad_y = CARD_PAD_Y * vs;
                    let top = origin_y + first_line.top - card_pad_y;
                    // The card must clear *both* the body and the last line of
                    // the run: a closing fence only occupies 0.62 of a line
                    // height, so ending the card at the body made the lower
                    // ```` ``` ```` overlap the frame while the upper one had
                    // room to spare. Same padding top and bottom, always.
                    let content_bottom = body_bottom
                        .map(|b| b.max(last_line.top + last_line.height))
                        .unwrap_or(last_line.top + last_line.height);
                    let bottom = origin_y + content_bottom + card_pad_y;
                    let card_w = (w + CODE_PAD_X + 2.0).max(160.0);
                    let card = Rect::new(text.x - scroll_x - 2.0, top, card_w, bottom - top);
                    p.round(card, 8.0, theme.code_bg);
                    p.stroke(card, 8.0, 1.0, theme.border_strong.with_alpha(0.5));

                    // ---- copy button ----
                    // Only while the pointer is over the block: a card that is not
                    // being pointed at stays quiet. The ticks it took over the
                    // source text are not needed here - the code is copied, not
                    // selected.
                    let zone = card.inset_xy(-6.0, -6.0);
                    if zone.contains(app.mouse) && !card.h.is_nan() {
                        let size = 22.0;
                        let btn = Rect::new(card.right() - size - 6.0, card.y + 5.0, size, size);
                        let hot = p.hovered(btn);
                        let copied = app.status_at.elapsed().as_millis() < 1400
                            && app.status.starts_with("Code copied");
                        p.round(btn, 5.0, if hot || copied { theme.bg_hover } else { theme.bg_elevated });
                        if !hot && !copied {
                            p.stroke(btn, 5.0, 1.0, theme.border_strong);
                        }
                        let ink = if copied { theme.text } else { theme.text_dim };
                        p.icon(if copied { "check" } else { "copy" }, btn.inset(4.0), ink);
                        late_hits.push((btn, Hit::CopyCode(first_line.line, last_line.line)));
                    }
                    i = j;
                }
                Block::Quote => {
                    // One continuous bar per nesting level for the whole run, so
                    // the bars of consecutive quote lines join up instead of
                    // breaking at every line break.
                    let mut j = i;
                    while j < layout.lines.len() && layout.lines[j].block() == Block::Quote {
                        j += 1;
                    }
                    // A quote in a right-to-left document is indented from the
                    // right, so its bar is drawn down the right side of the block.
                    let rtl_run = layout.lines.get(i).map(|l| l.rtl_para()).unwrap_or(false);
                    // A quote that holds a list item draws the item's bullet:
                    // `> * one` stays a list (the marker itself is hidden like every
                    // other marker). Without this, quoting a list turned all of it
                    // into quote paragraphs and the list was gone from the page.
                    for k in i..j {
                        let l = &layout.lines[k];
                        if !l.info.quote_bullet || tab.line_is_raw(mode, k) {
                            continue;
                        }
                        let bx = if l.rtl_para() {
                            text.right() - scroll_x - l.indent + 7.0
                        } else {
                            text.x - scroll_x + l.indent - 11.0
                        };
                        let by = origin_y + l.top + l.row_h * 0.5 - 2.0;
                        p.round(Rect::new(bx, by, 4.0, 4.0), 2.0, theme.bullet);
                    }
                    for (depth, y0, y1) in quote_bar_spans(layout, i, j) {
                        let x = if rtl_run {
                            text.right() - scroll_x - 2.5 - depth as f32 * QUOTE_INDENT
                        } else {
                            text.x - scroll_x + depth as f32 * QUOTE_INDENT
                        };
                        p.rect(Rect::new(x, origin_y + y0, 2.5, y1 - y0), theme.quote_bar);
                    }
                    i = j;
                }
                Block::Hr => {
                    // While the line is being edited it shows `---` as text; a rule
                    // painted through those characters is the bug this guards.
                    if !tab.line_is_raw(mode, i) {
                        let h = line.height;
                        let y = origin_y + line.top;
                        p.rect(Rect::new(text.x, y + h * 0.5, text.w.min(520.0), 1.5), theme.border_strong);
                    }
                    i += 1;
                }
                Block::Table => {
                    // Once per run, always from the run's *first* line: drawing the
                    // grid per visible row repeated the chrome (a grip on every
                    // column of every row, a `+` on every row) and, on a scrolled
                    // table, started the grid at a row in the middle.
                    draw_table_grid(app, p, text, origin_y, scroll_x, i, &mut late_hits);
                    // The rest of the run was part of that drawing; a scrolled
                    // table must never be drawn a second time from a middle row.
                    i += 1;
                    while i < layout.lines.len() && layout.lines[i].block() == Block::Table {
                        i += 1;
                    }
                }
                _ => i += 1,
            }
        }
    }

    // ---- washes: selection and search hits ----
    // Collected here and painted *under* the glyphs (see the text pass below).
    // Painting them after the text made the text invisible: the selection colour
    // is opaque, so a selection looked like a solid bar with no words in it, in
    // the editor and in every one-line field.
    let mut washes: Vec<(Rect, Color)> = Vec::new();

    // ---- selection (per visual row so wrapped lines highlight correctly) ----
    if has_sel {
        for i in first..=last.min(layout.lines.len().saturating_sub(1)) {
            let line = &layout.lines[i];
            if line.end_char() < sel_a || line.start_char > sel_b {
                continue;
            }
            // The row under the caret shows its inline markers, and the wash has
            // to be measured in the same variant the glyphs are: the two differ by
            // the width of every `**` on the row.
            let raw = tab.line_is_raw(mode, i) || tab.table_shows_markers(mode, i);
            let local_a = sel_a.saturating_sub(line.start_char).min(line.chars.len());
            let local_b = (sel_b.saturating_sub(line.start_char)).min(line.chars.len());
            let shift = line.align_offset_row(text.w, line.row_of_local_in(local_a, raw), raw);
            // A table row is highlighted piece by piece: each cell's part of the
            // selection is painted on its own band, at its own x. The paragraph
            // rule below assumes one left margin and one bar per visual row, which
            // turned a selection inside a wrapped cell into a bar across the whole
            // row.
            if !line.table_cols.is_empty() {
                for (band, x, w) in line.table_sel_rects(local_a, local_b, raw) {
                    let y = origin_y + line.top + band as f32 * line.row_h;
                    washes.push((
                        Rect::new(origin_x + line.indent + shift + x, y, w, line.row_h),
                        theme.selection.with_alpha(0.55),
                    ));
                }
                continue;
            }
            // Every visual row of this line, measured by the layout: which rows the
            // selection touches, and where its ink starts and ends on each of them.
            // A right-to-left row runs the other way, so one bar from "the first
            // selected offset" to "the second" was a sliver - see `row_span`.
            let spans = line.row_span(p.r.text_ref(), local_a, local_b, raw);
            let pad = match line.block() {
                Block::Code | Block::Fence => CODE_PAD_X,
                _ => 6.0,
            };
            for (ri, lo, hi) in spans {
                let row = line.row(ri);
                // A full-line selection stops just past the end of the text, and
                // inside a card it stops at the card's edge so the highlight never
                // spills over the rounded background.
                let full_row = local_b >= row.end && local_a <= row.start;
                let shift_row = line.align_offset_row(text.w, ri, raw);
                let x0 = origin_x + line.indent + shift_row + lo;
                let x1 = origin_x + line.indent + shift_row + hi;
                let x1 = if full_row {
                    (x1 + pad).min(origin_x + text.w)
                } else {
                    x1
                };
                let y = origin_y + line.top + ri as f32 * line.row_h;
                // Inside a card the highlight is a soft wash instead of a solid
                // bar: it reads as a selection without repainting the block.
                let color = match line.block() {
                    Block::Code | Block::Fence | Block::Table => theme.selection.with_alpha(0.55),
                    _ => theme.selection,
                };
                washes.push((Rect::new(x0, y, (x1 - x0).max(2.0), line.row_h), color));
            }
        }
    }

    // ---- search highlights (row aware, exactly like the selection) ----
    if tab.find.open && !tab.find.matches.is_empty() {
        for (mi, &(m_a, m_b)) in tab.find.matches.iter().enumerate() {
            let l0 = layout.line_of_char(m_a);
            let l1 = layout.line_of_char(m_b);
            for li in l0..=l1 {
                if li < first || li > last {
                    continue;
                }
                let line = &layout.lines[li];
                let raw = tab.line_is_raw(mode, li) || tab.table_shows_markers(mode, li);
                let local_a = m_a.max(line.start_char) - line.start_char;
                let local_b = (m_b.min(line.end_char())) - line.start_char;
                let (start, end) = (local_a.min(line.chars.len()), local_b.min(line.chars.len()));
                let is_current = tab.find.current == Some(mi);
                let color = if is_current {
                    theme.accent.with_alpha(0.42)
                } else {
                    theme.accent.with_alpha(0.16)
                };
                // The same measurement as the selection: a match is a stretch of
                // characters, so it is washed on the rows it covers, from the left
                // edge of its first glyph to the right edge of its last. Read in
                // index order it was right for Latin and a 3 px sliver in Hebrew.
                for (ri, lo, hi) in line.row_span(p.r.text_ref(), start, end.max(start), raw) {
                    let shift_row = line.align_offset_row(text.w, ri, raw);
                    let x0 = origin_x + line.indent + shift_row + lo;
                    let x1 = origin_x + line.indent + shift_row + hi;
                    let y = origin_y + line.top + ri as f32 * line.row_h;
                    let w = (x1 - x0).max(3.0);
                    washes.push((Rect::new(x0, y + 1.0, w, line.row_h - 2.0), color));
                }
            }
        }
    }

    // ---- pictures ----
    if decorated {
        for i in first..=last.min(layout.lines.len().saturating_sub(1)) {
            let line = &layout.lines[i];
            let Some(img) = &line.image else { continue };
            let revealed = tab.line_is_raw(mode, i) || tab.table_shows_markers(mode, i);
            let x = origin_x + line.indent + line.align_offset(text.w);
            let y = origin_y + line.top;
            let box_rect = Rect::new(x, y, img.w, img.h);
            let radius = 6.0;
            // The frame is always there, so a picture never floats unnamed.
            p.round(box_rect, radius, theme.bg_elevated);
            if img.missing {
                p.stroke(box_rect, radius, 1.0, theme.border_strong);
                let label = if img.alt.is_empty() { "image not found" } else { &img.alt };
                let size = theme.ui_size();
                let b = p.baseline_in(box_rect, size);
                let text_x = box_rect.x + 10.0;
                p.text(text_x, b, label, Face::Regular, size, theme.text_dim);
                let src = ellipsize(
                    p,
                    &img.path.to_string_lossy(),
                    Face::Regular,
                    theme.ui_small(),
                    (box_rect.w - 20.0).max(40.0),
                );
                p.text(text_x, b + size * 1.4, &src, Face::Regular, theme.ui_small(), theme.text_faint);
            } else {
                // While the caret is on the line the source is edited in place, so
                // the picture steps back instead of fighting the text.
                let tint = if revealed {
                    Color::rgba(1.0, 1.0, 1.0, 0.3)
                } else {
                    Color::rgba(1.0, 1.0, 1.0, 1.0)
                };
                let max_px = (1600.0 * p.r.scale) as u32;
                match p.r.image(&img.path, max_px) {
                    Some(tex) => p.r.draw_image(box_rect, tex, radius, tint),
                    None => {
                        p.stroke(box_rect, radius, 1.0, theme.border_strong);
                    }
                }
                if !revealed {
                    p.stroke(box_rect, radius, 1.0, theme.border.with_alpha(0.7));
                }
            }
        }
    }

    // ---- mermaid flowcharts ----
    if decorated {
        for i in first..=last.min(layout.lines.len().saturating_sub(1)) {
            let line = &layout.lines[i];
            let Some(d) = &line.diagram else { continue };
            let x = origin_x + line.indent + line.align_offset(text.w);
            let y = origin_y + line.top;
            draw_diagram(p, Rect::new(x, y, d.w, d.h), d);
        }
    }

    // ---- the washes, then the words on top of them ----
    for (rect, color) in &washes {
        p.rect(*rect, *color);
    }

    // ---- text (drawn per visual row) ----
    for i in first..=last.min(layout.lines.len().saturating_sub(1)) {
        let line = &layout.lines[i];
        // A table row reveals its *inline* markers while it is being edited;
        // `raw` is what selects the variant everywhere below, so the paint, the
        // washes and the caret all read the same geometry.
        let raw = tab.line_is_raw(mode, i) || tab.table_shows_markers(mode, i);
        let runs = line.runs(raw);
        // How many visual rows this line has matters: a paragraph with no line breaks
        // in it - a whole document pasted as one paragraph - is a single line with
        // thousands of rows, and the ones off screen are not worth a single quad.
        let rows = line.rows.len();
        let first_row = if line.row_h > 0.0 {
            (((text.y - (origin_y + line.top)) / line.row_h).floor().max(0.0) as usize).min(rows)
        } else {
            0
        };
        let last_row = if line.row_h > 0.0 {
            (((text.y + text.h - (origin_y + line.top)) / line.row_h).ceil().max(0.0) as usize)
                .min(rows)
        } else {
            rows
        };
        for (ri, row) in line.rows.iter().enumerate().take(last_row).skip(first_row) {
            let row_top = origin_y + line.top + ri as f32 * line.row_h;
            let (asc, desc) = p.r.metrics(Face::Regular, line.max_size);
            let base_y = row_top + line.row_h * 0.5 + (asc - desc) * 0.42;
            let shift = line.align_offset_row(text.w, ri, raw);
            // A table row is drawn from its wrapped bands: each band holds the
            // piece of every cell that belongs on that line, at the x of its own
            // column, so no cell can draw outside its column.
            let table_band = line.bands(raw).get(ri).filter(|b| !b.is_empty());
            // A bidirectional line is painted from the runs of *this row*, already
            // in visual order with their x from the row's left edge - the same
            // shape the table path uses, for the same reason: the row's own
            // geometry, not the line's.
            let bidi_row = line.bidi_row_runs(ri, raw);
            let cells: Vec<crate::layout::Run> = match table_band {
                Some(band) => band.clone(),
                // A table row is painted *only* from its bands. Falling back to the
                // line's runs on an empty band painted the whole row's source at
                // the line's left edge - text on top of the grid and over the
                // pointer - whenever every cell of that band was blank.
                None if !line.table_cols.is_empty() => continue,
                None => match bidi_row {
                    Some(v) => v.clone(),
                    None => runs.to_vec(),
                },
            };
            for run in &cells {
                let (a, b) = if table_band.is_some() || bidi_row.is_some() {
                    (run.start, run.end)
                } else {
                    (run.start.max(row.start), run.end.min(row.end))
                };
                if b <= a {
                    continue;
                }
                let offset = if table_band.is_some() || bidi_row.is_some() {
                    run.x
                } else {
                    line.x_abs(p.r.text_ref(), a, raw) - line.x_abs(p.r.text_ref(), row.start, raw)
                };
                let x = origin_x + line.indent + shift + offset;
                let text_slice: String = line.chars[a..b].iter().collect();
                // `==highlight==` is yellow *text*: the marked words stay the
                // size and weight they were, only their ink changes. A wash
                // behind them read as a highlighter pen and fought the selection
                // colour underneath it.
                let color = run.style.fg.unwrap_or_else(|| {
                    if run.style.highlight {
                        theme.mark
                    } else if run.style.ink == Inks::Marker {
                        theme.marker
                    } else {
                        ink_color(run.style.ink, &p.theme)
                    }
                });
                let run_w = run.w * (b - a) as f32 / (run.end - run.start).max(1) as f32;
                // Explicit background first, so the glyphs sit on top of it.
                if let Some(bg) = run.style.bg {
                    p.rect(
                        Rect::new(x - 1.0, row_top + 1.0, run_w + 2.0, line.row_h - 2.0),
                        bg,
                    );
                }
                // An inline code span gets a chip of its own. Monospaced text in the
                // middle of a proportional sentence is already recognisable, but
                // without a background it reads as a slip of the pen rather than as
                // `code`, and on a light page `code_text` is the same ink as the
                // body. The chip is the card colour with its own hairline edge, so
                // it reads on all three themes.
                let chip = run.style.code() && !raw;
                if chip {
                    let piece_w = p.width(&text_slice, run.style.face, run.style.size);
                    let r = Rect::new(
                        x - CODE_CHIP_PAD,
                        row_top + 1.0,
                        piece_w + CODE_CHIP_PAD * 2.0,
                        line.row_h - 2.0,
                    );
                    // A tint of the page itself, a tenth of the way to the text
                    // colour: `code_bg` is the *card's* colour and is nearly the
                    // page on all three themes, so a chip filled with it was
                    // invisible. The hairline keeps the edges crisp.
                    let chip_fill = theme.bg.mix(theme.text, 0.10);
                    p.round(r, 3.5, chip_fill);
                    p.stroke(r, 3.5, 1.0, theme.border_strong.with_alpha(0.55));
                }
                let dy = run.style.raise * run.style.size;
                let drawn_w = p.text_directed(
                    x,
                    base_y - dy,
                    &text_slice,
                    run.style.face,
                    run.style.size,
                    color,
                    run.style.weight,
                    run.rtl,
                );
                if run.style.underline {
                    let uy = base_y - dy + run.style.size * 0.13;
                    p.rect(Rect::new(x, uy, drawn_w, 1.2), color.with_alpha(0.75));
                }
                if run.style.strike {
                    let mid = base_y - dy - run.style.size * 0.3;
                    p.rect(Rect::new(x, mid, drawn_w, 1.0), color);
                }
            }
        }
        let y = origin_y + line.top;
        let (asc, desc) = p.r.metrics(Face::Regular, line.max_size);
        let base_y = y + line.row_h * 0.5 + (asc - desc) * 0.42;
        // list bullets / task checkboxes (live rendering only: on the caret's own
        // line the raw `-` / `[ ]` markers are already on screen)
        // A right-to-left paragraph starts at the right margin, so its marker
        // belongs between the text and that margin - the same place a bullet sits
        // in an Arabic newspaper - and not out in the empty left gutter.
        let rtl_para = line.rtl_para();
        let row_right = origin_x + line.indent + line.align_offset_row(text.w, 0, raw) + line.row_width_of(0, raw);
        match line.block() {
            _ if raw => {}
            Block::Bullet => {
                let bx = if rtl_para { row_right + 8.0 } else { origin_x + line.indent - 12.0 };
                let cy = base_y - p.theme.font_size * 0.32;
                p.round(Rect::new(bx, cy, 4.0, 4.0), 2.0, theme.bullet);
            }
            Block::Ordered => {
                // The number the writer typed, right-aligned in the gutter the way
                // a list marker sits in every other editor. It is the source's own
                // `1.` / `7)` / `12.`, never a count of the item's position: a list
                // that starts at `5.` keeps saying 5.
                let marker = &line.info.marker;
                if !marker.is_empty() {
                    let size = p.theme.font_size;
                    let w = p.width(marker, Face::Regular, size);
                    let mx = if rtl_para { row_right + 6.0 } else { origin_x + line.indent - 8.0 - w };
                    p.text(mx, base_y, marker, Face::Regular, size, theme.bullet);
                }
            }
            Block::Task(checked) => {
                // A checkbox the size of a capital letter, with a real gap before
                // the text (the gutter is wider for tasks for exactly this).
                let size = (p.theme.font_size * 1.0).max(16.0);
                let bx = if rtl_para { row_right + 6.0 } else { origin_x + line.indent - size - 6.0 };
                let by = base_y - size * 0.82;
                let r = Rect::new(bx, by, size, size);
                let hovered = p.hovered(r);
                if checked {
                    p.round(r, 3.5, theme.accent);
                    let b = p.baseline_in(r, size * 0.78);
                    p.text(r.x + 3.0, b, "\u{2713}", Face::Bold, size * 0.78, theme.accent_text);
                } else {
                    // An empty box needs an edge that reads on both themes: on
                    // white `border_strong` is almost the page colour.
                    let fill = if hovered { theme.bg_active } else { theme.bg_elevated };
                    let edge = if hovered { theme.accent } else { theme.text_dim };
                    // Fill and border in one quad: a bare stroke does not read at
                    // this size on a light page.
                    p.r.rect_bordered(r, 3.5, fill, 1.4, edge);
                }
                // The checkbox sits on top of the editor surface, so its hit
                // area is registered *after* `Hit::Editor` below.
                late_hits.push((r.inset_xy(-2.0, -2.0), Hit::Task(line.start_char)));
            }
            _ => {}
        }
    }

    // ---- caret ---- (geometry shared with `App::caret_rect`)
    // The empty state below paints a placeholder on the caret's own line, so the
    // caret would blink straight through the word "Write". It comes back with the
    // first character typed.
    let empty_state = app.shows_empty_state();
    if app.caret_is_painted() {
        let cr = app.caret_rect(p.r.text_ref());
        let visible = cr.y + 2.0 > text.y - 2.0 && cr.y < text.bottom() + 2.0;
        if visible && cr.w > 0.0 {
            p.rect(cr, theme.caret);
        }
    }

    // ---- empty state ----
    if empty_state {
        let size = p.theme.font_size;
        let b = text.y + size * 2.0;
        p.text(text.x, b, "Write Markdown.", Face::Bold, size, theme.text_faint);
        let hints = [
            "Ctrl+F find and replace   ·   Ctrl+O open file   ·   Ctrl+Shift+O open folder   ·   Ctrl+S save",
            "Ctrl+Shift+E workspace panel   ·   Ctrl+Shift+O outline   ·   Ctrl+Shift+M raw mode",
        ];
        for (i, line) in hints.iter().enumerate() {
            p.text(text.x, b + size * (1.6 + i as f32 * 1.25), line, Face::Regular, p.theme.ui_size(), theme.text_faint);
        }
    }

    p.r.pop_clip();

    // ---- click surface ----
    // The whole text column, plus the empty strip between it and the minimap: a
    // click out there is a click past the end of a row, and it has to reach the
    // editor to be turned into a caret. The scrollbars and the map are registered
    // below and after, so they still win inside their own boxes.
    let surface = Rect::new(
        text.x,
        text.y,
        (editor.right() - text.x).max(text.w),
        (editor.bottom() - text.y).max(text.h),
    );
    p.hit(surface, Hit::Editor);
    for (r, hit) in late_hits {
        p.hit(r, hit);
    }

    // ---- scrollbars ----
    // Only when the map is off. The minimap is the scroll indicator: showing the
    // plain bars as well gave the same information twice, in two different places,
    // and the pair disagreed about where you were.
    let total = layout.total_height.max(1.0);
    if !app.settings.minimap_open && total > text.h {
        // Wide enough to grab, and parked against the editor's right edge.
        let track = Rect::new(editor.right() - 13.0, text.y, 9.0, text.h);
        let frac = (text.h / total).clamp(0.06, 1.0);
        let thumb_h = (text.h * frac).max(24.0);
        let max_scroll = (total - text.h).max(1.0);
        let t = (scroll / max_scroll).clamp(0.0, 1.0);
        let thumb = Rect::new(track.x, track.y + (track.h - thumb_h) * t, track.w, thumb_h);
        p.round(thumb, 4.5, theme.scrollbar);
        p.hit(track.inset_xy(-3.0, 0.0), Hit::ScrollV);
    }
    if !app.settings.minimap_open && layout.max_row_width > text.w + 8.0 {
        let track = Rect::new(text.x, text.bottom() + 4.0, text.w.min(ch_w), 8.0);
        let frac = (text.w / layout.max_row_width).clamp(0.08, 1.0);
        let thumb_w = (text.w * frac).max(30.0);
        let max_scroll = (layout.max_row_width - text.w).max(1.0);
        let t = (scroll_x / max_scroll).clamp(0.0, 1.0);
        let thumb = Rect::new(track.x + (track.w - thumb_w) * t, track.y, thumb_w, track.h);
        p.round(thumb, 4.0, theme.scrollbar);
        p.hit(track, Hit::ScrollH);
    }
}

/// Vertical distance between two minimap rows when there is room to spare.
pub const MINIMAP_PITCH: f32 = 3.0;

/// Spacing-free position of every line on the minimap.
///
/// The map does not reuse the page's pixel geometry: block spacing (22 px of air
/// above an H1, a card's padding) would show up as big empty gaps between the
/// bars. Each line advances by its number of text rows, and a blank line by one.
/// How many map rows a line occupies.
///
/// Text is one row per wrapped row. A picture or a diagram is **one** bar however
/// many source lines it hides, but a bar tall enough to cover the space it takes
/// on the page - without that, a map with one image in it turned the lines around
/// the image into a dense stripe and squeezed the rest of the document.
pub fn minimap_virtual_span(line: &LineLayout) -> f32 {
    line.virtual_span()
}

pub fn minimap_virtual_tops(layout: &Layout) -> Vec<f32> {
    let mut out = Vec::with_capacity(layout.lines.len());
    let mut v = 0.0f32;
    for line in &layout.lines {
        out.push(v);
        v += minimap_virtual_span(line);
    }
    out
}

/// How the minimap maps the document onto its column.
///
/// Both the painter and the input layer go through this, so the bars you see and
/// the place a click lands cannot drift apart (the old code drew on a virtual,
/// spacing-free scale but scrolled with a plain `y / rect.h` fraction).
///
/// The map shows the **whole document**, scaled so it fits the column.
///
/// This is how ecode (and Sublime, and VS Code's fit-to-height mode) behaves, and
/// it is what makes the map trustworthy: there is exactly one linear mapping
/// between a document position and a map position, it never moves, and the
/// viewport indicator is simply `scroll` and `height` through that same mapping.
/// The previous model scrolled the map itself on a fixed 3 px pitch, which meant
/// the map had its own offset - and any bug in that offset showed up as the map
/// and the page disagreeing about where you were.
///
/// Short documents keep the full [`MINIMAP_PITCH`], so the bars stay thin lines
/// and the map looks like text; a long one scales the pitch down so that it still
/// fits, and the bars get finer rather than the column growing a scrollbar.
#[derive(Clone, Copy, Debug)]
pub struct MinimapGeom {
    /// Virtual rows (text rows only) in the document.
    pub vtotal: f32,
    /// Pixels per virtual row, never more than [`MINIMAP_PITCH`].
    pub scale: f32,
    /// Height the bars cover, from the top of the column (never past `view_h`).
    pub content_h: f32,
    /// Height of the column the map is painted in.
    pub view_h: f32,
    /// Bars thicker than one pixel? Used to skip colliding rows.
    pub bar_h: f32,
    /// Characters in the widest visual row of the document: what a full-width bar means.
    /// Computed once, here, in the same pass as `vtotal` - the painter used to rescan
    /// every row of the file on every frame to find it.
    pub max_row: usize,
    /// How far the map's own content is slid up, in pixels.
    ///
    /// Zero while the whole document fits the column. A very long document cannot
    /// be squeezed into the strip without the bars merging into a black smear, so
    /// the map slides instead: the
    /// window of the map you see follows the viewport, which is what "the minimap
    /// scrolls with the page" looks like.
    pub offset: f32,
}

impl MinimapGeom {
    /// Top of a virtual row, relative to the top of the minimap.
    pub fn y_of_virtual(&self, v: f32) -> f32 {
        MINIMAP_PAD + v * self.scale - self.offset
    }

    /// True when the map is sliding rather than showing the whole document.
    pub fn slides(&self) -> bool {
        self.offset > 0.5
    }

    /// The room the column gives the map, padding taken off.
    pub fn room(&self) -> f32 {
        (self.view_h - MINIMAP_PAD - 2.0).max(8.0)
    }

    /// Height of the map's handle (the viewport indicator), in map pixels.
    ///
    /// The handle is the *visible slice* of the document, so it is proportional
    /// to `text_h / total` - but it is measured against the bars, not against the
    /// column. A short document draws short bars (`MINIMAP_PITCH` keeps them
    /// lines, not blocks), and a handle the height of the whole window next to a
    /// nine-line map says nothing true about either. Its floor is the size of a
    /// grab target, so a handle can always be grabbed.
    /// Height of the handle: the share of the text the viewport can see.
    ///
    /// It depends only on the document - never on the scroll position or on a drag -
    /// so it cannot change size while being dragged.
    pub fn view_h_px(&self, total_doc_h: f32, text_h: f32) -> f32 {
        let bars = self.track_h().max(MIN_HANDLE_H);
        if total_doc_h <= text_h + 1.0 {
            return bars;
        }
        // The thumb is a constant-height handle, not a picture of the viewport.
        //
        // Sizing it by the viewport seemed right - it is what a scrollbar does - but over
        // the bars it eats the track it has to move in: forty rows draw a 123 px strip and
        // a viewport share of the text is 55 px of thumb, so the handle could move 68 px
        // over a map 1 200 px of text tall and read as "it only moves a bit". A handle of
        // one constant height (MIN_HANDLE_H) is what was asked for from the start, is the
        // same size at every document length, and leaves the rest of the bars as travel.
        // It is never allowed to fill its track, and a map too short for that height gets
        // a smaller handle rather than no travel.
        let cap = (bars * 0.45).max(MIN_HANDLE_H).min((bars - MIN_TRAVEL_PX).max(MIN_STUB_HANDLE_H));
        MIN_HANDLE_H.min(cap)
    }

    /// Top of the bars on screen, in pixels from the top of the column.
    ///
    /// While the whole document fits, the bars start after the map's padding and
    /// stop where the last line stops: the column below them is empty, and it
    /// belongs to no line. Once the map slides, the bars cover the column and this
    /// is `0`.
    pub fn bars_top(&self) -> f32 {
        (MINIMAP_PAD - self.offset).max(0.0)
    }

    /// Bottom of the last bar on screen, in pixels from the top of the column.
    pub fn bars_bottom(&self) -> f32 {
        let b = (MINIMAP_PAD + self.content_h - self.offset).min(self.view_h);
        b.max(self.bars_top())
    }

    /// The stretch of the column the bars really cover.
    ///
    /// This - not the column - is what the map *is*. A short document draws a few
    /// thin lines at the top of a tall empty column, and the empty part below them
    /// stands for nothing at all, so neither the handle nor the pointer may be
    /// mapped through it. Never smaller than a grab target, so a two-line map can
    /// still be dragged.
    pub fn bars_h(&self) -> f32 {
        (self.bars_bottom() - self.bars_top()).max(MIN_HANDLE_H)
    }

    /// Top of the scrollbar track.
    ///
    /// A map that fits its strip is tracked from its first bar, so the handle rides the
    /// text. A map that overflows is tracked from the top of the strip: its top padding
    /// stops existing the moment it starts sliding (`bars_top` becomes 0 as `offset` grows
    /// past the padding), and a track whose top moves by four pixels mid-drag makes the
    /// handle lurch on the first pixel of the press - which is exactly what made the same
    /// drag scroll 166 px at one height and 118 at another.
    pub fn track_top(&self) -> f32 {
        if self.content_h > self.room() + 1.0 {
            0.0
        } else {
            self.bars_top()
        }
    }

    /// Height of the scrollbar track: the bars.
    ///
    /// The handle and the drag stay where the lines are: on a document long enough to fill
    /// the strip that is the whole column, and on a short one it is the stub of text at the
    /// top, so a drag never happens "on the empty area after the text". The price is that
    /// for a short document the handle only moves inside that stub.
    pub fn track_h(&self) -> f32 {
        if self.content_h > self.room() + 1.0 {
            self.room()
        } else {
            self.bars_h().min(self.room())
        }
    }

    pub fn handle_travel(&self, total_doc_h: f32, text_h: f32) -> f32 {
        (self.track_h() - self.view_h_px(total_doc_h, text_h)).max(1.0)
    }

    /// Scroll position that puts the handle's *middle* at a map y.
    ///
    /// The inverse of [`MinimapGeom::indicator_y`], used by the press: pressing
    /// anywhere in the column puts the handle's middle under the finger, so what is
    /// under the finger comes into view.
    pub fn scroll_at_handle_y(&self, y: f32, total_doc_h: f32, text_h: f32) -> f32 {
        let view = self.view_h_px(total_doc_h, text_h);
        let travel = self.handle_travel(total_doc_h, text_h);
        let max_scroll = (total_doc_h - text_h).max(0.0);
        let t = ((y - self.track_top() - view * 0.5) / travel).clamp(0.0, 1.0);
        t * max_scroll
    }

    /// Scroll position for a handle whose top is `top` pixels below the track's top.
    pub fn scroll_at_handle_top(&self, top: f32, total_doc_h: f32, text_h: f32) -> f32 {
        let travel = self.handle_travel(total_doc_h, text_h);
        let max_scroll = (total_doc_h - text_h).max(0.0);
        (top / travel).clamp(0.0, 1.0) * max_scroll
    }

    /// Top of the viewport indicator, in map pixels, for a scroll position.
    ///
    /// One function for the drawing and for the tests, so the handle on screen and
    /// the handle the pointer drags can never be two different things.
    pub fn indicator_y(&self, scroll: f32, total_doc_h: f32, text_h: f32) -> f32 {
        let travel = self.handle_travel(total_doc_h, text_h);
        let max_scroll = (total_doc_h - text_h).max(0.0);
        let t = if max_scroll > 1.0 {
            (scroll / max_scroll).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let top = self.track_top();
        let room = (self.track_h() - self.view_h_px(total_doc_h, text_h)).max(0.0);
        top + room.min(travel) * t
    }

    /// Virtual coordinate of a document y in pixels.
    pub fn virtual_of_doc(layout: &Layout, vtop: &[f32], y: f32) -> f32 {
        let i = layout.line_at_y(y).min(layout.lines.len().saturating_sub(1));
        let Some(l) = layout.lines.get(i) else { return 0.0 };
        let frac = if l.height > 0.0 { ((y - l.top) / l.height).clamp(0.0, 1.0) } else { 0.0 };
        vtop[i] + frac * minimap_virtual_span(l)
    }

}

/// Thickness of one minimap bar, in pixels - constant for every document.
pub const MINIMAP_BAR_H: f32 = 2.0;

/// Geometry of the minimap for `layout` inside `rect`, with the map showing the
/// part of the document around `scroll` (in document pixels, `text_h` tall).
pub fn minimap_geom_at(layout: &Layout, rect: Rect, scroll: f32, text_h: f32) -> MinimapGeom {
    // The number of *rows* in the document, not the position where the last line starts.
    //
    // It used to be `tops.last() + 1`, which is the virtual top of the last line - so a
    // file whose text wraps across many rows inside one line counted as **one row**. A
    // single line of ten thousand characters is a hundred and eleven rows on the page and
    // claimed one row on the map: the bars came out three pixels tall, the map looked
    // like a stub, and the handle had ten pixels of travel. Every "the handle only moves a
    // bit" report for the last ten rounds traces back to this number.
    // Both numbers come from the layout, which totals them once when it is built: adding
    // up ten thousand lines here, on every frame, was most of the minimap's cost.
    let vtotal = layout.virtual_rows.max(1.0);
    let widest = layout.widest_row_chars.max(8);
    let room = (rect.h - MINIMAP_PAD - 2.0).max(8.0);
    // One pitch, always: three pixels a row, every document, no matter how long.
    //
    // The pitch used to shrink so that more of a long file fitted, down to 0.85 px a row -
    // and at a pitch that is not a whole number of pixels the bars stop being drawn on a
    // clean grid: the row that survives the collision check changes from frame to frame,
    // so the strip shimmers (moire) and the lines look thinner than they are. Lines of one
    // constant size, one constant gap apart, is what a minimap is for. So the pitch never
    // changes; a document too long for the column fills it, and the map *slides* with the
    // page - it is a window on the document, and dragging moves the window.
    let _ = (scroll, text_h);
    let scale = MINIMAP_PITCH;
    let content_h = vtotal * scale;
    let progress = if layout.total_height > text_h + 1.0 {
        (scroll / (layout.total_height - text_h)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let offset = ((content_h - room).max(0.0) * progress).round();
    MinimapGeom {
        vtotal,
        scale,
        content_h,
        view_h: rect.h,
        // A constant two pixels: the bars are lines, and a line that changes thickness
        // with the document is what made a long file look like a grey smear.
        bar_h: MINIMAP_BAR_H,
        max_row: widest,
        offset,
    }
}

/// Geometry of the minimap with the document scrolled to its top.
pub fn minimap_geom(layout: &Layout, rect: Rect) -> MinimapGeom {
    minimap_geom_at(layout, rect, 0.0, rect.h)
}

/// Pixels of empty space above the first minimap bar.
pub const MINIMAP_PAD: f32 = 4.0;

/// Travel (in pixels) a thumb leaves itself in its track, even when the track is short.
pub const MIN_TRAVEL_PX: f32 = 40.0;

/// Smallest thumb a short document's map may be squeezed to so it can be dragged.
pub const MIN_STUB_HANDLE_H: f32 = 12.0;

/// Shortest a minimap handle is ever drawn: below this it is hard to see, and the
/// only thing it still has to do is be grabbable.
pub const MIN_HANDLE_H: f32 = 22.0;

/// Vertical spans `(depth, top, bottom)` of the quote bars for the lines
/// `first..end`, in document pixels.
///
/// A bar is continuous for as long as the run stays at least that deep, so the
/// rows of one quote read as a single line on the page instead of a dashed one.
pub fn quote_bar_spans(layout: &Layout, first: usize, end: usize) -> Vec<(usize, f32, f32)> {
    let run = match layout.lines.get(first..end) {
        Some(r) if !r.is_empty() => r,
        _ => return Vec::new(),
    };
    let max_depth = run.iter().map(|l| l.factor_depth()).max().unwrap_or(1);
    let mut out = Vec::new();
    for d in 0..max_depth {
        let mut start: Option<f32> = None;
        let mut bottom = 0.0f32;
        for l in run {
            let has = l.factor_depth() > d;
            if has {
                start.get_or_insert(l.top);
                bottom = l.top + l.height;
            } else if let Some(y0) = start.take() {
                out.push((d, y0, bottom));
            }
        }
        if let Some(y0) = start {
            out.push((d, y0, bottom));
        }
    }
    out
}

/// Paints the grid of a table: a rounded card, vertical column separators and a
/// rule under the header row. The cell text itself is drawn by the normal text
/// pass, whose runs already sit at the column offsets - that is what keeps the
/// caret and the mouse in sync with what you see.
pub fn draw_table_grid(
    app: &App,
    p: &mut Painter,
    text: Rect,
    origin_y: f32,
    scroll_x: f32,
    first: usize,
    late: &mut Vec<(Rect, Hit)>,
) {
    let layout = &app.tab().layout;
    let theme = p.theme;
    let mut i = first;
    while i < layout.lines.len() && layout.lines[i].block() == Block::Table {
        i += 1;
    }
    let group = &layout.lines[first..i];
    let Some(line0) = group.first() else { return };
    let total = line0.table_width;
    if total <= 0.0 || line0.table_cols.is_empty() {
        return;
    }
    // Drawn once per run, always from the first line of the run (the caller walks
    // back to it), so scrolling inside a long table never loses the grid.
    let top = origin_y + line0.top - CARD_PAD_Y;
    let bottom = origin_y + group.last().map(|l| l.top + l.height).unwrap_or(line0.top) + CARD_PAD_Y * 1.5;
    // The box starts where the *text* starts: every cell is painted at
    // `text.x + line.indent + table_cols[ci]`, so a grid drawn without the indent
    // sat ten pixels to the left of its own contents - the header's words looked
    // pushed towards the middle of their cells.
    let x0 = text.x - scroll_x + line0.indent - 2.0;
    let w = total.min(text.w + 8.0);
    // Just the lines: a table is a grid on the page, not a card sitting on it.
    let card = Rect::new(x0, top, w, bottom - top);

    // ---- the ruled grid: where the horizontal rules are ----
    // Computed before the verticals, because the box they belong to is the one the
    // rules close: they run from the rule above the first row to the rule under
    // the last one, so nothing sticks out past the horizontal border.
    let mut first_row_top: Option<f32> = None;
    let mut last_row_bottom = card.y;
    let mut prev_bottom: Option<f32> = None;
    let mut row_rules: Vec<(f32, bool)> = Vec::new(); // (y, header separator?)
    for l in group {
        let ly = origin_y + l.top;
        if l.info.table_sep {
            row_rules.push((ly + l.height * 0.5, true));
            prev_bottom = None;
            continue;
        }
        if let Some(pb) = prev_bottom {
            row_rules.push((pb - 0.5, false));
        }
        first_row_top.get_or_insert(ly);
        last_row_bottom = ly + l.height;
        prev_bottom = Some(ly + l.height);
    }
    let grid_top = first_row_top.unwrap_or(card.y) - 1.0;
    let grid_bottom = last_row_bottom - 0.2;

    // ---- vertical rules, one per column edge ----
    let edge = |x: f32| x0 + x;
    let rule = theme.border_strong.with_alpha(0.75);
    for &col in line0.table_cols.iter() {
        p.vline(edge(col - TABLE_CELL_PAD), grid_top, grid_bottom - grid_top, rule);
    }
    p.vline(edge(w), grid_top, grid_bottom - grid_top, rule);

    // ---- hover widgets: + row, + column, and the row/column grips ----
    draw_table_widgets(
        app,
        p,
        text,
        origin_y,
        scroll_x,
        first,
        first + group.len() - 1,
        x0,
        w,
        &group,
        late,
    );

    // ---- pictures inside cells ----
    // A cell whose whole content is `![alt](src)` paints the picture, at the same
    // left edge its text would have. While the caret is in that cell the source is
    // shown instead (the layout drops the picture then), so the two never stack.
    for (idx, l) in group.iter().enumerate() {
        let abs = first + idx;
        if app.tab().table_shows_markers(app.mode, abs) {
            continue;
        }
        let ly = origin_y + l.top + TABLE_CELL_PAD;
        for (ci, img) in &l.table_images {
            let Some(&cx) = line0.table_cols.get(*ci) else { continue };
            let x = text.x - scroll_x + l.indent + cx;
            let box_rect = Rect::new(x, ly, img.w, img.h);
            if img.missing {
                p.round(box_rect, 5.0, theme.bg_elevated);
                p.stroke(box_rect, 5.0, 1.0, theme.border_strong);
                let size = theme.ui_small();
                let label = if img.alt.is_empty() { "image not found" } else { &img.alt };
                let b = p.baseline_in(box_rect, size);
                p.text(box_rect.x + 6.0, b, label, Face::Regular, size, theme.text_dim);
            } else {
                let max_px = (1600.0 * p.r.scale) as u32;
                match p.r.image(&img.path, max_px) {
                    Some(tex) => p.r.draw_image(box_rect, tex, 4.0, Color::rgba(1.0, 1.0, 1.0, 1.0)),
                    None => p.stroke(box_rect, 4.0, 1.0, theme.border_strong),
                }
            }
        }
    }

    // ---- horizontal rules between rows ----
    for &(y, is_header) in &row_rules {
        if is_header {
            p.rect(Rect::new(x0 + 1.0, y, w - 2.0, 1.4), theme.text_dim.with_alpha(0.7));
        } else {
            p.rect(Rect::new(x0 + 1.0, y, w - 2.0, 1.0), rule.with_alpha(0.45));
        }
    }
    // The grid is closed: a table has a rule above its first row and below its
    // last one, exactly like the rules between the rows. They span the full width
    // so they meet the verticals exactly, with no overhang in either direction.
    p.rect(Rect::new(x0, grid_top, w, 1.2), rule);
    p.rect(Rect::new(x0, grid_bottom, w, 1.2), rule);
}

/// Screen y of every horizontal rule a table draws: the rule above the first
/// row, the header separator, the gaps between rows and the rule under the last
/// row. Shared with the tests so "the grid is closed" is checkable.
pub fn table_rule_ys(layout: &Layout, origin_y: f32, scroll: f32) -> Vec<f32> {
    let Some(first) = layout.lines.iter().position(|l| l.block() == Block::Table) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut prev_bottom: Option<f32> = None;
    let mut first_top: Option<f32> = None;
    let mut last_bottom = 0.0f32;
    for l in layout.lines.iter().skip(first).take_while(|l| l.block() == Block::Table) {
        let ly = origin_y + l.top - scroll;
        if l.info.table_sep {
            out.push(ly + l.height * 0.5);
            prev_bottom = None;
            continue;
        }
        if let Some(pb) = prev_bottom {
            out.push(pb - 0.5);
        }
        first_top.get_or_insert(ly);
        last_bottom = ly + l.height;
        prev_bottom = Some(ly + l.height);
    }
    if let Some(top) = first_top {
        out.push(top - 1.0);
    }
    out.push(last_bottom - 0.2);
    out
}

/// Interactive chrome for a table: a `+` under the last row to add a row, a `+`
/// to the right to add a column, plus grips that open the row / column menus.
#[allow(clippy::too_many_arguments)]
pub fn draw_table_widgets(
    app: &App,
    p: &mut Painter,
    text: Rect,
    origin_y: f32,
    scroll_x: f32,
    first: usize,
    last: usize,
    x0: f32,
    table_w: f32,
    group: &[LineLayout],
    late: &mut Vec<(Rect, Hit)>,
) {
    // Only while the pointer is somewhere over the table: the widgets are chrome
    // for the table under the mouse, not decoration for every table on the page.
    let Some(line0) = group.first() else { return };
    let Some(line_last) = group.last() else { return };
    let top = origin_y + line0.top;
    let bottom = origin_y + line_last.top + line_last.height;
    let rect = Rect::new(x0, top, table_w, bottom - top);

    // The rows the grips belong to, in document-line order.
    let group_lines: Vec<&LineLayout> = group
        .iter()
        .zip(first..=last)
        .filter(|(l, _)| !l.info.table_sep)
        .map(|(l, _)| l)
        .collect();

    // While a grip is being dragged the chrome of *this* table stays visible (the
    // pointer is somewhere else by then) and a drop line shows where it lands.
    let drag = app.drag.filter(|d| d.moved && d.first == first || d.moved && d.kind == crate::app::DragKind::TableRow && d.first == first);
    if let Some(d) = drag {
        match d.kind {
            crate::app::DragKind::TableCol => {
                if let Some(&cx) = group.first().and_then(|l| l.table_cols.get(d.to)) {
                    let x = x0 + cx - TABLE_CELL_PAD;
                    p.rect(Rect::new(x - 1.5, rect.y - 4.0, 3.0, rect.h + 8.0), p.theme.accent);
                }
            }
            crate::app::DragKind::TableRow => {
                if let Some(line) = group_lines.get(d.to) {
                    let y = origin_y + line.top;
                    let edge = if d.to > d.from { y + line.height } else { y };
                    p.rect(Rect::new(rect.x - 20.0, edge - 1.5, rect.w + 40.0, 3.0), p.theme.accent);
                }
            }
            _ => {}
        }
    }

    let near = Rect::new(rect.x - 26.0, rect.y - 26.0, rect.w + 56.0, rect.h + 56.0);
    if !near.contains(app.mouse) && drag.is_none() {
        return;
    }
    let size = 16.0;
    // "+" under the table adds a row at the end; the one on the right adds a
    // column at the end. Both sit just outside the card so they never cover a
    // cell.
    let plus_row = Rect::new(rect.x + rect.w * 0.5 - size * 0.5, rect.bottom() + 5.0, size, size);
    let plus_col = Rect::new(rect.right() + 7.0, rect.y + rect.h * 0.5 - size * 0.5, size, size);
    for (r, action, line) in [
        (plus_row, crate::app::TableAction::InsertRowBelow, last),
        (plus_col, crate::app::TableAction::InsertColRight, first),
    ] {
        let hot = p.hovered(r);
        p.round(r, 4.0, if hot { p.theme.text } else { p.theme.bg_elevated });
        if !hot {
            p.stroke(r, 4.0, 1.0, p.theme.border_strong);
        }
        let c = if hot { p.theme.accent_text } else { p.theme.text_dim };
        p.rect(Rect::new(r.x + 3.5, r.y + r.h * 0.5 - 0.8, r.w - 7.0, 1.6), c);
        p.rect(Rect::new(r.x + r.w * 0.5 - 0.8, r.y + 3.5, 1.6, r.h - 7.0), c);
        late.push((r, Hit::TableZone(action, line)));
    }

    // Reordering works off **one** grip at a time: the row under the pointer and
    // the column under the pointer. Drawing a grip beside every row and on top of
    // every column buried the table in little handles (a four-row, three-column
    // table had seven of them) and made the table itself hard to read.
    // Which row the pointer is on: the one whose middle is nearest, so parking on
    // the thin header separator still gives you the row you meant.
    // The pad is wide enough to cover the grips and the `+` buttons, which sit
    // just outside the grid. Without it the grip vanished as soon as the pointer
    // reached it, so it could never be clicked or dragged.
    let inside = |pad: f32| {
        app.mouse.0 >= rect.x - pad
            && app.mouse.0 <= rect.right() + pad
            && app.mouse.1 >= rect.y - pad
            && app.mouse.1 <= rect.bottom() + pad
    };
    let hovered_row = if inside(24.0) {
        group_lines
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                let da = (origin_y + a.top + a.height * 0.5 - app.mouse.1).abs();
                let db = (origin_y + b.top + b.height * 0.5 - app.mouse.1).abs();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)
    } else {
        None
    };
    // Which column: the last one whose left edge is at or left of the pointer, so
    // anywhere inside a cell (and anywhere right of the table) answers.
    let cols = group.first().map(|l| l.table_cols.clone()).unwrap_or_default();
    let hovered_col = if inside(24.0) {
        let mut pick = None;
        for (i, &cx) in cols.iter().enumerate() {
            if app.mouse.0 >= x0 + cx - TABLE_CELL_PAD - 2.0 {
                pick = Some(i);
            }
        }
        pick
    } else {
        None
    };
    let dragged_row = app
        .drag
        .filter(|d| d.moved && d.kind == crate::app::DragKind::TableRow)
        .map(|d| d.from);
    let dragged_col = app
        .drag
        .filter(|d| d.moved && d.kind == crate::app::DragKind::TableCol)
        .map(|d| d.from);

    if let Some(offset) = hovered_row.or(dragged_row) {
        if let Some(line) = group_lines.get(offset) {
            let y = origin_y + line.top;
            let grip = Rect::new(rect.x - 17.0, y + (line.height - 14.0) * 0.5, 13.0, 14.0);
            let hot = p.hovered(grip) || dragged_row == Some(offset);
            if hot {
                p.round(grip, 3.0, p.theme.bg_active);
            }
            let c = if hot { p.theme.text } else { p.theme.text_faint.with_alpha(0.7) };
            p.icon("grip", grip, c);
            late.push((grip, Hit::RowGrip(first, offset)));
        }
    }
    if let Some(ci) = hovered_col.or(dragged_col) {
        if let Some(&cx) = cols.get(ci) {
            let grip = Rect::new(x0 + cx - TABLE_CELL_PAD, rect.y - 16.0, 13.0, 13.0);
            let hot = p.hovered(grip) || dragged_col == Some(ci);
            if hot {
                p.round(grip, 3.0, p.theme.bg_active);
            }
            let c = if hot { p.theme.text } else { p.theme.text_faint.with_alpha(0.7) };
            p.icon("grip", grip, c);
            late.push((grip, Hit::ColGrip(first, ci)));
        }
    }
    let _ = (scroll_x, text);
}

// ---------------------------------------------------------------- minimap

/// A minimap made of thin bars: one bar per *visual* row, width proportional to
/// how full that row is, so it reads like shrunken text rather than blocks.
pub fn draw_minimap(app: &App, p: &mut Painter, rect: Rect, text: Rect) {
    let theme = p.theme;
    p.rect(rect, theme.minimap_bg);
    p.vline(rect.x, rect.y, rect.h, theme.border);

    let tab = app.tab();
    let layout = &tab.layout;
    let total = layout.total_height.max(1.0);
    let inner_x = rect.x + 8.0;
    let inner_w = (rect.w - 14.0).max(8.0);

    // The minimap is not a scale model of the page: block spacing (a heading's
    // 22 px of air above it, a card's padding) would turn into big empty gaps
    // between the bars. Bars are placed on a *virtual* document that only counts
    // text rows, so headings sit right next to the lines around them.
    //
    // The virtual position is accumulated as the lines are walked instead of being
    // tabulated first: the table was a `Vec<f32>` of the whole document, built twice per
    // frame (once here, once by `minimap_geometry`), and the widest row was a third full
    // scan. On a ten-thousand-line file that was most of the minimap's frame cost.
    let geom = minimap_geom_at(layout, rect, tab.scroll, text.h);
    let scale_y = geom.scale;
    let max_row = geom.max_row;
    let bar_w = |chars: usize| -> f32 {
        let f = (chars as f32 / max_row as f32).clamp(0.0, 1.0);
        (f.powf(0.55) * inner_w).clamp(2.0, inner_w)
    };

    p.r.push_clip(rect);
    // The bar height is a *constant*, not a fraction of the document: mapping it
    // through the scroll scale made every bar change height as soon as a line
    // was added anywhere in the file.
    let bar_h = geom.bar_h;
    let mut last_y = f32::NEG_INFINITY;
    // Virtual row the current line starts at; advanced by every line, blank or not, so it
    // stays the same number `minimap_virtual_tops` would have tabulated.
    let mut v = 0.0f32;
    for line in layout.lines.iter() {
        let span = minimap_virtual_span(line);
        let base_y = rect.y + geom.y_of_virtual(v);
        v += span;
        // Everything below this line is below the strip: stop walking the file.
        if base_y > rect.bottom() + 1.0 {
            break;
        }
        if line.block() == Block::Blank {
            continue;
        }
        // A picture or a diagram is not text: it gets an outlined block the height
        // it really takes on the page, which is what keeps the map honest around
        // images instead of squeezing the lines between two of them together.
        if line.image.is_some() || line.diagram.is_some() {
            let h = span * scale_y;
            let y = base_y;
            if y < rect.bottom() && y + h > rect.y {
                let block = Rect::new(inner_x, y, inner_w * 0.86, h.max(3.0));
                p.round(block, 2.0, theme.minimap_line.with_alpha(0.35));
                last_y = y + h;
            }
            continue;
        }
        let (color, weight) = match line.block() {
            Block::Heading(_) => (theme.text.with_alpha(0.9), 1.15),
            Block::Code | Block::Fence => (theme.text_dim.with_alpha(0.55), 0.95),
            Block::Quote => (theme.text_dim.with_alpha(0.5), 0.95),
            _ => (theme.minimap_line, 1.0),
        };
        for (ri, row) in line.rows.iter().enumerate() {
            let chars = row.end.saturating_sub(row.start);
            if chars == 0 {
                continue;
            }
            let y = base_y + ri as f32 * scale_y;
            // Bars never touch: on a long document the pitch is finer than a bar,
            // and skipping the row that would collide keeps the map readable
            // instead of turning it into a solid block.
            if y < last_y + bar_h + 1.0 {
                continue;
            }
            if y > rect.y + rect.h - bar_h || y < rect.y - 2.0 {
                continue;
            }
            last_y = y;
            let w = (bar_w(chars) * weight).min(inner_w);
            p.rect(Rect::new(inner_x, y, w, bar_h), color);
        }
    }
    // viewport indicator: a soft wash plus a hairline frame, never a solid block.
    // It is the document's own top and height through the same mapping the bars
    // use, so it cannot disagree with them about where the page is.
    if total > text.h + 1.0 {
        // While a drag is in flight the handle is drawn at the exact screen `y` the
        // drag holds - the same number the page's scroll was computed from - so the
        // handle on screen and the page behind it cannot disagree.
        let y0 = app
            .map_handle
            .unwrap_or_else(|| rect.y + geom.indicator_y(tab.scroll, total, text.h));
        let view_h = geom.view_h_px(total, text.h);
        // The thumb lives on the text: it never slides into the empty column below
        // the last bar, which stands for no line.
        let b_top = rect.y + geom.track_top();
        let b_bottom = rect.y + geom.track_top() + geom.track_h();
        let view_y = y0.clamp(b_top, (b_bottom - view_h).max(b_top));
        let v = Rect::new(rect.x + 1.0, view_y, rect.w - 2.0, view_h.min(rect.h));
        p.round(v, 3.0, theme.minimap_view);
        p.stroke(v, 3.0, 1.0, theme.scrollbar);
    }
    p.r.pop_clip();
    // The whole column answers the pointer: the handle travels all of it, so every
    // point of it is a place the handle can be pressed to.
    p.hit(rect, Hit::Minimap);
}
