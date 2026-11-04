//! The editor itself: the state, the settings and the workspaces.
//!
//! ```text
//! app.rs        App: every tab, the caret, the menus, the dialogs, the commands
//! config.rs     settings.conf and session.conf: the on-disk shape of the state
//! workspace.rs  the folder tree in the left panel
//! ```
//!
//! Nothing here draws or touches the OS: the state is advanced by commands
//! (`App::insert_text`, `run_toolbar`, `handle_shortcut`, `handle_event`) and read
//! back by `crate::ui` for painting, which is what makes the whole editor testable
//! and renderable offscreen.

pub mod app;
pub mod config;
pub mod workspace;
