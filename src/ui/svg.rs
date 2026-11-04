//! A very small SVG rasteriser, just big enough for the icon set.
//!
//! Bare MD ships its toolbar icons as plain `.svg` files under `assets/icons/`,
//! so they can be edited with any vector editor and without touching Rust. This
//! module reads that subset of SVG and turns it into an alpha bitmap that the
//! renderer uploads into its atlas and tints with the theme colour.
//!
//! Supported: `<svg viewBox>` (including a non-zero origin), `<g>` (attributes and
//! transforms inherit), `<path>` with the full set of drawing commands (M L H V
//! C S Q T A Z, absolute and relative), `<circle>`, `<ellipse>`, `<rect>` (with
//! `rx`), `<line>`, `<polyline>`, `<polygon>` (arcs honour the large-arc and sweep
//! flags), the attributes `fill`, `stroke`, `stroke-width`, `fill-rule`
//! (`nonzero` / `evenodd`), `opacity`, and `transform` (`translate`, `scale`,
//! `rotate`, `matrix`).
//!
//! Two outputs: [`rasterize`] gives coverage (an alpha mask, which is what a
//! theme-tinted button icon wants) and [`rasterize_rgba`] gives the file's own
//! colours (which is what the application icon wants).
//!
//! Not supported (and not needed by the icon set): gradients, images, text,
//! dashes, masks and filters. A file that uses them still rasterises, it just
//! ignores those parts.

#[derive(Clone, Debug)]
enum Shape {
    /// Closed polygons, filled with the given rule (true = even-odd), in `rgb`.
    Fill(Vec<Vec<(f32, f32)>>, bool, [u8; 3]),
    /// Open polylines, stroked with a round pen of the given width, in `rgb`.
    Stroke(Vec<Vec<(f32, f32)>>, f32, [u8; 3]),
}

#[derive(Clone, Debug)]
pub struct Svg {
    pub width: f32,
    pub height: f32,
    /// Top-left corner of `viewBox` (`min-x`, `min-y`).
    ///
    /// Content is drawn relative to it: an SVG whose viewBox starts at, say,
    /// `-24 -24` must shift its shapes by `+24`, or everything lands off the top
    /// left corner of the bitmap and the icon comes out cropped.
    pub origin: (f32, f32),
    shapes: Vec<Shape>,
}

/// A 2D affine transform, as `transform="…"` writes one.
#[derive(Clone, Copy, Debug)]
struct Mat {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Mat {
    fn identity() -> Mat {
        Mat { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 }
    }

    fn apply(&self, p: (f32, f32)) -> (f32, f32) {
        (
            self.a * p.0 + self.c * p.1 + self.e,
            self.b * p.0 + self.d * p.1 + self.f,
        )
    }

    /// `self` then `inner` (SVG composes left to right, so the leftmost transform
    /// is the outermost one).
    fn then(&self, inner: Mat) -> Mat {
        Mat {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            e: self.a * inner.e + self.c * inner.f + self.e,
            f: self.b * inner.e + self.d * inner.f + self.f,
        }
    }

    /// Average scale factor, for scaling a stroke width.
    fn scale(&self) -> f32 {
        let sx = (self.a * self.a + self.b * self.b).sqrt();
        let sy = (self.c * self.c + self.d * self.d).sqrt();
        ((sx + sy) * 0.5).max(0.0001)
    }

    /// Parse a `transform` attribute: `translate`, `scale`, `rotate`, `matrix`,
    /// possibly several of them in one string.
    fn parse(value: &str) -> Mat {
        let mut out = Mat::identity();
        let mut rest = value;
        while let Some(open) = rest.find('(') {
            let name: String = rest[..open]
                .trim_end()
                .rsplit(|c: char| c.is_whitespace() || c == ',')
                .next()
                .unwrap_or("")
                .to_string();
            let Some(close) = rest[open..].find(')') else { break };
            let args: Vec<f32> = rest[open + 1..open + close]
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse().ok())
                .collect();
            let m = match name.as_str() {
                "translate" => Mat {
                    e: args.first().copied().unwrap_or(0.0),
                    f: args.get(1).copied().unwrap_or(0.0),
                    ..Mat::identity()
                },
                "scale" => {
                    let sx = args.first().copied().unwrap_or(1.0);
                    Mat { a: sx, d: args.get(1).copied().unwrap_or(sx), ..Mat::identity() }
                }
                "rotate" => {
                    let (deg, cx, cy) = (
                        args.first().copied().unwrap_or(0.0),
                        args.get(1).copied().unwrap_or(0.0),
                        args.get(2).copied().unwrap_or(0.0),
                    );
                    let (sin, cos) = deg.to_radians().sin_cos();
                    let rot = Mat { a: cos, b: sin, c: -sin, d: cos, e: 0.0, f: 0.0 };
                    let to = Mat { e: cx, f: cy, ..Mat::identity() };
                    let back = Mat { e: -cx, f: -cy, ..Mat::identity() };
                    to.then(rot).then(back)
                }
                "matrix" if args.len() >= 6 => Mat {
                    a: args[0],
                    b: args[1],
                    c: args[2],
                    d: args[3],
                    e: args[4],
                    f: args[5],
                },
                _ => Mat::identity(),
            };
            out = out.then(m);
            rest = &rest[open + close + 1..];
        }
        out
    }
}

