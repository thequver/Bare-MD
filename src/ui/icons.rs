//! The icon set, embedded straight from `assets/icons/*.svg`.
//!
//! Editing an icon means editing (or adding) an SVG file: nothing in the Rust
//! source knows what a bold glyph looks like. Files are rasterised on demand by
//! [`crate::svg`] and cached in the renderer's atlas, tinted with the theme's ink
//! colour, so one file serves both the black and the white theme.

/// Every icon the editor knows, as `(name, svg source)`.
///
/// The name is what the drawing code asks for (`p.icon("bold", …)`); the file is
/// `assets/icons/<name>.svg`.
pub const ALL: &[(&str, &str)] = &[
    ("align", include_str!("../../assets/icons/align.svg")),
    ("attach", include_str!("../../assets/icons/attach.svg")),
    ("bold", include_str!("../../assets/icons/bold.svg")),
    ("brackets", include_str!("../../assets/icons/brackets.svg")),
    ("bullet", include_str!("../../assets/icons/bullet.svg")),
    ("check", include_str!("../../assets/icons/check.svg")),
    ("chevron-up", include_str!("../../assets/icons/chevron-up.svg")),
    ("chevron", include_str!("../../assets/icons/chevron.svg")),
    ("clear", include_str!("../../assets/icons/clear.svg")),
    ("close", include_str!("../../assets/icons/close.svg")),
    ("code", include_str!("../../assets/icons/code.svg")),
    ("copy", include_str!("../../assets/icons/copy.svg")),
    ("color", include_str!("../../assets/icons/color.svg")),
    ("eye", include_str!("../../assets/icons/eye.svg")),
    ("file", include_str!("../../assets/icons/file.svg")),
    ("folder-open", include_str!("../../assets/icons/folder-open.svg")),
    ("folder", include_str!("../../assets/icons/folder.svg")),
    ("grip", include_str!("../../assets/icons/grip.svg")),
    ("heading", include_str!("../../assets/icons/heading.svg")),
    ("highlight", include_str!("../../assets/icons/highlight.svg")),
    ("image", include_str!("../../assets/icons/image.svg")),
    ("italic", include_str!("../../assets/icons/italic.svg")),
    ("link", include_str!("../../assets/icons/link.svg")),
    ("minimap", include_str!("../../assets/icons/minimap.svg")),
    ("movedown", include_str!("../../assets/icons/movedown.svg")),
    ("moveup", include_str!("../../assets/icons/moveup.svg")),
    ("ordered", include_str!("../../assets/icons/ordered.svg")),
    ("outline", include_str!("../../assets/icons/outline.svg")),
    ("plus", include_str!("../../assets/icons/plus.svg")),
    ("quote", include_str!("../../assets/icons/quote.svg")),
    ("redo", include_str!("../../assets/icons/redo.svg")),
    ("reload", include_str!("../../assets/icons/reload.svg")),
    ("renumber", include_str!("../../assets/icons/renumber.svg")),
    ("save", include_str!("../../assets/icons/save.svg")),
    ("search", include_str!("../../assets/icons/search.svg")),
    ("sidebar", include_str!("../../assets/icons/sidebar.svg")),
    ("strike", include_str!("../../assets/icons/strike.svg")),
    ("sub", include_str!("../../assets/icons/sub.svg")),
    ("sup", include_str!("../../assets/icons/sup.svg")),
    ("table", include_str!("../../assets/icons/table.svg")),
    ("task", include_str!("../../assets/icons/task.svg")),
    ("theme", include_str!("../../assets/icons/theme.svg")),
    ("tools", include_str!("../../assets/icons/tools.svg")),
    ("underline", include_str!("../../assets/icons/underline.svg")),
    ("undo", include_str!("../../assets/icons/undo.svg")),
    ("wrap", include_str!("../../assets/icons/wrap.svg")),
];

/// SVG source for an icon name.
pub fn source(name: &str) -> Option<&'static str> {
    ALL.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_has_a_unique_name_and_parses() {
        let mut seen = std::collections::HashSet::new();
        for (name, src) in ALL {
            assert!(seen.insert(*name), "duplicate icon {name}");
            assert!(src.contains("<svg"), "{name} is not an svg");
            assert!(
                crate::svg::parse(src).is_some(),
                "{name}.svg has nothing drawable in it"
            );
        }
        assert!(ALL.len() >= 30, "the icon set is unusually small");
    }

    #[test]
    fn drawing_code_only_asks_for_icons_that_exist() {
        // The toolbar and panels are checked through `source`: a typo in a name
        // shows up here instead of as a silently blank button.
        for name in [
            "bold", "italic", "underline", "strike", "highlight", "code", "link", "clear",
            "heading", "sup", "sub", "align", "color", "bullet", "ordered", "task", "quote",
            "renumber", "moveup", "movedown", "undo", "redo", "attach", "table", "tools",
            "chevron", "chevron-up", "folder", "folder-open", "file", "sidebar", "outline",
            "minimap", "reload", "theme", "close", "plus",
        ] {
            assert!(source(name).is_some(), "missing icon: {name}");
        }
    }
}
