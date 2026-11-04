//! Bare MD — a portable, pure-Rust WYSIWYG Markdown editor.
//!
//! Rendering is a from-scratch wgpu quad renderer (see `platform/gpu/`), text
//! shaping is rustybuzz, rasterisation is fontdue. No web views, no Electron, no
//! Tauri.
//!
//! The binary is deliberately thin: everything it does lives in
//! [`bare_md::platform::window`], next to the rest of the OS integration.
//!
//! On Windows a release build is a GUI subsystem binary, so double-clicking the
//! .exe does not flash a console window in front of the editor.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    bare_md::platform::run();
}