/// `#rgb`, `#rrggbb` and a couple of names; `None` for anything else.
fn parse_color(value: &str) -> Option<[u8; 3]> {
    let v = value.trim();
    if let Some(hex) = v.strip_prefix('#') {
        let digits: Vec<u8> = hex
            .chars()
            .filter_map(|c| c.to_digit(16).map(|d| d as u8))
            .collect();
        return match digits.len() {
            3 => Some([digits[0] * 17, digits[1] * 17, digits[2] * 17]),
            6 => Some([
                digits[0] * 16 + digits[1],
                digits[2] * 16 + digits[3],
                digits[4] * 16 + digits[5],
            ]),
            _ => None,
        };
    }
    match v {
        "black" => Some([0, 0, 0]),
        "white" => Some([255, 255, 255]),
        _ => v
            .strip_prefix("rgb(")
            .and_then(|r| r.strip_suffix(')'))
            .and_then(|r| {
                let n: Vec<u8> = r
                    .split(',')
                    .filter_map(|p| p.trim().parse::<f32>().ok())
                    .map(|f| f.clamp(0.0, 255.0) as u8)
                    .collect();
                (n.len() >= 3).then_some([n[0], n[1], n[2]])
            }),
    }
}

impl Svg {
    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    /// Number of shapes, for tests.
    pub fn shape_count(&self) -> usize {
        self.shapes.len()
    }
}

/// Parse an SVG document. Returns `None` when there is nothing drawable in it.
pub fn parse(src: &str) -> Option<Svg> {
    let box_attr = view_box(src);
    let mut shapes = Vec::new();
    let mut style = Style::default();
    let mut mat = Mat::identity();
    // A very small stack: `<g>` attributes and transforms apply to everything
    // inside it.
    let mut stack: Vec<(Style, Mat)> = Vec::new();

    let mut rest = src;
    while let Some(open) = rest.find('<') {
        rest = &rest[open..];
        if rest.starts_with("<!--") {
            rest = rest.find("-->").map(|i| &rest[i + 3..]).unwrap_or("");
            continue;
        }
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[..=end];
        rest = &rest[end + 1..];
        let closing = tag.starts_with("</");
        let name = tag
            .trim_start_matches('<')
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .next()
            .unwrap_or("")
            .to_string();
        if closing {
            match name.as_str() {
                "g" | "svg" => {
                    if let Some((s, m)) = stack.pop() {
                        style = s;
                        mat = m;
                    }
                }
                _ => {}
            }
            continue;
        }
        let self_closing = tag.ends_with("/>");
        match name.as_str() {
            "svg" => {
                // The root element carries the defaults every child inherits.
                style.merge(tag);
                if let Some(t) = attr(tag, "transform") {
                    mat = mat.then(Mat::parse(t));
                }
                continue;
            }
            "g" => {
                stack.push((style.clone(), mat));
                style.merge(tag);
                if let Some(t) = attr(tag, "transform") {
                    mat = mat.then(Mat::parse(t));
                }
                if self_closing {
                    if let Some((s, m)) = stack.pop() {
                        style = s;
                        mat = m;
                    }
                }
                continue;
            }
            _ => {}
        }

        let mut pen = style.clone();
        pen.merge(tag);
        let Some(points) = element_points(&name, tag) else { continue };
        if points.is_empty() {
            continue;
        }
        // A shape's own `transform` sits inside its group's.
        let shape_mat = match attr(tag, "transform") {
            Some(t) => mat.then(Mat::parse(t)),
            None => mat,
        };
        let points: Vec<Vec<(f32, f32)>> = points
            .iter()
            .map(|poly| poly.iter().map(|p| shape_mat.apply(*p)).collect())
            .collect();
        if pen.stroke && pen.stroke_width > 0.0 {
            shapes.push(Shape::Stroke(
                points.clone(),
                pen.stroke_width * shape_mat.scale(),
                pen.stroke_color,
            ));
        }
        if pen.fill {
            shapes.push(Shape::Fill(points, pen.even_odd, pen.fill_color));
        }
    }

    if shapes.is_empty() {
        return None;
    }
    // `viewBox` decides the canvas. Without one - a file drawn by hand - the
    // drawing's own bounding box does, which is the only way a shape at, say,
    // (0,0)-(256,256) is not scaled as if the canvas were 24 x 24 and then
    // cropped to its corner.
    let (origin, width, height) = match box_attr {
        Some((x, y, w, h)) if w > 0.0 && h > 0.0 => ((x, y), w, h),
        _ => match bbox(&shapes) {
            Some(((x, y), w, h)) => ((x, y), w, h),
            None => ((0.0, 0.0), 24.0, 24.0),
        },
    };
    Some(Svg { width, height, origin, shapes })
}

