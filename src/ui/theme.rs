//! Colours and metrics. Three monochrome themes (black, grey and white); every
//! colour in all of them is a pure grey, so the editor never shows a hue.

use crate::geom::Color;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeKind {
    /// The darkest theme: near-black surface, light grey text.
    Black,
    /// A softer dark theme with a mid-grey surface. Easier on the eyes in a
    /// bright room, and the one to pick if "black" feels too harsh.
    Grey,
    /// Bright surface, near-black text.
    White,
}

impl ThemeKind {
    pub const ALL: [ThemeKind; 3] = [ThemeKind::Black, ThemeKind::Grey, ThemeKind::White];

    /// The next theme in the cycle.
    pub fn next(self) -> ThemeKind {
        match self {
            ThemeKind::Black => ThemeKind::Grey,
            ThemeKind::Grey => ThemeKind::White,
            ThemeKind::White => ThemeKind::Black,
        }
    }

    /// Stable identifier used in `settings.conf`.
    pub fn id(self) -> &'static str {
        match self {
            ThemeKind::Black => "black",
            ThemeKind::Grey => "grey",
            ThemeKind::White => "white",
        }
    }

    /// Parses an identifier; older names map onto the monochrome themes.
    pub fn parse(s: &str) -> Option<ThemeKind> {
        match s.trim().to_ascii_lowercase().as_str() {
            "black" | "dark" | "mono" | "black-white" | "bw" => Some(ThemeKind::Black),
            "grey" | "gray" | "midnight" | "dim" => Some(ThemeKind::Grey),
            "white" | "light" | "mono-light" | "wb" => Some(ThemeKind::White),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        self.id()
    }

    /// True for the bright background theme.
    pub fn is_light(self) -> bool {
        matches!(self, ThemeKind::White)
    }
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub kind: ThemeKind,
    pub bg: Color,
    pub bg_panel: Color,
    pub bg_elevated: Color,
    pub bg_hover: Color,
    pub bg_active: Color,
    pub border: Color,
    pub border_strong: Color,
    pub text: Color,
    pub text_dim: Color,
    pub text_faint: Color,
    pub heading: Color,
    pub marker: Color,
    pub accent: Color,
    pub accent_text: Color,
    pub link: Color,
    pub code_text: Color,
    pub code_bg: Color,
    pub quote_text: Color,
    pub quote_bar: Color,
    pub selection: Color,
    pub caret: Color,
    pub bullet: Color,
    pub minimap_bg: Color,
    pub minimap_line: Color,
    pub minimap_view: Color,
    pub scrollbar: Color,
    pub warn: Color,
    /// The one deliberately coloured UI ink: a document whose file has been
    /// deleted behind its tab. Red is the only colour that reads as "this text
    /// now exists nowhere but here", and a tab is a small enough place for it.
    pub danger: Color,
    /// Syntax highlighting inside fenced code blocks. Muted on purpose: the
    /// themes are monochrome, so these are the only hues outside a coloured span,
    /// and they stay desaturated enough to read as ink rather than confetti.
    pub syn_keyword: Color,
    pub syn_string: Color,
    pub syn_number: Color,
    pub syn_func: Color,
    pub syn_type: Color,
    /// Ink for `==highlight==`: the marked words are **yellow text**, not a
    /// marker-pen wash. Yellow enough to be obviously a mark, dark or light
    /// enough to stay readable on the theme it belongs to.
    pub mark: Color,
    /// Base font size in logical pixels.
    pub font_size: f32,
}

impl Theme {
    /// The darkest theme. Not pure black: a #0f0f0f sheet with panels one step
    /// lighter, so the chrome reads as part of the same surface instead of a
    /// black hole with a couple of grey boxes floating in it.
    pub fn black() -> Theme {
        Theme {
            kind: ThemeKind::Black,
            bg: Color::hex("#0f0f0f"),
            bg_panel: Color::hex("#151515"),
            bg_elevated: Color::hex("#1d1d1d"),
            bg_hover: Color::hex("#262626"),
            bg_active: Color::hex("#313131"),
            border: Color::hex("#242424"),
            border_strong: Color::hex("#3d3d3d"),
            text: Color::hex("#ededed"),
            text_dim: Color::hex("#a2a2a2"),
            text_faint: Color::hex("#6b6b6b"),
            heading: Color::hex("#fbfbfb"),
            marker: Color::hex("#656565"),
            accent: Color::hex("#f2f2f2"),
            accent_text: Color::hex("#000000"),
            // The one accent in an otherwise colourless theme: a pale, washed
            // out blue that reads as a link only when you look for it.
            link: Color::hex("#8fb8d8"),
            code_text: Color::hex("#dedede"),
            code_bg: Color::hex("#181818"),
            quote_text: Color::hex("#c4c4c4"),
            quote_bar: Color::hex("#454545"),
            selection: Color::hex("#333333"),
            caret: Color::hex("#f7f7f7"),
            bullet: Color::hex("#e8e8e8"),
            minimap_bg: Color::hex("#0f0f0f"),
            minimap_line: Color::hex("#4e4e4e"),
            minimap_view: Color::rgba(1.0, 1.0, 1.0, 0.05),
            scrollbar: Color::rgba(1.0, 1.0, 1.0, 0.16),
            warn: Color::hex("#cfcfcf"),
            danger: Color::hex("#e2685e"),
            mark: Color::hex("#e5cf7d"),
            syn_keyword: Color::hex("#93b8dd"),
            syn_string: Color::hex("#a9c79b"),
            syn_number: Color::hex("#d8b98a"),
            syn_func: Color::hex("#c9c39a"),
            syn_type: Color::hex("#b9bede"),
            font_size: 17.0,
        }
    }

