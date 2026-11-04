//! Application state and editing commands.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::{Paths, Session, Settings};
use crate::doc::{word_bounds, Doc};
use crate::geom::Rect;
use crate::layout::{Inks, Layout, ViewMode};
use crate::markdown::Block;
use crate::search::{replace_all, FindFocus, FindState};
use crate::text::{Face, TextEngine};
use crate::theme::{Theme, ThemeKind};
use crate::workspace::Workspace;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Editor,
    FindQuery,
    FindReplace,
    Dialog,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolbarAction {
    New,
    Open,
    OpenFolder,
    Save,
    SaveAs,
    ReloadFile,
    Undo,
    Redo,
    Bold,
    Italic,
    Underline,
    Strike,
    Highlight,
    Code,
    ClearFormat,
    H1,
    H2,
    H3,
    Sup,
    Sub,
    AlignLeft,
    AlignCenter,
    AlignRight,
    Bullet,
    Ordered,
    Task,
    Quote,
    Renumber,
    MoveUp,
    MoveDown,
    Link,
    AttachFile,
    Table,
    Hr,
    ToggleRaw,
    ToggleSidebar,
    ToggleOutline,
    ToggleMinimap,
    /// Turn bracket/quote auto-completion on or off.
    ToggleAutoclose,
    /// Turn soft wrap (long lines breaking at the edge) on or off.
    ToggleWrap,
    ToggleTheme,
    FindReplace,
}

/// Which dropdown a `Hit::MenuButton` opens.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuKind {
    Heading,
    SupSub,
    Align,
    Color,
    Table,
    Tools,
    /// Right-click menu: editing, and table commands when the caret is in one.
    Context,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditAction {
    Cut,
    Copy,
    Paste,
    SelectAll,
    Undo,
    Redo,
    Reload,
    Save,
    /// Save into a folder and under a name chosen in the file dialog (Ctrl+Shift+S).
    SaveAs,
    Find,
}

/// Whole-document / selection clean-ups, offered by the Tools menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolsAction {
    InsertEmptyRows,
    RemoveEmptyRows,
    SplitRows,
    MergeRows,
    DedupeRows,
    CompressSpaces,
    RemoveSpaces,
    ListToTable,
    TableToList,
}

/// Table edits, offered by the row/column grips and the context menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TableAction {
    InsertRowAbove,
    InsertRowBelow,
    InsertColLeft,
    InsertColRight,
    DeleteRow,
    DeleteCol,
    MoveRowUp,
    MoveRowDown,
    MoveColLeft,
    MoveColRight,
    DuplicateRow,
    /// Drop a dragged row / column at an index (0 is the header when there is one).
    MoveRowTo(usize),
    MoveColTo(usize),
}

#[derive(Clone, Debug)]
pub enum MenuAction {
    /// Replace this menu with another one (a submenu).
    Menu(MenuKind),
    Toolbar(ToolbarAction),
    Edit(EditAction),
    Tools(ToolsAction),
    Table(TableAction),
    Heading(u8),
    Align(u8),
    /// Set the text colour of the selection (`None` clears it).
    Fg(Option<u32>),
    /// Set the background colour of the selection (`None` clears it).
    Bg(Option<u32>),
    /// Open the custom colour picker (true = background).
    ColourPicker(bool),
    /// Open the About box.
    About,
}

#[derive(Clone, Debug)]
pub struct MenuEntry {
    pub label: String,
    /// Short right-aligned hint, e.g. a shortcut or a colour swatch.
    pub hint: String,
    pub action: MenuAction,
    pub checked: bool,
    pub swatch: Option<crate::geom::Color>,
    /// Draw a separating line above this entry.
    pub divider: bool,
}

#[derive(Clone, Debug)]
pub struct Menu {
    pub kind: MenuKind,
    pub x: f32,
    pub y: f32,
    pub entries: Vec<MenuEntry>,
    /// For panels that are laid out in **two columns** (the colour tool): how many
    /// entries belong to the left column. `0` means a plain single-column list.
    pub split: usize,
    /// Headings for the two columns of a split panel.
    pub heads: (&'static str, &'static str),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Hit {
    Toolbar(ToolbarAction),
    Tab(usize),
    TabClose(usize),
    NewTab,
    SidebarEntry(usize),
    SidebarUp,
    /// The "open folder" button in the workspace header.
    SidebarOpenFolder,
    /// The colour picker dialog: the saturation/value square, the hue strip, the
    /// preset row and the editable hex field.
    /// Saturation/value square of a column (`true` = background).
    ColourSv(bool),
    /// Hue strip of a column.
    ColourHue(bool),
    /// Hex field of a column.
    ColourHex(bool),
    /// One of the presets in a column's row.
    ColourPreset(bool, usize),
    /// The "no colour / colour" switch of a column.
    ColourClear(bool),
    ColourOk,
    ColourCancel,
    OutlineItem(usize),
    /// The drag handle of an outline row (section index).
    OutlineGrip(usize),
    /// The copy button on a code block: (first doc line, last doc line).
    CopyCode(usize, usize),
    FindQuery,
    FindReplace,
    FindNext,
    FindPrev,
    FindToggleCase,
    FindToggleWord,
    FindReplaceOne,
    FindReplaceAll,
    FindClose,
    Task(usize),
    Editor,
    /// A toolbar dropdown button.
    MenuButton(MenuKind),
    /// An entry of the open menu.
    MenuItem(usize),
    /// The "+" / grip widgets drawn on a hovering table (target, table first line).
    TableZone(TableAction, usize),
    /// Opens the row menu for (table first line, row index).
    RowGrip(usize, usize),
    /// Opens the column menu for (table first line, column index).
    ColGrip(usize, usize),
    Minimap,
    ScrollV,
    ScrollH,
    DialogItem(usize),
    DialogUp,
    /// The picker's address bar.
    DialogPathField,
    /// "go": navigate to whatever the address bar says.
    DialogConfirm,
    DialogCancel,
    DialogNameField,
    DialogPlaces(usize),
    StatusSave,
    /// The document path in the status bar: clicking it copies the path.
    StatusPath,
    /// Workspace tab in the sidebar header.
    WorkspaceTab(usize),
    /// The close button of a workspace tab.
    WorkspaceTabClose(usize),
    /// "New workspace" chip next to the workspace tabs (the header's folder
    /// button does the same thing).
    WorkspaceAdd,
    /// A folder in the folder dialog's left-hand tree (select it).
    DialogTreeItem(usize),
    /// The disclosure triangle of a tree row (expand / collapse it).
    DialogTreeChevron(usize),
    /// The three answers to "this file changed on disk".
    ExternalReload,
    ExternalKeep,
    ExternalAuto,
    /// The About box's buttons and its scrollbar.
    AboutClose,
    AboutSave,
    AboutScroll,
}

#[derive(Default)]
pub struct Hits {
    entries: Vec<(Rect, Hit)>,
}

impl Hits {
    pub fn new(entries: Vec<(Rect, Hit)>) -> Hits {
        Hits { entries }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn push(&mut self, r: Rect, h: Hit) {
        self.entries.push((r, h));
    }

    /// First rectangle registered for a hit matching `pred` (test/automation aid).
    pub fn find(&self, pred: impl Fn(&Hit) -> bool) -> Option<Rect> {
        self.entries.iter().find(|(_, h)| pred(h)).map(|(r, _)| *r)
    }

    /// Every registered hit, in draw order (test/automation aid).
    pub fn entries_for_test(&self) -> &[(Rect, Hit)] {
        &self.entries
    }

    /// Topmost hit at a point (later draws win).
    pub fn at(&self, p: (f32, f32)) -> Option<Hit> {
        self.entries
            .iter()
            .rev()
            .find(|(r, _)| r.contains(p))
            .map(|(_, h)| h.clone())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickerKind {
    OpenFile,
    OpenFolder,
    SaveAs,
    /// Pick a file to link to at the caret.
    Attach,
}

#[derive(Clone, Debug)]
pub struct PickEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

#[derive(Clone, Debug)]
pub struct Picker {
    pub kind: PickerKind,
    pub dir: PathBuf,
    pub entries: Vec<PickEntry>,
    pub selected: usize,
    pub scroll: f32,
    /// The file-name box (a real text field: caret, selection, editing).
    pub name: crate::field::TextField,
    pub error: Option<String>,
    /// The address bar: a real editable path, so a folder can be reached by
    /// typing it instead of walking down from wherever the picker opened.
    pub path: crate::field::TextField,
    /// Which field the keyboard goes to.
    pub field: PickField,
    /// First row of the tree that is on screen: the left column has its own
    /// scroll position, so the wheel over it moves the tree, not the listing.
    pub tree_scroll: usize,
    /// First entry of the right-hand listing that is on screen. The wheel over
    /// the listing scrolls the list itself; it used to move the *selection*
    /// instead, which looked like the list was stuck.
    pub list_scroll: usize,
    /// The left-hand column of the folder dialog: the filesystem as a tree.
    pub tree: DirTree,
}

/// One row of the folder dialog's tree: a directory, at its depth.
#[derive(Clone, Debug)]
pub struct TreeRow {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub expanded: bool,
    /// True when the folder has any sub-folder, so the triangle is drawn only
    /// where there is something to open.
    pub has_dirs: bool,
}

/// The directory tree in the left column of the folder dialog.
///
/// The right column is a plain listing of one folder; the tree is how you get to
/// that folder in the first place. Both are driven by the same `Picker`, so a
/// click on a tree row simply re-points the listing.
#[derive(Clone, Debug)]
pub struct DirTree {
    /// Every volume root on this machine, in drive order.
    ///
    /// The column used to start at the root of whichever volume the folder happened to be
    /// on, so on Windows you saw one drive and nothing else - "only the current disk is
    /// displayed", in a dialog whose whole left column is supposed to be the filesystem.
    /// A tree rooted at every volume is what every other file dialog shows.
    pub roots: Vec<PathBuf>,
    expanded: Vec<PathBuf>,
    rows: Vec<TreeRow>,
}

impl DirTree {
    pub fn new(roots: Vec<PathBuf>) -> DirTree {
        let mut t = DirTree { roots, expanded: Vec::new(), rows: Vec::new() };
        t.rebuild();
        t
    }

    pub fn rows(&self) -> &[TreeRow] {
        &self.rows
    }

    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.iter().any(|p| p == path)
    }

    /// Directory a tree row stands for (`None` when the index is stale).
    pub fn entry_dir(&self, i: usize) -> Option<PathBuf> {
        self.rows.get(i).map(|r| r.path.clone())
    }

    pub fn toggle(&mut self, path: &Path) {
        if let Some(i) = self.expanded.iter().position(|p| p == path) {
            self.expanded.remove(i);
        } else {
            self.expanded.push(path.to_path_buf());
        }
        self.rebuild();
    }

    /// Expand the volume `path` lives on, and every folder between it and `path`, so the
    /// folder is visible in the tree.
    pub fn reveal(&mut self, path: &Path) {
        let Some(root) = self.roots.iter().find(|r| path.starts_with(r)).cloned() else {
            self.rebuild();
            return;
        };
        if !self.is_expanded(&root) {
            self.expanded.push(root.clone());
        }
        let mut chain: Vec<PathBuf> = Vec::new();
        let mut cur = Some(path);
        while let Some(p) = cur {
            if p == root {
                break;
            }
            chain.push(p.to_path_buf());
            cur = p.parent();
        }
        for p in chain.into_iter().rev() {
            if !self.is_expanded(&p) {
                self.expanded.push(p);
            }
        }
        self.rebuild();
    }

    fn rebuild(&mut self) {
        self.rows.clear();
        for i in 0..self.roots.len() {
            let root = self.roots[i].clone();
            self.push_root_row(root.clone(), 0);
            if self.is_expanded(&root) {
                self.walk(root, 0);
            }
        }
    }

    /// One row for a row's own folder, with the triangle only when it has sub-folders.
    fn push_root_row(&mut self, dir: PathBuf, depth: usize) {
        let has_dirs = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten().any(|e| {
                    !e.file_name().to_string_lossy().starts_with('.')
                        && e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                })
            })
            .unwrap_or(false);
        // A volume is named by its whole path ("C:\", "/"): its last component is empty.
        let name = match dir.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => dir.display().to_string(),
        };
        self.rows.push(TreeRow {
            path: dir.clone(),
            name,
            depth,
            expanded: self.is_expanded(&dir),
            has_dirs,
        });
    }

    fn walk(&mut self, dir: PathBuf, depth: usize) {
        if depth > 24 {
            return;
        }
        let mut kids: Vec<(String, PathBuf)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                kids.push((name, e.path()));
            }
        }
        kids.sort_by_key(|(n, _)| n.to_lowercase());
        if !self.is_expanded(&dir) {
            return;
        }
        for (name, path) in kids {
            let mut sub = std::fs::read_dir(&path)
                .map(|rd| {
                    rd.flatten().any(|e| {
                        !e.file_name().to_string_lossy().starts_with('.')
                            && e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                    })
                })
                .unwrap_or(false);
            if !sub {
                sub = false;
            }
            self.rows.push(TreeRow {
                path: path.clone(),
                name,
                depth: depth + 1,
                expanded: self.is_expanded(&path),
                has_dirs: sub,
            });
            if self.is_expanded(&path) {
                self.walk(path, depth + 1);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickField {
    Path,
    Name,
}

impl Picker {
    pub fn new(kind: PickerKind, dir: PathBuf, name: String) -> Picker {
        // The left column is the filesystem, so it lists every volume; the volume the
        // folder is on is the one expanded down to it.
        let mut roots = drive_roots();
        if !roots.iter().any(|r| dir.starts_with(r)) {
            roots.push(tree_root(&dir));
        }
        if roots.is_empty() {
            roots.push(tree_root(&dir));
        }
        let tree = DirTree::new(roots);
        let mut name_field = crate::field::TextField::new();
        name_field.set(&name);
        let mut path_field = crate::field::TextField::new();
        path_field.set(&dir.display().to_string());
        let mut p = Picker {
            kind,
            dir,
            entries: Vec::new(),
            selected: 0,
            scroll: 0.0,
            name: name_field,
            error: None,
            path: path_field,
            field: PickField::Path,
            tree_scroll: 0,
            list_scroll: 0,
            tree,
        };
        p.refresh();
        p.reveal_selected();
        p
    }

    /// Point the picker at `path`. A file is accepted too: the picker moves to
    /// its folder and selects it, which is what a typed-in filename means.
    pub fn navigate(&mut self, path: &str) -> bool {
        let mut text = path.trim().to_string();
        if text.is_empty() {
            return false;
        }
        // Strip a leading `~` (typed, not shell-expanded).
        if text == "~" || text.starts_with("~/") {
            if let Some(home) = home_dir() {
                let rest = text.trim_start_matches('~').trim_start_matches('/');
                text = home.join(rest).display().to_string();
            }
        }
        let candidate = PathBuf::from(&text);
        let target = if candidate.is_dir() {
            candidate
        } else if candidate.is_file() {
            let parent = candidate.parent().map(|p| p.to_path_buf()).unwrap_or(candidate.clone());
            let name = candidate
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            self.dir = parent;
            self.refresh();
            if let Some(i) = self.entries.iter().position(|e| e.name == name) {
                self.selected = i;
            }
            self.name.set(&name);
            self.path.set(&self.dir.display().to_string());
            self.error = None;
            self.reveal_selected();
            return true;
        } else {
            self.error = Some(format!("no such folder: {text}"));
            return false;
        };
        self.dir = target;
        self.selected = 0;
        self.refresh();
        self.path.set(&self.dir.display().to_string());
        self.error = None;
        self.reveal_selected();
        true
    }

    /// Select `dir` in the tree and show its contents in the right column.
    pub fn select_dir(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.path.set(&self.dir.display().to_string());
        self.selected = 0;
        self.refresh();
        self.reveal_selected();
    }

    /// Keep the tree open at the folder being listed.
    pub fn reveal_selected(&mut self) {
        let dir = self.dir.clone();
        self.tree.reveal(&dir);
    }

    /// Expand / collapse a tree row.
    pub fn toggle_tree(&mut self, i: usize) {
        if let Some(dir) = self.tree.entry_dir(i) {
            self.tree.toggle(&dir);
        }
    }

    /// Type into whichever field the keyboard owns.
    /// Scroll the right-hand listing by `delta` rows (negative = up).
    pub fn scroll_list(&mut self, delta: i32, visible: usize) {
        let max_start = self.entries.len().saturating_sub(visible.max(1));
        let next = self.list_scroll.saturating_add_signed(delta as isize);
        self.list_scroll = next.min(max_start);
    }

    /// Scroll the tree column by `delta` rows (negative = up), clamped to its
    /// last screen.
    pub fn scroll_tree(&mut self, delta: i32, visible: usize) {
        let rows = self.tree.rows().len();
        let max_start = rows.saturating_sub(visible.max(1));
        let next = self.tree_scroll.saturating_add_signed(delta as isize);
        self.tree_scroll = next.min(max_start);
    }

    /// The field the keyboard owns.
    pub fn field_mut(&mut self) -> &mut crate::field::TextField {
        match self.field {
            PickField::Path => &mut self.path,
            PickField::Name => &mut self.name,
        }
    }

    /// Type into whichever field the keyboard owns.
    pub fn insert_text(&mut self, text: &str) {
        self.field_mut().insert(text);
    }

    /// Backspace in whichever field the keyboard owns.
    pub fn backspace(&mut self) {
        self.field_mut().backspace();
    }

    /// Forward delete in whichever field the keyboard owns.
    pub fn delete_forward(&mut self) {
        self.field_mut().delete_forward();
    }

    pub fn refresh(&mut self) {
        self.entries.clear();
        let mut dirs: Vec<PickEntry> = Vec::new();
        let mut files: Vec<PickEntry> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                let path = e.path();
                let entry = PickEntry { name: name.clone(), path, is_dir };
                if is_dir {
                    dirs.push(entry);
                } else {
                    files.push(entry);
                }
            }
        }
        let key = |e: &PickEntry| e.name.to_lowercase();
        dirs.sort_by_key(key);
        files.sort_by_key(key);
        let mut all = Vec::new();
        if let Some(parent) = self.dir.parent() {
            all.push(PickEntry { name: "..".into(), path: parent.to_path_buf(), is_dir: true });
        }
        // The filesystem is reachable, not just the folder the picker opened in:
        // the root of every mounted volume is one click away, and on Windows so
        // is every drive letter. Without this, "pick a folder on another disk"
        // was impossible.
        let root = self.dir.ancestors().last().map(|p| p.to_path_buf());
        if let Some(root) = root {
            if root != self.dir {
                all.push(PickEntry { name: root.display().to_string(), path: root, is_dir: true });
            }
        }
        if let Some(home) = home_dir() {
            if home != self.dir {
                all.push(PickEntry {
                    name: format!("~ {}", home.display()),
                    path: home,
                    is_dir: true,
                });
            }
        }
        for drive in drive_roots() {
            if drive != self.dir && !all.iter().any(|e| e.path == drive) {
                all.push(PickEntry { name: drive.display().to_string(), path: drive, is_dir: true });
            }
        }
        all.extend(dirs);
        all.extend(files);
        self.entries = all;
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
    }

    pub fn selected_entry(&self) -> Option<&PickEntry> {
        self.entries.get(self.selected)
    }
}

/// Where the dialog's tree should start: the root of the volume the current
/// folder lives on, so "the whole disk" is one scroll away but the tree does not
/// open on a hundred mount points.
fn tree_root(dir: &Path) -> PathBuf {
    dir.ancestors().last().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("/"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// The machine's volume roots, for tests that check the dialog's left column.
pub fn volume_roots() -> Vec<PathBuf> {
    drive_roots()
}

/// Every volume root on this machine: `/` on Unix (plus whatever is mounted under the
/// usual mount points) and the drive letters that actually exist on Windows.
fn drive_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mut out = Vec::new();
        for letter in b'A'..=b'Z' {
            let path = PathBuf::from(format!("{}:\\", letter as char));
            if path.is_dir() {
                out.push(path);
            }
        }
        out
    }
    #[cfg(not(windows))]
    {
        // `/` plus whatever is mounted under /media, /mnt, /run/media/<user> and
        // /Volumes (macOS): on a desktop with a second disk, a USB stick or a network
        // share in the sidebar, those are volumes the file dialog has to show.
        let mut out = vec![PathBuf::from("/")];
        let mut bases = vec![
            PathBuf::from("/media"),
            PathBuf::from("/mnt"),
            PathBuf::from("/run/media"),
            PathBuf::from("/Volumes"),
        ];
        if let Some(home) = home_dir() {
            bases.push(home.join("..").join("..").join("..").join("media"));
        }
        for base in bases {
            let Ok(rd) = std::fs::read_dir(&base) else { continue };
            let mut found: Vec<PathBuf> = rd
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.path())
                .collect();
            found.sort();
            for p in found {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
        out
    }
}

/// A `[#rrggbb]` / `[bg:#rrggbb]` slot of the colour dialog: one column of the
/// two-column picker.
#[derive(Clone, Debug)]
pub struct ColourSlot {
    /// `false` = "no colour of this kind" (the Clear side of the toggle).
    pub on: bool,
    /// The user changed this column, so Apply should act on it.
    pub touched: bool,
    pub hue: f32,
    pub sat: f32,
    pub val: f32,
    /// The hex box: a real, editable text field.
    pub hex: crate::field::TextField,
}

impl ColourSlot {
    pub fn new(start: Option<u32>) -> ColourSlot {
        let (hue, sat, val) = match start {
            Some(v) => rgb_to_hsv(v),
            None => (12.0, 0.72, 0.88),
        };
        let mut slot = ColourSlot {
            on: true,
            touched: false,
            hue,
            sat,
            val,
            hex: crate::field::TextField::limited(crate::field::CharFilter::Hex, 6),
        };
        // Fill the box without *touching* the column: a slot that has not been
        // edited yet must not be applied.
        slot.hex.set(&format!("{:06x}", slot.rgb()));
        slot
    }

    pub fn rgb(&self) -> u32 {
        hsv_to_rgb(self.hue, self.sat, self.val)
    }

    pub fn set_rgb(&mut self, value: u32) {
        let (h, s, v) = rgb_to_hsv(value);
        self.hue = h;
        self.sat = s;
        self.val = v;
        self.touch();
    }

    /// Recompute the hex from the current HSV.
    pub fn sync_hex(&mut self) {
        self.hex.set(&format!("{:06x}", self.rgb()));
        self.touched = true;
    }

    /// Take the hex field as the truth, if it holds a full colour.
    pub fn commit_hex(&mut self) {
        let digits: String = self.hex.text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        if digits.len() == 6 {
            if let Ok(value) = u32::from_str_radix(&digits, 16) {
                let (h, s, v) = rgb_to_hsv(value);
                self.hue = h;
                self.sat = s;
                self.val = v;
            }
        }
        self.touched = true;
    }

    fn touch(&mut self) {
        self.on = true;
        self.touched = true;
        self.hex.set(&format!("{:06x}", self.rgb()));
    }
}

#[derive(Clone, Debug)]
pub enum Dialog {
    Picker(Picker),
    /// A yes/no question. `title` is the headline, `danger` paints it in the red
    /// reserved for "you are about to lose something", and `message` may hold `\n`
    /// - the box grows a line per line of text.
    Confirm {
        title: String,
        message: String,
        danger: bool,
        on_yes: ConfirmAction,
    },
    Colour(ColourPicker),
    /// The file behind a tab changed on disk while it was open.
    External(ExternalChange),
    /// What this program is, what it is built from, and the licences - all of them
    /// compiled into the binary, so this box works offline and from a copied `.exe`.
    About(crate::ui::about::About),
}

/// "This file changed on disk" - what changed, and what the three answers mean.
///
/// Offered as a dialog rather than a silent reload because the file may have
/// been rewritten by something the user did not intend to lose work to, and not
/// silently ignored either: an editor that keeps showing yesterday's text is
/// worse than one that asks.
#[derive(Clone, Debug)]
pub struct ExternalChange {
    pub tab: usize,
    pub path: PathBuf,
    /// True when the buffer also has unsaved edits: reloading drops them.
    pub dirty: bool,
}

/// The six presets offered in each colour column.
pub const COLOUR_PRESETS: [u32; 6] = [0xe05c5c, 0xe0a83c, 0x5fae6b, 0x4f8fd0, 0x8f7ad0, 0x9a9a9a];

/// Entries in each column of the two-column colour panel (six presets plus
/// "Custom…" and "No colour").
pub const COLOUR_MENU_COLUMN: usize = 8;

/// A small HSV colour picker, laid out in **two columns**: text colour on the
/// left, background colour on the right. Both are live at the same time, so a
/// word can get a colour and a highlight in one visit instead of two.
///
/// Each column carries its own hue strip, saturation/value square, presets, hex
/// field and a Clear switch; Apply writes every column the user touched.
#[derive(Clone, Debug)]
pub struct ColourPicker {
    pub text: ColourSlot,
    pub background: ColourSlot,
    /// Which column the keyboard (the hex field) is editing.
    pub active_bg: bool,
    /// Which part of the picker the pointer is dragging.
    pub drag: Option<ColourPart>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColourPart {
    Sv,
    Hue,
}

impl ColourPicker {
    pub fn new(bg: bool, start: Option<u32>) -> ColourPicker {
        let p = ColourPicker {
            text: ColourSlot::new(start),
            background: ColourSlot::new(None),
            active_bg: bg,
            drag: None,
        };
        // Neither column is marked as touched: Apply writes exactly the columns
        // the user actually changed, so picking a background never rewrites a
        // text colour that was already there. Pressing Apply without touching
        // anything still does something - it applies the column the dialog was
        // opened from (see `columns_to_apply`).
        p
    }

    /// Which columns Apply/Enter should write: the ones that were touched, or -
    /// when nothing was touched at all - the one the dialog was opened from.
    pub fn columns_to_apply(&self) -> (bool, bool) {
        if self.text.touched || self.background.touched {
            (self.text.touched, self.background.touched)
        } else {
            (self.active_bg == false, self.active_bg)
        }
    }

    pub fn title(&self) -> &'static str {
        if self.active_bg {
            "Background colour"
        } else {
            "Text colour"
        }
    }

    pub fn slot(&self, bg: bool) -> &ColourSlot {
        if bg {
            &self.background
        } else {
            &self.text
        }
    }

    pub fn slot_mut(&mut self, bg: bool) -> &mut ColourSlot {
        if bg {
            &mut self.background
        } else {
            &mut self.text
        }
    }
}

/// HSV (hue in degrees, saturation and value 0..1) → `0xrrggbb`.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> u32 {
    let h = h.rem_euclid(360.0);
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    let c = v * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let to = |f: f32| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    (to(r) << 16) | (to(g) << 8) | to(b)
}

