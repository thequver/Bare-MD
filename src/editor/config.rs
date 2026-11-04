//! Portable configuration and session persistence.
//!
//! Everything lives in **one** `settings.conf` file next to the executable.
//! Nothing is written to `%APPDATA%`, `%LOCALAPPDATA%`, `~/.config` or the
//! registry, and no extra folder is created: the binary plus that one file is
//! the whole install.
//!
//! The file is resolved as:
//!
//!   1. `$BAREMD_DATA_DIR/settings.conf` if the variable is set,
//!   2. `settings.conf` next to the executable,
//!   3. the user's own data folder (`$XDG_DATA_HOME/bare-md` or
//!      `~/.local/share/bare-md`) when the install folder is read-only - which is
//!      the normal case on Linux, where a package installs the binary into
//!      `/usr/bin` and the user cannot write there,
//!   4. `<cwd>/.bare-md/settings.conf` when even that is unavailable.

use std::fs;
use std::path::{Path, PathBuf};

use crate::layout::ViewMode;
use crate::theme::ThemeKind;

/// Settings and the session share one file; this line separates the sections.
const SESSION_MARKER: &str = "# --- session (written by Bare MD, safe to delete) ---";

#[derive(Clone, Debug)]
pub struct Paths {
    pub data_dir: PathBuf,
    pub config_file: PathBuf,
    /// Human readable note about how the file location was chosen.
    pub origin: String,
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf()))
}

/// The platform's per-user data folder for this app.
///
/// `$XDG_DATA_HOME` (or `~/.local/share`) on Linux and other Unixes,
/// `%APPDATA%` on Windows, `~/Library/Application Support` on macOS.
fn user_data_dir() -> Option<PathBuf> {
    use std::path::PathBuf;
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()) {
        return Some(dir.join("bare-md"));
    }
    #[cfg(windows)]
    {
        if let Some(dir) = std::env::var_os("APPDATA").map(PathBuf::from) {
            return Some(dir.join("bare-md"));
        }
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        #[cfg(target_os = "macos")]
        {
            return Some(home.join("Library/Application Support/bare-md"));
        }
        #[cfg(not(target_os = "macos"))]
        {
            return Some(home.join(".local/share/bare-md"));
        }
    }
    None
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(".bare-md-write-test");
    match fs::write(&probe, b"") {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Write `text` to `path` without ever leaving a half-written file behind.
///
/// The settings file is read by anything that starts the program, and it is
/// rewritten every ten seconds; `fs::write` truncates first, so a launch that
/// happens to look at that instant sees an empty (or partial) file, falls back to
/// the defaults and opens its window in the wrong place. A temporary file plus a
/// rename is atomic: a reader sees either the old content or the new one.
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("conf.tmp");
    fs::write(&tmp, text)?;
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            // A filesystem that cannot rename over an existing file still gets the
            // content written; it just loses the atomicity.
            let _ = fs::remove_file(&tmp);
            fs::write(path, text).map_err(|_| e)
        }
    }
}

impl Paths {
    pub fn resolve() -> Paths {
        let mut origin = String::new();
        // `BAREMD_DATA_DIR` is the documented name; `MARKPAD_DATA_DIR` is still
        // honoured so a profile set up for the app under its old name keeps
        // working after the rename.
        let env = std::env::var("BAREMD_DATA_DIR")
            .or_else(|_| std::env::var("MARKPAD_DATA_DIR"))
            .ok()
            .filter(|v| !v.is_empty());
        let dir = match env {
            Some(env) => {
                origin = format!("BAREMD_DATA_DIR={env}");
                PathBuf::from(env)
            }
            None => Paths::exe_or_fallback(&mut origin),
        };
        Paths {
            config_file: dir.join("settings.conf"),
            data_dir: dir,
            origin,
        }
    }