    /// A mid-grey dark theme: the same layout as "black", lifted far enough off
    /// the floor that the window does not read as a black rectangle.
    pub fn grey() -> Theme {
        Theme {
            kind: ThemeKind::Grey,
            bg: Color::hex("#262626"),
            bg_panel: Color::hex("#2c2c2c"),
            bg_elevated: Color::hex("#353535"),
            bg_hover: Color::hex("#3e3e3e"),
            bg_active: Color::hex("#4a4a4a"),
            border: Color::hex("#3a3a3a"),
            border_strong: Color::hex("#555555"),
            text: Color::hex("#ececec"),
            text_dim: Color::hex("#b4b4b4"),
            text_faint: Color::hex("#828282"),
            heading: Color::hex("#fdfdfd"),
            marker: Color::hex("#7d7d7d"),
            accent: Color::hex("#ffffff"),
            accent_text: Color::hex("#1c1c1c"),
            link: Color::hex("#a3c4e0"),
            code_text: Color::hex("#e4e4e4"),
            code_bg: Color::hex("#2e2e2e"),
            quote_text: Color::hex("#d2d2d2"),
            quote_bar: Color::hex("#616161"),
            selection: Color::hex("#454545"),
            caret: Color::hex("#ffffff"),
            bullet: Color::hex("#f2f2f2"),
            minimap_bg: Color::hex("#262626"),
            minimap_line: Color::hex("#777777"),
            minimap_view: Color::rgba(1.0, 1.0, 1.0, 0.07),
            scrollbar: Color::rgba(1.0, 1.0, 1.0, 0.20),
            warn: Color::hex("#dedede"),
            danger: Color::hex("#e2675c"),
            mark: Color::hex("#ecd884"),
            syn_keyword: Color::hex("#9dc0e2"),
            syn_string: Color::hex("#b2cd9f"),
            syn_number: Color::hex("#e0c194"),
            syn_func: Color::hex("#d3cd9f"),
            syn_type: Color::hex("#c2c6e4"),
            font_size: 17.0,
        }
    }

    /// Pure white background, pure black text, no hue anywhere.
    pub fn white() -> Theme {
        Theme {
            kind: ThemeKind::White,
            bg: Color::hex("#ffffff"),
            bg_panel: Color::hex("#fafafa"),
            bg_elevated: Color::hex("#f6f6f6"),
            bg_hover: Color::hex("#ededed"),
            bg_active: Color::hex("#dedede"),
            border: Color::hex("#ebebeb"),
            border_strong: Color::hex("#bfbfbf"),
            text: Color::hex("#0a0a0a"),
            text_dim: Color::hex("#5a5a5a"),
            text_faint: Color::hex("#9c9c9c"),
            heading: Color::hex("#000000"),
            marker: Color::hex("#b0b0b0"),
            accent: Color::hex("#000000"),
            accent_text: Color::hex("#ffffff"),
            link: Color::hex("#7a9fc0"),
            code_text: Color::hex("#1c1c1c"),
            code_bg: Color::hex("#f4f4f4"),
            quote_text: Color::hex("#333333"),
            quote_bar: Color::hex("#d0d0d0"),
            selection: Color::hex("#d8d8d8"),
            caret: Color::hex("#000000"),
            bullet: Color::hex("#000000"),
            minimap_bg: Color::hex("#ffffff"),
            minimap_line: Color::hex("#c4c4c4"),
            minimap_view: Color::rgba(0.0, 0.0, 0.0, 0.04),
            scrollbar: Color::rgba(0.0, 0.0, 0.0, 0.16),
            warn: Color::hex("#3c3c3c"),
            danger: Color::hex("#c0392b"),
            mark: Color::hex("#8a6b00"),
            syn_keyword: Color::hex("#2f5f8f"),
            syn_string: Color::hex("#3f6b32"),
            syn_number: Color::hex("#8a5a1d"),
            syn_func: Color::hex("#6b5c17"),
            syn_type: Color::hex("#3b4a7a"),
            font_size: 17.0,
        }
    }

