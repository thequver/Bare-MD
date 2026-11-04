//! The application icon, built from the SVG files in `assets/app/`.
//!
//! The window icon is a real, editable asset rather than a blob in the source
//! tree: `icon-bg.svg` is the plate, `icon-fg.svg` is the mark drawn on top of
//! it. Both are plain SVG, rasterised by [`crate::svg`] (the same tiny
//! rasteriser the toolbar icons use) and composited here into the RGBA bitmap
//! winit wants.
//!
//! Editing the icon means editing one of those two files — no Rust changes.

use crate::svg;

/// RGBA pixels for the icon at `px` pixels square, ready for `Icon::from_rgba`.
pub const ICON_SVG: &str = include_str!("../../assets/app/icon.svg");

pub fn rgba(px: u32) -> (u32, u32, Vec<u8>) {
    let size = px.clamp(16, 512);
    if let Some(layer) = svg::parse(ICON_SVG).filter(|s| !s.is_empty()) {
        let (w, h, pixels) = svg::rasterize_rgba(&layer, size);   // own colours
        if pixels.chunks_exact(4).any(|p| p[3] > 8) {                  // not blank
            return (w, h, pixels);
        }
    }
    (4, 4, vec![0; 16])
}

/// The icon winit takes, at the size desktops ask for most often.
pub fn window_icon() -> winit::window::Icon {
    let (w, h, rgba) = rgba(128);
    winit::window::Icon::from_rgba(rgba, w, h).expect("icon bitmap is not empty")
}