    /// The executable's own folder - Bare MD is portable by default, so its
    /// settings file simply sits next to the binary.
    fn exe_or_fallback(origin: &mut String) -> PathBuf {
        if let Some(dir) = exe_dir() {
            if writable(&dir) {
                *origin = format!("next to the executable: {}", dir.display());
                return dir;
            }
        }
        // A packaged install (`/usr/bin/bare-md`, a read-only AppImage mount, an
        // immutable OS) cannot keep its settings beside the binary, and dropping
        // them in the current directory would litter whatever folder the user
        // happened to start it from - which is `/` for a desktop launcher. The
        // per-user data folder is the place the platform provides for exactly
        // this.
        if let Some(dir) = user_data_dir() {
            if fs::create_dir_all(&dir).is_ok() && writable(&dir) {
                *origin = format!("per-user data folder (install dir not writable): {}", dir.display());
                return dir;
            }
        }
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let fallback = cwd.join(".bare-md");
        *origin = format!("fallback (no writable user folder): {}", fallback.display());
        let _ = fs::create_dir_all(&fallback);
        fallback
    }

    /// Everything from the `# --- session ---` marker down, as raw text.
    fn session_block(&self) -> String {
        let Ok(text) = fs::read_to_string(&self.config_file) else {
            return String::new();
        };
        match text.find(SESSION_MARKER) {
            Some(i) => text[i + SESSION_MARKER.len()..].to_string(),
            None => String::new(),
        }
    }

    /// Everything above the session marker, as raw text.
    fn settings_block(&self) -> String {
        let Ok(text) = fs::read_to_string(&self.config_file) else {
            return String::new();
        };
        match text.find(SESSION_MARKER) {
            Some(i) => text[..i].to_string(),
            None => text,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub theme: ThemeKind,
    pub font_size: f32,
    pub sidebar_open: bool,
    pub outline_open: bool,
    pub minimap_open: bool,
    /// Auto-close brackets and quotes while typing: `(` writes `()`, `"` writes `""`,
    /// typing a closing character that is already there steps over it, and typing one
    /// around a selection wraps it. Off means every character is typed exactly as pressed.
    pub autoclose: bool,
    /// Break long lines into visual rows at the width of the text column ("soft
    /// wrap"). Off means one row per line and the view scrolls sideways.
    pub wrap: bool,
    /// Live / Live+ / Raw, remembered between runs.
    pub view_mode: ViewMode,
    pub workspace: Option<PathBuf>,
    /// Every workspace tab that was open, in strip order (`workspace` is the
    /// active one).
    pub workspaces: Vec<PathBuf>,
    pub recent: Vec<PathBuf>,
    /// Inner size of the window, in logical pixels.
    pub window: Option<(f32, f32)>,
    /// Outer position of the window's top-left corner, in logical pixels.
    ///
    /// Kept as *logical* coordinates because that is what the window system hands
    /// back and what `set_outer_position` takes: on a 150 % display the physical
    /// pixels change when the user moves the window to another monitor, and a
    /// saved physical position would jump across the desk.
    pub window_pos: Option<(i32, i32)>,
    /// Whether the window was maximized. The size above is then the size to come
    /// back *to* when it is un-maximized, so it is not overwritten while maximized.
    pub window_maximized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeKind::Black,
            font_size: 17.0,
            sidebar_open: true,
            outline_open: false,
            minimap_open: true,
            autoclose: true,
            wrap: true,
            view_mode: ViewMode::Live,
            workspace: None,
            workspaces: Vec::new(),
            recent: Vec::new(),
            window: None,
            window_pos: None,
            window_maximized: false,
        }
    }
}