/// `0xrrggbb` → HSV.
pub fn rgb_to_hsv(rgb: u32) -> (f32, f32, f32) {
    let r = ((rgb >> 16) & 0xff) as f32 / 255.0;
    let g = ((rgb >> 8) & 0xff) as f32 / 255.0;
    let b = (rgb & 0xff) as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d < 1e-6 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max <= 0.0 { 0.0 } else { d / max };
    (h.rem_euclid(360.0), s, max)
}

#[derive(Clone, Debug)]
pub enum ConfirmAction {
    CloseTab(usize),
    Quit,
}

/// A workspace tab being dragged along the sidebar's strip.
#[derive(Clone, Copy, Debug)]
pub struct WorkspaceDrag {
    /// Where the tab lives right now (updated as it moves).
    pub index: usize,
    /// True once the pointer has travelled far enough to count as a drag.
    pub moved: bool,
    pub start: (f32, f32),
}

/// How long a middle-button fling takes to fall to 1/e of its speed, in seconds.
///
/// Long enough to read as a glide (a second and a half to a stop), short enough
/// that the page does not drift away from where it was thrown.
pub const FLING_TAU: f32 = 0.42;

/// A tab being dragged along the strip.
#[derive(Clone, Copy, Debug)]
pub struct TabDrag {
    /// Where the tab lives right now (updated as it moves).
    pub index: usize,
    /// True once the pointer has travelled far enough to count as a drag.
    pub moved: bool,
    pub start: (f32, f32),
}

/// What a drag on the minimap grabbed.
///
/// The map is frozen for the whole drag and the pointer is mapped through the
/// geometry it had when the button went down. Mapping every event through the
/// *live* geometry fed the document's own scrolling back into the position the
/// pointer pointed at, which is what made a drag accelerate the further it got.
#[derive(Clone, Debug)]
pub struct MapDrag {
    /// Where the pointer was when the button went down, in screen pixels.
    pub pointer_y0: f32,
    /// Top of the handle at that moment, in pixels below the top of the track.
    ///
    /// The drag is **absolute**: every pointer event puts the handle at
    /// `top0 + (pointer - pointer_y0)` and the page is read back from that, so the
    /// handle moves with the pointer by exactly the pointer's own movement, in either
    /// direction. No ratio, no scroll bookkeeping, nothing in between that could
    /// absorb it.
    pub top0: f32,
}

pub struct Tab {
    pub doc: Doc,
    pub caret: usize,
    pub anchor: usize,
    pub scroll: f32,
    pub scroll_x: f32,
    pub layout: Layout,
    pub find: FindState,
    pub goal_x: Option<f32>,
    pub last_caret_move: Instant,
}

impl Tab {
    pub fn new(doc: Doc) -> Tab {
        let caret = 0;
        Tab {
            doc,
            caret,
            anchor: caret,
            scroll: 0.0,
            scroll_x: 0.0,
            layout: Layout::empty(),
            find: FindState::default(),
            goal_x: None,
            last_caret_move: Instant::now(),
        }
    }

    pub fn title(&self) -> String {
        self.doc
            .path
            .as_ref()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "untitled".to_string())
    }

    pub fn sel(&self) -> (usize, usize) {
        if self.caret >= self.anchor {
            (self.anchor, self.caret)
        } else {
            (self.caret, self.anchor)
        }
    }

    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
    }

    pub fn ensure_layout(
        &mut self,
        te: &mut TextEngine,
        theme: &Theme,
        width: f32,
        mode: ViewMode,
        wrap: bool,
    ) {
        // A mermaid block shows its picture until the caret walks into it; then
        // the source comes back so it can be edited in place. That decision is
        // part of the layout (heights differ), so entering or leaving a block
        // forces a rebuild - hence the `editing` comparison below.
        let editing = self.mermaid_edit_target();
        if !self.layout.is_valid_for(&self.doc, width, theme.font_size, mode, wrap)
            || (mode != ViewMode::Raw && self.layout.editing != editing)
        {
            // An edit changed one block: lay that block out again and keep the rest,
            // which is the difference between a keystroke costing the whole document
            // and costing the paragraph it happened in.
            if !self.layout.try_patch(&self.doc, te, theme, width, mode, editing, wrap) {
                self.layout =
                    Layout::build_editing(&self.doc, te, theme, width, mode, editing, wrap);
            }
        }
    }

    /// Opening fence line of the mermaid block the caret is inside, if any.
    fn mermaid_edit_target(&self) -> Option<usize> {
        if self.layout.lines.is_empty() || self.layout.mermaid.is_empty() {
            return None;
        }
        let caret = self.layout.line_of_char(self.caret);
        self.layout
            .mermaid
            .iter()
            .copied()
            .find(|(a, b)| caret >= *a && caret <= *b)
            .map(|(open, _)| open)
    }

    /// True when the line should show raw Markdown markers (caret/selection inside).
    pub fn line_is_revealed(&self, line: usize) -> bool {
        let Some(ll) = self.layout.lines.get(line) else { return false };
        let caret_line = self.layout.line_of_char(self.caret);
        if caret_line == line {
            return true;
        }
        if self.has_selection() {
            let (a, b) = self.sel();
            return a <= ll.end_char() && b >= ll.start_char;
        }
        false
    }

    /// True when this source line has nothing visible once its block markers are
    /// hidden (an empty heading, list item, quote or checkbox). Such lines always
    /// show their markers, otherwise they would look like blank lines.
    pub fn line_has_no_content(&self, line: usize) -> bool {
        let Some(ll) = self.layout.lines.get(line) else { return false };
        let start = ll.info.prefix.min(ll.chars.len());
        ll.chars[start..].iter().all(|c| c.is_whitespace())
            && matches!(
                ll.info.block,
                Block::Heading(_) | Block::Bullet | Block::Ordered | Block::Task(_)
            )
    }

    /// Whether this table row is drawn with its inline markers on screen
    /// (`**bold**`, `[link](url)`) while its structural pipes stay hidden.
    ///
    /// A table is edited in its grid, so a link in a cell would otherwise be
    /// uneditable: the label was rendered, the destination was invisible, and a
    /// click on it followed the link instead of putting the caret in the text. The
    /// rule is the one the rest of the document already uses - the block under the
    /// caret shows what you would have to type - applied to the row instead of the
    /// whole line, because a row *is* the unit that is painted as a grid.
    pub fn table_shows_markers(&self, mode: ViewMode, line: usize) -> bool {
        if mode.shows_markers() {
            return false;
        }
        let Some(l) = self.layout.lines.get(line) else { return false };
        if l.block() != Block::Table || !l.has_inline_markers() {
            return false;
        }
        self.line_is_revealed(line)
    }

    /// Whether the fence run that `line` belongs to has no code in it, so the
    /// fence markers are the only thing the user could see (and must stay
    /// visible).
    fn fence_run_is_empty(&self, line: usize) -> bool {
        let is_code =
            |i: usize| matches!(self.layout.lines.get(i).map(|l| l.info.block), Some(Block::Code) | Some(Block::Fence));
        // Expand to the whole run of fence/code lines this line belongs to.
        let mut a = line;
        while a > 0 && is_code(a - 1) {
            a -= 1;
        }
        let mut b = line;
        while b + 1 < self.layout.lines.len() && is_code(b + 1) {
            b += 1;
        }
        // Blank lines inside a fence are still `Code`, so a single `Code` line in
        // the run means the block has a body.
        !(a..=b).any(|i| matches!(self.layout.lines[i].info.block, Block::Code))
    }

    /// Whether the line should be drawn with its Markdown markers visible.
    /// Every piece of geometry (caret, selection, hit-testing) must use this same
    /// answer, otherwise the caret drifts away from the text it belongs to.
    pub fn line_is_raw(&self, mode: ViewMode, line: usize) -> bool {
        if mode.shows_markers() {
            return true;
        }
        let block = self.layout.lines.get(line).map(|l| l.block());
        // Tables are edited *in place*: the grid is painted around the cells, so
        // flipping the row to source text while the caret is on it would break
        // the very layout the user is looking at. Live+ and Raw still show the
        // pipes; in the rendered view the row you are editing reveals its
        // *inline* markers instead (see `table_shows_markers`).
        if block == Some(Block::Table) {
            return false;
        }
        if self.line_is_revealed(line) || self.line_has_no_content(line) {
            return true;
        }
        // A fence marker is only worth showing when there is nothing else to see:
        // an empty block, or a fence opened with a language that is still waiting
        // for its body.
        block == Some(Block::Fence) && self.fence_run_is_empty(line)
    }
}

pub struct App {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub theme: Theme,
    pub settings: Settings,
    pub paths: Paths,
    /// The workspace being shown. It is also the entry `ws_active` of
    /// `workspaces`, which keeps the others (and their expanded folders) alive.
    pub ws: Workspace,
    /// Every open workspace, in the order their tabs are shown.
    pub workspaces: Vec<Workspace>,
    /// Index of the active workspace inside `workspaces`.
    pub ws_active: usize,
    /// Reload files by itself when they change on disk. Deliberately **not**
    /// persisted: it lasts for this session only, as a per-workspace choice.
    pub auto_reload: bool,
    /// When the files were last checked for external changes.
    pub disk_check_at: Instant,
    pub focus: Focus,
    pub dialog: Option<Dialog>,
    pub status: String,
    pub status_at: Instant,
    pub hits: Hits,
    pub mouse: (f32, f32),
    pub mouse_down: bool,
    pub dragging_selection: bool,
    /// Set while dragging inside a find / replace field.
    pub find_drag: Option<FindFocus>,
    /// Set while dragging inside a dialog text field: a drag selects, exactly
    /// like it does in the find boxes and in the document.
    pub field_drag: Option<Hit>,
    /// Set while the pointer is dragging inside the map.
    pub map_drag: Option<MapDrag>,
    /// Top of the handle while a map drag is in flight, in **screen** pixels.
    ///
    /// One number, in the space the painter draws in: the handle is drawn at this y
    /// and the page's scroll is computed from this y, so the handle on screen and the
    /// page behind it cannot disagree - not across a resize, not across a re-layout,
    /// not at any zoom.
    pub map_handle: Option<f32>,
    /// The About box's scrollbar, as last drawn: a drag is mapped through it, so the
    /// pointer and the thumb cannot disagree about where the track is.
    pub about_track: Rect,
    /// Set while the About box's scrollbar is being dragged: how far down the thumb
    /// the pointer grabbed it (0 = its top edge, 1 = its bottom edge).
    pub about_drag: Option<f32>,
    /// Alt held down: a click on a link edits it instead of following it.
    pub alt_down: bool,
    /// The last colour applied from the colour menu: the toolbar button is tinted
    /// with it, so the pen's load is visible at a glance.
    pub last_colour: Option<u32>,
    /// Tab rectangles of the last frame, so a tab can be dragged onto another.
    pub tab_rects: Vec<Rect>,
    /// A tab being dragged along the strip.
    pub tab_drag: Option<TabDrag>,
    /// A workspace tab being dragged along the sidebar strip.
    pub ws_drag: Option<WorkspaceDrag>,
    /// Rect of every workspace chip in the last frame, so a drag can tell which
    /// chip the pointer is over (the same trick the document tabs use).
    pub ws_tab_rects: Vec<Rect>,
    pub dragging_scrollbar: bool,
    pub dragging_minimap: bool,
    pub clipboard: Option<arboard::Clipboard>,
    pub mode: ViewMode,
    pub editor_rect: Rect,
    pub text_rect: Rect,
    pub minimap_rect: Rect,
    /// Rect of the folder dialog's tree column (set while a picker is open), so
    /// the wheel knows which column it is over.
    pub dialog_tree_rect: Rect,
    /// Rect of the folder dialog's listing (the right column), same purpose.
    pub dialog_list_rect: Rect,
    /// Rect of the outline panel, so a drag can be mapped back onto rows.
    pub outline_rect: Rect,
    pub tabbar_rect: Rect,
    pub sidebar_rect: Rect,
    pub last_frame: Instant,
    pub quit_after_confirm: bool,
    pub should_close: bool,
    pub click_count: u32,
    pub last_click: Option<(Instant, (f32, f32))>,
    pub hover: Option<Hit>,
    pub drag_scroll_anchor: (f32, f32),
    /// Middle-button panning: "grab the page and throw it".
    pub pan_active: bool,
    pub pan_anchor: (f32, f32),
    pub pan_scroll: (f32, f32),
    pub pan_velocity: (f32, f32),
    /// An open dropdown / context menu, if any.
    pub menu: Option<Menu>,
    /// Rect of the floating toolbar, so menus opened from the keyboard have an
    /// anchor.
    pub toolbar_rect: Rect,
    pan_last: (Instant, (f32, f32)),
    kinetic_at: Option<Instant>,
    pub scroll_start: (f32, f32),
    pub initial_files: Vec<PathBuf>,
    /// An in-progress grip drag, if any.
    pub drag: Option<DragState>,
}

impl App {
    /// Is the pointer owned by a drag right now?
    ///
    /// A drag belongs to the window, not to the tiny place it started in: while one is
    /// running the window keeps the pointer to itself (`CursorGrabMode::Confined`), so
    /// that a drag that slips over the window's edge keeps delivering movement. Without
    /// it, a minimap drag - the map is flush against the right edge of the window - froze
    /// the handle the moment the pointer drifted sideways out of the window, halfway down
    /// the strip and with the rest of the document still below it.
    pub fn dragging_something(&self) -> bool {
        self.map_drag.is_some()
            || self.dragging_minimap
            || self.dragging_scrollbar
            || self.dragging_selection
            || self.about_drag.is_some()
            || self.pan_active
            || self.ws_drag.is_some()
            || self.tab_drag.is_some()
            || self.find_drag.is_some()
            || self.field_drag.is_some()
    }
}
impl App {
    pub fn new(paths: Paths, settings: Settings, session: Session, files: Vec<PathBuf>) -> App {
        let mut theme = Theme::by_kind(settings.theme);
        theme.font_size = settings.font_size.clamp(10.0, 34.0);
        // Workspace root: configured folder, else the folder of the first file
        // given on the command line, else the current directory. Never the folder
        // holding settings.conf, so that never shows up as the "workspace".
        let workspace_root = settings
            .workspace
            .clone()
            .filter(|p| p.is_dir())
            .or_else(|| files.iter().find(|p| p.is_dir()).cloned())
            .or_else(|| {
                files
                    .iter()
                    .find(|p| p.is_file())
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            })
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let ws = Workspace::new(workspace_root);
        let settings_mode = settings.view_mode;
        let mut app = App {
            tabs: Vec::new(),
            active: 0,
            theme,
            settings,
            paths,
            workspaces: Vec::new(),
            ws_active: 0,
            auto_reload: false,
            disk_check_at: Instant::now(),
            ws,
            focus: Focus::Editor,
            dialog: None,
            status: String::new(),
            status_at: Instant::now(),
            hits: Hits::default(),
            mouse: (0.0, 0.0),
            mouse_down: false,
            dragging_selection: false,
            find_drag: None,
            field_drag: None,
            map_drag: None,
            map_handle: None,
            about_track: Rect::ZERO,
            about_drag: None,
            alt_down: false,
            last_colour: None,
            tab_rects: Vec::new(),
            tab_drag: None,
            ws_drag: None,
            ws_tab_rects: Vec::new(),
            dragging_scrollbar: false,
            dragging_minimap: false,
            clipboard: None,
            mode: settings_mode,
            editor_rect: Rect::ZERO,
            text_rect: Rect::ZERO,
            minimap_rect: Rect::ZERO,
            dialog_tree_rect: Rect::ZERO,
            dialog_list_rect: Rect::ZERO,
            tabbar_rect: Rect::ZERO,
            sidebar_rect: Rect::ZERO,
            last_frame: Instant::now(),
            quit_after_confirm: false,
            should_close: false,
            click_count: 0,
            last_click: None,
            hover: None,
            drag_scroll_anchor: (0.0, 0.0),
            pan_active: false,
            pan_anchor: (0.0, 0.0),
            pan_scroll: (0.0, 0.0),
            pan_velocity: (0.0, 0.0),
            menu: None,
            toolbar_rect: Rect::ZERO,
            pan_last: (Instant::now(), (0.0, 0.0)),
            kinetic_at: None,
            scroll_start: (0.0, 0.0),
            initial_files: files,
            drag: None,
            outline_rect: Rect::ZERO,
        };

        // Restore session tabs, then open CLI files.
        let mut restored = false;
        for st in &session.tabs {
            if let Some(p) = &st.path {
                if let Ok(text) = std::fs::read_to_string(p) {
                    let mut tab = Tab::new(Doc::new(&text, Some(p.clone())));
                    tab.caret = st.caret.min(tab.doc.len_chars());
                    tab.anchor = tab.caret;
                    tab.scroll = st.scroll;
                    app.tabs.push(tab);
                    restored = true;
                }
            }
        }
        if restored {
            app.active = session.active.min(app.tabs.len().saturating_sub(1));
        }
        if !app.initial_files.is_empty() {
            let files = app.initial_files.clone();
            for f in files {
                if f.is_dir() {
                    app.ws.set_root(f);
                } else {
                    app.open_path(&f);
                }
            }
            app.active = app.tabs.len().saturating_sub(1);
        }
        if app.tabs.is_empty() {
            app.tabs.push(Tab::new(Doc::empty()));
        }
        app.restore_workspaces();
        app
    }

    /// Fill the workspace tabs from the settings, keeping the one that became
    /// active above as the first tab.
    fn restore_workspaces(&mut self) {
        let active_root = self.ws.root.clone();
        // The saved order is the tab order; the active workspace is restored to
        // the tab it was on, not moved to the front.
        let mut roots: Vec<PathBuf> = Vec::new();
        for root in self.settings.workspaces.clone() {
            if root.is_dir() && !roots.iter().any(|r| *r == root) {
                roots.push(root);
            }
        }
        if !roots.iter().any(|r| *r == active_root) {
            roots.insert(0, active_root.clone());
        }
        roots.truncate(8);
        let active = roots.iter().position(|r| *r == active_root).unwrap_or(0);
        // Keep the workspace that is already built as its own tab instead of
        // walking the disk again for it.
        self.workspaces = roots
            .iter()
            .enumerate()
            .map(|(i, root)| {
                if i == active {
                    self.ws.clone()
                } else {
                    Workspace::new(root.clone())
                }
            })
            .collect();
        self.ws_active = active;
        self.settings.workspace = Some(active_root);
        self.settings.workspaces = roots;
    }