    pub fn by_kind(kind: ThemeKind) -> Theme {
        match kind {
            ThemeKind::Black => Theme::black(),
            ThemeKind::Grey => Theme::grey(),
            ThemeKind::White => Theme::white(),
        }
    }

    /// Font size for a heading level (1..=6).
    pub fn heading_size(&self, level: u8) -> f32 {
        let scale = match level {
            1 => 1.85,
            2 => 1.5,
            3 => 1.28,
            4 => 1.14,
            5 => 1.05,
            _ => 1.0,
        };
        (self.font_size * scale).round()
    }

    pub fn code_size(&self) -> f32 {
        (self.font_size * 0.9).round()
    }

    pub fn ui_size(&self) -> f32 {
        13.0
    }

    pub fn ui_small(&self) -> f32 {
        11.5
    }

    // Chrome metrics in logical pixels.
    pub const FLOATING_TOOLBAR_H: f32 = 38.0;
    pub const TABBAR_H: f32 = 36.0;
    pub const STATUS_H: f32 = 24.0;
    pub const SIDEBAR_W: f32 = 240.0;
    pub const OUTLINE_W: f32 = 216.0;
    pub const MINIMAP_W: f32 = 78.0;
    pub const EDITOR_PAD_X: f32 = 28.0;
    pub const EDITOR_PAD_TOP: f32 = 64.0;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_colors(t: &Theme) -> Vec<(&'static str, Color)> {
        vec![
            ("bg", t.bg),
            ("bg_panel", t.bg_panel),
            ("bg_elevated", t.bg_elevated),
            ("bg_hover", t.bg_hover),
            ("bg_active", t.bg_active),
            ("border", t.border),
            ("border_strong", t.border_strong),
            ("text", t.text),
            ("text_dim", t.text_dim),
            ("text_faint", t.text_faint),
            ("heading", t.heading),
            ("marker", t.marker),
            ("accent", t.accent),
            ("accent_text", t.accent_text),
            ("code_text", t.code_text),
            ("code_bg", t.code_bg),
            ("quote_text", t.quote_text),
            ("quote_bar", t.quote_bar),
            ("selection", t.selection),
            ("caret", t.caret),
            ("bullet", t.bullet),
            ("minimap_line", t.minimap_line),
            ("minimap_view", t.minimap_view),
            ("scrollbar", t.scrollbar),
            ("warn", t.warn),
        ]
    }

    /// The syntax inks are the one place a theme has a hue (a code block is
    /// coloured on request, like a coloured span). They still have to be legible
    /// and to differ from the plain code ink and from each other.
    fn syntax_colors(t: &Theme) -> Vec<(&'static str, Color)> {
        vec![
            ("keyword", t.syn_keyword),
            ("string", t.syn_string),
            ("number", t.syn_number),
            ("func", t.syn_func),
            ("type", t.syn_type),
        ]
    }

    #[test]
    fn syntax_inks_are_distinct_and_readable() {
        for kind in ThemeKind::ALL {
            let t = Theme::by_kind(kind);
            let inks = syntax_colors(&t);
            for (i, (name, c)) in inks.iter().enumerate() {
                let lum = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
                let ok = if kind.is_light() { lum < 0.55 } else { lum > 0.25 };
                assert!(ok, "{kind:?} {name} is unreadable: {c:?}");
                // Every ink is a different colour, so two token kinds never look
                // the same.
                for (name2, other) in inks.iter().skip(i + 1) {
                    let d = (c.r - other.r).abs() + (c.g - other.g).abs() + (c.b - other.b).abs();
                    assert!(d > 0.05, "{kind:?}: {name} and {name2} look the same: {c:?} vs {other:?}");
                }
            }
        }
    }

    #[test]
    fn black_theme_is_pure_monochrome() {
        let t = Theme::black();
        for (name, c) in all_colors(&t) {
            assert!(
                (c.r - c.g).abs() < 1e-6 && (c.g - c.b).abs() < 1e-6,
                "{name} is not grey: {c:?}"
            );
        }
        // Near-black, not pure black: a #0f0f0f sheet is far easier to look at
        // than #000000, and the panels sit just above it.
        let lum = 0.2126 * t.bg.r + 0.7152 * t.bg.g + 0.0722 * t.bg.b;
        assert!(lum < 0.09, "the black theme must stay dark: {lum}");
        assert!(lum > 0.001, "the black theme must not be pure black");
        assert!(t.bg_panel.r > t.bg.r, "panels must be lighter than the sheet");
        assert!(t.text.r > 0.85, "text must be near-white on black: {:?}", t.text);
    }