impl Settings {
    pub fn load(paths: &Paths) -> Settings {
        let mut s = Settings::default();
        let Ok(text) = fs::read_to_string(&paths.config_file) else {
            return s;
        };
        let text = text.split(SESSION_MARKER).next().unwrap_or("").to_string();
        for line in text.lines() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once(' ') else { continue };
            match key {
                "theme" => {
                    if let Some(kind) = ThemeKind::parse(value) {
                        s.theme = kind;
                    }
                }
                "font_size" => s.font_size = value.parse().unwrap_or(17.0),
                "sidebar" => s.sidebar_open = value == "1",
                "outline" => s.outline_open = value == "1",
                "minimap" => s.minimap_open = value == "1",
                "autoclose" => s.autoclose = value == "1",
                "wrap" => s.wrap = value == "1",
                "view" => {
                    if let Some(mode) = ViewMode::parse(value) {
                        s.view_mode = mode;
                    }
                }
                "workspace" => s.workspace = Some(PathBuf::from(value)),
                "workspaces" => s.workspaces.push(PathBuf::from(value)),
                "recent" => s.recent.push(PathBuf::from(value)),
                "window" => {
                    if let Some((w, h)) = value.split_once('x') {
                        if let (Ok(w), Ok(h)) = (w.parse(), h.parse()) {
                            s.window = Some((w, h));
                        }
                    }
                }
                "window_pos" => {
                    if let Some((x, y)) = value.split_once(',') {
                        if let (Ok(x), Ok(y)) = (x.parse(), y.parse()) {
                            // A position must be paired with a size to be usable.
                            s.window_pos = Some((x, y));
                        }
                    }
                }
                "window_maximized" => s.window_maximized = value == "1",
                _ => {}
            }
        }
        s
    }

    pub fn save(&self, paths: &Paths) {
        let session = paths.session_block();
        let mut out = String::new();
        out.push_str("# Bare MD settings - portable, lives next to the executable\n");
        out.push_str(&format!("theme {}\n", self.theme.id()));
        out.push_str(&format!("font_size {}\n", self.font_size));
        out.push_str(&format!("sidebar {}\n", self.sidebar_open as u8));
        out.push_str(&format!("outline {}\n", self.outline_open as u8));
        out.push_str(&format!("minimap {}\n", self.minimap_open as u8));
        out.push_str(&format!("autoclose {}\n", self.autoclose as u8));
        out.push_str(&format!("wrap {}\n", self.wrap as u8));
        out.push_str(&format!("view {}\n", self.view_mode.id()));
        if let Some(ws) = &self.workspace {
            out.push_str(&format!("workspace {}\n", ws.display()));
        }
        for ws in self.workspaces.iter().take(8) {
            out.push_str(&format!("workspaces {}\n", ws.display()));
        }
        for p in self.recent.iter().take(12) {
            out.push_str(&format!("recent {}\n", p.display()));
        }
        if let Some((w, h)) = self.window {
            out.push_str(&format!("window {}x{}\n", w.round() as u32, h.round() as u32));
        }
        if let Some((x, y)) = self.window_pos {
            out.push_str(&format!("window_pos {x},{y}\n"));
        }
        out.push_str(&format!("window_maximized {}\n", self.window_maximized as u8));
        out.push('\n');
        out.push_str(SESSION_MARKER);
        out.push_str(&session);
        let _ = fs::create_dir_all(&paths.data_dir);
        let _ = write_atomic(&paths.config_file, &out);
    }

    pub fn push_recent(&mut self, path: &Path) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(12);
    }
}

/// Smallest window the layout can still be used at, in logical pixels.
pub const MIN_WINDOW: (f32, f32) = (520.0, 360.0);

/// A remembered window size, made to fit the screens that exist now.
///
/// Settings travel: a size saved on a 4K desktop is half a laptop panel when the
/// same folder is opened there, and a window larger than the screen hides its own
/// title bar off the edge. The size is clamped to the largest screen (and to the
/// minimum), which is the one thing that can be done without guessing.
pub fn fit_window(size: (f32, f32), screens: &[(f32, f32, f32, f32)]) -> (f32, f32) {
    let mut w = size.0.max(MIN_WINDOW.0);
    let mut h = size.1.max(MIN_WINDOW.1);
    if let Some((mw, mh)) = screens
        .iter()
        .map(|&(_, _, sw, sh)| (sw.max(MIN_WINDOW.0), sh.max(MIN_WINDOW.1)))
        .fold(None, |acc: Option<(f32, f32)>, (sw, sh)| {
            Some(match acc {
                Some((aw, ah)) => (aw.max(sw), ah.max(sh)),
                None => (sw, sh),
            })
        })
    {
        w = w.min(mw);
        h = h.min(mh);
    }
    (w, h)
}

/// Whether a window rectangle (`pos` and logical `size`) overlaps a screen that
/// exists *now*.
///
/// The saved position is replayed on the next start, and a monitor that has since
/// been unplugged, rotated away or rearranged would put the window somewhere the
/// user cannot see or reach it. A window is only put back where it was when at
/// least a grabbable corner of it - 80 x 40 logical pixels - is on a real screen;
/// otherwise the window system's own placement is used.
pub fn rect_on_screens(pos: (i32, i32), size: (f32, f32), screens: &[(f32, f32, f32, f32)]) -> bool {
    if screens.is_empty() {
        // Nothing to check against: trust the saved value rather than throwing it
        // away (a headless or not-yet-enumerated session).
        return true;
    }
    const GRAB_W: f32 = 80.0;
    const GRAB_H: f32 = 40.0;
    let (x0, y0) = (pos.0 as f32, pos.1 as f32);
    let (x1, y1) = (x0 + size.0.max(1.0), y0 + size.1.max(1.0));
    screens.iter().any(|&(sx, sy, sw, sh)| {
        let (sx1, sy1) = (sx + sw, sy + sh);
        let w = x1.min(sx1) - x0.max(sx);
        let h = y1.min(sy1) - y0.max(sy);
        w >= GRAB_W.min(sw) && h >= GRAB_H.min(sh)
    })
}