    /// Show workspace tab `i`, keeping the expanded folders of the others.
    pub fn switch_workspace(&mut self, i: usize) {
        if i >= self.workspaces.len() || i == self.ws_active {
            return;
        }
        self.workspaces[self.ws_active] = self.ws.clone();
        self.ws = self.workspaces[i].clone();
        self.ws_active = i;
        self.settings.workspace = Some(self.ws.root.clone());
        // Auto-reload is a per-workspace, per-session choice: switching folders
        // starts again from "ask me".
        self.auto_reload = false;
        let name = self
            .ws
            .root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| self.ws.root.display().to_string());
        self.set_status(format!("workspace: {name}"));
    }

    /// Open `root` as an extra workspace tab (or switch to it when it is open).
    pub fn add_workspace(&mut self, root: PathBuf) {
        if let Some(i) = self.workspaces.iter().position(|w| w.root == root) {
            self.switch_workspace(i);
            return;
        }
        self.workspaces[self.ws_active] = self.ws.clone();
        let ws = Workspace::new(root.clone());
        self.workspaces.push(ws.clone());
        self.ws_active = self.workspaces.len() - 1;
        self.ws = ws;
        if !self.settings.workspaces.iter().any(|r| *r == root) {
            self.settings.workspaces.push(root.clone());
        }
        self.settings.workspace = Some(root);
        self.auto_reload = false;
        self.settings.sidebar_open = true;
    }

    /// Move workspace tab `i` to position `to`.
    pub fn move_workspace(&mut self, i: usize, to: usize) {
        if i == to || i >= self.workspaces.len() || to >= self.workspaces.len() {
            return;
        }
        let ws = self.workspaces.remove(i);
        self.workspaces.insert(to, ws);
        if self.ws_active == i {
            self.ws_active = to;
        } else if i < self.ws_active && to >= self.ws_active {
            self.ws_active -= 1;
        } else if i > self.ws_active && to <= self.ws_active {
            self.ws_active += 1;
        }
        self.settings.workspaces = self.workspaces.iter().map(|w| w.root.clone()).collect();
        self.settings.workspace = Some(self.ws.root.clone());
    }

    /// Close workspace tab `i`. The last one cannot be closed.
    pub fn close_workspace(&mut self, i: usize) {
        if self.workspaces.len() <= 1 || i >= self.workspaces.len() {
            return;
        }
        let root = self.workspaces[i].root.clone();
        self.workspaces.remove(i);
        self.settings.workspaces.retain(|r| *r != root);
        if i < self.ws_active {
            self.ws_active -= 1;
        } else if i == self.ws_active {
            self.ws_active = i.min(self.workspaces.len() - 1);
            self.ws = self.workspaces[self.ws_active].clone();
            self.settings.workspace = Some(self.ws.root.clone());
        }
        self.set_status(format!("closed workspace {}", root.display()));
    }

    // ---------- accessors ----------

    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active.min(self.tabs.len() - 1)]
    }

    pub fn tab_mut(&mut self) -> &mut Tab {
        let i = self.active.min(self.tabs.len() - 1);
        &mut self.tabs[i]
    }

    /// The code of a block, given the first and last document lines of the card.
    /// Fence markers are left out: what you copy is what the card shows.
    pub fn code_text(&self, first: usize, last: usize) -> String {
        let t = self.tab();
        let last = last.min(t.doc.len_lines().saturating_sub(1));
        let mut body: Vec<String> = Vec::new();
        for line in first..=last {
            let text = t.doc.line_text(line);
            let is_fence = match t.layout.lines.get(line).map(|l| l.block()) {
                Some(Block::Fence) => true,
                None => text.trim_start().starts_with("```"),
                _ => false,
            };
            if is_fence {
                continue;
            }
            body.push(text);
        }
        body.join("\n")
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
        self.status_at = Instant::now();
    }

    pub fn clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut()
    }

    // ---------- tabs ----------

    /// The link target under a document char offset, if that char is inside a
    /// `[label](target)` link's label.
    pub fn link_target_at(&self, pos: usize) -> Option<String> {
        let t = self.tab();
        let pos = pos.min(t.doc.len_chars());
        let line = t.doc.line_of_char(pos);
        let start = t.doc.line_start(line);
        let local = pos.saturating_sub(start);
        let chars: Vec<char> = t.doc.line_text(line).chars().collect();
        crate::markdown::link_targets(&chars)
            .into_iter()
            .find(|(a, b, _)| local >= *a && local <= *b)
            .map(|(_, _, url)| url)
    }

    /// Follow a link: an `http(s)://` or `mailto:` target goes to the browser, a
    /// relative path is resolved next to the document and opened in the editor.
    pub fn open_link(&mut self, target: &str) {
        let target = target.trim();
        if target.is_empty() {
            return;
        }
        let external = target.starts_with("http://")
            || target.starts_with("https://")
            || target.starts_with("mailto:")
            || target.starts_with("ftp://");
        if external {
            match open_external(target) {
                Ok(()) => self.set_status(format!("Opening {target}")),
                Err(e) => self.set_status(format!("cannot open {target}: {e}")),
            }
            return;
        }
        // An in-document anchor: jump to the heading it names.
        if let Some(anchor) = target.strip_prefix('#') {
            let want = anchor.replace('-', " ").to_lowercase();
            let hits: Vec<(usize, String)> = self
                .tab()
                .layout
                .headings
                .iter()
                .map(|(line, _, text)| (*line, text.to_lowercase()))
                .collect();
            if let Some((line, _)) = hits
                .into_iter()
                .find(|(_, text)| text.trim() == want || text.contains(&want))
            {
                let start = self.tab().doc.line_start(line);
                self.set_caret(start, false);
                self.scroll_to_line(line);
                return;
            }
            self.set_status(format!("no heading matches #{anchor}"));
            return;
        }
        let path = {
            // `file:///…` and `%20` are what a pasted or exported link looks like;
            // neither resolved before, so such a link did nothing at all.
            let cleaned = target.split('#').next().unwrap_or(target);
            let cleaned = cleaned.strip_prefix("file://").unwrap_or(cleaned);
            let cleaned = percent_decode(cleaned);
            if Path::new(&cleaned).is_absolute() {
                PathBuf::from(cleaned)
            } else {
                let base = self
                    .tab()
                    .doc
                    .path
                    .as_ref()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                base.join(cleaned)
            }
        };
        if path.exists() {
            self.open_path(&path);
        } else {
            self.set_status(format!("link target not found: {}", path.display()));
        }
    }

    pub fn open_path(&mut self, path: &Path) {
        if let Some(i) = self.tabs.iter().position(|t| t.doc.path.as_deref() == Some(path)) {
            self.active = i;
            return;
        }
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let mut tab = Tab::new(Doc::new(&text, Some(path.to_path_buf())));
                tab.caret = 0;
                self.tabs.push(tab);
                self.active = self.tabs.len() - 1;
                self.settings.push_recent(path);
                self.ws.reveal(path);
                self.focus = Focus::Editor;
            }
            Err(e) => self.set_status(format!("cannot open {}: {e}", path.display())),
        }
    }

    pub fn new_tab(&mut self) {
        self.tabs.push(Tab::new(Doc::empty()));
        self.active = self.tabs.len() - 1;
        self.focus = Focus::Editor;
    }

    pub fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        let dirty = self.tabs[i].doc.dirty;
        let gone = self.tabs[i].doc.file_gone;
        if dirty || gone {
            // The text in this tab is the only copy left, so closing it throws the
            // document away - say so before doing it.
            let title = self.tabs[i].title();
            let (heading, message) = match (dirty, gone) {
                (true, true) => (
                    "File deleted",
                    format!(
                        "\"{title}\" was deleted from disk and has unsaved changes.\n\
                         Closing this tab throws the text away for good."
                    ),
                ),
                (true, false) => (
                    "Unsaved changes",
                    format!("\"{title}\" has unsaved changes."),
                ),
                (false, true) => (
                    "File deleted",
                    format!(
                        "The file \"{title}\" has been deleted from disk.\n\
                         This tab is the only copy of the text: closing it throws that away."
                    ),
                ),
                _ => unreachable!(),
            };
            self.dialog = Some(Dialog::Confirm {
                title: heading.to_string(),
                message,
                danger: gone,
                on_yes: ConfirmAction::CloseTab(i),
            });
            self.focus = Focus::Dialog;
            return;
        }
        self.force_close_tab(i);
    }

    /// Tabs whose text exists nowhere else: unsaved edits, or a file that has been
    /// deleted from under them. Quitting has to ask about these.
    pub fn tabs_at_risk(&self) -> Vec<usize> {
        self.tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| t.doc.dirty || t.doc.file_gone)
            .map(|(i, _)| i)
            .collect()
    }

    pub fn force_close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        self.tabs.remove(i);
        if self.tabs.is_empty() {
            self.tabs.push(Tab::new(Doc::empty()));
            self.active = 0;
        } else if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
    }

    pub fn next_tab(&mut self, forward: bool) {
        if self.tabs.len() < 2 {
            return;
        }
        let n = self.tabs.len();
        self.active = if forward { (self.active + 1) % n } else { (self.active + n - 1) % n };
    }

    pub fn save(&mut self) -> bool {
        let path = self.tab().doc.path.clone();
        match path {
            Some(p) => {
                let text = self.tab().doc.text();
                match std::fs::write(&p, text) {
                    Ok(_) => {
                        self.tab_mut().doc.dirty = false;
                        self.tab_mut().doc.note_disk();
                        self.set_status(format!("saved {}", p.display()));
                        true
                    }
                    Err(e) => {
                        self.set_status(format!("save failed: {e}"));
                        false
                    }
                }
            }
            None => {
                self.open_picker(PickerKind::SaveAs);
                false
            }
        }
    }

    /// Re-read the active tab from disk. Unsaved edits are kept unless the user
    /// asks for them to be dropped, so this never destroys work by accident.
    pub fn reload_file(&mut self) {
        let i = self.active.min(self.tabs.len().saturating_sub(1));
        if self.reload_tab(i) {
            self.set_status(format!("reloaded {}", self.tabs[i].doc.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default()));
        }
    }

    /// Re-read tab `i` from disk, keeping its caret and scroll. Returns whether
    /// anything changed.
    pub fn reload_tab(&mut self, i: usize) -> bool {
        let Some(tab) = self.tabs.get(i) else { return false };
        let Some(path) = tab.doc.path.clone() else {
            self.set_status("nothing to reload: the buffer has no file");
            return false;
        };
        let disk = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                self.set_status(format!("reload failed: {e}"));
                return false;
            }
        };
        if disk == self.tabs[i].doc.text() {
            // Same text but a new stamp (a touch, a formatter run): remember it so
            // the question is not asked again.
            self.tabs[i].doc.note_disk();
            self.set_status(format!("{} is already up to date", self.tabs[i].title()));
            return false;
        }
        let t = &mut self.tabs[i];
        let scroll = t.scroll;
        let scroll_x = t.scroll_x;
        let caret = t.caret;
        t.doc = Doc::new(&disk, Some(path));
        t.layout = Layout::empty();
        t.caret = caret.min(t.doc.len_chars());
        t.anchor = t.caret;
        t.scroll = scroll;
        t.scroll_x = scroll_x;
        true
    }

    /// Remember the file's current stamp as its known-good state (the "keep my
    /// version" answer).
    pub fn note_disk_state(&mut self, i: usize) {
        if let Some(tab) = self.tabs.get_mut(i) {
            tab.doc.note_disk();
        }
    }

    /// Look for files that changed on disk since they were opened.
    ///
    /// Runs on a timer, not every frame (one `stat` per open file is cheap but
    /// pointless at 120 Hz). With auto-reload on, the file is swapped in
    /// silently as long as the buffer has no unsaved edits; a dirty buffer always
    /// asks first, because a reload throws those edits away.
    /// Check the files for outside changes, at most a few times a second.
    pub fn poll_disk_if_due(&mut self) {
        if self.disk_check_at.elapsed().as_millis() < 400 {
            return;
        }
        self.disk_check_at = Instant::now();
        self.poll_disk();
    }

    pub fn poll_disk(&mut self) {
        if self.dialog.is_some() {
            return;
        }
        for i in 0..self.tabs.len() {
            // A file can also *vanish* - deleted, moved, or a sync client replacing
            // the folder - which is not "changed on disk" (reloading it would empty
            // the buffer). The tab turns red instead, and closing it asks first.
            let was_gone = self.tabs[i].doc.file_gone();
            let gone_now = self.tabs[i].doc.refresh_file_gone();
            if gone_now && !was_gone {
                // Announce it once, when it happens: a status line rewritten every
                // poll would drown everything else the editor has to say.
                let title = self.tabs[i].title();
                self.set_status(format!("{title} was deleted from disk; the text is still open here"));
            }
            let tab = &self.tabs[i];
            if tab.doc.path.is_none() || tab.doc.file_gone || !tab.doc.changed_on_disk() {
                continue;
            }
            if self.auto_reload && !tab.doc.dirty {
                self.reload_tab(i);
                self.set_status(format!("reloaded {} (auto)", self.tabs[i].title()));
                self.tabs[i].layout = Layout::empty();
                continue;
            }
            let path = tab.doc.path.clone().unwrap_or_default();
            let dirty = tab.doc.dirty;
            self.dialog = Some(Dialog::External(ExternalChange { tab: i, path, dirty }));
            self.focus = Focus::Dialog;
            return;
        }
    }

    pub fn save_as(&mut self, path: &Path) {
        let text = self.tab().doc.text();
        match std::fs::write(path, text) {
            Ok(_) => {
                let t = self.tab_mut();
                t.doc.path = Some(path.to_path_buf());
                t.doc.dirty = false;
                t.doc.note_disk();
                self.settings.push_recent(path);
                self.ws.reveal(path);
                self.set_status(format!("saved {}", path.display()));
            }
            Err(e) => self.set_status(format!("save failed: {e}")),
        }
    }

    pub fn open_picker(&mut self, kind: PickerKind) {
        let dir = match kind {
            PickerKind::OpenFolder => self.ws.root.clone(),
            _ => self
                .tab()
                .doc
                .path
                .as_ref()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| self.ws.root.clone()),
        };
        let name = match kind {
            PickerKind::SaveAs => self.tab().title(),
            _ => String::new(),
        };
        self.dialog = Some(Dialog::Picker(Picker::new(kind, dir, name)));
        self.focus = Focus::Dialog;
    }

    // ---------- caret / selection ----------

    pub fn set_caret(&mut self, pos: usize, extend: bool) {
        let len = self.tab().doc.len_chars();
        let pos = pos.min(len);
        let t = self.tab_mut();
        if !extend {
            t.anchor = pos;
        }
        t.caret = pos;
        t.goal_x = None;
        t.last_caret_move = Instant::now();
        t.doc.break_undo_group();
    }

    pub fn select_range(&mut self, a: usize, b: usize) {
        let t = self.tab_mut();
        t.anchor = a.min(t.doc.len_chars());
        t.caret = b.min(t.doc.len_chars());
        t.goal_x = None;
    }

    pub fn select_all(&mut self) {
        let len = self.tab().doc.len_chars();
        self.select_range(0, len);
    }

    pub fn select_word_at(&mut self, pos: usize) {
        let text: Vec<char> = self.tab().doc.text().chars().collect();
        let (a, b) = word_bounds(&text, pos);
        self.select_range(a, b);
    }

    pub fn selected_text(&self) -> Option<String> {
        let t = self.tab();
        if !t.has_selection() {
            return None;
        }
        let (a, b) = t.sel();
        Some(t.doc.slice(a, b))
    }

    // ---------- editing ----------

    /// Delete the current selection if any; returns true when something was deleted.
    fn delete_selection(&mut self) -> bool {
        let t = self.tab_mut();
        if !t.has_selection() {
            return false;
        }
        let (a, b) = t.sel();
        t.doc.remove_atomic(a, b);
        t.caret = a;
        t.anchor = a;
        true
    }

    pub fn insert_text(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.delete_selection();
        let t = self.tab_mut();
        let pos = t.caret;
        let new = t.doc.insert(pos, s);
        t.caret = new;
        t.anchor = new;
        t.goal_x = None;
    }

    /// Turn soft wrap on or off.
    ///
    /// One entry point for the three ways to reach it - the toolbar chip, the
    /// document menu's ticked item and `Alt+W` - so they cannot disagree. Every
    /// layout is dropped rather than patched: wrap is part of the layout (it decides
    /// the rows), and the next frame rebuilds each tab with the new answer.
    pub fn toggle_wrap(&mut self) -> bool {
        self.settings.wrap = !self.settings.wrap;
        for t in &mut self.tabs {
            t.layout = Layout::empty();
        }
        self.settings.wrap
    }

    /// Switch bracket/quote auto-completion on or off.
    ///
    /// One entry point for the three ways to reach it - the toolbar chip, the document
    /// menu's ticked item and `Alt+B` - so they cannot disagree about what the setting is.
    pub fn toggle_autoclose(&mut self) -> bool {
        self.settings.autoclose = !self.settings.autoclose;
        self.settings.autoclose
    }

    /// Typing entry point that adds bracket/quote auto-completion.
    ///
    /// Only single characters are considered, so pasted text and IME commits go
    /// straight through.
    pub fn insert_typed(&mut self, s: &str) {
        // Auto-close can be switched off, and then typing is typing: `(` is a `(`, `"` is a
        // `"`, and nothing jumps over an existing character or wraps a selection. It is a
        // preference, not a mode of the document, so it lives in the settings and is
        // remembered between runs.
        if !self.settings.autoclose {
            self.insert_text(s);
            return;
        }
        let mut chars = s.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            self.insert_text(s);
            return;
        };
        const PAIRS: [(char, char); 6] =
            [('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\''), ('`', '`')];
        let opener = PAIRS.iter().find(|(o, _)| *o == c).map(|(_, cl)| *cl);
        let is_closer = PAIRS.iter().any(|(_, cl)| *cl == c);
        if opener.is_none() && !is_closer {
            self.insert_text(s);
            return;
        }
        let (caret, next) = {
            let t = self.tab();
            (t.caret, t.doc.slice(t.caret, (t.caret + 1).min(t.doc.len_chars())))
        };
        // Typing a closing character that is already there steps over it.
        if is_closer && opener.is_none() && next == c.to_string() {
            let t = self.tab_mut();
            t.caret = caret + 1;
            t.anchor = t.caret;
            t.goal_x = None;
            return;
        }
        let Some(close) = opener else {
            self.insert_text(s);
            return;
        };
        if self.tab().has_selection() {
            // Wrap the selection instead of replacing it.
            self.toggle_inline(&c.to_string(), &close.to_string());
            return;
        }
        let t = self.tab_mut();
        t.doc.insert_atomic(caret, &format!("{c}{close}"));
        t.caret = caret + 1;
        t.anchor = t.caret;
        t.goal_x = None;
    }

    /// A path dropped onto the window: folders become the workspace root, files
    /// open in a tab.
    pub fn open_dropped(&mut self, path: &Path) {
        if path.is_dir() {
            self.ws.set_root(path.to_path_buf());
            self.settings.workspace = Some(path.to_path_buf());
        } else if path.is_file() {
            self.open_path(path);
        }
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        let t = self.tab_mut();
        if t.caret == 0 {
            return;
        }
        // Removing a list/quote/heading marker prefix deletes it in one go.
        let (line, col) = t.doc.line_and_col(t.caret);
        let ll_info = t.layout.lines.get(line).map(|l| (l.info.clone(), l.chars.clone()));
        if let Some((info, chars)) = ll_info {
            let prefix = info.prefix;
            if col == prefix && prefix > 0 {
                let start = t.doc.line_start(line);
                let text: String = chars.iter().collect();
                // 1. the line's own indentation goes first, so a nested checkbox
                //    steps out one level before its `- [ ]` is touched.
                let indent = leading_spaces(&text).min(2);
                if indent > 0 {
                    t.doc.remove_atomic(start, start + indent);
                    t.caret = (start + col).saturating_sub(indent).max(start);
                    t.anchor = t.caret;
                    return;
                }
                // 2. then the block marker itself.
                let is_block = matches!(
                    info.block,
                    Block::Heading(_) | Block::Bullet | Block::Ordered | Block::Task(_) | Block::Quote
                );
                if is_block && chars.iter().take(prefix).any(|c| !c.is_whitespace()) {
                    t.doc.remove_atomic(start, start + prefix);
                    t.caret = start;
                    t.anchor = start;
                    return;
                }
            }
        }
        let start = t.caret - 1;
        let removed = t.doc.remove(start, start + 1);
        t.caret = removed;
        t.anchor = removed;
    }

    pub fn delete_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        let t = self.tab_mut();
        if t.caret >= t.doc.len_chars() {
            return;
        }
        let pos = t.caret;
        t.doc.remove(pos, pos + 1);
    }

    /// Delete the word before (or after) the caret.
    pub fn delete_word(&mut self, forward: bool) {
        if self.delete_selection() {
            return;
        }
        let t = self.tab_mut();
        let caret = t.caret;
        let mut probe = Doc::new(&t.doc.text(), None);
        let _ = &mut probe;
        let text: Vec<char> = t.doc.text().chars().collect();
        let mut i = caret;
        if forward {
            while i < text.len() && text[i].is_whitespace() {
                i += 1;
            }
            while i < text.len() && !text[i].is_whitespace() {
                i += 1;
            }
            t.doc.remove_atomic(caret, i);
        } else {
            while i > 0 && text.get(i - 1).map(|c| c.is_whitespace()).unwrap_or(false) {
                i -= 1;
            }
            while i > 0 && !text.get(i - 1).map(|c| c.is_whitespace()).unwrap_or(true) {
                i -= 1;
            }
            t.doc.remove_atomic(i, caret);
            t.caret = i;
            t.anchor = i;
        }
    }

    pub fn newline(&mut self) {
        self.newline_ex(false);
    }

    /// `hard` = Shift+Enter: insert a plain line break without continuing the
    /// current block (that is how you leave a quote or a list).
    pub fn newline_ex(&mut self, hard: bool) {
        // Inside a table Enter is *navigation*, not an edit: it goes to the cell
        // below and selects it (see `table_enter`). The selection is therefore not
        // deleted first - the cell Enter just selected must survive the next Enter,
        // or moving down the column would eat it.
        {
            let t = self.tab();
            let line = t.layout.line_of_char(t.caret);
            if let Some(info) = t.layout.lines.get(line).map(|l| l.info.clone()) {
                if info.block == Block::Table && !info.table_sep && !info.cells.is_empty() {
                    let col = t.doc.line_and_col(t.caret).1;
                    self.table_enter(line, col, &info);
                    return;
                }
            }
        }
        self.delete_selection();
        let t = self.tab_mut();
        let (line, col) = t.doc.line_and_col(t.caret);
        let info = t.layout.lines.get(line).map(|l| l.info.clone());
        let mut insertion = String::from("\n");
        if let Some(info) = info {
            let line_text = t.doc.line_text(line);
            let chars: Vec<char> = line_text.chars().collect();
            let content_empty = chars.iter().skip(info.prefix).all(|c| c.is_whitespace());
            // Enter *at* (or inside) the marker.
            let at_marker = col <= info.prefix;
            let prefix: String = chars.iter().take(info.prefix).collect();
            let next_prefix = match info.block {
                Block::Ordered => increment_ordered_marker(&prefix),
                _ => prefix.clone(),
            };
            let continues = matches!(
                info.block,
                Block::Bullet | Block::Ordered | Block::Task(_) | Block::Quote
            );
            if continues && content_empty && hard {
                // Shift+Enter on an empty item is how a list ends: the marker is
                // removed and the caret stays on a plain line.
                let start = t.doc.line_start(line);
                t.doc.replace_range(start, start + info.prefix, "");
                t.caret = start;
                t.anchor = start;
                t.goal_x = None;
                return;
            }
            if continues && content_empty {
                // Enter on an empty item keeps the list going (a new item), which
                // is what a writer expects.
                insertion.push_str(&next_prefix);
            } else if continues && at_marker && !hard {
                // Enter at the very start of the marker breaks the list there
                // instead of writing a second marker in front of the first.
                let pos = t.caret;
                t.doc.insert_atomic(pos, "\n");
                t.caret = pos;
                t.anchor = pos;
                t.goal_x = None;
                return;
            } else if continues && !content_empty && !hard {
                insertion.push_str(&next_prefix);
            } else if mention_guard(info.block) {
                let indent: String = chars.iter().take_while(|c| **c == ' ').collect();
                insertion.push_str(&indent);
            }
        }
        let pos = t.caret;
        let new = t.doc.insert_atomic(pos, &insertion);
        t.caret = new;
        t.anchor = new;
        t.goal_x = None;
    }

    /// Enter inside a table row: go to the cell below, the way a table is meant to
    /// be filled in.
    ///
    /// A cell cannot hold a line break - a newline in the source ends the row - so
    /// Enter does what it does in a spreadsheet: the caret moves down one row in the
    /// same column and the cell it lands in is selected, so typing replaces what was
    /// there. On the last row the row below is created (with the row's own number of
    /// columns) and the caret waits in the same column.
    ///
    /// Two edges:
    ///
    /// * the header row jumps past the `| --- |` rule into the first body row -
    ///   a row inserted between a header and its rule would stop the rule being a
    ///   separator and the whole table would fall back to paragraphs;
    /// * Enter on an **empty last row** leaves the table, exactly like Enter on an
    ///   empty list item ends a list. Otherwise there would be no way out of a table
    ///   from the keyboard.
    fn table_enter(&mut self, line: usize, col: usize, info: &crate::markdown::LineInfo) {
        let ncols = info.cells.len().max(1);
        // The column the caret is in: the last one whose start is at or before it.
        let mut ci = 0usize;
        for (i, &(a, _)) in info.cells.iter().enumerate() {
            if col >= a {
                ci = i;
            }
        }
        let (line_start, text) = {
            let t = self.tab();
            (t.doc.line_start(line), t.doc.line_text(line))
        };
        let row_empty = {
            let t = self.tab();
            info.cells
                .iter()
                .all(|&(a, b)| t.doc.line_text(line).chars().skip(a).take(b - a).all(|c| c.is_whitespace()))
        };
        // The table run this row belongs to, plus the line after it.
        let (last, next_row) = {
            let t = self.tab();
            let lines = &t.layout.lines;
            let is_table = |i: usize| {
                lines
                    .get(i)
                    .map(|l| l.block() == Block::Table)
                    .unwrap_or(false)
            };
            let mut last = line;
            while is_table(last + 1) {
                last += 1;
            }
            // The next row of text below this one: the separator is not a row you
            // can put a caret in.
            let next = (line + 1..=last).find(|&i| !lines[i].info.table_sep);
            (last, next)
        };
        // Nothing below: an empty last row means "leave the table", anything else
        // opens a new row at the bottom.
        if next_row.is_none() && row_empty {
            let t = self.tab_mut();
            let n = text.chars().count();
            t.doc.replace_range(line_start, line_start + n, "");
            t.caret = line_start;
            t.anchor = line_start;
            t.goal_x = None;
            self.ensure_caret_visible();
            return;
        }
        let Some(target) = next_row else {
            // Append a row of empty cells under the last row and put the caret in
            // the same column of it. The row is written here, so its column start is
            // known without waiting for the next layout: `|` plus `  |` per column,
            // so the pipe that closes column `ci` - which is where the parser puts
            // an empty cell's caret - is at `1 + 3 * (ci + 1)`.
            let mut row = String::from("|");
            for _ in 0..ncols {
                row.push_str("  |");
            }
            let t = self.tab_mut();
            let at = t.doc.line_start(last) + t.doc.line_text(last).chars().count();
            t.doc.insert_atomic(at, &format!("\n{row}"));
            let caret = at + 1 + 3 * (ci.min(ncols - 1) + 1);
            t.caret = caret;
            t.anchor = caret;
            t.goal_x = None;
            self.ensure_caret_visible();
            return;
        };
        // Select the cell the caret lands in, so typing replaces its text.
        let (start, end) = {
            let t = self.tab();
            let base = t.doc.line_start(target);
            match t.layout.lines.get(target).and_then(|l| l.info.cells.get(ci).copied()) {
                Some((a, b)) if b > a => (base + a, base + b),
                Some((a, _)) => (base + a, base + a),
                // The target row has not been laid out yet: its first cell starts
                // just after the `| ` that opens it.
                None => (base + 2, base + 2),
            }
        };
        let t = self.tab_mut();
        t.anchor = start;
        t.caret = end.max(start);
        t.goal_x = None;
        self.ensure_caret_visible();
    }

    pub fn indent(&mut self, outdent: bool) {
        let t = self.tab_mut();
        if t.has_selection() {
            let (a, b) = t.sel();
            let first = t.doc.line_of_char(a);
            let mut last = t.doc.line_of_char(b);
            if last > first && b == t.doc.line_start(last) {
                last -= 1;
            }
            for line in (first..=last).rev() {
                let start = t.doc.line_start(line);
                if outdent {
                    let text = t.doc.line_text(line);
                    let mut n = 0;
                    for c in text.chars() {
                        if c == ' ' && n < 2 {
                            n += 1;
                        } else if c == '\t' {
                            n += 1;
                            break;
                        } else {
                            break;
                        }
                    }
                    if n > 0 {
                        t.doc.remove_atomic(start, start + n);
                    }
                } else {
                    t.doc.insert_atomic(start, "  ");
                }
            }
            // Keep the selection over the same lines.
            let delta = if outdent { 0 } else { 2 * (last - first + 1) };
            let _ = delta;
            let t = self.tab_mut();
            t.anchor = t.doc.line_start(first);
            t.caret = t.doc.line_start(last) + t.doc.line_len(last);
        } else if outdent {
            let (line, col) = t.doc.line_and_col(t.caret);
            let start = t.doc.line_start(line);
            let text = t.doc.line_text(line);
            let n = leading_spaces(&text).min(2);
            if n > 0 {
                t.doc.remove_atomic(start, start + n);
                t.caret = t.caret.saturating_sub(n).max(start);
                t.anchor = t.caret;
            }
            let _ = col;
        } else {
            // Indent the *line*, not just the caret: a bullet, checkbox or quote
            // marker moves together with its text.
            let (line, _) = t.doc.line_and_col(t.caret);
            let start = t.doc.line_start(line);
            t.doc.insert_atomic(start, "  ");
            t.caret += 2;
            t.anchor = t.caret;
        }
    }

    pub fn undo(&mut self) {
        let t = self.tab_mut();
        if let Some(pos) = t.doc.undo() {
            t.caret = pos.min(t.doc.len_chars());
            t.anchor = t.caret;
        }
    }

    pub fn redo(&mut self) {
        let t = self.tab_mut();
        if let Some(pos) = t.doc.redo() {
            t.caret = pos.min(t.doc.len_chars());
            t.anchor = t.caret;
        }
    }

    // ---------- dialog text fields ----------

    /// The text field the dialog owns right now, if any.
    pub fn dialog_field_mut(&mut self) -> Option<&mut crate::field::TextField> {
        match &mut self.dialog {
            Some(Dialog::Colour(cp)) => {
                let bg = cp.active_bg;
                Some(&mut cp.slot_mut(bg).hex)
            }
            Some(Dialog::Picker(pk)) => Some(pk.field_mut()),
            _ => None,
        }
    }

    pub fn dialog_field(&self) -> Option<&crate::field::TextField> {
        match &self.dialog {
            Some(Dialog::Colour(cp)) => Some(&cp.slot(cp.active_bg).hex),
            Some(Dialog::Picker(pk)) => Some(match pk.field {
                PickField::Path => &pk.path,
                PickField::Name => &pk.name,
            }),
            _ => None,
        }
    }

    /// Re-derive whatever depends on the field after an edit (the hex box drives
    /// the colour swatch).
    fn dialog_field_changed(&mut self) {
        if let Some(Dialog::Colour(cp)) = &mut self.dialog {
            let bg = cp.active_bg;
            cp.slot_mut(bg).commit_hex();
        }
    }

    pub fn dialog_select_all(&mut self) -> bool {
        match self.dialog_field_mut() {
            Some(f) => {
                f.select_all();
                true
            }
            None => false,
        }
    }

    pub fn dialog_copy(&mut self) -> bool {
        let Some(f) = self.dialog_field() else { return false };
        let Some((a, b)) = f.selection() else { return true };
        let text: String = f.text.chars().skip(a).take(b - a).collect();
        if let Some(cb) = self.clipboard() {
            let _ = cb.set_text(text);
            self.set_status("copied");
        }
        true
    }

    pub fn dialog_cut(&mut self) -> bool {
        if !self.dialog_copy() {
            return false;
        }
        if let Some(f) = self.dialog_field_mut() {
            if let Some((a, b)) = f.selection() {
                f.place_caret(a, false);
                for _ in a..b {
                    f.delete_forward();
                }
            }
        }
        self.dialog_field_changed();
        true
    }

    pub fn dialog_paste(&mut self) -> bool {
        let text = self.clipboard().and_then(|cb| cb.get_text().ok());
        let Some(text) = text else { return true };
        let one_line = text.replace(['\r', '\n'], "").replace('\t', " ");
        if let Some(f) = self.dialog_field_mut() {
            f.insert(&one_line);
        }
        self.dialog_field_changed();
        true
    }

    pub fn copy(&mut self) {
        if let Some(text) = self.selected_text() {
            if let Some(cb) = self.clipboard() {
                let _ = cb.set_text(text);
                self.set_status("copied");
            }
        }
    }

    pub fn cut(&mut self) {
        if let Some(text) = self.selected_text() {
            if let Some(cb) = self.clipboard() {
                let _ = cb.set_text(text);
            }
            self.delete_selection();
        }
    }

    pub fn paste(&mut self) {
        let text = self.clipboard().and_then(|cb| cb.get_text().ok());
        if let Some(text) = text {
            let clean = text.replace("\r\n", "\n");
            self.insert_text(&clean);
        }
    }

    // ---------- middle-button panning ----------

    /// Largest scroll offsets for the active tab (0 when the page fits).
    pub fn scroll_limits(&self) -> (f32, f32) {
        let t = self.tab();
        let view = self.text_rect;
        let max_y = (t.layout.total_height - view.h).max(0.0);
        let max_x = (t.layout.max_row_width - view.w).max(0.0);
        (max_x, max_y)
    }

    fn set_scroll(&mut self, x: f32, y: f32) {
        let (max_x, max_y) = self.scroll_limits();
        let t = self.tab_mut();
        t.scroll = y.clamp(0.0, max_y);
        t.scroll_x = x.clamp(0.0, max_x);
    }

    pub fn pan_begin(&mut self) {
        self.pan_active = true;
        self.kinetic_at = None;
        self.pan_velocity = (0.0, 0.0);
        self.pan_anchor = self.mouse;
        self.pan_scroll = (self.tab().scroll_x, self.tab().scroll);
        self.pan_last = (Instant::now(), self.pan_scroll);
    }

    pub fn pan_to(&mut self, pos: (f32, f32)) {
        if !self.pan_active {
            return;
        }
        let dx = pos.0 - self.pan_anchor.0;
        let dy = pos.1 - self.pan_anchor.1;
        let (x, y) = (self.pan_scroll.0 - dx, self.pan_scroll.1 - dy);
        self.set_scroll(x, y);
        // Velocity in document pixels per second, smoothed so a jittery mouse
        // does not fling the page across the screen.
        let now = Instant::now();
        let dt = now.duration_since(self.pan_last.0).as_secs_f32().max(0.001);
        let (px, py) = self.pan_last.1;
        let (nx, ny) = (self.tab().scroll_x, self.tab().scroll);
        let vx = (nx - px) / dt;
        let vy = (ny - py) / dt;
        // Favour the newest movement: a quick flick has to feel immediate.
        self.pan_velocity = (
            self.pan_velocity.0 * 0.35 + vx * 0.65,
            self.pan_velocity.1 * 0.35 + vy * 0.65,
        );
        self.pan_last = (now, (nx, ny));
    }

    pub fn pan_end(&mut self) {
        self.pan_active = false;
        // Releasing after a pause is not a throw; releasing right after a drag is.
        // The window is generous on purpose: letting go of a drag is rarely
        // instantaneous, and a fling that died because the button came up 200 ms
        // after the last movement felt like the drag had stopped by itself.
        let idle = self.pan_last.0.elapsed().as_secs_f32();
        let speed = (self.pan_velocity.0.powi(2) + self.pan_velocity.1.powi(2)).sqrt();
        if idle > 0.12 || speed < 40.0 {
            self.pan_velocity = (0.0, 0.0);
            self.kinetic_at = None;
        } else {
            // Decay the throw as if the time since the last movement had already
            // been spent gliding, so a slow release does not start with a jolt.
            let k = (-idle / FLING_TAU).exp();
            self.pan_velocity = (self.pan_velocity.0 * k, self.pan_velocity.1 * k);
            self.kinetic_at = Some(Instant::now());
        }
    }

    /// Whether a fling is still running (the event loop uses this to stay awake).
    pub fn kinetic_active(&self) -> bool {
        self.kinetic_at.is_some()
    }

    /// Advance a middle-button fling by one frame.
    pub fn step_kinetic(&mut self) {
        let Some(at) = self.kinetic_at else { return };
        let now = Instant::now();
        let dt = now.duration_since(at).as_secs_f32().min(0.05);
        if dt < 0.001 {
            // Nothing measurable happened yet: wait for the next frame instead of
            // spinning a thousand times a second.
            return;
        }
        self.kinetic_at = Some(now);
        let (vx, vy) = self.pan_velocity;
        if vx.abs() < 4.0 && vy.abs() < 4.0 {
            self.pan_velocity = (0.0, 0.0);
            self.kinetic_at = None;
            return;
        }
        let (x, y) = (self.tab().scroll_x + vx * dt, self.tab().scroll + vy * dt);
        self.set_scroll(x, y);
        // Exponential decay with a time constant, so the glide is the same shape
        // whether the frames come at 60 or at 144 Hz. `0.965` per frame was tied
        // to the frame rate and died in well under a second on a fast display.
        let k = (-dt / FLING_TAU).exp();
        self.pan_velocity = (vx * k, vy * k);
        // Hitting an edge stops the fling instead of grinding against it.
        let (max_x, max_y) = self.scroll_limits();
        let t = self.tab();
        if (t.scroll <= 0.0 && vy < 0.0)
            || (t.scroll >= max_y && vy > 0.0)
            || (t.scroll_x <= 0.0 && vx < 0.0)
            || (t.scroll_x >= max_x && vx > 0.0)
        {
            self.pan_velocity = (0.0, 0.0);
            self.kinetic_at = None;
        }
    }

    // ---------- navigation ----------

    pub fn move_horizontal(&mut self, forward: bool, extend: bool) {
        let t = self.tab_mut();
        let caret = t.caret;
        if !extend && t.has_selection() {
            let (a, b) = t.sel();
            let pos = if forward { b } else { a };
            self.set_caret(pos, false);
            return;
        }
        // Right means right on the screen. In a right-to-left run - a Hebrew or
        // Arabic paragraph, or a cell of either inside a table - the next position
        // *visually* to the right is the previous one in the document, so a plain
        // `caret + 1` walked the arrows backwards. The layout knows the visual order
        // of the positions of the row the caret is on; when it has nothing to say
        // (an ordinary line, or a step that leaves the row) movement stays logical.
        let raw = self.line_is_raw_here(caret);
        let visual = self
            .tab()
            .layout
            .lines
            .get(self.tab().layout.line_of_char(caret))
            .and_then(|line| {
                let local = caret - line.start_char;
                line.visual_step(local, forward, raw).map(|step| line.start_char + step)
            });
        let pos = match visual {
            Some(pos) => pos,
            None => {
                let pos = if forward {
                    (caret + 1).min(self.tab().doc.len_chars())
                } else {
                    caret.saturating_sub(1)
                };
                self.skip_hidden_markers(pos, forward)
            }
        };
        self.set_caret(pos, extend);
    }

    /// Whether the line holding `caret` is showing its Markdown source.
    fn line_is_raw_here(&self, caret: usize) -> bool {
        let line = self.tab().layout.line_of_char(caret);
        self.tab().line_is_raw(self.mode, line) || self.tab().table_shows_markers(self.mode, line)
    }

    /// Block markers that are hidden in the rendered view (`> `, `- `, `# `) are
    /// not addressable: stepping into them would put the caret in front of a
    /// glyph that is not on screen. Moving left over one jumps straight past it
    /// to the previous line, moving right lands after it.
    fn skip_hidden_markers(&self, pos: usize, forward: bool) -> usize {
        if self.mode.shows_markers() {
            return pos;
        }
        let t = self.tab();
        let (line, col) = t.doc.line_and_col(pos);
        let Some(ll) = t.layout.lines.get(line) else { return pos };
        let prefix = ll.info.prefix;
        if prefix == 0 || !ll.marks_a_block() {
            return pos;
        }
        if !forward {
            // `col == prefix` is the position right after the marker - the start
            // of the visible text - so that one is a real stop. Only a caret
            // *inside* the hidden marker jumps over it.
            if col >= prefix {
                return pos;
            }
            let start = t.doc.line_start(line);
            return start.saturating_sub(1);
        }
        if col < prefix {
            return t.doc.line_start(line) + prefix;
        }
        pos
    }

    pub fn move_vertical(&mut self, down: bool, extend: bool) {
        let tab = self.tab();
        let line_idx = tab.layout.line_of_char(tab.caret);
        let Some(line) = tab.layout.lines.get(line_idx) else { return };
        let local = tab.caret - line.start_char;
        let ri = line.row_of_local(local);
        let goal = tab.goal_x;
        let row = line.row(ri);
        let raw = tab.line_is_raw(self.mode, line_idx);
        let x = goal.unwrap_or_else(|| approx_x(line, local, raw) - approx_x(line, row.start, raw));

        // Move inside the same line first when it is wrapped.
        let (target_line, target_row) = if down {
            if ri + 1 < line.row_count() {
                (line_idx, ri + 1)
            } else if line_idx + 1 < tab.layout.lines.len() {
                (line_idx + 1, 0)
            } else {
                let end = tab.doc.len_chars();
                self.set_caret(end, extend);
                return;
            }
        } else if ri > 0 {
            (line_idx, ri - 1)
        } else if line_idx > 0 {
            let prev = &tab.layout.lines[line_idx - 1];
            (line_idx - 1, prev.row_count() - 1)
        } else {
            self.set_caret(0, extend);
            return;
        };

        let target_raw = self.tab().line_is_raw(self.mode, target_line);
        let target = &self.tab().layout.lines[target_line];
        let local_new = approx_char_at_row_x(target, target_row, x, target_raw);
        let pos = target.start_char + local_new;
        self.set_caret(pos, extend);
        self.tab_mut().goal_x = Some(x);
    }

    pub fn move_line_boundary(&mut self, end: bool, extend: bool) {
        let t = self.tab_mut();
        let (line, _) = t.doc.line_and_col(t.caret);
        let pos = if end {
            t.doc.line_start(line) + t.doc.line_len(line)
        } else {
            t.doc.line_start(line)
        };
        self.set_caret(pos, extend);
    }

    pub fn move_doc_boundary(&mut self, end: bool, extend: bool) {
        let pos = if end { self.tab().doc.len_chars() } else { 0 };
        self.set_caret(pos, extend);
    }

    pub fn move_word(&mut self, forward: bool, extend: bool) {
        let text: Vec<char> = self.tab().doc.text().chars().collect();
        let caret = self.tab().caret;
        let mut i = caret;
        if forward {
            while i < text.len() && text[i].is_whitespace() {
                i += 1;
            }
            while i < text.len() && !text[i].is_whitespace() {
                i += 1;
            }
        } else {
            while i > 0 && text.get(i - 1).map(|c| c.is_whitespace()).unwrap_or(false) {
                i -= 1;
            }
            while i > 0 && !text.get(i - 1).map(|c| c.is_whitespace()).unwrap_or(true) {
                i -= 1;
            }
        }
        self.set_caret(i, extend);
    }

    pub fn page(&mut self, down: bool, extend: bool) {
        let (_, h) = (self.editor_rect.w, self.editor_rect.h);
        let step = (h / self.theme.font_size.max(1.0) / 1.4).round().max(3.0) as isize;
        let t = self.tab();
        let line = t.layout.line_of_char(t.caret) as isize;
        let target = (line + if down { step } else { -step })
            .clamp(0, self.tab().layout.lines.len().saturating_sub(1) as isize) as usize;
        let goal = self.tab().goal_x.unwrap_or_else(|| {
            let ll = &self.tab().layout.lines[line as usize];
            let raw = self.active_line_raw(line as usize);
            approx_x(ll, self.tab().caret - ll.start_char, raw)
        });
        let target_raw = self.active_line_raw(target);
        let target_line = &self.tab().layout.lines[target];
        let pos = target_line.start_char + approx_char_at_row_x(target_line, 0, goal, target_raw);
        self.set_caret(pos, extend);
        self.tab_mut().goal_x = Some(goal);
        // Scroll so the caret stays at the same relative position.
        let tab = self.tab_mut();
        let delta = if down { h * 0.8 } else { -h * 0.8 };
        tab.scroll = (tab.scroll + delta).max(0.0);
    }

    // ---------- formatting ----------

    /// Wraps or unwraps an inline marker pair around the selection (or the caret).
    ///
    /// Pressing the button repeatedly is idempotent: empty markers next to the
    /// caret are removed instead of piling up `****` into the document.
    pub fn toggle_inline(&mut self, open: &str, close: &str) {
        // `*`/`_` (and `**`/`__`) are interchangeable in Markdown, so a document
        // written with one syntax must respond to the button that would have
        // inserted the other.
        let mut pairs: Vec<(&str, &str)> = vec![(open, close)];
        if open == close {
            let alt = match open {
                "**" => Some("__"),
                "__" => Some("**"),
                "*" => Some("_"),
                "_" => Some("*"),
                _ => None,
            };
            if let Some(alt) = alt {
                pairs.push((alt, alt));
            }
        }
        self.toggle_inline_pairs(&pairs);
    }

    /// Toggle the first marker pair, and un-wrap any of the equivalent `pairs`
    /// that already surround the text.
    fn toggle_inline_pairs(&mut self, pairs: &[(&str, &str)]) {
        let (open, close) = pairs[0];
        let open_len = open.chars().count();
        let t = self.tab_mut();

        // ---- 1. a selection ----
        if t.has_selection() {
            let (a, b) = t.sel();
            let selected = t.doc.slice(a, b);
            for &(o, c) in pairs {
                let (ol, cl) = (o.chars().count(), c.chars().count());
                let inner_start = a.saturating_sub(ol);
                let before = t.doc.slice(inner_start, a);
                let after = t.doc.slice(b, (b + cl).min(t.doc.len_chars()));
                if selected.starts_with(o)
                    && selected.ends_with(c)
                    && selected.chars().count() >= ol + cl
                {
                    let inner: String = selected.chars().skip(ol).collect::<String>();
                    let inner: String =
                        inner.chars().take(inner.chars().count() - cl).collect();
                    t.doc.replace_range(a, b, &inner);
                    let new_b = a + inner.chars().count();
                    t.anchor = a;
                    t.caret = new_b;
                    return;
                }
                if before == o && after == c {
                    // The markers are already outside the selection: remove them.
                    t.doc.replace_range(a - ol, a, "");
                    let (a2, b2) = (a - ol, b - ol);
                    t.doc.replace_range(b2, b2 + cl, "");
                    t.anchor = a2;
                    t.caret = b2;
                    return;
                }
            }
            t.doc.replace_range(a, b, &format!("{open}{selected}{close}"));
            t.anchor = a + open_len;
            t.caret = a + open_len + selected.chars().count();
            return;
        }

        let caret = t.caret;

        // ---- 2. the caret is inside (or touching) an existing span ----
        for &(o, c) in pairs {
            if let Some((a, end)) = Self::enclosing_span(&t.doc, caret, o, c) {
                let ol = o.chars().count();
                let cl = c.chars().count();
                let inner = t.doc.slice(a + ol, end);
                t.doc.replace_range(a, end + cl, &inner);
                let pos = caret.saturating_sub(ol).min(a + inner.chars().count());
                t.caret = pos;
                t.anchor = pos;
                return;
            }
        }

        // ---- 3. a word right before the caret (e.g. `word|` -> `**word**`) ----
        let chars: Vec<char> = t.doc.text().chars().collect();
        let (ws, we) = word_bounds(&chars, caret);
        if we > ws {
            // Already formatted with any equivalent syntax? Then un-wrap it
            // instead of stacking a second pair of markers on top.
            for &(o, c) in pairs {
                let (ol, cl) = (o.chars().count(), c.chars().count());
                if ws >= ol
                    && chars.get(ws - ol..ws) == Some(o.chars().collect::<Vec<_>>().as_slice())
                    && chars.get(we..we + cl) == Some(c.chars().collect::<Vec<_>>().as_slice())
                {
                    t.doc.remove_atomic(we, we + cl);
                    t.doc.remove_atomic(ws - ol, ws);
                    t.caret = we - ol;
                    t.anchor = t.caret;
                    return;
                }
            }
            let word = t.doc.slice(ws, we);
            t.doc.replace_range(ws, we, &format!("{open}{word}{close}"));
            t.caret = ws + open_len + word.chars().count();
            t.anchor = t.caret;
            return;
        }

        // ---- 4. nothing to format: leave an empty pair with the caret inside ----
        t.doc.insert_atomic(caret, &format!("{open}{close}"));
        t.caret = caret + open_len;
        t.anchor = t.caret;
    }

    /// Insert a fenced block around the selected lines (or an empty block when
    /// there is no selection). A single-line selection gets inline back-ticks.
    pub fn toggle_code_block(&mut self) {
        let t = self.tab_mut();
        let multiline = if t.has_selection() {
            let (a, b) = t.sel();
            t.doc.slice(a, b).contains('\n')
        } else {
            false
        };
        if !multiline {
            self.toggle_inline("`", "`");
            return;
        }
        let t = self.tab_mut();
        let (a, b) = t.sel();
        let first = t.doc.line_of_char(a);
        let last = t.doc.line_of_char(b);
        let start = t.doc.line_start(first);
        let mut end = t.doc.line_start(last) + t.doc.line_len(last);
        // A selection ending exactly at a line start should not swallow that line.
        if b == t.doc.line_start(last) && last > first {
            end = t.doc.line_start(last.saturating_sub(0)).min(end);
        }
        let body = t.doc.slice(start, end);
        let fenced = format!("```\n{body}\n```");
        t.doc.replace_range(start, end, &fenced);
        t.anchor = start;
        t.caret = start + 3 + 1 + body.chars().count();
    }

    /// Find an existing `open`..`close` span on the caret's line that the caret
    /// touches — inside it, or immediately before/after it.
    ///
    /// Returns `(opener_start, closer_start)` in document coordinates. Pairing is
    /// done by scanning the line, not by searching outwards from the caret, so a
    /// caret sitting right after `**bold**` still finds the pair (that used to
    /// make repeated Bold presses stack markers).
    fn enclosing_span(doc: &Doc, caret: usize, open: &str, close: &str) -> Option<(usize, usize)> {
        let (line, _) = doc.line_and_col(caret);
        let start = doc.line_start(line);
        let len = doc.line_len(line);
        let text = doc.slice(start, start + len);
        let chars: Vec<char> = text.chars().collect();
        let local = caret.saturating_sub(start).min(chars.len());
        let op: Vec<char> = open.chars().collect();
        let cl: Vec<char> = close.chars().collect();
        if op.is_empty() || cl.is_empty() {
            return None;
        }
        let at = |i: usize, p: &[char]| -> bool { i + p.len() <= chars.len() && chars[i..i + p.len()] == *p };

        let mut best: Option<(usize, usize)> = None;
        if op == cl {
            // Same marker on both sides (`**`, `~~`, "`"): pair the occurrences in
            // order, otherwise a closer would also look like an opener.
            let mut hits = Vec::new();
            let mut i = 0usize;
            while i < chars.len() {
                if at(i, &op) {
                    hits.push(i);
                    i += op.len();
                } else {
                    i += 1;
                }
            }
            let mut k = 0usize;
            while k + 1 < hits.len() {
                let (o, c) = (hits[k], hits[k + 1]);
                k += 2;
                let span_end = c + cl.len();
                if local >= o && local <= span_end {
                    if best.map(|(bo, bc)| span_end - o < bc - bo).unwrap_or(true) {
                        best = Some((o, c));
                    }
                }
            }
        } else {
            // Collect openers and closers left to right, then pair them up.
            let (mut opens, mut closes) = (Vec::new(), Vec::new());
            let mut i = 0usize;
            while i < chars.len() {
                if at(i, &op) {
                    opens.push(i);
                    i += op.len();
                } else if at(i, &cl) {
                    closes.push(i);
                    i += cl.len();
                } else {
                    i += 1;
                }
            }
            if !opens.is_empty() && !closes.is_empty() {
                let mut ci = 0usize;
                for &o in &opens {
                    while ci < closes.len() && closes[ci] < o + op.len() {
                        ci += 1;
                    }
                    let Some(&c) = closes.get(ci) else { break };
                    ci += 1;
                    let span_end = c + cl.len();
                    if local >= o && local <= span_end {
                        if best.map(|(bo, bc)| span_end - o < bc - bo).unwrap_or(true) {
                            best = Some((o, c));
                        }
                    }
                }
            }
        }
        best.map(|(o, c)| (start + o, start + c))
    }

    fn set_line_prefix(&mut self, transform: impl Fn(Option<&str>) -> Option<String>) {
        let t = self.tab_mut();
        let (line, _) = t.doc.line_and_col(t.caret);
        let start = t.doc.line_start(line);
        let text = t.doc.line_text(line);
        let chars: Vec<char> = text.chars().collect();
        // The marker length is what the layout parse reported for this line.
        let prefix_len = t.layout.lines.get(line).map(|l| l.info.prefix).unwrap_or_else(|| {
            let hashes = chars.iter().take_while(|c| **c == '#').count();
            if hashes > 0 && chars.get(hashes) == Some(&' ') {
                hashes + 1
            } else {
                0
            }
        });
        let existing: Option<String> = if prefix_len > 0 {
            Some(chars[..prefix_len].iter().collect())
        } else {
            None
        };
        let existing = existing.filter(|p| !p.trim().is_empty());
        match transform(existing.as_deref()) {
            Some(new_prefix) => {
                t.doc.replace_range(start, start + prefix_len, &new_prefix);
                t.caret = start + new_prefix.chars().count();
                t.anchor = t.caret;
            }
            None => {
                if prefix_len > 0 {
                    t.doc.replace_range(start, start + prefix_len, "");
                    t.caret = start;
                    t.anchor = start;
                }
            }
        }
    }

    pub fn toggle_heading(&mut self, level: u8) {
        let target = format!("{} ", "#".repeat(level as usize));
        let cmp = target.clone();
        self.set_line_prefix(move |existing| match existing {
            Some(p) if p.starts_with('#') && p == cmp => None,
            _ => Some(target.clone()),
        });
    }

    /// Turn the selection into a list of `kind`, or switch it to that kind.
    ///
    /// Two things used to be wrong here, and both are about the *selection*
    /// rather than the line under the caret:
    ///
    /// * Only the caret's line was rewritten. Selecting four paragraphs and
    ///   pressing the list button produced **one** item - the last one - which read
    ///   as "list makes 1 row into a list instead of logical points in the whole
    ///   text".
    /// * A line that already was a list kept its old marker, so switching a bullet
    ///   list to an ordered one (or the other way round) renumbered nothing and
    ///   changed nothing except the line under the caret.
    ///
    /// Now every selected line is planned first and written afterwards, bottom-up,
    /// so the line numbers stay valid while the text moves. Pressing the button
    /// that is already in force removes the markers from the whole selection;
    /// pressing any other list button *switches* it, and an ordered list is
    /// numbered from 1 in document order - so a converted selection never comes
    /// out as four `1.`s.
    pub fn toggle_list(&mut self, kind: Block) {
        let (l0, l1) = self.selected_lines();
        let want = match kind {
            Block::Ordered => Block::Ordered,
            Block::Task(_) => Block::Task(false),
            Block::Quote => Block::Quote,
            _ => Block::Bullet,
        };
        // The kind the toolbar asked for, in the form a line reports after parsing.
        let same_kind = |a: Block, b: Block| match (a, b) {
            (Block::Task(_), Block::Task(_)) => true,
            (Block::Heading(x), Block::Heading(y)) => x == y,
            (a, b) => a == b,
        };
        struct Line {
            /// Chars the current block marker occupies.
            prefix: usize,
            block: Block,
            /// Nothing to mark: a blank line, a heading, a card or a table row.
            skip: bool,
        }
        let lines: Vec<Line> = {
            let t = self.tab();
            (l0..=l1)
                .map(|line| {
                    let text = t.doc.line_text(line);
                    let chars: Vec<char> = text.chars().collect();
                    let mut fenced = None;
                    let info = crate::markdown::parse(&chars, &mut fenced);
                    let blank = chars.iter().all(|c| c.is_whitespace());
                    let card = matches!(
                        info.block,
                        Block::Heading(_)
                            | Block::Code
                            | Block::Fence
                            | Block::Table
                            | Block::Hr
                            | Block::Blank
                    );
                    Line { prefix: info.prefix.min(chars.len()), block: info.block, skip: blank || card }
                })
                .collect()
        };
        let marked: Vec<&Line> = lines.iter().filter(|l| !l.skip).collect();
        if marked.is_empty() {
            return;
        }
        // Already this kind everywhere? Then the press means "take it off".
        let all_same = marked.iter().all(|l| same_kind(l.block, want));
        let mut plan: Vec<(usize, usize, String)> = Vec::new();
        let mut n = 0usize;
        for (k, l) in lines.iter().enumerate() {
            if l.skip {
                continue;
            }
            let marker = if all_same {
                String::new()
            } else {
                match want {
                    Block::Ordered => {
                        n += 1;
                        format!("{n}. ")
                    }
                    Block::Task(_) => "- [ ] ".to_string(),
                    Block::Quote => "> ".to_string(),
                    _ => "- ".to_string(),
                }
            };
            plan.push((l0 + k, l.prefix, marker));
        }
        let t = self.tab_mut();
        let caret_line = t.doc.line_of_char(t.caret);
        let anchor_line = t.doc.line_of_char(t.anchor);
        let caret_col = t.caret - t.doc.line_start(caret_line);
        t.doc.begin_compound();
        for (line, old, new) in plan.iter().rev() {
            let start = t.doc.line_start(*line);
            if *old > 0 || !new.is_empty() {
                t.doc.replace_range(start, start + *old, new);
            }
        }
        t.doc.end_compound();
        // The caret keeps its line and its column, shifted by what its own line
        // gained or lost: a selection that is still a selection is what lets the
        // next press switch the kind again.
        let shift = plan
            .iter()
            .find(|(line, _, _)| *line == caret_line)
            .map(|(_, old, new)| new.chars().count() as isize - *old as isize)
            .unwrap_or(0);
        let col = (caret_col as isize + shift).clamp(0, t.doc.line_len(caret_line) as isize) as usize;
        t.caret = t.doc.line_start(caret_line) + col;
        if t.caret == t.anchor || anchor_line == caret_line {
            t.anchor = t.caret;
        }
        t.goal_x = None;
        t.layout = Layout::empty();
    }

    pub fn insert_link(&mut self) {
        let t = self.tab_mut();
        if t.has_selection() {
            let (a, b) = t.sel();
            let text = t.doc.slice(a, b);
            let url = "https://";
            let new = format!("[{text}]({url})");
            t.doc.replace_range(a, b, &new);
            // Put the caret in the URL.
            let url_start = a + 1 + text.chars().count() + 2;
            t.anchor = url_start;
            t.caret = url_start + url.chars().count();
        } else {
            let caret = t.caret;
            t.doc.insert_atomic(caret, "[](https://)");
            t.anchor = caret + 1;
            t.caret = caret + 1;
        }
    }

    pub fn insert_block(&mut self, text: &str) {
        let t = self.tab_mut();
        let (line, _) = t.doc.line_and_col(t.caret);
        let end = t.doc.line_start(line) + t.doc.line_len(line);
        let mut insertion = String::new();
        if end > 0 && !t.doc.slice(end.saturating_sub(1), end).ends_with('\n') {
            insertion.push('\n');
        }
        insertion.push_str(text);
        let new = t.doc.insert_atomic(end, &insertion);
        t.caret = new;
        t.anchor = new;
    }

    /// Move the current line (or the selected lines) up or down.
    pub fn move_line(&mut self, down: bool) {
        let t = self.tab_mut();
        let (a, b) = if t.has_selection() { t.sel() } else { (t.caret, t.caret) };
        let l0 = t.doc.line_of_char(a);
        let l1 = t.doc.line_of_char(b);
        if down {
            if l1 + 1 >= t.doc.len_lines() {
                return;
            }
            let start = t.doc.line_start(l0);
            let end = t.doc.line_start(l1) + t.doc.line_len(l1);
            let block = t.doc.slice(start, end);
            let nstart = t.doc.line_start(l1 + 1);
            let nend = nstart + t.doc.line_len(l1 + 1);
            let next = t.doc.slice(nstart, nend);
            let shift = next.chars().count() as isize + 1;
            let new = format!("{next}\n{block}");
            t.doc.begin_compound();
            t.doc.replace_range(start, nend, &new);
            t.doc.end_compound();
            t.anchor = (t.anchor as isize + shift).max(0) as usize;
            t.caret = (t.caret as isize + shift).max(0) as usize;
        } else {
            if l0 == 0 {
                return;
            }
            let pstart = t.doc.line_start(l0 - 1);
            let pend = pstart + t.doc.line_len(l0 - 1);
            let prev = t.doc.slice(pstart, pend);
            let start = t.doc.line_start(l0);
            let end = t.doc.line_start(l1) + t.doc.line_len(l1);
            let block = t.doc.slice(start, end);
            let shift = prev.chars().count() as isize + 1;
            let new = format!("{block}\n{prev}");
            t.doc.begin_compound();
            t.doc.replace_range(pstart, end, &new);
            t.doc.end_compound();
            t.anchor = (t.anchor as isize - shift).max(0) as usize;
            t.caret = (t.caret as isize - shift).max(0) as usize;
        }
    }

    pub fn toggle_task_at(&mut self, pos: usize) {
        let t = self.tab_mut();
        let (line, _) = t.doc.line_and_col(pos);
        let text = t.doc.line_text(line);
        let chars: Vec<char> = text.chars().collect();
        if let Some(open) = chars.iter().position(|c| *c == '[') {
            if let Some(&state) = chars.get(open + 1) {
                let new = if state == ' ' { "x" } else { " " };
                let at = t.doc.line_start(line) + open + 1;
                t.doc.replace_range(at, at + 1, new);
            }
        }
    }

    // ---------- find ----------

    pub fn open_find(&mut self, with_replace: bool) {
        self.focus = Focus::FindQuery;
        let t = self.tab_mut();
        t.find.open = true;
        t.find.show_replace = with_replace;
        t.find.focus = FindFocus::Query;
        if t.has_selection() {
            let (a, b) = t.sel();
            let sel = t.doc.slice(a, b);
            if !sel.contains('\n') && sel.chars().count() < 120 {
                t.find.query = sel;
            }
        }
        let doc_version = t.doc.version;
        let t = self.tab_mut();
        let _ = doc_version;
        t.find.version = u64::MAX;
        // The whole query is selected, the way every other editor does it: the
        // next keystroke starts a new search instead of appending to the old one.
        t.find.select_all(FindFocus::Query);
        t.find.replace_caret = t.find.replace.chars().count();
        t.find.replace_anchor = t.find.replace_caret;
    }

    /// Open the About box.
    ///
    /// It takes no work beyond building its text, because everything it shows is
    /// already in the binary (`crate::ui::about`).
    pub fn open_about(&mut self) {
        self.menu = None;
        self.dialog = Some(Dialog::About(crate::ui::about::About::new()));
        self.focus = Focus::Dialog;
    }

    /// Write the whole notice - the program licence, every crate, the font licences -
    /// to `LICENSES.txt`, next to the executable when that is writable and in the data
    /// folder otherwise. Returns where it went.
    ///
    /// The point of a notice is that a *copy* can travel with the binary in a form
    /// people can read, so the box that shows it can also hand it over.
    pub fn save_licences(&mut self) -> std::io::Result<PathBuf> {
        let name = "LICENSES.txt";
        let beside_exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join(name)));
        let target = match beside_exe {
            Some(p) if std::fs::write(&p, crate::ui::about::notice()).is_ok() => p,
            _ => {
                let p = self.paths.data_dir.join(name);
                let _ = std::fs::create_dir_all(&self.paths.data_dir);
                std::fs::write(&p, crate::ui::about::notice())?;
                p
            }
        };
        Ok(target)
    }

    /// Begin dragging the About box's scrollbar, with the pointer at `pos`.
    ///
    /// Grabbing the thumb keeps the grab where it was taken, so the notice does not
    /// jump to meet the pointer; pressing the empty part of the track moves the thumb
    /// to the pointer (grabbed by its middle) and then drags from there - which is how
    /// every scrollbar behaves.
    pub fn about_drag_begin(&mut self, pos: (f32, f32)) {
        let Some(Dialog::About(a)) = &self.dialog else { return };
        let track = self.about_track;
        let thumb = crate::ui::about::thumb_rect(track, a.scroll, a.text.len(), a.visible);
        let grab = if thumb.contains(pos) {
            ((pos.1 - thumb.y) / thumb.h.max(1.0)).clamp(0.0, 1.0)
        } else {
            0.5
        };
        self.about_drag = Some(grab);
        self.about_drag_to(pos.1);
    }

    /// Keep dragging: the pointer's height down the track is the notice's position.
    pub fn about_drag_to(&mut self, y: f32) {
        let Some(grab) = self.about_drag else { return };
        let track = self.about_track;
        if let Some(Dialog::About(a)) = &mut self.dialog {
            a.scroll = crate::ui::about::scroll_for_pointer(track, grab, y, a.text.len(), a.visible);
        }
    }

    pub fn close_find(&mut self) {
        self.tab_mut().find.open = false;
        self.focus = Focus::Editor;
        self.focus_editor();
    }

    pub fn find_step(&mut self, forward: bool) {
        let t = self.tab_mut();
        let doc = &t.doc;
        t.find.ensure_matches(doc);
        if t.find.current.is_none() {
            t.find.select_at_or_after(t.caret);
        }
        if let Some((a, b)) = t.find.step(forward) {
            self.select_range(a, b);
        }
    }

    pub fn find_replace_one(&mut self) {
        let (a, b, replacement) = {
            let t = self.tab();
            match t.find.current_match() {
                Some((a, b)) => (a, b, t.find.replace.clone()),
                None => return,
            }
        };
        let t = self.tab_mut();
        t.doc.replace_range(a, b, &replacement);
        let caret = a + replacement.chars().count();
        t.caret = caret;
        t.anchor = caret;
        t.find.version = u64::MAX;
        self.find_step(true);
    }

    pub fn find_replace_all(&mut self) {
        let (query, replace, case, word) = {
            let t = self.tab();
            (t.find.query.clone(), t.find.replace.clone(), t.find.case_sensitive, t.find.whole_word)
        };
        if query.is_empty() {
            return;
        }
        let t = self.tab_mut();
        let n = replace_all(&mut t.doc, &query, &replace, case, word);
        t.find.version = u64::MAX;
        self.set_status(format!("replaced {n} occurrence(s)"));
    }

    pub fn focus_editor(&mut self) {
        self.focus = Focus::Editor;
    }

    /// Caret rectangle in window coordinates, exactly as the painter draws it.
    pub fn caret_rect(&self, te: &mut TextEngine) -> Rect {
        let tab = self.tab();
        let line_idx = tab.layout.line_of_char(tab.caret);
        let Some(line) = tab.layout.lines.get(line_idx) else {
            return Rect::ZERO;
        };
        let raw = tab.line_is_raw(self.mode, line_idx) || tab.table_shows_markers(self.mode, line_idx);
        let local = tab.caret - line.start_char;
        let ri = line.row_of_local_in(local, raw);
        let y = self.text_rect.y - tab.scroll + line.top + ri as f32 * line.row_h;
        let cx = self.text_rect.x - tab.scroll_x
            + line.indent
            + line.align_offset_row(self.text_rect.w, ri, raw)
            + line.x_of(te, local, raw);
        // Never shorter than a text line: an empty line still gets a real caret.
        let text_lh = te.line_height(Face::Regular, line.max_size.max(self.theme.font_size));
        let h = (line.row_h - 2.0).max(text_lh * 0.92);
        // No clamp to the row here: the only rows shorter than a line of text are
        // blank lines (which deliberately carry a full caret, they have no frame to
        // spill out of) and the `| --- |` rule. A fence row used to be one of them
        // and clipped the caret to half its height; that is fixed in the layout -
        // it is as tall as a row of code.
        let h = h.max(9.0);
        let dy = ((line.row_h - h) * 0.5).max(0.5);
        Rect::new(cx, y + dy, 2.0, h)
    }

    /// Dispatch a left click at `pos` exactly as the window layer does.
    pub fn click(&mut self, pos: (f32, f32), r: &mut crate::gpu::Renderer) {
        self.mouse = pos;
        self.on_mouse_down(r);
    }

    // ---------- misc ----------

    pub fn toggle_theme(&mut self) {
        self.set_theme(self.settings.theme.next());
    }

    /// Switch to a specific theme (keeps the user's font size).
    pub fn set_theme(&mut self, kind: ThemeKind) {
        self.settings.theme = kind;
        let size = self.theme.font_size;
        self.theme = Theme::by_kind(kind);
        self.theme.font_size = size;
        for t in &mut self.tabs {
            t.layout = Layout::empty();
        }
    }

    /// Drop every cached layout so the next frame rebuilds it (font size, view
    /// mode or width changes go through here).
    pub fn invalidate_layouts(&mut self) {
        for t in &mut self.tabs {
            t.layout = Layout::empty();
        }
    }

    /// Zoom the text. The scroll offsets are scaled with the font size so the
    /// document stays where it was: zooming must never look like scrolling.
    pub fn zoom(&mut self, delta: f32) {
        let old = self.theme.font_size;
        let size = (old + delta).clamp(10.0, 34.0);
        if (size - old).abs() < 0.01 {
            return;
        }
        let k = size / old;
        self.theme.font_size = size;
        self.settings.font_size = size;
        for t in &mut self.tabs {
            t.scroll *= k;
            t.scroll_x *= k;
            t.layout = Layout::empty();
        }
    }

    /// Whether line `line` of the active tab is painted with Markdown markers.
    pub fn line_is_raw(&self, line: usize) -> bool {
        self.tab().line_is_raw(self.mode, line)
    }

    /// Whether line `line` is drawn with its Markdown markers on screen.
    pub fn active_line_raw(&self, line: usize) -> bool {
        self.tab().line_is_raw(self.mode, line)
    }

    /// Cycle Live -> Live+ -> Raw, keeping every layout in step.
    pub fn cycle_view_mode(&mut self) {
        self.set_view_mode(self.mode.next());
    }

    pub fn set_view_mode(&mut self, mode: ViewMode) {
        self.mode = mode;
        self.settings.view_mode = mode;
        for t in &mut self.tabs {
            t.layout = Layout::empty();
        }
    }

    /// Keep the caret inside the visible text area.
    pub fn ensure_caret_visible(&mut self) {
        let view = self.text_rect;
        if view.is_empty() {
            return;
        }
        let i = self.active;
        let tab = &mut self.tabs[i];
        let line = tab.layout.line_of_char(tab.caret);
        let Some(ll) = tab.layout.lines.get(line) else { return };
        let ri = ll.row_of_local(tab.caret - ll.start_char);
        let top = ll.top + ri as f32 * ll.row_h;
        let bottom = top + ll.row_h;
        let margin = 10.0;
        if top - margin < tab.scroll {
            tab.scroll = (top - margin).max(0.0);
        } else if bottom + margin > tab.scroll + view.h {
            tab.scroll = bottom + margin - view.h;
        }
        let row = ll.row(ri);
        let raw = tab.line_is_raw(self.mode, line);
        let x = approx_x(ll, tab.caret - ll.start_char, raw)
            - approx_x(ll, row.start, raw)
            + ll.indent;
        // The horizontal camera only moves when the caret really is outside the
        // view - and never for a caret that sits at the *end* of a row, which is
        // where soft wrap puts it on every full line. The old rule kept a 24 px
        // margin on the right, so reaching the last character of a full row
        // scrolled the view 24 px to the left under a caret that was already fully
        // visible: "all text visually offsets left from it". A row that soft-wraps
        // is never wider than the view, so with wrap on this never fires and the
        // camera stays at 0; with wrap off the row can be wider and the caret is
        // followed with a small margin.
        if x < tab.scroll_x + 6.0 {
            tab.scroll_x = (x - 6.0).max(0.0);
        } else if ll.row_width_of(ri, raw) + ll.indent > view.w + 0.5
            && x > tab.scroll_x + view.w - 12.0
        {
            tab.scroll_x = (x - view.w + 12.0).max(0.0);
        }
        tab.scroll = tab.scroll.max(0.0);
        tab.scroll_x = tab.scroll_x.max(0.0);
    }

    /// Scroll so that a given source line is at the top of the viewport.
    pub fn scroll_to_line(&mut self, line: usize) {
        let tab = &mut self.tabs[self.active];
        if let Some(ll) = tab.layout.lines.get(line) {
            tab.scroll = (ll.top - 12.0).max(0.0);
        }
    }

    pub fn save_session(&self) {
        let mut session = Session::default();
        for t in &self.tabs {
            if t.doc.path.is_some() {
                session.tabs.push(crate::config::SessionTab {
                    path: t.doc.path.clone(),
                    caret: t.caret,
                    scroll: t.scroll,
                });
            }
        }
        session.active = self.active;
        session.save(&self.paths);
    }

    pub fn word_count(&self) -> usize {
        self.tab().layout.word_count
    }
}


