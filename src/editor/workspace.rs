//! Workspace folder tree for the sidebar panel.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
    pub expanded: bool,
}

#[derive(Clone)]
pub struct Workspace {
    pub root: PathBuf,
    expanded: HashSet<PathBuf>,
    entries: Vec<Entry>,
    pub truncated: bool,
}

const MAX_ENTRIES: usize = 4000;

impl Workspace {
    pub fn new(root: PathBuf) -> Workspace {
        let mut ws = Workspace { root, expanded: HashSet::new(), entries: Vec::new(), truncated: false };
        ws.expanded.insert(ws.root.clone());
        ws.refresh();
        ws
    }

    pub fn set_root(&mut self, root: PathBuf) {
        self.root = root;
        self.expanded.clear();
        self.expanded.insert(self.root.clone());
        self.refresh();
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn toggle(&mut self, path: &Path) {
        if self.expanded.contains(path) {
            self.expanded.remove(path);
        } else {
            self.expanded.insert(path.to_path_buf());
        }
        self.refresh();
    }

    pub fn expand(&mut self, path: &Path) {
        self.expanded.insert(path.to_path_buf());
        self.refresh();
    }

    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }

    /// Parents of `path` are expanded so it is visible in the tree.
    pub fn reveal(&mut self, path: &Path) {
        let mut cur = path.parent();
        while let Some(p) = cur {
            if p.starts_with(&self.root) {
                self.expanded.insert(p.to_path_buf());
            }
            if p == self.root {
                break;
            }
            cur = p.parent();
        }
        self.refresh();
    }

    pub fn refresh(&mut self) {
        self.entries.clear();
        self.truncated = false;
        if !self.root.is_dir() {
            return;
        }
        self.walk(&self.root.clone(), 0);
    }

    /// Depth-first walk emitting visible entries in display order.
    fn walk(&mut self, dir: &Path, depth: usize) {
        if depth > 16 {
            return;
        }
        for child in read_dir_sorted(dir) {
            if self.entries.len() >= MAX_ENTRIES {
                self.truncated = true;
                return;
            }
            let is_dir = child.is_dir;
            let path = child.path.clone();
            let expanded = is_dir && self.expanded.contains(&path);
            self.entries.push(Entry {
                path: path.clone(),
                name: child.name,
                is_dir,
                depth,
                expanded,
            });
            if expanded {
                self.walk(&path, depth + 1);
            }
        }
    }
}

struct Child {
    path: PathBuf,
    name: String,
    is_dir: bool,
}

fn read_dir_sorted(dir: &Path) -> Vec<Child> {
    let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Child> = Vec::new();
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir && !is_editable(&name) {
            continue;
        }
        out.push(Child { path, name, is_dir });
    }
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

fn is_editable(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(
        Path::new(&lower).extension().and_then(|e| e.to_str()),
        Some("md") | Some("markdown") | Some("mdown") | Some("txt") | Some("rst") | Some("json") | Some("toml") | Some("yaml") | Some("yml") | Some("conf") | Some("cfg") | Some("ini") | Some("log") | Some("csv")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_lists_and_expands() {
        let root = std::env::temp_dir().join(format!("bare-md-ws-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("a.md"), "# a").unwrap();
        fs::write(root.join("sub/b.md"), "# b").unwrap();
        fs::write(root.join("notes.png"), "x").unwrap();

        let mut ws = Workspace::new(root.clone());
        let names: Vec<_> = ws.entries().iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, vec!["sub", "a.md"], "dirs first, images filtered");
        let sub = root.join("sub");
        ws.toggle(&sub);
        let names: Vec<_> = ws.entries().iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, vec!["sub", "b.md", "a.md"]);
        ws.toggle(&sub);
        assert_eq!(ws.entries().len(), 2);
        let _ = fs::remove_dir_all(&root);
    }
}
