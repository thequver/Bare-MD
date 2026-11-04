//! Bare MD — a portable, pure-Rust WYSIWYG Markdown editor.
//!
//! The crate is split by *layer*, not by widget:
//!
//! ```text
//! platform/   winit + wgpu: the window, the event loop, the renderer, and the
//!             translation of OS events into editor commands
//! text/       the text rendering engine: shaping, metrics, glyph atlas lookups
//! ui/         everything drawn *around* the document: painter primitives,
//!             buttons, chips, panels, dialogs, themes, icons
//! markdown/   everything that knows about Markdown: the parser, the line
//!             layout, syntax highlighting, mermaid, documents and search
//! editor/     the editor state itself: App, settings, workspaces
//! ```
//!
//! `geom` holds the two primitives every layer shares (a `Rect` and a `Color`).
//!
//! For compatibility the old flat names stay valid at the root (`crate::app`,
//! `crate::layout`, …), so a module is never imported by its *old* path in new
//! code while the re-exports keep older paths working.

pub mod editor;
pub mod geom;
pub mod markdown;
pub mod platform;
pub mod text;
pub mod ui;

// ---- the layer aliases ----
pub use editor::{app, config, workspace};
pub use markdown::{doc, highlight, layout, media, mermaid, search};
pub use platform::{gpu, input};
pub use ui::{appicon, field, icons, svg, theme};