/// Number of leading spaces on a line, treating a tab as two.
fn leading_spaces(text: &str) -> usize {
    let mut n = 0;
    for c in text.chars() {
        match c {
            ' ' => n += 1,
            '\t' => n += 2,
            _ => break,
        }
    }
    n
}

fn increment_ordered_marker(prefix: &str) -> String {
    let mut digits = String::new();
    for c in prefix.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            break;
        }
    }
    if let Ok(n) = digits.parse::<u32>() {
        let rest = &prefix[digits.len()..];
        return format!("{}{}", n + 1, rest);
    }
    prefix.to_string()
}

/// Decode `%20`-style escapes in a link target.
///
/// A path with a space in it is percent-encoded by every tool that writes links,
/// and `base.join("my%20note.md")` then looks for a file whose name really does
/// contain `%20` - so the link silently did nothing.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| -> Option<u8> {
                match b {
                    b'0'..=b'9' => Some(b - b'0'),
                    b'a'..=b'f' => Some(b - b'a' + 10),
                    b'A'..=b'F' => Some(b - b'A' + 10),
                    _ => None,
                }
            };
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Horizontal offset of a char index using cached run widths only (no shaping).
pub fn approx_x(ll: &crate::layout::LineLayout, local: usize, raw: bool) -> f32 {
    // A bidirectional line already has an exact table of caret positions - it was
    // built without an engine, so it is readable here too - and reading it is both
    // more accurate than interpolating inside a run and the only way to be right in
    // a right-to-left run, where the characters run *backwards* inside their own
    // box. This is what the find highlight and the typing goal use.
    if let Some(b) = &ll.bidi {
        let ri = ll.row_of_local_in(local, raw);
        if let Some(pts) = b.caret(raw).get(ri) {
            if !pts.is_empty() {
                return match pts.binary_search_by_key(&local, |p| p.0) {
                    Ok(k) => pts[k].1,
                    Err(k) => pts.get(k.saturating_sub(1)).map(|p| p.1).unwrap_or(0.0),
                };
            }
        }
    }
    let mut last = 0.0;
    for run in ll.runs(raw) {
        if local < run.start {
            // In a gap between runs the caret hugs the text before it.
            return last;
        }
        if local <= run.end {
            let frac = (local - run.start) as f32 / (run.end - run.start).max(1) as f32;
            return run.x + run.w * frac;
        }
        last = run.x + run.w;
    }
    last
}

