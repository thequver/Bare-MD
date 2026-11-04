//! A Mermaid flowchart renderer, written from scratch.
//!
//! ```` ```mermaid ```` blocks are not code in a Markdown note, they are a
//! picture; this module turns the common flowchart subset of the Mermaid
//! language into a laid-out diagram of boxes, edges and labels that the painter
//! can draw with the quad primitive it already has (no SVG, no browser, no
//! extra dependency).
//!
//! The pipeline is the classic one:
//!
//! 1. **parse** - statements like `A[Start] -->|yes| B{Ok?}` become nodes and
//!    edges. Chains (`A --> B --> C`), `&` groups, the four arrow styles, edge
//!    labels and the eight node shapes are understood; layout-only directives
//!    (`subgraph`, `classDef`, `style`, `click`, ...) are skipped instead of
//!    being mistaken for nodes.
//! 2. **rank** - every node gets a rank by the longest path from a root, so an
//!    edge always points from a lower rank to a higher one. Cycles are legal:
//!    the relaxation stops after `n` rounds and the back edge is simply drawn
//!    upwards.
//! 3. **order** - a few barycenter sweeps sort each rank to cut crossings.
//! 4. **place** - ranks are stacked along the flow axis, nodes are spread across
//!    it, and every edge is routed orthogonally through the gap between its
//!    ranks, which is what makes the result look drawn rather than tangled.
//!
//! Everything is measured with the real text engine, so a wide label reserves a
//! wide box.

use crate::text::{Face, TextEngine};
use std::collections::HashMap;

/// Gap between two ranks, in logical pixels (before zoom scaling).
const RANK_GAP: f32 = 48.0;
/// Gap between neighbouring nodes inside one rank.
const CROSS_GAP: f32 = 26.0;
/// Breathing room around the whole drawing.
const PAD: f32 = 18.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Rect,
    Round,
    Stadium,
    Circle,
    Diamond,
    Hexagon,
    Cylinder,
    Subroutine,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Line {
    Solid,
    Dotted,
    Thick,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Down,
    Up,
    Right,
    Left,
}

