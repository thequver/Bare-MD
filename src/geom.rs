//! Basic geometry and colour helpers shared by the renderer and the UI.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const ZERO: Rect = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };

    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }

    pub fn contains(&self, p: (f32, f32)) -> bool {
        p.0 >= self.x && p.0 < self.right() && p.1 >= self.y && p.1 < self.bottom()
    }

    pub fn inset(&self, d: f32) -> Rect {
        Rect::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }

    pub fn inset_xy(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, (self.w - 2.0 * dx).max(0.0), (self.h - 2.0 * dy).max(0.0))
    }

    pub fn intersect(&self, o: Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.right().min(o.right());
        let y1 = self.bottom().min(o.bottom());
        Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }

    pub fn translate(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// Split off a horizontal band of `height` from the top, returning (band, rest).
    pub fn split_top(&self, height: f32) -> (Rect, Rect) {
        let h = height.min(self.h);
        (
            Rect::new(self.x, self.y, self.w, h),
            Rect::new(self.x, self.y + h, self.w, self.h - h),
        )
    }

    /// Split off a vertical band of `width` from the left, returning (band, rest).
    pub fn split_left(&self, width: f32) -> (Rect, Rect) {
        let w = width.min(self.w);
        (
            Rect::new(self.x, self.y, w, self.h),
            Rect::new(self.x + w, self.y, self.w - w, self.h),
        )
    }

    /// Split off a vertical band of `width` from the right, returning (band, rest).
    pub fn split_right(&self, width: f32) -> (Rect, Rect) {
        let w = width.min(self.w);
        (
            Rect::new(self.right() - w, self.y, w, self.h),
            Rect::new(self.x, self.y, self.w - w, self.h),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Color { r, g, b, a }
    }

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Color { r, g, b, a: 1.0 }
    }

    pub const WHITE: Color = Color::rgb(1.0, 1.0, 1.0);
    pub const TRANSPARENT: Color = Color::rgba(0.0, 0.0, 0.0, 0.0);

    /// Parse `#rrggbb` / `#rrggbbaa` (the leading `#` is optional).
    pub fn hex(s: &str) -> Color {
        let s = s.trim_start_matches('#');
        let parse = |i: usize| -> f32 {
            u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(255) as f32 / 255.0
        };
        match s.len() {
            6 => Color::rgb(parse(0), parse(2), parse(4)),
            8 => Color::rgba(parse(0), parse(2), parse(4), parse(6)),
            _ => Color::WHITE,
        }
    }

    pub fn with_alpha(&self, a: f32) -> Color {
        Color::rgba(self.r, self.g, self.b, a)
    }

    /// Linear blend towards `other`: `t = 0` keeps this colour, `t = 1` takes
    /// `other`'s. Used to derive a tint of the page without adding a colour to
    /// the theme for every one of them.
    pub fn mix(&self, other: Color, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        Color {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }

    pub fn scale(&self, f: f32) -> Color {
        Color::rgba(self.r * f, self.g * f, self.b * f, self.a)
    }

}