/// Bounding box of every shape, `((min_x, min_y), width, height)`.
fn bbox(shapes: &[Shape]) -> Option<((f32, f32), f32, f32)> {
    let mut min = (f32::MAX, f32::MAX);
    let mut max = (f32::MIN, f32::MIN);
    let mut seen = false;
    for shape in shapes {
        let polys: &[Vec<(f32, f32)>] = match shape {
            Shape::Fill(polys, _, _) | Shape::Stroke(polys, _, _) => polys,
        };
        for p in polys.iter().flatten() {
            seen = true;
            min = (min.0.min(p.0), min.1.min(p.1));
            max = (max.0.max(p.0), max.1.max(p.1));
        }
    }
    if !seen {
        return None;
    }
    Some((min, (max.0 - min.0).max(1.0), (max.1 - min.1).max(1.0)))
}

#[derive(Clone, Debug)]
struct Style {
    fill: bool,
    stroke: bool,
    stroke_width: f32,
    even_odd: bool,
    opacity: f32,
    fill_color: [u8; 3],
    stroke_color: [u8; 3],
}

impl Default for Style {
    fn default() -> Self {
        // SVG defaults: black fill, no stroke.
        Style {
            fill: true,
            stroke: false,
            stroke_width: 1.0,
            even_odd: false,
            opacity: 1.0,
            fill_color: [0, 0, 0],
            stroke_color: [0, 0, 0],
        }
    }
}

impl Style {
    fn merge(&mut self, tag: &str) {
        if let Some(v) = attr(tag, "fill") {
            self.fill = !matches!(v.trim(), "none" | "transparent");
            if let Some(c) = parse_color(v) {
                self.fill_color = c;
            }
        }
        if let Some(v) = attr(tag, "stroke") {
            self.stroke = !matches!(v.trim(), "none" | "transparent");
            if let Some(c) = parse_color(v) {
                self.stroke_color = c;
            }
        }
        if let Some(v) = attr(tag, "stroke-width") {
            if let Ok(w) = v.trim().parse::<f32>() {
                self.stroke_width = w;
            }
        }
        if let Some(v) = attr(tag, "fill-rule") {
            self.even_odd = v.trim() == "evenodd";
        }
        if let Some(v) = attr(tag, "opacity") {
            if let Ok(o) = v.trim().parse::<f32>() {
                self.opacity = o;
            }
        }
        // `fill="none" stroke="none"` inside a <g> must not cancel the group.
        let _ = self.opacity;
    }
}

fn view_box(src: &str) -> Option<(f32, f32, f32, f32)> {
    let start = src.find("<svg")?;
    let end = src[start..].find('>')? + start;
    let tag = &src[start..=end];
    if let Some(vb) = attr(tag, "viewBox").or_else(|| attr(tag, "viewbox")) {
        let parts: Vec<f32> = vb
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        if parts.len() == 4 {
            return Some((parts[0], parts[1], parts[2], parts[3]));
        }
    }
    let px = |name: &str| {
        attr(tag, name)
            .map(|v| v.trim().trim_end_matches("px").to_string())
            .and_then(|v| v.parse::<f32>().ok())
    };
    match (px("width"), px("height")) {
        (Some(w), Some(h)) => Some((0.0, 0.0, w, h)),
        _ => None,
    }
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut from = 0usize;
    while let Some(pos) = tag[from..].find(name) {
        let start = from + pos;
        let before_ok = start == 0
            || tag[..start]
                .chars()
                .next_back()
                .map(|c| c.is_whitespace() || c == '<')
                .unwrap_or(false);
        let after = &tag[start + name.len()..];
        let after_trim = after.trim_start();
        if before_ok && after_trim.starts_with('=') {
            let value = after_trim[1..].trim_start();
            let quote = value.chars().next()?;
            if quote == '"' || quote == '\'' {
                let body = &value[1..];
                let end = body.find(quote)?;
                return Some(&body[..end]);
            }
        }
        from = start + name.len();
    }
    None
}

fn num(tag: &str, name: &str) -> Option<f32> {
    attr(tag, name).and_then(|v| v.trim().parse::<f32>().ok())
}