/// Approximate x of a local index inside its visual row (no shaping).
pub fn approx_char_at_row_x(ll: &crate::layout::LineLayout, row_idx: usize, x: f32, raw: bool) -> usize {
    let row = ll.row(row_idx);
    // Same table again: nearest recorded position wins, which is what a pointer
    // means in either direction.
    if let Some(b) = &ll.bidi {
        if let Some(pts) = b.caret(raw).get(row_idx) {
            if !pts.is_empty() {
                let mut best = (f32::INFINITY, row.start);
                for (idx, cx) in pts {
                    if (cx - x).abs() < best.0 {
                        best = ((cx - x).abs(), *idx);
                    }
                }
                return best.1.clamp(row.start, row.end);
            }
        }
    }
    let base = approx_x(ll, row.start, raw);
    let target = x + base;
    for run in ll.runs(raw) {
        if run.end <= row.start {
            continue;
        }
        if run.start >= row.end {
            break;
        }
        let lo = run.start.max(row.start);
        let hi = run.end.min(row.end);
        let before = if lo > run.start {
            // Approximate: width of the part of the run before this row.
            run.w * (lo - run.start) as f32 / (run.end - run.start).max(1) as f32
        } else {
            0.0
        };
        if target <= run.x + before {
            return lo;
        }
        if target < run.x + run.w {
            let frac = ((target - run.x - before) / (run.w - before).max(0.01)).clamp(0.0, 1.0);
            let len = hi.saturating_sub(lo);
            return lo + (frac * len as f32).round() as usize;
        }
    }
    row.end
}

