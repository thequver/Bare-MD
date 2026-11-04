//! `Painter`: the one door between drawing code and the renderer.
//!
//! It carries the theme, the pointer position and the hit map, and every call
//! that puts something on screen goes through it, so a control cannot forget to
//! register itself.

use super::*;

pub struct Painter<'a> {
    pub r: &'a mut Renderer,
    pub theme: Theme,
    pub mouse: (f32, f32),
    pub hits: Vec<(Rect, Hit)>,
    /// Hints collected while drawing, painted in [`flush_tooltips`] once the
    /// whole frame is down: a hint drawn where the button sits would be covered
    /// by whatever is painted after the tab strip.
    pub tooltips: Vec<(Rect, String)>,
}

impl<'a> Painter<'a> {
    pub fn new(r: &'a mut Renderer, theme: Theme, mouse: (f32, f32)) -> Self {
        Painter { r, theme, mouse, hits: Vec::new(), tooltips: Vec::new() }
    }

    pub fn hovered(&self, rect: Rect) -> bool {
        rect.contains(self.mouse)
    }

    pub fn hit(&mut self, rect: Rect, h: Hit) {
        self.hits.push((rect, h));
    }

    pub fn rect(&mut self, rect: Rect, c: Color) {
        self.r.rect(rect, c);
    }

    pub fn round(&mut self, rect: Rect, radius: f32, c: Color) {
        self.r.round_rect(rect, radius, c);
    }

    pub fn stroke(&mut self, rect: Rect, radius: f32, w: f32, c: Color) {
        self.r.stroke_rect(rect, radius, w, c);
    }

    /// A panel: fill and border in **one** quad.
    ///
    /// `round` followed by `stroke` paints the same area twice - two full passes of
    /// fragment work, and the border pixels blended over the fill. On a software
    /// rasteriser (a VM, a machine with no GPU driver) that was a third of the cost of
    /// painting a dialog, and a dialog that does not scroll smoothly in a VM is exactly
    /// the complaint this fixes.
    pub fn panel(&mut self, rect: Rect, radius: f32, fill: Color, w: f32, border: Color) {
        self.r.rect_bordered(rect, radius, fill, w, border);
    }

    pub fn text(&mut self, x: f32, baseline: f32, s: &str, face: Face, size: f32, c: Color) -> f32 {
        self.r.text_run(x, baseline, s, face, size, c)
    }


    /// Text shaped and painted in its own direction (see
    /// [`crate::gpu::Renderer::text_run_weighted_dir`]).
    #[allow(clippy::too_many_arguments)]
    pub fn text_directed(
        &mut self,
        x: f32,
        baseline: f32,
        s: &str,
        face: Face,
        size: f32,
        c: Color,
        weight: f32,
        rtl: bool,
    ) -> f32 {
        self.r.text_run_weighted_dir(x, baseline, s, face, size, c, weight, rtl)
    }

    /// A single-line text field: placeholder, selection wash and caret, all
    /// measured from the same string so the caret sits where the glyphs do.
    ///
    /// Returns nothing: the caller registers the hit itself.
    pub fn field(
        &mut self,
        rect: Rect,
        text: &str,
        placeholder: &str,
        selection: Option<(usize, usize)>,
        caret: Option<usize>,
        focused: bool,
        hit: Hit,
    ) {
        let size = self.theme.ui_size();
        self.round(rect, 5.0, self.theme.bg);
        if focused {
            self.stroke(rect, 5.0, 1.0, self.theme.accent);
        }
        let x0 = rect.x + 8.0;
        let b = self.baseline_in(rect, size);
        let chars: Vec<char> = text.chars().collect();
        let width_of = |p: &mut Painter, up_to: usize| -> f32 {
            if up_to == 0 {
                return 0.0;
            }
            let prefix: String = chars[..up_to.min(chars.len())].iter().collect();
            p.width(&prefix, Face::Regular, size)
        };
        // The selection wash goes *under* the text. Painting it afterwards covered
        // the glyphs with an opaque bar, which is why a selected word in the find
        // box looked like a solid block of colour.
        if let Some((a, b_sel)) = selection {
            let xa = x0 + width_of(self, a);
            let xb = x0 + width_of(self, b_sel);
            self.rect(
                Rect::new(xa, rect.y + 3.0, (xb - xa).max(1.5), rect.h - 6.0),
                self.theme.selection,
            );
        }
        if text.is_empty() {
            self.text(x0, b, placeholder, Face::Regular, size, self.theme.text_faint);
        } else {
            self.text(x0, b, text, Face::Regular, size, self.theme.text);
        }
        if focused {
            if let Some(c) = caret {
                let x = x0 + width_of(self, c);
                self.rect(Rect::new(x, rect.y + 4.0, 1.5, rect.h - 8.0), self.theme.caret);
            }
        }
        self.hit(rect, hit);
    }

    /// Queue a one-line hint under a button. Painted at the end of the frame by
    /// [`flush_tooltips`], so nothing drawn later can cover it.
    pub fn tooltip(&mut self, anchor: Rect, text: &str) {
        self.tooltips.push((anchor, text.to_string()));
    }