/// Turn one element into polygons in view-box coordinates.
fn element_points(name: &str, tag: &str) -> Option<Vec<Vec<(f32, f32)>>> {
    match name {
        "path" => {
            let d = attr(tag, "d")?;
            Some(flatten_path(d))
        }
        "polyline" | "polygon" => {
            let pts = attr(tag, "points")?;
            let nums: Vec<f32> = pts
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse().ok())
                .collect();
            let mut poly = Vec::new();
            for pair in nums.chunks(2) {
                if pair.len() == 2 {
                    poly.push((pair[0], pair[1]));
                }
            }
            Some(vec![poly])
        }
        "line" => {
            let x1 = num(tag, "x1")?;
            let y1 = num(tag, "y1")?;
            let x2 = num(tag, "x2")?;
            let y2 = num(tag, "y2")?;
            Some(vec![vec![(x1, y1), (x2, y2)]])
        }
        "rect" => {
            let x = num(tag, "x").unwrap_or(0.0);
            let y = num(tag, "y").unwrap_or(0.0);
            let w = num(tag, "width")?;
            let h = num(tag, "height")?;
            let r = attr(tag, "rx")
                .and_then(|v| v.trim().parse::<f32>().ok())
                .or_else(|| attr(tag, "ry").and_then(|v| v.trim().parse::<f32>().ok()))
                .unwrap_or(0.0)
                .min(w * 0.5)
                .min(h * 0.5);
            if r <= 0.01 {
                return Some(vec![vec![(x, y), (x + w, y), (x + w, y + h), (x, y + h)]]);
            }
            let steps = 6;
            let mut poly = Vec::new();
            let corners = [
                (x + w - r, y + r, -std::f32::consts::FRAC_PI_2),
                (x + w - r, y + h - r, 0.0),
                (x + r, y + h - r, std::f32::consts::FRAC_PI_2),
                (x + r, y + r, std::f32::consts::PI),
            ];
            for (cx, cy, base) in corners {
                for i in 0..=steps {
                    let a = base + std::f32::consts::FRAC_PI_2 * i as f32 / steps as f32;
                    poly.push((cx + r * a.cos(), cy + r * a.sin()));
                }
            }
            Some(vec![poly])
        }
        "circle" | "ellipse" => {
            let cx = num(tag, "cx").unwrap_or(0.0);
            let cy = num(tag, "cy").unwrap_or(0.0);
            let rx = if name == "circle" {
                num(tag, "r")?
            } else {
                num(tag, "rx").or_else(|| num(tag, "r"))?
            };
            let ry = if name == "circle" { rx } else { num(tag, "ry").or_else(|| num(tag, "r"))? };
            let steps = 24;
            let mut poly = Vec::new();
            for i in 0..steps {
                let a = std::f32::consts::TAU * i as f32 / steps as f32;
                poly.push((cx + rx * a.cos(), cy + ry * a.sin()));
            }
            Some(vec![poly])
        }
        _ => None,
    }
}

// ---------------------------------------------------------------- path data