/// Colour lookup for a run ink.
pub fn ink_color(ink: Inks, theme: &Theme) -> crate::geom::Color {
    match ink {
        Inks::Text => theme.text,
        Inks::Dim => theme.text_dim,
        Inks::Heading => theme.heading,
        Inks::Code => theme.code_text,
        Inks::Link => theme.link,
        Inks::Marker => theme.marker,
        Inks::Quote => theme.quote_text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with(text: &str) -> App {
        let dir = std::env::temp_dir().join(format!("bare-md-app-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = Paths {
            data_dir: dir.clone(),
            config_file: dir.join("settings.conf"),
            origin: "test".into(),
        };
        let mut app = App::new(paths, Settings::default(), Session::default(), vec![]);
        app.tabs[0] = Tab::new(Doc::new(text, None));
        app.active = 0;
        app
    }

    fn rand_suffix() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos() as u64
    }

    #[test]
    fn typing_inserts_at_caret() {
        let mut app = app_with("");
        app.insert_text("hello");
        assert_eq!(app.tab().doc.text(), "hello");
        assert_eq!(app.tab().caret, 5);
        app.insert_text("!");
        assert_eq!(app.tab().doc.text(), "hello!");
    }

    #[test]
    fn selection_replaced_by_typing() {
        let mut app = app_with("abcdef");
        app.select_range(2, 4);
        app.insert_text("XY");
        assert_eq!(app.tab().doc.text(), "abXYef");
        assert_eq!(app.tab().caret, 4);
    }

    #[test]
    fn backspace_removes_marker_prefix() {
        let mut app = app_with("- item");
        app.layout_for_test();
        app.set_caret(2, false);
        app.backspace();
        assert_eq!(app.tab().doc.text(), "item");
    }

    #[test]
    fn enter_continues_list() {
        let mut app = app_with("- one");
        app.layout_for_test();
        app.set_caret(5, false);
        app.newline();
        assert_eq!(app.tab().doc.text(), "- one\n- ");
    }

    #[test]
    fn enter_continues_numbered_list_incrementing() {
        let mut app = app_with("3. three");
        app.layout_for_test();
        app.set_caret(8, false);
        app.newline();
        assert_eq!(app.tab().doc.text(), "3. three\n4. ");
    }

    #[test]
    fn enter_on_empty_list_item_starts_the_next_item() {
        let mut app = app_with("- ");
        app.layout_for_test();
        app.set_caret(2, false);
        app.newline();
        assert_eq!(app.tab().doc.text(), "- \n- ", "Enter keeps the list going");
        // Shift+Enter is what ends it.
        let mut app = app_with("- ");
        app.layout_for_test();
        app.set_caret(2, false);
        app.newline_ex(true);
        assert_eq!(app.tab().doc.text(), "", "Shift+Enter leaves the list");
    }

    #[test]
    fn bold_wraps_selection_and_unwraps() {
        let mut app = app_with("hello world");
        app.layout_for_test();
        app.select_range(0, 5);
        app.toggle_inline("**", "**");
        assert_eq!(app.tab().doc.text(), "**hello** world");
        app.select_range(2, 7);
        app.toggle_inline("**", "**");
        assert_eq!(app.tab().doc.text(), "hello world");
    }

    #[test]
    fn heading_toggle() {
        let mut app = app_with("title");
        app.layout_for_test();
        app.set_caret(2, false);
        app.toggle_heading(2);
        assert_eq!(app.tab().doc.text(), "## title");
        app.layout_for_test();
        app.toggle_heading(2);
        assert_eq!(app.tab().doc.text(), "title");
    }

    #[test]
    fn insert_link_wraps_selection() {
        let mut app = app_with("click here");
        app.layout_for_test();
        app.select_range(0, 5);
        app.insert_link();
        assert_eq!(app.tab().doc.text(), "[click](https://) here");
    }

    #[test]
    fn task_toggle_flips_checkbox() {
        let mut app = app_with("- [ ] todo");
        app.layout_for_test();
        app.toggle_task_at(0);
        assert_eq!(app.tab().doc.text(), "- [x] todo");
    }

    #[test]
    fn indent_selection_indents_lines() {
        let mut app = app_with("a\nb");
        app.layout_for_test();
        app.select_range(0, 3);
        app.indent(false);
        assert_eq!(app.tab().doc.text(), "  a\n  b");
    }
}

#[cfg(test)]
impl App {
    /// Builds the layout with a throw-away text engine (tests only).
    pub fn layout_for_test(&mut self) {
        let mut te = TextEngine::new();
        for t in &mut self.tabs {
            let theme = self.theme;
            t.ensure_layout(&mut te, &theme, 800.0, ViewMode::Live, true);
        }
    }
}

// ------------------------------------------------------------ round 5: actions

impl App {
    // ---------- line helpers ----------

    /// Line range covered by the selection, or the caret's own line.
    fn selected_lines(&self) -> (usize, usize) {
        let t = self.tab();
        let (a, b) = if t.has_selection() { t.sel() } else { (t.caret, t.caret) };
        let l0 = t.doc.line_of_char(a);
        let mut l1 = t.doc.line_of_char(b);
        // A selection that ends exactly at a line start does not include it.
        if l1 > l0 && b == t.doc.line_start(l1) {
            l1 -= 1;
        }
        (l0, l1)
    }

    /// Rewrite every selected line through `f`, keeping the selection on its text.
    ///
    /// The selection has to survive the rewrite, because these commands are
    /// all-or-nothing: quoting a list and pressing quote again has to unquote the
    /// whole list, and it did not. The old version put the caret back on its own
    /// line and *collapsed the selection there* (`anchor = caret`), so the second
    /// press no longer saw the list - it saw one line, and only the last item went
    /// back. The same collapse made the selection vanish under a bullet or quote
    /// command, which is a second reason to keep it: the lines are still selected
    /// afterwards, so the next command still applies to them.
    ///
    /// Each line is edited as a whole, so a position inside a rewritten line moves
    /// by that line's own length change, and positions in the other lines move by
    /// the sum of the changes before them - which is exactly what `line_start`
    /// already reports once the edits are in.
    fn map_selected_lines(&mut self, mut f: impl FnMut(&str) -> Option<String>) {
        let (l0, l1) = self.selected_lines();
        let t = self.tab_mut();
        let (sel_a, sel_b) = t.sel();
        let had_selection = t.has_selection();
        let caret = t.caret;
        let (a_line, a_col) = (t.doc.line_of_char(sel_a), sel_a - t.doc.line_start(t.doc.line_of_char(sel_a)));
        let (b_line, b_col) = (t.doc.line_of_char(sel_b), sel_b - t.doc.line_start(t.doc.line_of_char(sel_b)));
        let (c_line, c_col) = (t.doc.line_of_char(caret), caret - t.doc.line_start(t.doc.line_of_char(caret)));
        // Length change of every rewritten line, by line number.
        let mut delta = vec![0isize; l1 - l0 + 1];
        t.doc.begin_compound();
        for line in (l0..=l1).rev() {
            let start = t.doc.line_start(line);
            let text = t.doc.line_text(line);
            if let Some(new) = f(&text) {
                let old_len = text.chars().count();
                let new_len = new.chars().count();
                t.doc.replace_range(start, start + old_len, &new);
                delta[line - l0] = new_len as isize - old_len as isize;
            }
        }
        t.doc.end_compound();
        // A line that was rewritten moved its own characters by its own delta; the
        // lines outside the range are placed by `line_start` alone.
        let place = |t: &Tab, line: usize, col: usize| -> usize {
            let d = if line >= l0 && line <= l1 { delta[line - l0] } else { 0 };
            let len = t.doc.line_len(line);
            t.doc.line_start(line) + (col as isize + d).clamp(0, len as isize) as usize
        };
        if had_selection {
            let (a, b) = (place(&*t, a_line, a_col), place(&*t, b_line, b_col));
            t.anchor = a;
            t.caret = b;
        } else {
            let c = place(&*t, c_line, c_col);
            t.caret = c;
            t.anchor = c;
        }
        t.layout = Layout::empty();
    }

    // ---------- inline formatting ----------

    /// Remove every inline marker from the selection (or the caret's line).
    pub fn clear_formatting(&mut self) {
        let (l0, l1) = self.selected_lines();
        let t = self.tab_mut();
        let start = t.doc.line_start(l0);
        let end = t.doc.line_start(l1) + t.doc.line_len(l1);
        let text = t.doc.slice(start, end);
        let cleaned = strip_inline_markers(&text);
        if cleaned != text {
            t.doc.begin_compound();
            t.doc.replace_range(start, end, &cleaned);
            t.doc.end_compound();
            t.caret = (start + cleaned.chars().count()).min(t.doc.len_chars());
            t.anchor = t.caret;
            t.layout = Layout::empty();
        }
    }

    /// `[#rrggbb]…[/]` around the selection, or the same for a background.
    /// Apply (or clear) a text / background colour.
    ///
    /// Works in every situation the pointer or the keyboard can produce:
    ///
    /// * a selection: the selection gets wrapped, or loses the colour;
    /// * a bare caret inside a coloured span: that span is recoloured, or loses
    ///   its colour;
    /// * a bare caret on a plain line: the line gets wrapped, and clearing strips
    ///   whatever colour the line carries.
    ///
    /// The early `return` without a selection, plus only stripping a tag when the
    /// selection happened to be exactly one wrapped span, were why "remove
    /// colour" did nothing.
    pub fn set_text_color(&mut self, bg: bool, color: Option<u32>) {
        let t = self.tab();
        let (sel_a, sel_b) = t.sel();
        let has_sel = t.has_selection();
        let caret = t.caret;
        let line = t.doc.line_of_char(caret);
        let line_start = t.doc.line_start(line);
        let chars: Vec<char> = t.doc.line_text(line).chars().collect();
        let local_caret = caret.saturating_sub(line_start).min(chars.len());

        let spans = colour_spans(&chars);
        // The innermost span wins, so recolouring a span inside a span does not
        // rewrite the outer one.
        let enclosing = spans
            .iter()
            .filter(|sp| sp.bg == bg && local_caret >= sp.open && local_caret <= sp.close + 3)
            .last()
            .copied();

        // ---- the range the command acts on (document char offsets) ----
        // A span is replaced *whole*, tags included: replacing only its body and
        // writing a fresh tag in front of it stacked a second pair of tags on top
        // of the first, and the tags then grew with every press.
        let (mut a, mut b) = if has_sel {
            (sel_a, sel_b)
        } else if let Some(sp) = enclosing {
            (line_start + sp.open, (line_start + sp.close + 3).min(line_start + chars.len()))
        } else {
            let (ls, le) = stripped_line_bounds(&chars);
            (line_start + ls, line_start + le)
        };
        // A selection that is exactly the body of a coloured span acts on the
        // whole span: otherwise a second press would wrap the already coloured
        // text once more and the tags would pile up.
        let line_end = line_start + chars.len();
        if a >= line_start && b <= line_end {
            if let Some(sp) = spans
                .iter()
                .filter(|sp| sp.bg == bg)
                .find(|sp| line_start + sp.body_start == a && line_start + sp.body_end == b)
            {
                a = line_start + sp.open;
                b = (line_start + sp.close + 3).min(line_end);
            }
        }
        if a >= b {
            return;
        }
        if let Some(c) = color {
            self.last_colour = Some(c);
        }

        let t = self.tab_mut();
        let body = t.doc.slice(a, b);
        let inner = strip_colour_tags(&body, bg);

        match color {
            // ---- clear ----
            None => {
                if inner != body {
                    t.doc.begin_compound();
                    t.doc.replace_range(a, b, &inner);
                    t.doc.end_compound();
                    let len = t.doc.len_chars();
                    t.anchor = a.min(len);
                    t.caret = (a + inner.chars().count()).min(len);
                } else {
                    // Nothing of that kind inside the target: strip every span of
                    // that kind on the line, so "remove" always removes something
                    // the user can see.
                    let (ls, le) = stripped_line_bounds(&chars);
                    let (ls, le) = (line_start + ls, line_start + le);
                    let line_body = t.doc.slice(ls, le.max(ls));
                    let stripped = strip_colour_tags(&line_body, bg);
                    if stripped == line_body {
                        return;
                    }
                    t.doc.begin_compound();
                    t.doc.replace_range(ls, le.max(ls), &stripped);
                    t.doc.end_compound();
                    t.anchor = ls;
                    t.caret = ls + stripped.chars().count();
                }
                t.layout = Layout::empty();
            }
            // ---- apply ----
            Some(colour) => {
                let tag = if bg {
                    format!("[bg:#{colour:06x}]")
                } else {
                    format!("[#{colour:06x}]")
                };
                // Recolouring an existing span replaces its tags instead of
                // stacking a second pair on top of the first.
                let new = format!("{tag}{inner}[/]");
                t.doc.begin_compound();
                t.doc.replace_range(a, b, &new);
                t.doc.end_compound();
                let tag_len = tag.chars().count();
                let body_len = inner.chars().count();
                t.anchor = a + tag_len;
                t.caret = t.anchor + body_len;
                t.layout = Layout::empty();
            }
        }
    }

    /// `{center}` / `{right}` / `{left}` at the end of the selected lines.
    pub fn set_alignment(&mut self, align: u8) {
        let name = match align {
            1 => "center",
            2 => "right",
            3 => "justify",
            _ => "left",
        };
        self.map_selected_lines(|text| {
            let base = strip_align_tag(text);
            if align == 0 {
                Some(base)
            } else {
                Some(format!("{base} {{{name}}}"))
            }
        });
    }

    // ---------- block markers ----------

    /// Quote / un-quote the selected lines **without touching what is already
    /// there**: `> - [ ] task` keeps its checkbox, and every selected line is
    /// treated the same way (not just the caret's line).
    pub fn toggle_quote(&mut self) {
        let (l0, l1) = self.selected_lines();
        let t = self.tab();
        let mut all_quoted = true;
        let mut any = false;
        for line in l0..=l1 {
            let text = t.doc.line_text(line);
            if text.trim().is_empty() {
                continue;
            }
            any = true;
            if !is_quoted(&text) {
                all_quoted = false;
            }
        }
        if !any {
            return;
        }
        let remove = all_quoted;
        self.map_selected_lines(move |text| {
            if text.trim().is_empty() {
                return if remove { None } else { Some("> ".to_string()) };
            }
            Some(if remove { unquote_line(text) } else { quote_line(text) })
        });
    }

    /// Renumber an ordered list so the markers read 1..n.
    pub fn renumber_list(&mut self) {
        let (l0, l1) = self.selected_lines();
        // Widen to the whole contiguous ordered list around the selection.
        let t = self.tab();
        let is_ordered = |i: usize| {
            t.layout
                .lines
                .get(i)
                .map(|l| l.info.block == Block::Ordered)
                .unwrap_or(false)
        };
        let mut first = l0;
        while first > 0 && is_ordered(first - 1) {
            first -= 1;
        }
        let mut last = l1;
        while last + 1 < t.doc.len_lines() && is_ordered(last + 1) {
            last += 1;
        }
        // The numbers are counted **forwards** and written backwards. Counting
        // them inside the closure numbered the list from the bottom up, so
        // "Renumber" produced a list that ran 4, 3, 2, 1 - the "the numbered list
        // works backwards" report.
        let texts: Vec<String> = (first..=last).map(|i| self.tab().doc.line_text(i)).collect();
        let mut n = 1usize;
        let mut out: Vec<Option<String>> = Vec::with_capacity(texts.len());
        for text in &texts {
            let chars: Vec<char> = text.chars().collect();
            let indent: String = chars.iter().take_while(|c| **c == ' ').collect();
            let rest: String = chars[indent.chars().count()..].iter().collect();
            let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
            if digits == 0 {
                out.push(None);
                continue;
            }
            let after = &rest[digits..];
            if !(after.starts_with(". ") || after.starts_with(") ")) {
                out.push(None);
                continue;
            }
            out.push(Some(format!("{indent}{n}. {}", &after[2..])));
            n += 1;
        }
        // `map_selected_lines_in` walks the lines from the last to the first (so
        // the document never has to shift the text it has not written yet), which
        // is why the pre-computed list is reversed to match it.
        let mut it = out.into_iter().rev();
        self.map_selected_lines_in(first, last, |_| it.next().flatten());
    }

    fn map_selected_lines_in(&mut self, l0: usize, l1: usize, mut f: impl FnMut(&str) -> Option<String>) {
        let t = self.tab_mut();
        let caret_line = t.doc.line_of_char(t.caret);
        let caret_col = t.caret - t.doc.line_start(caret_line);
        t.doc.begin_compound();
        for line in (l0..=l1).rev() {
            let start = t.doc.line_start(line);
            let text = t.doc.line_text(line);
            if let Some(new) = f(&text) {
                let old_len = text.chars().count();
                t.doc.replace_range(start, start + old_len, &new);
            }
        }
        t.doc.end_compound();
        let line_start = t.doc.line_start(caret_line);
        t.caret = line_start + caret_col.min(t.doc.line_len(caret_line));
        t.anchor = t.caret;
        t.layout = Layout::empty();
    }

    // ---------- grip drags ----------

    /// Start dragging the grip behind `hit`, if it has one. Returns true when the
    /// press was consumed, so a click can still open the menu on release.
    pub fn drag_begin(&mut self, hit: &Hit, pos: (f32, f32)) -> bool {
        let (kind, first, from) = match hit {
            Hit::RowGrip(first, row) => (DragKind::TableRow, *first, *row),
            Hit::ColGrip(first, col) => (DragKind::TableCol, *first, *col),
            Hit::OutlineGrip(i) => (DragKind::Outline, 0, *i),
            _ => return false,
        };
        self.drag = Some(DragState { kind, first, from, to: from, anchor: pos, moved: false });
        true
    }

    /// Follow the pointer. Returns true while a drag is running.
    pub fn drag_update(&mut self, pos: (f32, f32)) -> bool {
        let Some(mut d) = self.drag else { return false };
        if !d.moved {
            let dx = pos.0 - d.anchor.0;
            let dy = pos.1 - d.anchor.1;
            if dx * dx + dy * dy < 16.0 {
                return true;
            }
            d.moved = true;
        }
        d.to = match d.kind {
            DragKind::TableRow => self.row_drop_target(d.first, pos.1).unwrap_or(d.to),
            DragKind::TableCol => self.col_drop_target(d.first, pos.0).unwrap_or(d.to),
            DragKind::Outline => self.outline_drop_target(pos.1).unwrap_or(d.to),
        };
        self.drag = Some(d);
        true
    }

    /// Finish a drag. A drag that never moved falls through to the regular click
    /// behaviour (which opens the row / column menu).
    pub fn drag_end(&mut self, r: &mut crate::gpu::Renderer) -> bool {
        let Some(d) = self.drag.take() else { return false };
        if !d.moved {
            return false;
        }
        match d.kind {
            DragKind::TableRow => {
                let lines = self.table_row_lines(d.first);
                if let Some(&line) = lines.get(d.from) {
                    self.caret_to_table_line(line);
                }
                self.run_table_action(TableAction::MoveRowTo(d.to));
                self.after_edit(r);
            }
            DragKind::TableCol => {
                self.caret_to_table_line(d.first);
                self.caret_to_column(d.first, d.from);
                self.run_table_action(TableAction::MoveColTo(d.to));
                self.after_edit(r);
            }
            DragKind::Outline => {
                self.move_outline_section(d.from, d.to);
                self.after_edit(r);
            }
        }
        true
    }

    /// Document lines of the table that starts at `first` (separators excluded).
    fn table_row_lines(&self, first: usize) -> Vec<usize> {
        let layout = &self.tab().layout;
        let mut out = Vec::new();
        let mut i = first;
        while let Some(line) = layout.lines.get(i) {
            if line.block() != Block::Table {
                break;
            }
            if !line.info.table_sep {
                out.push(i);
            }
            i += 1;
        }
        out
    }

    /// Which row index the pointer is over, using the table's own geometry.
    fn row_drop_target(&self, first: usize, y: f32) -> Option<usize> {
        let rows = self.table_row_lines(first);
        if rows.is_empty() {
            return None;
        }
        let layout = &self.tab().layout;
        let top = self.text_rect.y - self.tab().scroll;
        let y = y - top;
        let mut best = 0usize;
        let mut best_d = f32::MAX;
        for (idx, &line) in rows.iter().enumerate() {
            let Some(l) = layout.lines.get(line) else { continue };
            let mid = l.top + l.height * 0.5;
            let d = (y - mid).abs();
            if d < best_d {
                best_d = d;
                best = idx;
            }
        }
        Some(best)
    }

    fn col_drop_target(&self, first: usize, x: f32) -> Option<usize> {
        let layout = &self.tab().layout;
        let line = layout.lines.get(first)?;
        let cols = line.table_cols.clone();
        if cols.is_empty() {
            return None;
        }
        let left = self.text_rect.x - self.tab().scroll_x;
        let x = x - left;
        let mut best = 0usize;
        let mut best_d = f32::MAX;
        for (idx, &cx) in cols.iter().enumerate() {
            let d = (x - cx).abs();
            if d < best_d {
                best_d = d;
                best = idx;
            }
        }
        Some(best)
    }

    fn outline_drop_target(&self, y: f32) -> Option<usize> {
        let sections = self.outline_sections();
        if sections.is_empty() {
            return None;
        }
        let top = self.outline_rect.y + crate::ui::OUTLINE_HEAD_H;
        let row_h = crate::ui::OUTLINE_ROW_H;
        let idx = ((y - top) / row_h).floor();
        Some((idx.max(0.0) as usize).min(sections.len() - 1))
    }

    /// (first line, end line exclusive, level, text) for every heading.
    pub fn outline_sections(&self) -> Vec<(usize, usize, u8, String)> {
        let layout = &self.tab().layout;
        let n = layout.lines.len();
        let mut out = Vec::new();
        for (i, (line, level, text)) in layout.headings.iter().enumerate() {
            let mut end = n;
            for (line2, level2, _) in layout.headings.iter().skip(i + 1) {
                if *level2 <= *level {
                    end = *line2;
                    break;
                }
            }
            out.push((*line, end, *level, text.clone()));
        }
        out
    }

    /// Headings read straight from the text, so a move works even when the
    /// layout is stale (it is invalidated by the previous move).
    fn doc_sections(&self) -> Vec<(usize, usize, u8, String)> {
        let doc = &self.tab().doc;
        let n = doc.len_lines();
        let mut heads: Vec<(usize, u8, String)> = Vec::new();
        for line in 0..n {
            let text = doc.line_text(line);
            let trimmed = text.trim_start();
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            if level == 0 || level > 6 {
                continue;
            }
            let rest = trimmed[level..].trim();
            if !text.trim_start().starts_with(&"#".repeat(level)) || rest.is_empty() {
                continue;
            }
            heads.push((line, level as u8, rest.trim_end_matches('#').trim().to_string()));
        }
        let mut out = Vec::new();
        for (i, (line, level, text)) in heads.iter().enumerate() {
            let mut end = n;
            for (line2, level2, _) in heads.iter().skip(i + 1) {
                if level2 <= level {
                    end = *line2;
                    break;
                }
            }
            out.push((*line, end, *level, text.clone()));
        }
        out
    }

    /// Move a whole outline section (its heading plus everything under it).
    ///
    /// The move works on whole lines so nothing is lost at the seams: the section
    /// takes its body with it, trailing blank lines stay behind as the paragraph
    /// gap, and exactly one blank line is kept between blocks afterwards.
    pub fn move_outline_section(&mut self, from: usize, to: usize) {
        let sections = self.doc_sections();
        if from >= sections.len() || to >= sections.len() || from == to {
            return;
        }
        let (start_line, end_line, _, _) = sections[from];
        let dest_line = if to > from {
            // Moving down: land after the target section (its end is exclusive).
            sections[to].1
        } else {
            sections[to].0
        };
        let t = self.tab_mut();
        let text = t.doc.text();
        let had_trailing_nl = text.ends_with('\n');
        let mut lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
        if had_trailing_nl && lines.len() > 1 {
            lines.pop();
        }
        let end_line = end_line.min(lines.len());
        if start_line >= end_line {
            return;
        }
        // Does this document separate blocks with blank lines? If it does, keep a
        // blank line around the moved block; if it does not (a tight list of
        // headings), adding one would be a surprise.
        let uses_gaps = lines.iter().any(|l| l.trim().is_empty());
        let mut block: Vec<String> = lines[start_line..end_line].to_vec();
        while block.first().map(|l| l.trim().is_empty()).unwrap_or(false) {
            block.remove(0);
        }
        while block.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
            block.pop();
        }
        if block.is_empty() {
            return;
        }
        let moved = block.len();
        lines.drain(start_line..end_line);
        let dest = if dest_line > end_line {
            dest_line - (end_line - start_line)
        } else {
            dest_line
        }
        .min(lines.len());
        let mut insert: Vec<String> = Vec::with_capacity(moved + 2);
        let gap_before = uses_gaps
            && dest > 0
            && !lines[dest - 1].trim().is_empty();
        if gap_before {
            insert.push(String::new());
        }
        let block_at = dest + insert.len();
        insert.extend(block);
        if uses_gaps && dest < lines.len() && !lines[dest].trim().is_empty() {
            insert.push(String::new());
        }
        lines.splice(dest..dest, insert);
        if had_trailing_nl {
            // A section that used to sit last may have left its gap behind at EOF.
            while lines.len() > 1 && lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
                lines.pop();
            }
        }
        let mut new_text = lines.join("\n");
        if had_trailing_nl {
            new_text.push('\n');
        }
        t.doc.begin_compound();
        let all = t.doc.len_chars();
        t.doc.replace_range(0, all, &new_text);
        t.doc.end_compound();
        let caret = t
            .doc
            .line_start(block_at.min(t.doc.len_lines().saturating_sub(1)));
        t.caret = caret;
        t.anchor = caret;
        t.layout = Layout::empty();
    }

    /// The minimap's geometry for the current frame: the whole document fitted to
    /// the column, so both the painter and the input layer ask here and cannot
    /// disagree about where a line is.
    /// The map's geometry alone.
    ///
    /// [`App::minimap_geometry`] also returns the per-line virtual-tops table, which is a
    /// pass over the whole document; the press and the drag need only the geometry, and
    /// they run on every mouse move.
    pub fn minimap_geom_only(&self) -> crate::ui::MinimapGeom {
        let tab = self.tab();
        crate::ui::minimap_geom_at(&tab.layout, self.minimap_rect, tab.scroll, self.text_rect.h)
    }

    pub fn minimap_geometry(&self) -> (crate::ui::MinimapGeom, Vec<f32>) {
        let tab = self.tab();
        let layout = &tab.layout;
        let rect = self.minimap_rect;
        let vtop = crate::ui::minimap_virtual_tops(layout);
        // The map has to know where the page is: a long document cannot be fitted
        // into the column without turning the bars into a smear, so it slides.
        let geom = crate::ui::minimap_geom_at(layout, rect, tab.scroll, self.text_rect.h);
        (geom, vtop)
    }

    /// True while the "Write Markdown." placeholder stands in for the document.
    pub fn shows_empty_state(&self) -> bool {
        self.tab().doc.len_chars() == 0 && self.mode == ViewMode::Live
    }

    /// Whether this frame paints the caret. The empty state draws a placeholder
    /// on the caret's own line, so the two never appear together: a caret
    /// blinking through the word "Write" is what round 8 reported.
    pub fn caret_is_painted(&self) -> bool {
        self.focus == Focus::Editor && !self.shows_empty_state()
    }

    /// Put the caret at the start of column `col` on `line`.
    pub fn caret_to_column(&mut self, line: usize, col: usize) {
        let t = self.tab_mut();
        let line = line.min(t.doc.len_lines().saturating_sub(1));
        let cells = t
            .layout
            .lines
            .get(line)
            .map(|l| l.info.cells.clone())
            .unwrap_or_default();
        let start = t.doc.line_start(line);
        let pos = cells.get(col).map(|(a, _)| *a).unwrap_or(0);
        t.caret = (start + pos).min(t.doc.len_chars());
        t.anchor = t.caret;
        t.goal_x = None;
    }

    // ---------- menus ----------

    pub fn close_menu(&mut self) {
        self.menu = None;
    }

    pub fn open_menu(&mut self, kind: MenuKind, anchor: Rect) {
        let entries = self.menu_entries(kind);
        let x = anchor.x;
        let y = anchor.bottom() + 6.0;
        let (split, heads) = match kind {
            // Text on the left, background on the right: the tool used to be one
            // long column of fifteen rows in which half of them were backgrounds.
            MenuKind::Color => (COLOUR_MENU_COLUMN, ("TEXT", "BACKGROUND")),
            _ => (0, ("", "")),
        };
        self.menu = Some(Menu { kind, x, y, entries, split, heads });
    }

    /// Right-click: editing actions, plus the table's own commands when the
    /// pointer (or the caret) is inside one.
    pub fn open_context_menu(&mut self, pos: (f32, f32)) {
        let has_selection = self.tab().has_selection();
        let mut entries = vec![
            entry("Cut", "Ctrl+X", MenuAction::Edit(EditAction::Cut), false),
            entry("Copy", "Ctrl+C", MenuAction::Edit(EditAction::Copy), false),
            entry("Paste", "Ctrl+V", MenuAction::Edit(EditAction::Paste), false),
            entry("Select all", "Ctrl+A", MenuAction::Edit(EditAction::SelectAll), false),
            divider_entry("Undo", "Ctrl+Z", MenuAction::Edit(EditAction::Undo), false),
        ];
        if let Some((first, row, col)) = self.table_position() {
            let _ = (row, col);
            entries.push(divider_entry("Insert row above", "", MenuAction::Table(TableAction::InsertRowAbove), false));
            entries.push(entry("Insert row below", "", MenuAction::Table(TableAction::InsertRowBelow), false));
            entries.push(entry("Insert column left", "", MenuAction::Table(TableAction::InsertColLeft), false));
            entries.push(entry("Insert column right", "", MenuAction::Table(TableAction::InsertColRight), false));
            entries.push(divider_entry("Move row up", "", MenuAction::Table(TableAction::MoveRowUp), false));
            entries.push(entry("Move row down", "", MenuAction::Table(TableAction::MoveRowDown), false));
            entries.push(entry("Move column left", "", MenuAction::Table(TableAction::MoveColLeft), false));
            entries.push(entry("Move column right", "", MenuAction::Table(TableAction::MoveColRight), false));
            entries.push(divider_entry("Duplicate row", "", MenuAction::Table(TableAction::DuplicateRow), false));
            entries.push(entry("Delete row", "", MenuAction::Table(TableAction::DeleteRow), false));
            entries.push(entry("Delete column", "", MenuAction::Table(TableAction::DeleteCol), false));
            let _ = first;
        }
        if has_selection {
            entries.push(divider_entry("Clear formatting", "", MenuAction::Toolbar(ToolbarAction::ClearFormat), false));
            entries.push(entry("Quote", "", MenuAction::Toolbar(ToolbarAction::Quote), false));
            entries.push(entry("Colour…", "", MenuAction::Menu(MenuKind::Color), false));
        }
        entries.push(divider_entry("Save", "Ctrl+S", MenuAction::Edit(EditAction::Save), false));
        entries.push(entry("Save as…", "Ctrl+Shift+S", MenuAction::Edit(EditAction::SaveAs), false));
        entries.push(MenuEntry {
            checked: self.settings.autoclose,
            ..entry(
                "Auto-close brackets",
                "Alt+B",
                MenuAction::Toolbar(ToolbarAction::ToggleAutoclose),
                false,
            )
        });
        entries.push(MenuEntry {
            checked: self.settings.wrap,
            ..entry(
                "Soft wrap",
                "Alt+W",
                MenuAction::Toolbar(ToolbarAction::ToggleWrap),
                false,
            )
        });
        entries.push(entry("Reload file", "", MenuAction::Edit(EditAction::Reload), false));
        entries.push(entry("Document tools…", "Ctrl+Shift+T", MenuAction::Menu(MenuKind::Tools), false));
        self.menu = Some(Menu { kind: MenuKind::Context, x: pos.0, y: pos.1, entries, split: 0, heads: ("", "") });
    }

    fn menu_entries(&self, kind: MenuKind) -> Vec<MenuEntry> {
        let current_align = self
            .tab()
            .layout
            .lines
            .get(self.tab().layout.line_of_char(self.tab().caret))
            .map(|l| l.info.align)
            .unwrap_or(0);
        let in_table = self.table_position().is_some();
        match kind {
            MenuKind::Heading => (1..=6)
                .map(|level| MenuEntry {
                    label: format!("Heading {level}"),
                    hint: format!("Ctrl+{level}"),
                    action: MenuAction::Heading(level),
                    checked: false,
                    swatch: None,
                    divider: false,
                })
                .collect(),
            MenuKind::SupSub => vec![
                entry("Superscript", "x^2^", MenuAction::Toolbar(ToolbarAction::Sup), false),
                entry("Subscript", "x~2~", MenuAction::Toolbar(ToolbarAction::Sub), false),
            ],
            MenuKind::Align => vec![
                MenuEntry { checked: current_align == 0, ..entry("Left", "", MenuAction::Align(0), false) },
                MenuEntry { checked: current_align == 1, ..entry("Centre", "", MenuAction::Align(1), false) },
                MenuEntry { checked: current_align == 2, ..entry("Right", "", MenuAction::Align(2), false) },
                MenuEntry { checked: current_align == 3, ..entry("Justify", "", MenuAction::Align(3), false) },
            ],
            MenuKind::Color => {
                const SWATCHES: [(u32, &str); 6] = [
                    (0xe05c5c, "Red"),
                    (0xe0a83c, "Amber"),
                    (0x5fae6b, "Green"),
                    (0x4f8fd0, "Blue"),
                    (0x8f7ad0, "Violet"),
                    (0x9a9a9a, "Grey"),
                ];
                // Two equal columns: every row is "this colour, that colour", text
                // on the left and background on the right.
                let mut out = Vec::new();
                for (value, name) in SWATCHES {
                    out.push(MenuEntry {
                        swatch: Some(crate::layout::rgb_color(value)),
                        ..entry(name, "", MenuAction::Fg(Some(value)), false)
                    });
                }
                out.push(entry("Custom…", "", MenuAction::ColourPicker(false), false));
                out.push(divider_entry("No colour", "", MenuAction::Fg(None), false));
                for (value, name) in SWATCHES {
                    out.push(MenuEntry {
                        swatch: Some(crate::layout::rgb_color(value)),
                        ..entry(name, "", MenuAction::Bg(Some(value)), false)
                    });
                }
                out.push(entry("Custom…", "", MenuAction::ColourPicker(true), false));
                out.push(divider_entry("No colour", "", MenuAction::Bg(None), false));
                out
            }
            MenuKind::Table => {
                let mut out = Vec::new();
                if in_table {
                    out.push(entry("Insert row above", "", MenuAction::Table(TableAction::InsertRowAbove), false));
                    out.push(entry("Insert row below", "", MenuAction::Table(TableAction::InsertRowBelow), false));
                    out.push(entry("Insert column left", "", MenuAction::Table(TableAction::InsertColLeft), false));
                    out.push(entry("Insert column right", "", MenuAction::Table(TableAction::InsertColRight), false));
                    out.push(divider_entry("Duplicate row", "", MenuAction::Table(TableAction::DuplicateRow), false));
                    out.push(entry("Delete row", "", MenuAction::Table(TableAction::DeleteRow), false));
                    out.push(entry("Delete column", "", MenuAction::Table(TableAction::DeleteCol), false));
                }
                out.push(divider_entry("New table", "", MenuAction::Toolbar(ToolbarAction::Table), false));
                out.push(entry("List → table", "", MenuAction::Tools(ToolsAction::ListToTable), false));
                out.push(entry("Table → list", "", MenuAction::Tools(ToolsAction::TableToList), false));
                out
            }
            MenuKind::Tools => vec![
                entry("Insert empty rows", "", MenuAction::Tools(ToolsAction::InsertEmptyRows), false),
                entry("Remove empty rows", "", MenuAction::Tools(ToolsAction::RemoveEmptyRows), false),
                divider_entry("Split rows", "", MenuAction::Tools(ToolsAction::SplitRows), false),
                entry("Merge rows", "", MenuAction::Tools(ToolsAction::MergeRows), false),
                entry("Remove duplicate rows", "", MenuAction::Tools(ToolsAction::DedupeRows), false),
                divider_entry("Compress extra spaces", "", MenuAction::Tools(ToolsAction::CompressSpaces), false),
                entry("Remove all spaces", "", MenuAction::Tools(ToolsAction::RemoveSpaces), false),
                divider_entry("List → table", "", MenuAction::Tools(ToolsAction::ListToTable), false),
                entry("Table → list", "", MenuAction::Tools(ToolsAction::TableToList), false),
                divider_entry("About Bare MD", "F1", MenuAction::About, false),
            ],
            MenuKind::Context => Vec::new(),
        }
    }

    pub fn run_menu_action(&mut self, action: MenuAction, r: &mut crate::gpu::Renderer) {
        self.menu = None;
        match action {
            MenuAction::Menu(kind) => {
                // A submenu opens where the current menu stood.
                let anchor = Rect::new(self.mouse.0, self.mouse.1 - 24.0, 1.0, 1.0);
                self.open_menu(kind, anchor);
                return;
            }
            MenuAction::Toolbar(a) => {
                self.run_toolbar_action(a, r);
            }
            MenuAction::Edit(a) => self.run_edit_action(a, r),
            MenuAction::Tools(a) => {
                self.run_tools_action(a);
                self.after_edit(r);
            }
            MenuAction::Table(a) => {
                self.run_table_action(a);
                self.after_edit(r);
            }
            MenuAction::Heading(level) => {
                self.toggle_heading(level);
                self.after_edit(r);
            }
            MenuAction::Align(a) => {
                self.set_alignment(a);
                self.after_edit(r);
            }
            MenuAction::Fg(c) => {
                self.set_text_color(false, c);
                self.after_edit(r);
            }
            MenuAction::Bg(c) => {
                self.set_text_color(true, c);
                self.after_edit(r);
            }
            MenuAction::ColourPicker(bg) => {
                self.dialog = Some(Dialog::Colour(ColourPicker::new(bg, self.last_colour)));
                self.focus = Focus::Dialog;
            }
            MenuAction::About => self.open_about(),
        }
    }

    pub fn run_edit_action(&mut self, action: EditAction, r: &mut crate::gpu::Renderer) {
        match action {
            EditAction::Cut => {
                self.cut();
                self.after_edit(r);
            }
            EditAction::Copy => self.copy(),
            EditAction::Paste => {
                self.paste();
                self.after_edit(r);
            }
            EditAction::SelectAll => {
                self.tab_mut().anchor = 0;
                self.tab_mut().caret = self.tab().doc.len_chars();
                self.after_navigation(r);
            }
            EditAction::Undo => {
                self.undo();
                self.after_edit(r);
            }
            EditAction::Redo => {
                self.redo();
                self.after_edit(r);
            }
            EditAction::Reload => {
                self.reload_file();
                self.after_navigation(r);
            }
            EditAction::Save => {
                self.save();
            }
            EditAction::SaveAs => self.open_picker(PickerKind::SaveAs),
            EditAction::Find => self.open_find(true),
        }
    }

    pub fn run_toolbar_action(&mut self, action: ToolbarAction, r: &mut crate::gpu::Renderer) {
        // The input layer owns the implementation; this is the entry point used
        // by menus and tests.
        self.dispatch_toolbar(action, r);
    }

    /// Attach a file: insert a link to it at the caret.
    pub fn attach_file(&mut self, path: &Path) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        // A picture goes in as a picture (on its own line, so it renders), any
        // other file as a link with its name as the label.
        let is_image = matches!(
            path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref(),
            Some("png") | Some("jpg") | Some("jpeg") | Some("jxl")
        );
        if is_image {
            // Prefer a path relative to the document, so the note stays portable.
            let base = self.tab().doc.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf()));
            let shown = match base.and_then(|b| path.strip_prefix(&b).ok().map(|p| p.to_path_buf())) {
                Some(rel) => rel.to_string_lossy().replace('\\', "/"),
                None => path.display().to_string(),
            };
            let (line, _) = self.tab().doc.line_and_col(self.tab().caret);
            let at_line_start = self.tab().doc.line_text(line).trim().is_empty();
            let snippet = if at_line_start {
                format!("![{name}]({shown})")
            } else {
                format!("\n\n![{name}]({shown})\n")
            };
            let t = self.tab_mut();
            let caret = t.caret;
            let new = t.doc.insert_atomic(caret, &snippet);
            t.caret = new;
            t.anchor = new;
            t.layout = Layout::empty();
            return;
        }
        // Relative to the document when it can be, exactly like a picture: an
        // absolute path in a link breaks the moment the note is moved, and it made
        // a link to a file in the same folder look like a different file to the
        // reader. `open_link` resolves it next to the document.
        let base = self.tab().doc.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf()));
        let shown = match base.and_then(|b| path.strip_prefix(&b).ok().map(|p| p.to_path_buf())) {
            Some(rel) => rel.to_string_lossy().replace('\\', "/"),
            None => path.display().to_string(),
        };
        let link = format!("[{name}]({shown})");
        let t = self.tab_mut();
        let caret = t.caret;
        let new = t.doc.insert_atomic(caret, &link);
        t.caret = new;
        t.anchor = new;
        t.layout = Layout::empty();
    }
}