    /// Draw an icon from `assets/icons/<name>.svg`, tinted with `color`.
    ///
    /// Rasterised at the *physical* pixel height so it stays crisp on hi-dpi
    /// displays, then cached in the renderer's atlas.
    pub fn icon(&mut self, name: &str, rect: Rect, color: Color) -> bool {
        let s = self.r.scale;
        // Everything below is in physical pixels: an SVG icon is rasterised at
        // exactly the size it is drawn, so its texels land on screen pixels one
        // for one. Drawing it a fraction of a pixel off - or a pixel or two
        // smaller than it was rasterised - resamples it and every 1-pixel stroke
        // turns into two grey ones, which is what made the toolbar look soft.
        let slot_h = (rect.h * s).round().max(8.0);
        let Some(mut tex) = self.r.icon(name, slot_h as u32) else {
            // A missing or broken SVG draws a faint box instead of nothing, so
            // the button is never invisible.
            self.stroke(rect, 2.0, 1.0, color.with_alpha(0.35));
            return false;
        };
        // Keep the icon's own aspect ratio inside the slot.
        let aspect = tex.w as f32 / tex.h.max(1) as f32;
        let mut w = (rect.w * s).round();
        let mut h = slot_h;
        if w / h > aspect {
            w = (h * aspect).round().max(1.0);
        } else {
            h = (w / aspect).round().max(1.0);
        }
        // A slot that is wider than it is tall draws the icon smaller than it was
        // rasterised: ask for that size instead of scaling the bitmap down.
        if (h - tex.h as f32).abs() > 0.5 && h >= 8.0 {
            if let Some(t) = self.r.icon(name, h as u32) {
                tex = t;
                w = tex.w as f32;
                h = tex.h as f32;
            }
        }
        let ox = ((rect.x * s) + ((rect.w * s) - w) * 0.5).round() / s;
        let oy = ((rect.y * s) + ((rect.h * s) - h) * 0.5).round() / s;
        self.r.draw_icon(Rect::new(ox, oy, w / s, h / s), tex, color);
        true
    }

    pub fn width(&mut self, s: &str, face: Face, size: f32) -> f32 {
        self.r.text_width(s, face, size)
    }

    /// Baseline that vertically centres text of `size` inside `rect`.
    pub fn baseline_in(&mut self, rect: Rect, size: f32) -> f32 {
        let (asc, desc) = self.r.metrics(Face::Regular, size);
        rect.y + (rect.h - (asc + desc)) * 0.5 + asc
    }

    /// Left-aligned, vertically centred label with padding.
    pub fn label(&mut self, rect: Rect, pad: f32, s: &str, face: Face, size: f32, c: Color) -> f32 {
        let b = self.baseline_in(rect, size);
        let x = rect.x + pad;
        self.text(x, b, s, face, size, c)
    }

    pub fn label_center(&mut self, rect: Rect, s: &str, face: Face, size: f32, c: Color) {
        let w = self.width(s, face, size);
        let b = self.baseline_in(rect, size);
        self.text(rect.x + (rect.w - w) * 0.5, b, s, face, size, c);
    }

    pub fn label_right(&mut self, rect: Rect, pad: f32, s: &str, face: Face, size: f32, c: Color) {
        let w = self.width(s, face, size);
        let b = self.baseline_in(rect, size);
        self.text(rect.right() - pad - w, b, s, face, size, c);
    }

    pub fn hline(&mut self, x: f32, y: f32, w: f32, c: Color) {
        self.rect(Rect::new(x, y, w, 1.0), c);
    }

    pub fn vline(&mut self, x: f32, y: f32, h: f32, c: Color) {
        self.rect(Rect::new(x, y, 1.0, h), c);
    }

    /// Simple push-button with hover/pressed feedback.
    pub fn button(&mut self, rect: Rect, label: &str, active: bool, hit: Hit) {
        let hovered = self.hovered(rect);
        let bg = if active {
            self.theme.accent
        } else if hovered {
            self.theme.bg_hover
        } else {
            self.theme.bg_elevated
        };
        let fg = if active { self.theme.accent_text } else { self.theme.text_dim };
        self.round(rect, 5.0, bg);
        self.label_center(rect, label, Face::Regular, self.theme.ui_size(), fg);
        self.hit(rect, hit);
    }

    pub fn icon_button(&mut self, rect: Rect, active: bool, hit: Hit, draw: impl FnOnce(&mut Painter, Rect, Color)) {
        let hovered = self.hovered(rect);
        let bg = if active {
            self.theme.bg_active
        } else if hovered {
            self.theme.bg_hover
        } else {
            self.theme.bg_elevated
        };
        self.round(rect, 5.0, bg);
        let fg = if active { self.theme.accent } else { self.theme.text };
        draw(self, rect.inset(7.0), fg);
        self.hit(rect, hit);
    }
}

// ---------------------------------------------------------------- icons

// ---------------------------------------------------------------- chrome layout