impl Dir {
    /// True when the flow runs along the vertical axis.
    pub fn vertical(self) -> bool {
        matches!(self, Dir::Down | Dir::Up)
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub label: String,
    pub shape: Shape,
    /// Top-left corner and size, relative to the diagram's own origin.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Debug)]
pub struct Edge {
    /// Indices into [`Diagram::nodes`].
    pub from: usize,
    pub to: usize,
    pub line: Line,
    /// Arrowhead at the target / at the source.
    pub arrow: bool,
    pub start_arrow: bool,
    /// Polyline points in diagram space (at least two).
    pub points: Vec<(f32, f32)>,
    pub label: Option<String>,
    /// Top-left of the edge label's box.
    pub label_at: (f32, f32),
    pub label_w: f32,
}

/// A laid-out flowchart: everything the painter needs, in logical pixels.
#[derive(Clone, Debug)]
pub struct Diagram {
    pub dir: Dir,
    pub w: f32,
    pub h: f32,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Font size node labels are drawn at (already scaled with the document).
    pub font: f32,
}

impl Diagram {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

// ---------------------------------------------------------------- parsing ---

#[derive(Clone, Debug)]
struct Spec {
    id: String,
    label: String,
    shape: Shape,
}

#[derive(Clone, Debug)]
struct RawEdge {
    from: usize,
    to: usize,
    line: Line,
    arrow: bool,
    start_arrow: bool,
    invisible: bool,
    label: Option<String>,
}

#[derive(Default)]
struct Raw {
    specs: Vec<Spec>,
    index: HashMap<String, usize>,
    edges: Vec<RawEdge>,
    dir: Option<Dir>,
}

impl Raw {
    fn upsert(&mut self, spec: Spec) -> usize {
        if let Some(&i) = self.index.get(&spec.id) {
            // A later statement may add the label or change the shape; the
            // label wins only if this one actually carries text.
            if !spec.label.is_empty() && spec.label != spec.id {
                self.specs[i].label = spec.label;
            }
            if spec.shape != Shape::Rect || self.specs[i].shape == Shape::Rect {
                self.specs[i].shape = spec.shape;
            }
            i
        } else {
            let i = self.specs.len();
            self.index.insert(spec.id.clone(), i);
            self.specs.push(spec);
            i
        }
    }
}

fn skip_ws(s: &[char], mut i: usize) -> usize {
    while i < s.len() && s[i].is_whitespace() {
        i += 1;
    }
    i
}

fn starts_with(s: &[char], i: usize, pat: &str) -> bool {
    let mut j = i;
    for c in pat.chars() {
        if s.get(j) != Some(&c) {
            return false;
        }
        j += 1;
    }
    true
}

fn find_pat(s: &[char], from: usize, pat: &str) -> Option<usize> {
    let mut j = from;
    while j < s.len() {
        if starts_with(s, j, pat) {
            return Some(j);
        }
        j += 1;
    }
    None
}

fn is_id_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.' || c == '-'
}

/// Trim a node label: drop wrapping quotes, fold `<br>` to a space, and refuse
/// to keep Mermaid's own markup around plain text.
fn clean_label(raw: &[char]) -> String {
    let mut t: String = raw.iter().collect();
    t = t.trim().to_string();
    if t.len() >= 2 {
        let quoted = (t.starts_with('"') && t.ends_with('"'))
            || (t.starts_with('\'') && t.ends_with('\''));
        if quoted {
            t = t[1..t.len() - 1].to_string();
        }
    }
    t = t.replace("<br/>", " ").replace("<br>", " ").replace("<br />", " ");
    // `**bold**` inside a label reads as plain text here.
    let mut out = String::with_capacity(t.len());
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '*' || c == '`' {
            continue;
        }
        out.push(c);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Read `id[shape]` / `id(shape)` / `id{shape}` starting at `i`.
fn read_node(s: &[char], i: usize) -> Option<(Spec, usize)> {
    let i = skip_ws(s, i);
    let start = i;
    let mut j = i;
    while j < s.len() && is_id_char(s[j]) {
        j += 1;
    }
    if j == start {
        return None;
    }
    let id: String = s[start..j].iter().collect();
    let mut spec = Spec { id: id.clone(), label: id.clone(), shape: Shape::Rect };

    // Shape openers, longest first so `[[` is not read as `[`.
    let shapes: [(&str, &str, Shape); 8] = [
        ("[[", "]]", Shape::Subroutine),
        ("[(", ")]", Shape::Cylinder),
        ("([", "])", Shape::Stadium),
        ("((", "))", Shape::Circle),
        ("{{", "}}", Shape::Hexagon),
        ("[", "]", Shape::Rect),
        ("(", ")", Shape::Round),
        ("{", "}", Shape::Diamond),
    ];
    for (open, close, shape) in shapes {
        if starts_with(s, j, open) {
            let body = j + open.chars().count();
            let Some(end) = find_pat(s, body, close) else { break };
            spec.label = clean_label(&s[body..end]);
            if spec.label.is_empty() {
                spec.label = id.clone();
            }
            spec.shape = shape;
            j = end + close.chars().count();
            break;
        }
    }
    // `A:::cls` - a class assignment, not part of the label.
    if starts_with(s, j, ":::") {
        j += 3;
        while j < s.len() && is_id_char(s[j]) {
            j += 1;
        }
    }
    Some((spec, j))
}

struct Link {
    line: Line,
    arrow: bool,
    start_arrow: bool,
    invisible: bool,
    label: Option<String>,
    next: usize,
}

/// Read an arrow/link (`-->`, `---`, `-.->`, `==>`, `~~~`, with or without a
/// label) starting at `i`.
fn read_link(s: &[char], i: usize) -> Option<Link> {
    let i = skip_ws(s, i);
    let mut k = i;
    let mut start_arrow = false;
    if s.get(k) == Some(&'<') {
        start_arrow = true;
        k += 1;
    }
    let mut line = Line::Solid;
    let mut arrow = false;
    let mut invisible = false;
    let mut label: Option<String> = None;
    let next;

    if starts_with(s, k, "~~~") {
        invisible = true;
        next = k + 3;
    } else if starts_with(s, k, "==") {
        line = Line::Thick;
        let j = k + 2;
        if s.get(j) == Some(&'>') {
            arrow = true;
            next = j + 1;
        } else if starts_with(s, j, "=>") {
            arrow = true;
            next = j + 2;
        } else if s.get(j) == Some(&'=') {
            if s.get(j + 1) == Some(&'>') {
                arrow = true;
                next = j + 2;
            } else {
                next = j + 1;
            }
        } else if let Some(e) = find_pat(s, j, "==>") {
            label = Some(clean_label(&s[j..e]));
            arrow = true;
            next = e + 3;
        } else if let Some(e) = find_pat(s, j, "==") {
            label = Some(clean_label(&s[j..e]));
            next = e + 2;
        } else {
            return None;
        }
    } else if starts_with(s, k, "-.") {
        line = Line::Dotted;
        let j = k + 2;
        if starts_with(s, j, "->") {
            arrow = true;
            next = j + 2;
        } else if s.get(j) == Some(&'-') {
            next = j + 1;
        } else if let Some(e) = find_pat(s, j, ".->") {
            label = Some(clean_label(&s[j..e]));
            arrow = true;
            next = e + 3;
        } else if let Some(e) = find_pat(s, j, ".-") {
            label = Some(clean_label(&s[j..e]));
            next = e + 2;
        } else {
            return None;
        }
    } else if starts_with(s, k, "--") {
        let j = k + 2;
        if s.get(j) == Some(&'>') {
            arrow = true;
            next = j + 1;
        } else if s.get(j) == Some(&'-') {
            if s.get(j + 1) == Some(&'>') {
                arrow = true;
                next = j + 2;
            } else {
                next = j + 1;
            }
        } else if let Some(e) = find_pat(s, j, "-->") {
            label = Some(clean_label(&s[j..e]));
            arrow = true;
            next = e + 3;
        } else if let Some(e) = find_pat(s, j, "---") {
            label = Some(clean_label(&s[j..e]));
            next = e + 3;
        } else {
            return None;
        }
    } else {
        return None;
    }

    // `-->|text|`
    let mut next = next;
    let mut k2 = skip_ws(s, next);
    if s.get(k2) == Some(&'|') {
        if let Some(e) = find_pat(s, k2 + 1, "|") {
            label = Some(clean_label(&s[k2 + 1..e]));
            k2 = e + 1;
            next = k2;
        }
    }
    if let Some(l) = &label {
        if l.is_empty() {
            label = None;
        }
    }
    Some(Link { line, arrow, start_arrow, invisible, label, next })
}

fn parse(src: &str) -> Raw {
    let mut raw = Raw::default();
    for raw_line in src.lines() {
        let text = raw_line.trim();
        if text.is_empty() || text.starts_with("%%") {
            continue;
        }
        let lower = text.to_lowercase();
        // The header names the direction.
        if let Some(rest) = lower
            .strip_prefix("flowchart")
            .or_else(|| lower.strip_prefix("graph"))
        {
            if raw.dir.is_none() {
                let token = rest.trim().split_whitespace().next().unwrap_or("");
                raw.dir = Some(match token {
                    "bt" => Dir::Up,
                    "lr" => Dir::Right,
                    "rl" => Dir::Left,
                    _ => Dir::Down,
                });
            }
            // `graph LR; A-->B` - the rest of the line is still a statement.
            if let Some(pos) = text.find(';') {
                for stmt in text[pos + 1..].split(';') {
                    parse_statement(stmt, &mut raw);
                }
            }
            continue;
        }
        // Directives that are not drawing instructions.
        let head = lower.split_whitespace().next().unwrap_or("");
        if matches!(
            head,
            "subgraph"
                | "end"
                | "direction"
                | "classdef"
                | "class"
                | "style"
                | "linkstyle"
                | "click"
                | "accTitle"
                | "accDescr"
                | "note"
        ) || lower.starts_with("acctitle")
            || lower.starts_with("accdescr")
        {
            continue;
        }
        for stmt in text.split(';') {
            parse_statement(stmt, &mut raw);
        }
    }
    raw
}

fn parse_statement(text: &str, raw: &mut Raw) {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0usize;
    // The nodes to the left of the link about to be read.
    let mut left: Vec<usize> = Vec::new();
    let mut pending: Option<Link> = None;
    loop {
        // A group of nodes: `A` or `A & B & C`.
        let mut group: Vec<usize> = Vec::new();
        loop {
            let Some((spec, next)) = read_node(&chars, i) else { return };
            i = next;
            group.push(raw.upsert(spec));
            let j = skip_ws(&chars, i);
            if chars.get(j) == Some(&'&') {
                i = j + 1;
                continue;
            }
            i = j;
            break;
        }
        if let Some(link) = pending.take() {
            for &a in &left {
                for &b in &group {
                    if a == b && link.line == Line::Solid {
                        continue;
                    }
                    raw.edges.push(RawEdge {
                        from: a,
                        to: b,
                        line: link.line,
                        arrow: link.arrow,
                        start_arrow: link.start_arrow,
                        invisible: link.invisible,
                        label: link.label.clone(),
                    });
                }
            }
        }
        match read_link(&chars, i) {
            Some(link) => {
                i = link.next;
                left = group;
                pending = Some(link);
            }
            None => return,
        }
    }
}

// --------------------------------------------------------------- layout ----

#[derive(Clone, Copy)]
struct Box2 {
    along: f32,
    cross: f32,
}

/// Lay a ```` ```mermaid ```` block out for a page `avail` pixels wide.
///
/// Returns `None` when the source has no nodes at all (a half-typed block), so
/// the caller can fall back to plain code.
pub fn layout(src: &str, te: &mut TextEngine, avail: f32, font: f32) -> Option<Diagram> {
    let raw = parse(src);
    if raw.specs.is_empty() {
        return None;
    }
    let dir = raw.dir.unwrap_or(Dir::Down);
    let node_font = (font * 0.86).max(6.0);
    let lh = te.line_height(Face::Regular, node_font);

    // ---- sizes ----
    let mut dims: Vec<Box2> = Vec::with_capacity(raw.specs.len());
    let mut sizes: Vec<(f32, f32)> = Vec::with_capacity(raw.specs.len());
    for spec in &raw.specs {
        let tw = te.width(Face::Regular, &spec.label, node_font);
        let (pad_x, extra_h, min_w) = match spec.shape {
            Shape::Rect | Shape::Round => (16.0, 8.0, 54.0),
            Shape::Stadium => (20.0, 9.0, 60.0),
            Shape::Circle => (14.0, 10.0, 60.0),
            Shape::Diamond => (tw * 0.42 + 18.0, 14.0, 70.0),
            Shape::Hexagon => (tw * 0.16 + 20.0, 9.0, 62.0),
            Shape::Cylinder => (18.0, 12.0, 58.0),
            Shape::Subroutine => (20.0, 8.0, 56.0),
        };
        let mut w = (tw + pad_x * 2.0).max(min_w);
        let mut h = (lh + extra_h * 2.0).max(34.0);
        if spec.shape == Shape::Circle {
            let side = w.max(h);
            w = side;
            h = side;
        }
        let (along, cross) = if dir.vertical() { (h, w) } else { (w, h) };
        dims.push(Box2 { along, cross });
        sizes.push((w, h));
    }

    // ---- ranking: longest path from a root, over a DAG ----
    // A flowchart may contain loops (`D -.-> A`). Relaxing the longest path over
    // a cyclic graph never settles - it pushes the whole cycle to the end of the
    // page - so the back edges of a DFS are dropped from the ranking (they are
    // drawn as the upward links they are) and the rest is a DAG again.
    let n = raw.specs.len();
    let real: Vec<&RawEdge> = raw.edges.iter().filter(|e| !e.invisible).collect();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in &real {
        adj[e.from].push(e.to);
    }
    let mut color = vec![0u8; n]; // 0 = unvisited, 1 = on the stack, 2 = done
    let mut back: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    for start in 0..n {
        if color[start] != 0 {
            continue;
        }
        color[start] = 1;
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some((v, ci)) = stack.pop() {
            if ci < adj[v].len() {
                stack.push((v, ci + 1));
                let w = adj[v][ci];
                match color[w] {
                    0 => {
                        color[w] = 1;
                        stack.push((w, 0));
                    }
                    1 => {
                        back.insert((v, w));
                    }
                    _ => {}
                }
            } else {
                color[v] = 2;
            }
        }
    }
    let mut rank = vec![0usize; n];
    for _ in 0..=n {
        let mut changed = false;
        for e in real.iter().filter(|e| !back.contains(&(e.from, e.to))) {
            if rank[e.to] < rank[e.from] + 1 {
                rank[e.to] = rank[e.from] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let n_ranks = rank.iter().copied().max().unwrap_or(0) + 1;

    // ---- order inside each rank: barycenter sweeps ----
    let mut ranks: Vec<Vec<usize>> = vec![Vec::new(); n_ranks];
    for (i, &r) in rank.iter().enumerate() {
        ranks[r].push(i);
    }
    for sweep in 0..4 {
        let down = sweep % 2 == 0;
        let order = if down {
            (0..n_ranks).collect::<Vec<_>>()
        } else {
            (0..n_ranks).rev().collect()
        };
        // Positions from the neighbouring rank, already sorted.
        let mut pos = vec![0.0f32; n];
        for &r in &order {
            for (k, &node) in ranks[r].iter().enumerate() {
                pos[node] = k as f32;
            }
        }
        for &r in &order {
            let mut keyed: Vec<(f32, usize)> = ranks[r]
                .iter()
                .map(|&node| {
                    let neighbours: Vec<f32> = real
                        .iter()
                        .filter_map(|e| {
                            if down && e.to == node && rank[e.from] < r {
                                Some(pos[e.from])
                            } else if !down && e.from == node && rank[e.to] > r {
                                Some(pos[e.to])
                            } else {
                                None
                            }
                        })
                        .collect();
                    let key = if neighbours.is_empty() {
                        pos[node]
                    } else {
                        neighbours.iter().sum::<f32>() / neighbours.len() as f32
                    };
                    (key, node)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            ranks[r] = keyed.into_iter().map(|(_, n)| n).collect();
        }
    }

    // ---- place: ranks along the flow axis, nodes spread across it ----
    let mut along_band = vec![0.0f32; n_ranks];
    for (r, nodes) in ranks.iter().enumerate() {
        for &node in nodes {
            along_band[r] = along_band[r].max(dims[node].along);
        }
    }
    let mut rank_at = vec![0.0f32; n_ranks];
    let mut acc = 0.0f32;
    for r in 0..n_ranks {
        rank_at[r] = acc;
        acc += along_band[r] + RANK_GAP;
    }
    let total_along = (acc - RANK_GAP).max(0.0);

    let mut cross_total = 0.0f32;
    for nodes in &ranks {
        let sum: f32 = nodes.iter().map(|&i| dims[i].cross).sum::<f32>()
            + CROSS_GAP * (nodes.len().saturating_sub(1)) as f32;
        cross_total = cross_total.max(sum);
    }

    // Abstract coordinates: `along` runs with the flow, `cross` across it.
    let mut a_of = vec![0.0f32; n];
    let mut c_of = vec![0.0f32; n];
    for (r, nodes) in ranks.iter().enumerate() {
        let sum: f32 = nodes.iter().map(|&i| dims[i].cross).sum::<f32>()
            + CROSS_GAP * (nodes.len().saturating_sub(1)) as f32;
        let mut c = (cross_total - sum) * 0.5;
        for &node in nodes {
            a_of[node] = rank_at[r] + (along_band[r] - dims[node].along) * 0.5;
            c_of[node] = c;
            c += dims[node].cross + CROSS_GAP;
        }
    }

    // Map abstract (along, cross) to page (x, y), then add the padding.
    let (diag_w, diag_h) = if dir.vertical() {
        (cross_total, total_along)
    } else {
        (total_along, cross_total)
    };
    let map = |a: f32, c: f32, size: Box2, dir: Dir| -> (f32, f32) {
        // `size` is the node's extent; the point is its top-left corner.
        let (x, y) = match dir {
            Dir::Down => (c, a),
            Dir::Up => (c, total_along - a - size.along),
            Dir::Right => (a, c),
            Dir::Left => (total_along - a - size.along, c),
        };
        (x + PAD, y + PAD)
    };
    // A point on the flow axis (edges only care about the axis).
    let map_pt = |a: f32, c: f32, dir: Dir| -> (f32, f32) {
        let (x, y) = match dir {
            Dir::Down => (c, a),
            Dir::Up => (c, total_along - a),
            Dir::Right => (a, c),
            Dir::Left => (total_along - a, c),
        };
        (x + PAD, y + PAD)
    };

    let mut nodes = Vec::with_capacity(n);
    for (i, spec) in raw.specs.iter().enumerate() {
        let (w, h) = sizes[i];
        let a = a_of[i];
        let c = c_of[i];
        // The abstract box uses (along, cross); convert back to (w, h).
        let (x, y) = if dir.vertical() {
            let size = Box2 { along: h, cross: w };
            map(a, c, size, dir)
        } else {
            let size = Box2 { along: w, cross: h };
            map(a, c, size, dir)
        };
        nodes.push(Node { label: spec.label.clone(), shape: spec.shape, x, y, w, h });
    }

    // ---- edges: orthogonal routing through the gap after the source rank ----
    let mut edges = Vec::with_capacity(real.len() + (raw.edges.len() - real.len()));
    for e in &raw.edges {
        let from = e.from;
        let to = e.to;
        let dim_from = dims[from];
        let dim_to = dims[to];
        let a_exit = a_of[from] + dim_from.along;
        let a_enter = a_of[to];
        let c_exit = c_of[from] + dim_from.cross * 0.5;
        let c_enter = c_of[to] + dim_to.cross * 0.5;
        let mut points: Vec<(f32, f32)> = Vec::new();
        if a_enter >= a_exit {
            // Forward edge: leave the bottom, cross in the gap, come in the top.
            let gap_mid = (a_exit + a_enter) * 0.5;
            points.push(map_pt(a_exit, c_exit, dir));
            if (c_exit - c_enter).abs() > 0.5 {
                points.push(map_pt(gap_mid, c_exit, dir));
                points.push(map_pt(gap_mid, c_enter, dir));
            }
            points.push(map_pt(a_enter, c_enter, dir));
        } else {
            // Back edge (a cycle): step out to the right and around.
            let c_side = cross_total + CROSS_GAP * 0.6;
            points.push(map_pt(a_exit - dim_from.along * 0.5, c_exit, dir));
            points.push(map_pt(a_exit - dim_from.along * 0.5, c_side, dir));
            points.push(map_pt(a_enter + dim_to.along * 0.5, c_side, dir));
            points.push(map_pt(a_enter + dim_to.along * 0.5, c_enter, dir));
            points.push(map_pt(a_enter, c_enter, dir));
        }
        // Label sits at the middle of the longest segment.
        let mut best = (0.0f32, (0.0f32, 0.0f32));
        for w in points.windows(2) {
            let d = (w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs();
            if d > best.0 {
                best = (d, ((w[0].0 + w[1].0) * 0.5, (w[0].1 + w[1].1) * 0.5));
            }
        }
        let label_w = match &e.label {
            Some(t) => te.width(Face::Regular, t, node_font * 0.92) + 8.0,
            None => 0.0,
        };
        edges.push(Edge {
            from,
            to,
            line: e.line,
            arrow: e.arrow,
            start_arrow: e.start_arrow,
            points,
            label: e.label.clone(),
            label_at: (best.1 .0 - label_w * 0.5, best.1 .1 - node_font * 0.5),
            label_w,
        });
    }

    let mut w = diag_w + PAD * 2.0;
    let mut h = diag_h + PAD * 2.0;
    // The back-edge detour sticks out to the right of the drawing.
    for e in &edges {
        for p in &e.points {
            w = w.max(p.0 + PAD);
            h = h.max(p.1 + PAD);
            w = w.max(p.0);
            h = h.max(p.1);
        }
    }

    // Too wide for the page: shrink the whole drawing (labels included) rather
    // than pushing a horizontal scrollbar into the document.
    let avail = avail.max(120.0);
    let mut k = 1.0f32;
    if w > avail {
        k = avail / w;
        for node in &mut nodes {
            node.x *= k;
            node.y *= k;
            node.w *= k;
            node.h *= k;
        }
        for e in &mut edges {
            for p in &mut e.points {
                p.0 *= k;
                p.1 *= k;
            }
            e.label_at = (e.label_at.0 * k, e.label_at.1 * k);
            e.label_w *= k;
        }
        w *= k;
        h *= k;
    }

    Some(Diagram { dir, w, h, nodes, edges, font: node_font * k })
}

// ------------------------------------------------------------------ tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Rect;

    fn engine() -> TextEngine {
        TextEngine::new()
    }

    fn build(src: &str) -> Diagram {
        let mut te = engine();
        layout(src, &mut te, 700.0, 17.0).expect("a diagram")
    }

    #[test]
    fn a_chain_of_three_nodes_gets_three_ranks() {
        let d = build("flowchart TD\n  A[Start] --> B{Ready?}\n  B -->|yes| C[Done]");
        assert_eq!(d.nodes.len(), 3);
        assert_eq!(d.edges.len(), 2);
        assert_eq!(d.dir, Dir::Down);
        let (a, b, c) = (&d.nodes[0], &d.nodes[1], &d.nodes[2]);
        assert!(a.y < b.y, "A above B");
        assert!(b.y < c.y, "B above C");
        assert_eq!(b.label, "Ready?");
        assert_eq!(c.label, "Done");
        assert_eq!(d.edges[1].label.as_deref(), Some("yes"));
        assert!(d.w <= 700.0 && d.h > 100.0);
    }

    #[test]
    fn direction_lr_lays_the_flow_out_across() {
        let d = build("graph LR\n A --> B --> C");
        assert_eq!(d.dir, Dir::Right);
        assert!(d.nodes[0].x < d.nodes[1].x && d.nodes[1].x < d.nodes[2].x);
        assert!((d.nodes[0].y - d.nodes[2].y).abs() < 1.0, "one row");
    }

    #[test]
    fn shapes_and_ampersand_groups_are_understood() {
        let d = build(
            "flowchart TD\n A[(DB)] --> B\n X((round)) --> B\n P & Q ==> R[[sub]]\n S{{hex}} --- T([stad])",
        );
        let find = |needle: &str| d.nodes.iter().find(|n| n.label == needle).cloned();
        assert_eq!(find("DB").unwrap().shape, Shape::Cylinder);
        assert_eq!(find("round").unwrap().shape, Shape::Circle);
        assert_eq!(find("sub").unwrap().shape, Shape::Subroutine);
        assert_eq!(find("hex").unwrap().shape, Shape::Hexagon);
        assert_eq!(find("stad").unwrap().shape, Shape::Stadium);
        // `P & Q ==> R` is two edges into R.
        let r = d.nodes.iter().position(|n| n.label == "sub").unwrap();
        assert_eq!(d.edges.iter().filter(|e| e.to == r).count(), 2);
        assert!(d.edges.iter().any(|e| e.line == Line::Thick));
    }

    #[test]
    fn dotted_and_labelled_links_parse() {
        let d = build("flowchart LR\n A -.-> B\n A -- maybe --> C\n A --- D");
        assert_eq!(d.edges.len(), 3);
        assert_eq!(d.edges[0].line, Line::Dotted);
        assert!(d.edges[0].arrow);
        assert_eq!(d.edges[1].label.as_deref(), Some("maybe"));
        assert!(!d.edges[2].arrow, "a plain `---` has no arrowhead");
    }

    #[test]
    fn directives_are_not_nodes() {
        let d = build(
            "flowchart TD\n%% a comment\n subgraph one\n direction TB\n A --> B\n end\n classDef x fill:#fff\n style A fill:#000\n click A call f()",
        );
        let labels: Vec<&str> = d.nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, vec!["A", "B"]);
    }

    #[test]
    fn a_cycle_does_not_hang_or_overlap() {
        let d = build("flowchart TD\n A --> B\n B --> C\n C --> A");
        assert_eq!(d.nodes.len(), 3);
        assert_eq!(d.edges.len(), 3);
        for e in &d.edges {
            assert!(e.points.len() >= 2);
        }
        assert!(d.h > 0.0 && d.w > 0.0);
    }

    #[test]
    fn a_wide_diagram_is_scaled_to_the_page() {
        let mut te = engine();
        let src = "flowchart LR\n root --> a_very_long_node_label_here --> another_extremely_long_label";
        let d = layout(src, &mut te, 320.0, 17.0).unwrap();
        assert!(d.w <= 320.5, "got width {}", d.w);
        assert!(d.font < 17.0);
        // Everything scales together, so a node is still wider than it is tall.
        for n in &d.nodes {
            assert!(n.w > 0.0 && n.h > 0.0);
        }
    }

    #[test]
    fn an_empty_block_has_no_diagram() {
        let mut te = engine();
        assert!(layout("", &mut te, 600.0, 17.0).is_none());
        assert!(layout("flowchart TD", &mut te, 600.0, 17.0).is_none());
    }

    #[test]
    fn edge_points_stay_inside_the_drawing() {
        let d = build("flowchart TD\n A --> B\n A --> C\n B --> D\n C --> D");
        for e in &d.edges {
            for p in &e.points {
                assert!(p.0 >= -1.0 && p.0 <= d.w + 1.0, "x {} of {}", p.0, d.w);
                assert!(p.1 >= -1.0 && p.1 <= d.h + 1.0, "y {} of {}", p.1, d.h);
            }
        }
    }

    #[test]
    fn rect_of_a_node_is_a_rect() {
        let d = build("flowchart TD\n A[One] --> B[Two]");
        let r = Rect::new(d.nodes[0].x, d.nodes[0].y, d.nodes[0].w, d.nodes[0].h);
        assert!(r.w > 20.0 && r.h > 20.0);
    }
}