fn entry(label: &str, hint: &str, action: MenuAction, checked: bool) -> MenuEntry {
    MenuEntry {
        label: label.to_string(),
        hint: hint.to_string(),
        action,
        checked,
        swatch: None,
        divider: false,
    }
}

fn divider_entry(label: &str, hint: &str, action: MenuAction, checked: bool) -> MenuEntry {
    MenuEntry { divider: true, ..entry(label, hint, action, checked) }
}

// ------------------------------------------------------- text transformations

/// Remove inline emphasis / strike / highlight / underline / sub / sup markers
/// from a block of text, leaving links and code alone.
pub fn strip_inline_markers(text: &str) -> String {
    // Line by line, because the colour and alignment wrappers are per line.
    let mut out_lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        out_lines.push(strip_line_markers(line));
    }
    out_lines.join("\n")
}

fn strip_line_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '[' => {
                // `[#rrggbb]`, `[bg:#rrggbb]` and `[/]` are formatting, not text.
                let rest: String = chars.clone().collect();
                let skip = if let Some(n) = color_tag_len(&rest) {
                    Some(n)
                } else {
                    None
                };
                match skip {
                    Some(n) => {
                        for _ in 0..n {
                            chars.next();
                        }
                    }
                    None => out.push(c),
                }
            }
            '*' | '_' | '~' | '=' | '+' | '^' => {
                // Drop marker runs, but keep a lone `~` or `^` that is not closed.
                let rest: String = chars.clone().collect();
                let doubled = matches!(
                    (c, chars.peek()),
                    ('*', Some('*')) | ('_', Some('_')) | ('~', Some('~')) | ('=', Some('=')) | ('+', Some('+'))
                );
                let marker_len = if doubled { 2 } else { 1 };
                let closes = if doubled {
                    let pair: String = std::iter::repeat(c).take(2).collect();
                    rest.contains(&pair)
                } else {
                    rest.contains(c)
                };
                if closes {
                    for _ in 1..marker_len {
                        chars.next();
                    }
                } else {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    // The alignment tag lives at the end of the line.
    strip_align_tag(&out).trim_end().to_string()
}

/// A `[#rrggbb]…[/]` (or `[bg:#rrggbb]…[/]`) span inside one line.
#[derive(Clone, Copy, Debug)]
pub struct ColourSpan {
    pub bg: bool,
    /// Char offsets inside the line: the opening tag, the body, the closing tag.
    pub open: usize,
    pub body_start: usize,
    pub body_end: usize,
    pub close: usize,
}

/// Every colour span of a line, outermost first.
pub fn colour_spans(chars: &[char]) -> Vec<ColourSpan> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '[' {
            let rest: String = chars[i + 1..].iter().collect();
            if let Some(len) = color_tag_len(&rest) {
                let tag: String = chars[i + 1..i + 1 + len].iter().collect();
                // `[/]` cannot open a span.
                if !tag.starts_with('/') {
                    let bg = tag.starts_with("bg:#");
                    let body_start = i + 1 + len;
                    // The matching `[/]`, counting nested colour tags.
                    let mut depth = 1usize;
                    let mut j = body_start;
                    let mut close = None;
                    while j < chars.len() {
                        if chars[j] == '[' {
                            let r: String = chars[j + 1..].iter().collect();
                            if let Some(l2) = color_tag_len(&r) {
                                let t2: String = chars[j + 1..j + 1 + l2].iter().collect();
                                if t2.starts_with('/') {
                                    depth -= 1;
                                    if depth == 0 {
                                        close = Some(j);
                                        break;
                                    }
                                } else {
                                    depth += 1;
                                }
                                j += 1 + l2;
                                continue;
                            }
                        }
                        j += 1;
                    }
                    if let Some(close) = close {
                        out.push(ColourSpan { bg, open: i, body_start, body_end: close, close });
                        // Keep scanning *inside* the span: a background colour is
                        // nested in a text colour, and skipping the body made the
                        // inner tag invisible to "remove background".
                    }
                }
            }
        }
        i += 1;
    }
    out
}

/// Remove the tags of one kind from `text`, leaving the styled text behind.
pub fn strip_colour_tags(text: &str, bg: bool) -> String {
    let chars: Vec<char> = text.chars().collect();
    let spans = colour_spans(&chars);
    if spans.is_empty() {
        return text.to_string();
    }
    let mut drop = vec![false; chars.len()];
    for sp in spans.iter().filter(|sp| sp.bg == bg) {
        for d in drop.iter_mut().take(sp.body_start).skip(sp.open) {
            *d = true;
        }
        // The closing tag `[/]` is three chars.
        for k in sp.close..(sp.close + 3).min(chars.len()) {
            drop[k] = true;
        }
    }
    chars.iter().enumerate().filter(|(i, _)| !drop[*i]).map(|(_, c)| *c).collect()
}

/// First and last non-space char of a line.
fn stripped_line_bounds(chars: &[char]) -> (usize, usize) {
    let start = chars.iter().position(|c| !c.is_whitespace()).unwrap_or(0);
    let end = chars.iter().rposition(|c| !c.is_whitespace()).map(|i| i + 1).unwrap_or(0);
    (start, end)
}

/// Length of a colour tag that *follows* a `[`, if there is one.
fn color_tag_len(rest: &str) -> Option<usize> {
    // `[/]`
    if let Some(r) = rest.strip_prefix('/') {
        if r.starts_with(']') {
            return Some(2);
        }
    }
    let mut i = 0;
    let bytes: Vec<char> = rest.chars().collect();
    if bytes.starts_with(&['b', 'g', ':']) {
        i = 3;
    }
    if bytes.get(i) != Some(&'#') {
        return None;
    }
    i += 1;
    let mut digits = 0;
    while let Some(&c) = bytes.get(i) {
        if c.is_ascii_hexdigit() {
            digits += 1;
            i += 1;
        } else {
            break;
        }
    }
    if digits == 0 || bytes.get(i) != Some(&']') {
        return None;
    }
    Some(i + 1)
}

/// Remove a trailing `{center}` / `{right}` / `{left}` / `{justify}` tag.
pub fn strip_align_tag(text: &str) -> String {
    let trimmed = text.trim_end();
    if let Some(open) = trimmed.rfind(" {") {
        let tag = &trimmed[open + 2..];
        if let Some(word) = tag.strip_suffix('}') {
            let w = word.trim().to_lowercase();
            if matches!(w.as_str(), "center" | "centre" | "right" | "left" | "justify") {
                return trimmed[..open].to_string();
            }
        }
    }
    text.trim_end().to_string()
}