#[derive(Clone, Debug, Default)]
pub struct SessionTab {
    pub path: Option<PathBuf>,
    pub caret: usize,
    pub scroll: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Session {
    pub tabs: Vec<SessionTab>,
    pub active: usize,
}

impl Session {
    pub fn load(paths: &Paths) -> Session {
        let mut s = Session::default();
        let Ok(text) = fs::read_to_string(&paths.config_file) else { return s };
        let Some(pos) = text.find(SESSION_MARKER) else { return s };
        let body = &text[pos + SESSION_MARKER.len()..];
        for line in body.lines() {
            if let Some(rest) = line.strip_prefix("active ") {
                s.active = rest.parse().unwrap_or(0);
            } else if let Some(rest) = line.strip_prefix("tab ") {
                // tab <caret> <scroll> <path or empty>
                let mut parts = rest.splitn(3, ' ');
                let caret = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                let scroll = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                let path = parts.next().filter(|p| !p.is_empty()).map(PathBuf::from);
                s.tabs.push(SessionTab { path, caret, scroll });
            }
        }
        s
    }

    pub fn save(&self, paths: &Paths) {
        let mut out = String::new();
        out.push_str(&format!("active {}\n", self.active));
        for t in &self.tabs {
            out.push_str(&format!(
                "tab {} {} {}\n",
                t.caret,
                t.scroll.round() as i64,
                t.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default()
            ));
        }
        let mut file = paths.settings_block();
        file.push('\n');
        file.push_str(SESSION_MARKER);
        file.push('\n');
        file.push_str(&out);
        let _ = fs::create_dir_all(&paths.data_dir);
        let _ = write_atomic(&paths.config_file, &file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_geometry_roundtrips() {
        let dir = std::env::temp_dir().join(format!("bare-md-window-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let paths = Paths {
            data_dir: dir.clone(),
            config_file: dir.join("settings.conf"),
            origin: "test".into(),
        };
        let mut s = Settings::default();
        s.window = Some((1440.0, 900.0));
        s.window_pos = Some((120, -40));
        s.window_maximized = true;
        s.save(&paths);
        let back = Settings::load(&paths);
        assert_eq!(back.window, Some((1440.0, 900.0)), "the window size was not kept");
        assert_eq!(back.window_pos, Some((120, -40)), "the window position was not kept");
        assert!(back.window_maximized, "the maximized flag was not kept");
        // …and a settings file from before this existed still loads.
        fs::write(&paths.config_file, "theme black\nfont_size 17\nwindow 1180x780\n").unwrap();
        let old = Settings::load(&paths);
        assert_eq!(old.window, Some((1180.0, 780.0)));
        assert_eq!(old.window_pos, None);
        assert!(!old.window_maximized);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_remembered_size_is_made_to_fit_the_screen_it_opens_on() {
        let laptop = [(0.0, 0.0, 1366.0, 768.0)];
        // Saved on a big desktop, opened on a laptop.
        assert_eq!(fit_window((2560.0, 1440.0), &laptop), (1366.0, 768.0));
        // Fits already: untouched.
        assert_eq!(fit_window((1180.0, 740.0), &laptop), (1180.0, 740.0));
        // One pixel too tall for the panel is still too tall.
        assert_eq!(fit_window((1180.0, 780.0), &laptop), (1180.0, 768.0));
        // Absurdly small: the minimum wins.
        assert_eq!(fit_window((100.0, 80.0), &laptop), MIN_WINDOW);
        // No screens known: only the minimum applies.
        assert_eq!(fit_window((2000.0, 1200.0), &[]), (2000.0, 1200.0));
        // Two screens: the largest one decides.
        let two = [(0.0, 0.0, 1280.0, 720.0), (1280.0, 0.0, 2560.0, 1440.0)];
        assert_eq!(fit_window((3000.0, 2000.0), &two), (2560.0, 1440.0));
    }

    #[test]
    fn settings_are_written_atomically() {
        let dir = std::env::temp_dir().join(format!("bare-md-atomic-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let paths = Paths {
            data_dir: dir.clone(),
            config_file: dir.join("settings.conf"),
            origin: "test".into(),
        };
        let mut s = Settings::default();
        s.window = Some((1200.0, 800.0));
        s.save(&paths);
        // The temporary file is gone and the real one parses.
        assert!(!dir.join("settings.conf.tmp").exists(), "a temporary file was left behind");
        assert_eq!(Settings::load(&paths).window, Some((1200.0, 800.0)));
        // A reader never sees a partially written file: it either parses or it is
        // the previous content, never empty.
        for _ in 0..20 {
            s.font_size += 1.0;
            s.save(&paths);
            let text = fs::read_to_string(&paths.config_file).unwrap();
            assert!(text.contains("font_size"), "a reader saw a truncated settings file");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_window_is_only_restored_onto_a_screen_that_exists() {
        let screens = [(0.0, 0.0, 1920.0, 1080.0), (1920.0, 0.0, 1280.0, 1024.0)];
        // On the first screen.
        assert!(rect_on_screens((100, 80), (1180.0, 780.0), &screens));
        // Straddling the seam between the two: still visible.
        assert!(rect_on_screens((1800, 100), (1180.0, 780.0), &screens));
        // On the second screen.
        assert!(rect_on_screens((2200, 100), (1180.0, 780.0), &screens));
        // A monitor that is gone: nothing of the window is grabbable.
        assert!(!rect_on_screens((5000, 3000), (1180.0, 780.0), &screens));
        // Dragged almost entirely off the left edge: the title bar is off-screen.
        assert!(!rect_on_screens((-1150, 200), (1180.0, 780.0), &screens));
        // Mostly off the right edge, but a grabbable strip remains.
        assert!(rect_on_screens((1800, 200), (1180.0, 780.0), &screens));
        // No screens at all: keep what was saved.
        assert!(rect_on_screens((100, 100), (1180.0, 780.0), &[]));
    }

    #[test]
    fn every_theme_roundtrips() {
        let dir = std::env::temp_dir().join(format!("bare-md-theme-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let paths = Paths {
            data_dir: dir.clone(),
            config_file: dir.join("settings.conf"),
            origin: "test".into(),
        };
        for kind in ThemeKind::ALL {
            let mut s = Settings::default();
            s.theme = kind;
            s.save(&paths);
            let back = Settings::load(&paths);
            assert_eq!(back.theme, kind, "theme {} did not round-trip", kind.id());
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_roundtrip() {
        let dir = std::env::temp_dir().join(format!("bare-md-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let paths = Paths {
            data_dir: dir.clone(),
            config_file: dir.join("settings.conf"),
            origin: "test".into(),
        };
        let mut s = Settings::default();
        s.font_size = 18.0;
        s.theme = ThemeKind::White;
        s.minimap_open = false;
        s.workspace = Some(PathBuf::from("/tmp/notes"));
        s.push_recent(Path::new("/tmp/notes/a.md"));
        s.push_recent(Path::new("/tmp/notes/b.md"));
        s.save(&paths);
        let back = Settings::load(&paths);
        assert_eq!(back.font_size, 18.0);
        assert_eq!(back.theme, ThemeKind::White);
        assert!(!back.minimap_open);
        assert_eq!(back.workspace.unwrap(), PathBuf::from("/tmp/notes"));
        assert_eq!(back.recent.len(), 2);
        assert_eq!(back.recent[0], PathBuf::from("/tmp/notes/b.md"));

        let session = Session {
            tabs: vec![SessionTab { path: Some(PathBuf::from("/tmp/notes/a.md")), caret: 12, scroll: 40.0 }],
            active: 0,
        };
        session.save(&paths);
        let s2 = Session::load(&paths);
        assert_eq!(s2.tabs.len(), 1);
        assert_eq!(s2.tabs[0].caret, 12);
        let _ = fs::remove_dir_all(&dir);
    }
}