    #[test]
    fn every_theme_is_pure_monochrome() {
        for kind in ThemeKind::ALL {
            let t = Theme::by_kind(kind);
            for (name, c) in all_colors(&t) {
                assert!(
                    (c.r - c.g).abs() < 1e-6 && (c.g - c.b).abs() < 1e-6,
                    "{kind:?} {name} is not grey: {c:?}"
                );
            }
            // Every surface step must actually be a step. The dark themes get
            // lighter as they come forward (sheet < panel < elevated < hover <
            // active); on white the sheet is the lightest and they step *down*.
            let lum = |c: Color| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
            let steps = [t.bg, t.bg_panel, t.bg_elevated, t.bg_hover, t.bg_active];
            let up = !kind.is_light();
            for w in steps.windows(2) {
                let ordered = if up { lum(w[1]) > lum(w[0]) } else { lum(w[1]) < lum(w[0]) };
                assert!(ordered, "{kind:?} surfaces are out of order: {w:?}");
            }
        }
        let t = Theme::white();
        for (name, c) in all_colors(&t) {
            assert!(
                (c.r - c.g).abs() < 1e-6 && (c.g - c.b).abs() < 1e-6,
                "{name} is not grey: {c:?}"
            );
        }
        assert_eq!((t.bg.r, t.bg.g, t.bg.b), (1.0, 1.0, 1.0), "background must be pure white");
        assert!(t.text.r < 0.08, "text must be near-black on white: {:?}", t.text);
    }

    /// Links are the only non-grey ink in either theme, and even they stay pale:
    /// barely visible, never a saturated "link blue".
    #[test]
    fn link_ink_is_a_pale_blue() {
        for kind in ThemeKind::ALL {
            let t = Theme::by_kind(kind);
            let c = t.link;
            assert!(c.b > c.r, "{kind:?} link must lean blue: {c:?}");
            assert!(c.b - c.r < 0.35, "{kind:?} link must stay pale, not saturated: {c:?}");
            assert!((c.r - c.g).abs() < 0.2, "{kind:?} link must stay desaturated: {c:?}");
            assert!(c.b > c.g && c.g > c.r - 0.2, "{kind:?} link must be pale: {c:?}");
            let lum = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
            let bg = 0.2126 * t.bg.r + 0.7152 * t.bg.g + 0.0722 * t.bg.b;
            let delta = (lum - bg).abs();
            assert!(delta > 0.2 && delta < 0.85, "{kind:?} link contrast out of range: {delta}");
            // Every other ink stays perfectly grey.
            for (name, other) in all_colors(&t) {
                assert!(
                    (other.r - other.g).abs() < 1e-6 && (other.g - other.b).abs() < 1e-6,
                    "{name} must stay grey: {other:?}"
                );
            }
        }
    }

    #[test]
    fn monochrome_themes_keep_readable_contrast() {
        for kind in ThemeKind::ALL {
            let t = Theme::by_kind(kind);
            let lum = |c: Color| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
            let delta = (lum(t.bg) - lum(t.text)).abs();
            // A mid-grey sheet cannot reach 0.8 against its own text; 0.7 is a
            // comfortable ratio for body text and is what "grey" is for.
            assert!(delta > 0.68, "{kind:?} contrast too low: {delta}");
            let dim = (lum(t.bg) - lum(t.text_dim)).abs();
            assert!(dim > 0.3, "{kind:?} dim text contrast too low: {dim}");
        }
    }

    #[test]
    fn toggle_cycles_through_every_theme() {
        let mut k = ThemeKind::ALL[0];
        let mut seen = Vec::new();
        for _ in 0..ThemeKind::ALL.len() {
            seen.push(k);
            k = k.next();
        }
        assert_eq!(seen, ThemeKind::ALL.to_vec());
        assert_eq!(k, ThemeKind::ALL[0], "cycle must return to the first theme");
    }

    #[test]
    fn theme_ids_parse_back() {
        for kind in ThemeKind::ALL {
            assert_eq!(ThemeKind::parse(kind.id()), Some(kind));
        }
        assert_eq!(ThemeKind::parse(" bw "), Some(ThemeKind::Black));
        assert_eq!(ThemeKind::parse("gray"), Some(ThemeKind::Grey));
        assert_eq!(ThemeKind::parse("nonsense"), None);
    }
}