struct PathScanner<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> PathScanner<'a> {
    fn new(s: &'a str) -> Self {
        PathScanner { s: s.as_bytes(), i: 0 }
    }

    fn skip_sep(&mut self) {
        while self.i < self.s.len() {
            let c = self.s[self.i] as char;
            if c.is_whitespace() || c == ',' {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    /// An arc flag: a lone `0` or `1`.
    fn flag(&mut self) -> Option<bool> {
        self.skip_sep();
        let c = *self.s.get(self.i)?;
        match c {
            b'0' => {
                self.i += 1;
                Some(false)
            }
            b'1' => {
                self.i += 1;
                Some(true)
            }
            _ => None,
        }
    }

    fn number(&mut self) -> Option<f32> {
        self.skip_sep();
        let start = self.i;
        if self.i < self.s.len() && (self.s[self.i] == b'-' || self.s[self.i] == b'+') {
            self.i += 1;
        }
        let mut seen_digit = false;
        while self.i < self.s.len() {
            let c = self.s[self.i] as char;
            if c.is_ascii_digit() {
                seen_digit = true;
                self.i += 1;
            } else if c == '.' && !seen_digit || (c == '.' && seen_digit) {
                if c == '.' && self.s[start..self.i].contains(&b'.') {
                    break;
                }
                self.i += 1;
            } else if c == 'e' || c == 'E' {
                self.i += 1;
                if self.i < self.s.len() && (self.s[self.i] == b'-' || self.s[self.i] == b'+') {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
        if self.i == start {
            return None;
        }
        std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok()
    }
}

/// Flatten path data into polygons (one per subpath).
pub fn flatten_path(d: &str) -> Vec<Vec<(f32, f32)>> {
    let mut out: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut cur: Vec<(f32, f32)> = Vec::new();
    let mut sc = PathScanner::new(d);
    let mut pos = (0.0f32, 0.0f32);
    let mut start = (0.0f32, 0.0f32);
    let mut last_ctrl: Option<(f32, f32)> = None;
    let mut cmd = 'M';
    loop {
        sc.skip_sep();
        if sc.i >= sc.s.len() {
            break;
        }
        let c = sc.s[sc.i] as char;
        if c.is_ascii_alphabetic() {
            cmd = c;
            sc.i += 1;
        } else if !matches!(cmd, 'M' | 'm') {
            // Implicit repetition of the previous command.
        }
        let rel = cmd.is_ascii_lowercase();
        let base = if rel { pos } else { (0.0, 0.0) };
        match cmd.to_ascii_uppercase() {
            'M' => {
                let Some(x) = sc.number() else { break };
                let Some(y) = sc.number() else { break };
                let p = (base.0 + x, base.1 + y);
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                cur.push(p);
                pos = p;
                start = p;
                last_ctrl = None;
                // Following coordinate pairs are implicit lineto.
                cmd = if rel { 'l' } else { 'L' };
            }
            'L' => {
                let Some(x) = sc.number() else { break };
                let Some(y) = sc.number() else { break };
                let p = (base.0 + x, base.1 + y);
                cur.push(p);
                pos = p;
                last_ctrl = None;
            }
            'H' => {
                let Some(x) = sc.number() else { break };
                let p = (base.0 + x, pos.1);
                cur.push(p);
                pos = p;
                last_ctrl = None;
            }
            'V' => {
                let Some(y) = sc.number() else { break };
                let p = (pos.0, base.1 + y);
                cur.push(p);
                pos = p;
                last_ctrl = None;
            }
            'C' | 'S' => {
                let c1 = if cmd.eq_ignore_ascii_case(&'S') {
                    last_ctrl
                        .map(|c| (2.0 * pos.0 - c.0, 2.0 * pos.1 - c.1))
                        .unwrap_or(pos)
                } else {
                    let Some(x) = sc.number() else { break };
                    let Some(y) = sc.number() else { break };
                    (base.0 + x, base.1 + y)
                };
                let Some(x2) = sc.number() else { break };
                let Some(y2) = sc.number() else { break };
                let Some(x3) = sc.number() else { break };
                let Some(y3) = sc.number() else { break };
                let c2 = (base.0 + x2, base.1 + y2);
                let end = (base.0 + x3, base.1 + y3);
                let from = pos;
                let steps = 16;
                for i in 1..=steps {
                    let t = i as f32 / steps as f32;
                    let mt = 1.0 - t;
                    let x = mt * mt * mt * from.0
                        + 3.0 * mt * mt * t * c1.0
                        + 3.0 * mt * t * t * c2.0
                        + t * t * t * end.0;
                    let y = mt * mt * mt * from.1
                        + 3.0 * mt * mt * t * c1.1
                        + 3.0 * mt * t * t * c2.1
                        + t * t * t * end.1;
                    cur.push((x, y));
                }
                pos = end;
                last_ctrl = Some(c2);
            }
            'Q' | 'T' => {
                let c1 = if cmd.eq_ignore_ascii_case(&'T') {
                    last_ctrl
                        .map(|c| (2.0 * pos.0 - c.0, 2.0 * pos.1 - c.1))
                        .unwrap_or(pos)
                } else {
                    let Some(x) = sc.number() else { break };
                    let Some(y) = sc.number() else { break };
                    (base.0 + x, base.1 + y)
                };
                let Some(x2) = sc.number() else { break };
                let Some(y2) = sc.number() else { break };
                let end = (base.0 + x2, base.1 + y2);
                let from = pos;
                let steps = 12;
                for i in 1..=steps {
                    let t = i as f32 / steps as f32;
                    let mt = 1.0 - t;
                    let x = mt * mt * from.0 + 2.0 * mt * t * c1.0 + t * t * end.0;
                    let y = mt * mt * from.1 + 2.0 * mt * t * c1.1 + t * t * end.1;
                    cur.push((x, y));
                }
                pos = end;
                last_ctrl = Some(c1);
            }
            'A' => {
                let Some(rx) = sc.number() else { break };
                let Some(ry) = sc.number() else { break };
                let Some(rot) = sc.number() else { break };
                // The two flags are single digits and may be written without a
                // separator ("a5 5 0 0110 10"), so they are *not* read as
                // numbers: "01" would parse as one number and swallow the other.
                let Some(large_arc) = sc.flag() else { break };
                let Some(sweep) = sc.flag() else { break };
                let Some(x) = sc.number() else { break };
                let Some(y) = sc.number() else { break };
                let end = (base.0 + x, base.1 + y);
                for p in arc_points(pos, end, rx, ry, rot, large_arc, sweep) {
                    cur.push(p);
                }
                pos = end;
                last_ctrl = None;
            }
            'Z' => {
                if !cur.is_empty() {
                    if let Some(&first) = cur.first() {
                        if pos != first {
                            cur.push(first);
                        }
                    }
                    out.push(std::mem::take(&mut cur));
                }
                pos = start;
                last_ctrl = None;
            }
            _ => break,
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Endpoint-parameterised elliptical arc → points (SVG implementation notes,
/// F.6.5). `large_arc` and `sweep` are the two flags of the `A` command; getting
/// the sweep wrong mirrors every curved icon, which is exactly how a "redo"
/// arrow ends up looking like a random hook.
fn arc_points(
    from: (f32, f32),
    to: (f32, f32),
    rx: f32,
    ry: f32,
    rot_deg: f32,
    large_arc: bool,
    sweep: bool,
) -> Vec<(f32, f32)> {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx < 0.001 || ry < 0.001 || from == to {
        return vec![to];
    }
    let phi = rot_deg.to_radians();
    let (sin_phi, cos_phi) = phi.sin_cos();
    let dx2 = (from.0 - to.0) * 0.5;
    let dy2 = (from.1 - to.1) * 0.5;
    // Rotate the midpoint difference into the ellipse frame.
    let x1p = cos_phi * dx2 + sin_phi * dy2;
    let y1p = -sin_phi * dx2 + cos_phi * dy2;
    // Scale the radii up if they are too small to span the two points.
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = (rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p).max(0.0);
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut coef = if den > 0.0 { (num / den).sqrt() } else { 0.0 };
    if large_arc == sweep {
        coef = -coef;
    }
    let cxp = coef * rx * y1p / ry;
    let cyp = -coef * ry * x1p / rx;
    let cx = cos_phi * cxp - sin_phi * cyp + (from.0 + to.0) * 0.5;
    let cy = sin_phi * cxp + cos_phi * cyp + (from.1 + to.1) * 0.5;
    // The angle of a vector in the ellipse frame, in radians.
    let angle = |x: f32, y: f32| -> f32 { y.atan2(x) };
    // Start angle and sweep, both in the ellipse frame.
    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;
    let a0 = angle(ux, uy);
    let mut delta = {
        let dot = ux * vx + uy * vy;
        let len = ((ux * ux + uy * uy) * (vx * vx + vy * vy)).sqrt();
        let mut d = (dot / len.max(1e-6)).clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            d = -d;
        }
        d
    };
    if !sweep && delta > 0.0 {
        delta -= std::f32::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f32::consts::TAU;
    }
    let steps = ((delta.abs() / std::f32::consts::FRAC_PI_8).ceil() as usize).clamp(4, 32);
    let mut out = Vec::with_capacity(steps);
    for i in 1..=steps {
        let a = a0 + delta * i as f32 / steps as f32;
        let (sin_a, cos_a) = a.sin_cos();
        let x = cx + rx * cos_a * cos_phi - ry * sin_a * sin_phi;
        let y = cy + rx * cos_a * sin_phi + ry * sin_a * cos_phi;
        out.push((x, y));
    }
    out
}

// ---------------------------------------------------------------- raster

/// Rasterise to an 8-bit coverage bitmap of `px` × `px` (alpha only: the renderer
/// tints it, so one bitmap serves both themes).
pub fn rasterize(svg: &Svg, px: u32) -> (u32, u32, Vec<u8>) {
    let (side, hi, sup) = draw_to_supersampled(svg, px, &[]);
    let hw = side * sup;
    let mut out = vec![0u8; (side * side) as usize];
    for y in 0..side {
        for x in 0..side {
            let mut acc = 0.0f32;
            for sy in 0..sup {
                for sx in 0..sup {
                    acc += hi[((y * sup + sy) * hw + (x * sup + sx)) as usize];
                }
            }
            let cov = (acc / (sup * sup) as f32).clamp(0.0, 1.0);
            out[(y * side + x) as usize] = (cov * 255.0).round() as u8;
        }
    }
    (side, side, out)
}

/// Rasterise `svg` in its **own colours**, as straight RGBA.
///
/// This is what the application icon wants: a hand-made single-layer icon keeps
/// the fills it was drawn with. Each shape is rasterised on its own and composited
/// `source over` in document order, so a mark really does sit on top of the plate
/// it was drawn on. The result is always square (`px` x `px`), with the drawing
/// letterboxed inside it, because a window icon is a square bitmap.
pub fn rasterize_rgba(svg: &Svg, px: u32) -> (u32, u32, Vec<u8>) {
    // Coverage of each shape, one after another, composited into the result.
    let mut out = vec![0f32; (px * px * 4) as usize];
    for i in 0..svg.shapes.len() {
        let (side, hi, sup) = draw_to_supersampled(svg, px, &[i]);
        debug_assert_eq!(side, px);
        let color = match &svg.shapes[i] {
            Shape::Fill(_, _, c) | Shape::Stroke(_, _, c) => *c,
        };
        let hw = side * sup;
        for y in 0..side {
            for x in 0..side {
                let mut acc = 0.0f32;
                for sy in 0..sup {
                    for sx in 0..sup {
                        acc += hi[((y * sup + sy) * hw + (x * sup + sx)) as usize];
                    }
                }
                let a = (acc / (sup * sup) as f32).clamp(0.0, 1.0);
                if a <= 0.0 {
                    continue;
                }
                let i = ((y * side + x) * 4) as usize;
                let da = out[i + 3];
                let oa = a + da * (1.0 - a);
                for c in 0..3 {
                    let src = color[c] as f32 / 255.0;
                    out[i + c] = if oa > 0.0 {
                        (src * a + out[i + c] * da * (1.0 - a)) / oa
                    } else {
                        src
                    };
                }
                out[i + 3] = oa;
            }
        }
    }
    let mut bytes = vec![255u8; (px * px * 4) as usize];
    for (i, chunk) in out.chunks_exact(4).enumerate() {
        bytes[i * 4] = (chunk[0].clamp(0.0, 1.0) * 255.0).round() as u8;
        bytes[i * 4 + 1] = (chunk[1].clamp(0.0, 1.0) * 255.0).round() as u8;
        bytes[i * 4 + 2] = (chunk[2].clamp(0.0, 1.0) * 255.0).round() as u8;
        bytes[i * 4 + 3] = (chunk[3].clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    (px, px, bytes)
}

/// Draw the shapes at `only` (or all of them) into a supersampled coverage
/// buffer: `(side in pixels, the buffer, the supersampling factor)`.
///
/// The drawing is fitted into a **square** box and centred there: a window icon is
/// square whatever shape the file has, and the old code sized the bitmap from the
/// file's aspect ratio while the caller indexed it as if it were square, which
/// smeared a non-square icon along the diagonal.
fn draw_to_supersampled(svg: &Svg, px: u32, only: &[usize]) -> (u32, Vec<f32>, u32) {
    let side = px.clamp(4, 1024);
    let sup = 4u32; // supersampling factor
    let hw = side * sup;
    let mut hi = vec![0f32; (hw * hw) as usize];

    // One uniform scale, so nothing is stretched; `viewBox` may start anywhere.
    let k = hw as f32 / svg.width.max(0.001).max(svg.height.max(0.001));
    let ox = (hw as f32 - svg.width.max(0.001) * k) * 0.5;
    let oy = (hw as f32 - svg.height.max(0.001) * k) * 0.5;
    let (vx, vy) = svg.origin;
    let map = |p: (f32, f32)| (ox + (p.0 - vx) * k, oy + (p.1 - vy) * k);

    for (i, shape) in svg.shapes.iter().enumerate() {
        if !only.is_empty() && !only.contains(&i) {
            continue;
        }
        match shape {
            Shape::Fill(polys, even_odd, _) => {
                let mapped: Vec<Vec<(f32, f32)>> = polys
                    .iter()
                    .map(|p| p.iter().map(|q| map(*q)).collect())
                    .collect();
                fill_scanlines(&mapped, *even_odd, &mut hi, hw, hw);
            }
            Shape::Stroke(polys, width, _) => {
                let pen = (width * k * 0.5).max(0.6);
                for poly in polys {
                    let mapped: Vec<(f32, f32)> = poly.iter().map(|q| map(*q)).collect();
                    stroke_polyline(&mapped, pen, &mut hi, hw, hw);
                }
            }
        }
    }
    (side, hi, sup)
}

fn fill_scanlines(polys: &[Vec<(f32, f32)>], even_odd: bool, dst: &mut [f32], w: u32, h: u32) {
    for y in 0..h {
        let sy = y as f32 + 0.5;
        let mut crossings: Vec<(f32, i32)> = Vec::new();
        for poly in polys {
            for i in 0..poly.len() {
                let a = poly[i];
                let b = poly[(i + 1) % poly.len()];
                if (a.1 <= sy && b.1 > sy) || (b.1 <= sy && a.1 > sy) {
                    let t = (sy - a.1) / (b.1 - a.1);
                    let x = a.0 + t * (b.0 - a.0);
                    let dir = if b.1 > a.1 { 1 } else { -1 };
                    crossings.push((x, dir));
                }
            }
        }
        if crossings.is_empty() {
            continue;
        }
        crossings.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut winding = 0;
        let mut span_start = 0.0f32;
        let mut inside = false;
        for (x, dir) in crossings {
            let was_inside = inside;
            if even_odd {
                winding = 1 - winding;
            } else {
                winding += dir;
            }
            inside = winding != 0;
            if !was_inside && inside {
                span_start = x;
            } else if was_inside && !inside {
                let x0 = span_start.max(0.0);
                let x1 = x.min(w as f32);
                if x1 > x0 {
                    let row = (y * w) as usize;
                    for xi in x0 as u32..x1.ceil() as u32 {
                        if xi < w {
                            dst[row + xi as usize] = 1.0;
                        }
                    }
                }
            }
        }
    }
}

fn stroke_polyline(points: &[(f32, f32)], radius: f32, dst: &mut [f32], w: u32, h: u32) {
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let x0 = (a.0.min(b.0) - radius).floor().max(0.0) as u32;
        let x1 = ((a.0.max(b.0) + radius).ceil() as u32).min(w);
        let y0 = (a.1.min(b.1) - radius).floor().max(0.0) as u32;
        let y1 = ((a.1.max(b.1) + radius).ceil() as u32).min(h);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = dx * dx + dy * dy;
        for y in y0..y1 {
            for x in x0..x1 {
                let px = x as f32 + 0.5 - a.0;
                let py = y as f32 + 0.5 - a.1;
                let t = if len2 > 0.0 { ((px * dx + py * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
                let cx = px - dx * t;
                let cy = py - dy * t;
                if cx * cx + cy * cy <= radius * radius {
                    dst[(y * w + x) as usize] = 1.0;
                }
            }
        }
    }
    // Round caps at the ends of an open polyline.
    for p in [points.first(), points.last()].into_iter().flatten() {
        let x0 = (p.0 - radius).floor().max(0.0) as u32;
        let x1 = ((p.0 + radius).ceil() as u32).min(w);
        let y0 = (p.1 - radius).floor().max(0.0) as u32;
        let y1 = ((p.1 + radius).ceil() as u32).min(h);
        for y in y0..y1 {
            for x in x0..x1 {
                let dx = x as f32 + 0.5 - p.0;
                let dy = y as f32 + 0.5 - p.1;
                if dx * dx + dy * dy <= radius * radius {
                    dst[(y * w + x) as usize] = 1.0;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_path_and_rasterises_it() {
        let svg = parse(
            r##"<svg viewBox="0 0 24 24"><path d="M4 12 L20 12" stroke="#fff" stroke-width="2" fill="none"/></svg>"##,
        )
        .expect("svg");
        assert_eq!(svg.shape_count(), 1);
        let (w, h, px) = rasterize(&svg, 24);
        assert_eq!((w, h), (24, 24));
        // The horizontal stroke must leave ink in the middle row only.
        let row = 12 * w as usize;
        assert!(px[row..row + w as usize].iter().any(|p| *p > 128));
        assert!(px[0..w as usize].iter().all(|p| *p == 0), "outside the stroke");
    }

    #[test]
    fn fills_a_closed_shape() {
        let svg = parse(
            r#"<svg viewBox="0 0 24 24"><rect x="4" y="4" width="16" height="16" rx="3"/></svg>"#,
        )
        .expect("svg");
        let (w, _, px) = rasterize(&svg, 24);
        let mid = 12 * w as usize + 12;
        assert!(px[mid] > 200, "the rect is filled in the middle");
        assert_eq!(px[0], 0, "and empty in the corner");
    }

    #[test]
    fn relative_commands_and_curves_flatten() {
        let polys = flatten_path("M2 2 c 4 0 6 4 8 8 s 6 8 10 8 z");
        assert_eq!(polys.len(), 1);
        assert!(polys[0].len() > 10, "curves are flattened into many points");
    }

    /// The sweep flag has to be honoured: a mirrored pair of arcs (an undo arrow
    /// and a redo arrow) must end up as mirror images, not one correct arc and
    /// one hook.
    #[test]
    fn arc_sweep_flag_mirrors_the_curve() {
        let cw = flatten_path("M4 4 a8 8 0 0 1 0 16");
        let ccw = flatten_path("M4 4 a8 8 0 0 0 0 16");
        let end_y = |p: &Vec<(f32, f32)>| p.last().unwrap().1;
        assert!((end_y(&cw[0]) - 20.0).abs() < 0.01, "arc must reach its endpoint");
        assert!((end_y(&ccw[0]) - 20.0).abs() < 0.01, "arc must reach its endpoint");
        // Sweep=1 bulges to the right of the line, sweep=0 to the left.
        let max_x_cw = cw[0].iter().fold(0.0f32, |m, p| m.max(p.0));
        let max_x_ccw = ccw[0].iter().fold(0.0f32, |m, p| m.max(p.0));
        assert!(max_x_cw > 8.0, "sweep=1 must bulge right: {max_x_cw}");
        assert!(max_x_ccw < 4.1, "sweep=0 must bulge left: {max_x_ccw}");
    }

    #[test]
    fn every_shipped_icon_rasterises() {
        for (name, src) in crate::icons::ALL {
            let svg = parse(src).unwrap_or_else(|| panic!("{name}.svg did not parse"));
            let (w, h, px) = rasterize(&svg, 20);
            assert!(w > 0 && h > 0);
            let ink = px.iter().filter(|p| **p > 40).count();
            assert!(ink > 6, "{name}.svg is (almost) empty: {ink} ink pixels");
        }
    }
}