fn split_indent(text: &str) -> (String, String) {
    let indent: String = text.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let rest: String = text.chars().skip(indent.chars().count()).collect();
    (indent, rest)
}

pub fn is_quoted(text: &str) -> bool {
    let (_, rest) = split_indent(text);
    rest.starts_with('>')
}

/// `- [ ] a` → `> - [ ] a` (the marker underneath is untouched).
pub fn quote_line(text: &str) -> String {
    let (indent, rest) = split_indent(text);
    format!("{indent}> {rest}")
}

/// `> - [ ] a` → `- [ ] a`, one level at a time.
pub fn unquote_line(text: &str) -> String {
    let (indent, rest) = split_indent(text);
    let stripped = rest.strip_prefix("> ").or_else(|| rest.strip_prefix('>')).unwrap_or(&rest);
    format!("{indent}{stripped}")
}

// ------------------------------------------------------------ table machinery

/// A table read out of the document: where it lives and what it contains.
/// A pointer drag on a grip (table row, table column, outline section).
#[derive(Clone, Copy, Debug)]
pub struct DragState {
    pub kind: DragKind,
    /// First document line of the table (unused for outline drags).
    pub first: usize,
    /// Where the drag started.
    pub from: usize,
    /// Where it would land right now.
    pub to: usize,
    pub anchor: (f32, f32),
    /// Set once the pointer has actually moved, so a plain click still opens a menu.
    pub moved: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DragKind {
    TableRow,
    TableCol,
    Outline,
}

pub struct TableData {
    pub first: usize,
    pub last: usize,
    /// Row index of the caret (into `rows`, separators excluded).
    pub row: usize,
    pub col: usize,
    /// Cell contents, separators removed.
    pub rows: Vec<Vec<String>>,
    /// True when the table has a `| --- |` separator after the first row.
    pub header: bool,
}

/// Char offset just inside cell `col` of a `| a | b |` line (after the pipe and
/// its single padding space), so the caret lands where typing belongs.
fn cell_caret_offset(line: &str, col: usize) -> usize {
    let mut pipes = 0usize;
    for (i, ch) in line.chars().enumerate() {
        if ch == '|' {
            pipes += 1;
            if pipes == col + 1 {
                return (i + 2).min(line.chars().count());
            }
        }
    }
    line.chars().count()
}

/// Split a `| a | b |` line into trimmed cells.
pub fn split_cells(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut cells = Vec::new();
    let mut current = String::new();
    let start = chars.iter().position(|c| *c == '|').unwrap_or(0);
    for &c in chars.iter().skip(start + 1) {
        if c == '|' {
            cells.push(current.trim().to_string());
            current.clear();
        } else {
            current.push(c);
        }
    }
    let tail = current.trim().to_string();
    // A trailing pipe leaves an empty tail: keep it only when the line does not
    // end with a pipe.
    if !tail.is_empty() {
        cells.push(tail);
    }
    cells
}

fn is_separator_row(text: &str) -> bool {
    let cells = split_cells(text);
    !cells.is_empty()
        && cells.iter().all(|c| {
            let t = c.trim().trim_start_matches(':').trim_end_matches(':');
            !t.is_empty() && t.chars().all(|ch| ch == '-')
        })
}

fn format_table(rows: &[Vec<String>], header: bool) -> String {
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
    let mut widths = vec![3usize; cols];
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let mut out = String::new();
    for (ri, row) in rows.iter().enumerate() {
        out.push('|');
        for (ci, w) in widths.iter().enumerate() {
            let cell = row.get(ci).map(|s| s.as_str()).unwrap_or("");
            out.push(' ');
            out.push_str(cell);
            for _ in cell.chars().count()..*w {
                out.push(' ');
            }
            out.push_str(" |");
        }
        out.push('\n');
        if ri == 0 && header {
            out.push('|');
            for w in &widths {
                out.push(' ');
                for _ in 0..*w {
                    out.push('-');
                }
                out.push_str(" |");
            }
            out.push('\n');
        }
    }
    out
}

impl App {
    /// Read the table around the caret (or around line `line` when given).
    pub fn table_at(&self, around: Option<usize>) -> Option<TableData> {
        let t = self.tab();
        let is_table_line = |i: usize| match t.layout.lines.get(i) {
            Some(l) => l.info.block == Block::Table,
            None => t.doc.line_text(i).trim_start().starts_with('|'),
        };
        let caret_line = t.doc.line_of_char(t.caret);
        let line = match around {
            Some(l) => l,
            None => {
                // A selection that starts inside the table counts: selecting the
                // whole table must act on the table, not on the empty line below
                // it where the caret happens to be.
                let (sel_a, _) = t.sel();
                let sel_line = t.doc.line_of_char(sel_a.min(t.doc.len_chars()));
                if is_table_line(caret_line) {
                    caret_line
                } else if t.has_selection() && is_table_line(sel_line) {
                    sel_line
                } else {
                    caret_line
                }
            }
        };
        // The layout may be stale (a table edit invalidates it, and the next
        // command often arrives before the next frame), so fall back to reading
        // the source: a table row is a line that starts with a pipe.
        let is_table = |i: usize| match t.layout.lines.get(i) {
            Some(l) => l.info.block == Block::Table,
            None => t
                .doc
                .line_text(i)
                .trim_start()
                .starts_with('|'),
        };
        if !is_table(line) {
            return None;
        }
        let mut first = line;
        while first > 0 && is_table(first - 1) {
            first -= 1;
        }
        let mut last = line;
        while last + 1 < t.doc.len_lines() && is_table(last + 1) {
            last += 1;
        }
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut header = false;
        let mut caret_row = 0usize;
        for i in first..=last {
            let text = t.doc.line_text(i);
            if is_separator_row(&text) {
                if rows.len() == 1 {
                    header = true;
                }
                continue;
            }
            if i <= line {
                caret_row = rows.len();
            }
            rows.push(split_cells(&text));
        }
        if rows.is_empty() {
            return None;
        }
        let col = {
            let (l, c) = t.doc.line_and_col(t.caret);
            if l >= first && l <= last {
                // Which cell holds the caret? Count the pipes before it.
                let text = t.doc.line_text(l);
                text.chars()
                    .take(c)
                    .filter(|ch| *ch == '|')
                    .count()
                    .saturating_sub(1)
                    .min(rows[caret_row.min(rows.len() - 1)].len().saturating_sub(1))
            } else {
                0
            }
        };
        Some(TableData { first, last, row: caret_row.min(rows.len() - 1), col, rows, header })
    }

    /// Put the caret on a given line so table commands act on the right table.
    pub fn caret_to_table_line(&mut self, line: usize) {
        let t = self.tab_mut();
        let line = line.min(t.doc.len_lines().saturating_sub(1));
        let pos = t.doc.line_start(line);
        t.anchor = pos;
        t.caret = pos;
        t.goal_x = None;
    }

    /// Put the caret in the last cell of `line` (just before the closing pipe).
    pub fn caret_to_last_cell(&mut self, line: usize) {
        let t = self.tab_mut();
        let line = line.min(t.doc.len_lines().saturating_sub(1));
        let start = t.doc.line_start(line);
        let len = t.doc.line_len(line);
        let text = t.doc.line_text(line);
        let trimmed = text.trim_end();
        let keep = if trimmed.ends_with('|') { trimmed.len() - 1 } else { trimmed.len() };
        t.anchor = start + keep.min(len);
        t.caret = start + keep.min(len);
        t.goal_x = None;
    }

    /// The row menu that a grip opens. `row` counts the table's rows (separators
    /// excluded), which is exactly what a `Hit::RowGrip` carries.
    pub fn open_row_menu(&mut self, first: usize, row: usize) {
        let lines = self.table_row_lines(first);
        if let Some(&line) = lines.get(row) {
            self.caret_to_table_line(line);
        }
        let entries = vec![
            entry("Move row up", "", MenuAction::Table(TableAction::MoveRowUp), false),
            entry("Move row down", "", MenuAction::Table(TableAction::MoveRowDown), false),
            divider_entry("Insert row above", "", MenuAction::Table(TableAction::InsertRowAbove), false),
            entry("Insert row below", "", MenuAction::Table(TableAction::InsertRowBelow), false),
            entry("Duplicate row", "", MenuAction::Table(TableAction::DuplicateRow), false),
            divider_entry("Delete row", "", MenuAction::Table(TableAction::DeleteRow), false),
        ];
        let x = self.mouse.0.min(self.editor_rect.right() - 200.0).max(self.editor_rect.x + 4.0);
        self.menu = Some(Menu { kind: MenuKind::Table, x, y: self.mouse.1, entries, split: 0, heads: ("", "") });
    }

    /// The column menu that a grip opens.
    pub fn open_col_menu(&mut self, first: usize, col: usize) {
        self.caret_to_table_line(first);
        self.caret_to_column(first, col);
        let entries = vec![
            entry("Move column left", "", MenuAction::Table(TableAction::MoveColLeft), false),
            entry("Move column right", "", MenuAction::Table(TableAction::MoveColRight), false),
            divider_entry("Insert column left", "", MenuAction::Table(TableAction::InsertColLeft), false),
            entry("Insert column right", "", MenuAction::Table(TableAction::InsertColRight), false),
            divider_entry("Delete column", "", MenuAction::Table(TableAction::DeleteCol), false),
        ];
        let x = self.mouse.0.min(self.editor_rect.right() - 200.0).max(self.editor_rect.x + 4.0);
        self.menu = Some(Menu { kind: MenuKind::Table, x, y: self.mouse.1, entries, split: 0, heads: ("", "") });
    }

    /// `(table first line, row, column)` when the caret is inside a table.
    pub fn table_position(&self) -> Option<(usize, usize, usize)> {
        let t = self.table_at(None)?;
        Some((t.first, t.row, t.col))
    }

    fn write_table(&mut self, data: &TableData) {
        let text = format_table(&data.rows, data.header);
        let body = text.trim_end_matches('\n').to_string();
        let t = self.tab_mut();
        let start = t.doc.line_start(data.first);
        let last = data.last.min(t.doc.len_lines().saturating_sub(1));
        let end = t.doc.line_start(last) + t.doc.line_len(last);
        t.doc.begin_compound();
        t.doc.replace_range(start, end, &body);
        t.doc.end_compound();
        // Put the caret *inside* the cell the command was about, not at the end
        // of the first line: leaving it on the header made the next keystroke
        // append to `| A | B |` and look like the table had stopped working.
        // The `| --- |` separator sits after row 0, so every later row is one
        // line further down.
        let skip = usize::from(data.header && data.row > 0);
        let row_line = (data.row + skip).min(body.lines().count().saturating_sub(1));
        let line = body.lines().nth(row_line).unwrap_or("");
        let prefix: usize = body.lines().take(row_line).map(|l| l.chars().count() + 1).sum();
        let in_line = cell_caret_offset(line, data.col);
        let caret = t.doc.line_start(data.first) + prefix;
        t.caret = (caret + in_line).min(t.doc.len_chars());
        t.anchor = t.caret;
        t.goal_x = None;
        t.layout = Layout::empty();
    }

    pub fn run_table_action(&mut self, action: TableAction) {
        // Row/column grips pass their own table, the rest use the caret's.
        let Some(mut data) = self.table_at(None) else { return };
        let cols = data.rows.iter().map(|r| r.len()).max().unwrap_or(1);
        let blank = vec![String::new(); cols];
        match action {
            // The caret follows the row/column that was created: the point of
            // pressing "+" is to type into the new cell, not to be dropped at the
            // end of the header line where the next keystroke breaks the table.
            TableAction::InsertRowAbove => data.rows.insert(data.row, blank),
            TableAction::InsertRowBelow => {
                data.rows.insert(data.row + 1, blank);
                data.row += 1;
            }
            TableAction::DuplicateRow => {
                let copy = data.rows[data.row].clone();
                data.rows.insert(data.row + 1, copy);
                data.row += 1;
            }
            TableAction::DeleteRow => {
                if data.rows.len() > 1 {
                    data.rows.remove(data.row);
                }
            }
            TableAction::InsertColLeft | TableAction::InsertColRight => {
                let at = if action == TableAction::InsertColLeft {
                    data.col
                } else {
                    data.col + 1
                };
                let at = at.min(cols);
                for row in &mut data.rows {
                    while row.len() < cols {
                        row.push(String::new());
                    }
                    row.insert(at, String::new());
                }
                data.col = at.min(data.rows.first().map(|r| r.len().saturating_sub(1)).unwrap_or(0));
            }
            TableAction::DeleteCol => {
                if cols > 1 {
                    let at = data.col.min(cols - 1);
                    for row in &mut data.rows {
                        if at < row.len() {
                            row.remove(at);
                        }
                    }
                }
            }
            TableAction::MoveRowUp => {
                if data.row > 0 {
                    data.rows.swap(data.row, data.row - 1);
                }
            }
            TableAction::MoveRowDown => {
                if data.row + 1 < data.rows.len() {
                    data.rows.swap(data.row, data.row + 1);
                }
            }
            TableAction::MoveColLeft => {
                if data.col > 0 {
                    for row in &mut data.rows {
                        if data.col < row.len() {
                            row.swap(data.col, data.col - 1);
                        }
                    }
                }
            }
            TableAction::MoveColRight => {
                if data.col + 1 < cols {
                    for row in &mut data.rows {
                        if data.col + 1 < row.len() {
                            row.swap(data.col, data.col + 1);
                        }
                    }
                }
            }
            TableAction::MoveRowTo(target) => {
                move_item(&mut data.rows, data.row, target, if data.header { 1 } else { 0 });
            }
            TableAction::MoveColTo(target) => {
                move_cell_column(&mut data.rows, data.col, target);
            }
        }
        self.write_table(&data);
    }

    // ---------- tools ----------

    pub fn run_tools_action(&mut self, action: ToolsAction) {
        match action {
            ToolsAction::InsertEmptyRows => self.tool_insert_empty_rows(),
            ToolsAction::RemoveEmptyRows => {
                if self.table_at(None).is_some() {
                    self.tool_table_edit(|rows| {
                        rows.retain(|r| !r.iter().all(|c| c.trim().is_empty()));
                    });
                } else {
                    self.tool_remove_empty_lines();
                }
            }
            ToolsAction::SplitRows => self.tool_split_rows(),
            ToolsAction::MergeRows => self.tool_merge_rows(),
            ToolsAction::DedupeRows => self.tool_dedupe_rows(),
            ToolsAction::CompressSpaces => {
                self.map_selected_lines(|text| {
                    let collapsed = compress_spaces(text);
                    if collapsed == text { None } else { Some(collapsed) }
                });
            }
            ToolsAction::RemoveSpaces => {
                self.map_selected_lines(|text| {
                    let stripped: String = text.chars().filter(|c| *c != ' ').collect();
                    if stripped == text { None } else { Some(stripped) }
                });
            }
            ToolsAction::ListToTable => self.tool_list_to_table(),
            ToolsAction::TableToList => self.tool_table_to_list(),
        }
    }

    /// Collapse the blank lines in the selection (or in the whole document when
    /// there is no selection) into one blank line between paragraphs.
    fn tool_remove_empty_lines(&mut self) {
        let (l0, l1) = self.selected_lines();
        let t = self.tab_mut();
        let start = t.doc.line_start(l0);
        let end = t.doc.line_start(l1) + t.doc.line_len(l1);
        let text = t.doc.slice(start, end);
        let mut out: Vec<&str> = Vec::new();
        for line in text.split('\n') {
            if line.trim().is_empty() {
                continue;
            }
            out.push(line);
        }
        let new = out.join("\n");
        t.doc.begin_compound();
        t.doc.replace_range(start, end, &new);
        t.doc.end_compound();
        t.caret = start.min(t.doc.len_chars());
        t.anchor = t.caret;
        t.layout = Layout::empty();
    }

    fn tool_insert_empty_rows(&mut self) {
        let Some(mut data) = self.table_at(None) else {
            // Outside a table: add a plain empty line, which is the same idea.
            let t = self.tab_mut();
            let (line, _) = t.doc.line_and_col(t.caret);
            let end = t.doc.line_start(line) + t.doc.line_len(line);
            t.doc.insert_atomic(end, "\n");
            t.layout = Layout::empty();
            return;
        };
        let cols = data.rows.iter().map(|r| r.len()).max().unwrap_or(1);
        let at = data.row + 1;
        for _ in 0..3 {
            data.rows.insert(at, vec![String::new(); cols]);
        }
        self.write_table(&data);
    }

    fn tool_table_edit(&mut self, f: impl FnOnce(&mut Vec<Vec<String>>)) {
        let Some(mut data) = self.table_at(None) else { return };
        f(&mut data.rows);
        if data.rows.is_empty() {
            return;
        }
        self.write_table(&data);
    }

    /// Split each cell at `;` into several rows (one row per part).
    fn tool_split_rows(&mut self) {
        let Some(mut data) = self.table_at(None) else {
            self.map_selected_lines(|text| {
                if !text.contains(';') {
                    return None;
                }
                let parts: Vec<&str> = text.split(';').map(|p| p.trim_end()).collect();
                Some(parts.join("\n"))
            });
            return;
        };
        let row = data.rows[data.row].clone();
        let parts: Vec<Vec<String>> = row
            .iter()
            .map(|c| c.split(';').map(|p| p.trim().to_string()).collect())
            .collect();
        let n = parts.iter().map(|p| p.len()).max().unwrap_or(1).max(1);
        if n <= 1 {
            return;
        }
        let mut new_rows = Vec::new();
        for i in 0..n {
            new_rows.push(
                parts
                    .iter()
                    .map(|p| p.get(i).cloned().unwrap_or_default())
                    .collect::<Vec<String>>(),
            );
        }
        data.rows.splice(data.row..data.row + 1, new_rows);
        self.write_table(&data);
    }

    /// Join the selected table rows into one row (cells separated by spaces).
    fn tool_merge_rows(&mut self) {
        let Some(mut data) = self.table_at(None) else {
            let (l0, l1) = self.selected_lines();
            if l1 > l0 {
                let t = self.tab();
                let joined: Vec<String> = (l0..=l1)
                    .map(|i| t.doc.line_text(i).trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                let text = joined.join(" ");
                let t = self.tab_mut();
                let start = t.doc.line_start(l0);
                let end = t.doc.line_start(l1) + t.doc.line_len(l1);
                t.doc.replace_range(start, end, &text);
                t.caret = start + text.chars().count();
                t.anchor = t.caret;
                t.layout = Layout::empty();
            }
            return;
        };
        let (l0, l1) = self.selected_lines();
        let from = data.first;
        let (a, b) = if l1 > l0 && l0 >= from {
            (l0 - from, l1 - from)
        } else {
            (data.row, data.row)
        };
        if b <= a || b >= data.rows.len() {
            return;
        }
        let cols = data.rows.iter().map(|r| r.len()).max().unwrap_or(1);
        let mut merged = vec![String::new(); cols];
        for row in &data.rows[a..=b] {
            for (i, cell) in row.iter().enumerate() {
                if i >= merged.len() || cell.trim().is_empty() {
                    continue;
                }
                if !merged[i].is_empty() {
                    merged[i].push(' ');
                }
                merged[i].push_str(cell.trim());
            }
        }
        data.rows.splice(a..=b, vec![merged]);
        self.write_table(&data);
    }

    fn tool_dedupe_rows(&mut self) {
        self.tool_table_edit(|rows| {
            let mut seen: Vec<Vec<String>> = Vec::new();
            rows.retain(|r| {
                let key: Vec<String> = r.iter().map(|c| c.trim().to_string()).collect();
                if seen.contains(&key) {
                    false
                } else {
                    seen.push(key);
                    true
                }
            });
        });
    }

    /// `- a` / `1. a: b` lines → a table.
    fn tool_list_to_table(&mut self) {
        let (l0, l1) = self.selected_lines();
        let t = self.tab();
        let mut rows: Vec<Vec<String>> = Vec::new();
        for i in l0..=l1 {
            let text = t.doc.line_text(i);
            let Some(item) = strip_list_marker(&text) else { continue };
            if item.trim().is_empty() {
                continue;
            }
            match item.split_once(':') {
                Some((key, value)) if !key.trim().is_empty() => {
                    rows.push(vec![key.trim().to_string(), value.trim().to_string()])
                }
                _ => rows.push(vec![item.trim().to_string(), String::new()]),
            }
        }
        if rows.is_empty() {
            return;
        }
        rows.insert(0, vec!["Item".to_string(), "Detail".to_string()]);
        let header = true;
        let text = format_table(&rows, header).trim_end_matches('\n').to_string();
        let t = self.tab_mut();
        let start = t.doc.line_start(l0);
        let end = t.doc.line_start(l1) + t.doc.line_len(l1);
        t.doc.begin_compound();
        t.doc.replace_range(start, end, &text);
        t.doc.end_compound();
        t.caret = start;
        t.anchor = start;
        t.layout = Layout::empty();
    }

    /// Table rows → `- cell | cell` lines.
    fn tool_table_to_list(&mut self) {
        let Some(data) = self.table_at(None) else { return };
        let (l0, l1) = self.selected_lines();
        let (a, b) = if l1 > l0 && l0 >= data.first {
            (l0 - data.first, l1 - data.first)
        } else {
            (0, data.rows.len().saturating_sub(1))
        };
        let mut lines: Vec<String> = Vec::new();
        for row in data.rows.iter().skip(a).take(b + 1 - a) {
            let cells: Vec<&str> = row.iter().map(|c| c.trim()).filter(|c| !c.is_empty()).collect();
            if cells.is_empty() {
                continue;
            }
            lines.push(format!("- {}", cells.join(" | ")));
        }
        if lines.is_empty() {
            return;
        }
        let text = lines.join("\n");
        let t = self.tab_mut();
        let start = t.doc.line_start(data.first);
        let last = data.last.min(t.doc.len_lines().saturating_sub(1));
        let end = t.doc.line_start(last) + t.doc.line_len(last);
        t.doc.begin_compound();
        t.doc.replace_range(start, end, &text);
        t.doc.end_compound();
        t.caret = start;
        t.anchor = start;
        t.layout = Layout::empty();
    }
}

/// Take `items[from]` out and put it back so that it lands at `target`, never
/// before `min` (the header row has to stay on top).
fn move_item<T>(items: &mut Vec<T>, from: usize, target: usize, min: usize) {
    if from >= items.len() {
        return;
    }
    let target = target.min(items.len().saturating_sub(1)).max(min);
    if target == from {
        return;
    }
    let item = items.remove(from);
    items.insert(target.min(items.len()), item);
}

fn move_cell_column(rows: &mut [Vec<String>], from: usize, target: usize) {
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if cols == 0 {
        return;
    }
    let target = target.min(cols.saturating_sub(1));
    if target == from {
        return;
    }
    for row in rows.iter_mut() {
        while row.len() < cols {
            row.push(String::new());
        }
        let cell = row.remove(from);
        row.insert(target.min(row.len()), cell);
    }
}

fn mention_guard(block: Block) -> bool {
    matches!(block, Block::Code | Block::Fence)
}

fn strip_list_marker(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    let indent = text.len() - trimmed.len();
    let _ = indent;
    if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")).or_else(|| trimmed.strip_prefix("+ ")) {
        return Some(rest.trim_start_matches("[ ] ").trim_start_matches("[x] ").to_string());
    }
    let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 {
        let rest = &trimmed[digits..];
        if let Some(rest) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return Some(rest.to_string());
        }
    }
    None
}

/// Collapse runs of spaces to one and trim the end of the line.
pub fn compress_spaces(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for c in text.chars() {
        if c == ' ' {
            if !last_space {
                out.push(c);
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod colour_tests {
    /// `[#rrggbb]text[/]` and `[bg:#rrggbb]text[/]` are found and stripped
    /// without touching the other kind, nested or not.
    #[test]
    fn colour_tags_are_found_and_stripped() {
        let chars: Vec<char> = "[#e05c5c]hello world[/]".chars().collect();
        let spans = super::colour_spans(&chars);
        assert_eq!(spans.len(), 1);
        let out = super::strip_colour_tags("[#e05c5c]hello world[/]", false);
        assert_eq!(out, "hello world");
        let bg = super::strip_colour_tags("[#e05c5c][bg:#4f8fd0]marked[/][/]", true);
        assert_eq!(bg, "[#e05c5c]marked[/]");
    }
}

/// Hand a URL to the system's default handler. Portable by design: there is no
/// bundled launcher and no new dependency, just the shell command every desktop
/// already understands.
fn open_external(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd").args(["/C", "start", ""]).arg(url).spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    }
    Ok(())
}
