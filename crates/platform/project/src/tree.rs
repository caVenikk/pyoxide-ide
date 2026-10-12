//! The file tree of a project: directories are listed when they are first
//! expanded, and the visible rows are kept as a flat list for virtualized
//! drawing.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use crate::filter::Filter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    File,
}

/// One directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: EntryKind,
    /// A symbolic link (shown with its target's kind).
    pub symlink: bool,
    /// Shown, but marked as excluded and not expanded on its own.
    pub excluded: bool,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Dir
    }
}

/// Folders first, then files; names compared case-insensitively, then
/// exactly, so the order is stable.
fn order(a: &Entry, b: &Entry) -> Ordering {
    (b.is_dir())
        .cmp(&a.is_dir())
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.name.cmp(&b.name))
}

/// Lists a directory (blocking): hidden entries are left out, the rest is
/// sorted and marked.
pub fn list_dir(dir: &Path, filter: &Filter) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for item in std::fs::read_dir(dir)? {
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        if filter.is_hidden(&name) {
            continue;
        }
        let path = item.path();
        let Ok(file_type) = item.file_type() else {
            continue;
        };
        let symlink = file_type.is_symlink();
        let is_dir = if symlink {
            std::fs::metadata(&path).is_ok_and(|m| m.is_dir())
        } else {
            file_type.is_dir()
        };
        let excluded = filter.is_excluded(&path, is_dir);
        out.push(Entry {
            name,
            path,
            kind: if is_dir {
                EntryKind::Dir
            } else {
                EntryKind::File
            },
            symlink,
            excluded,
        });
    }
    out.sort_by(order);
    Ok(out)
}

/// Index of a node in a [`FileTree`].
pub type NodeId = usize;

#[derive(Debug, Clone)]
pub struct Node {
    pub entry: Entry,
    pub depth: usize,
    pub parent: Option<NodeId>,
    /// `None` until the directory was listed.
    pub children: Option<Vec<NodeId>>,
    pub expanded: bool,
    /// Listing failed (shown as an empty folder).
    pub error: Option<String>,
}

/// The tree of a project folder.
#[derive(Debug, Clone)]
pub struct FileTree {
    nodes: Vec<Node>,
    rows: Vec<NodeId>,
}

impl FileTree {
    /// A tree with only the root, expanded but not listed yet.
    pub fn new(root: &Path) -> Self {
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        let mut tree = Self {
            nodes: vec![Node {
                entry: Entry {
                    name,
                    path: root.to_owned(),
                    kind: EntryKind::Dir,
                    symlink: false,
                    excluded: false,
                },
                depth: 0,
                parent: None,
                children: None,
                expanded: true,
                error: None,
            }],
            rows: Vec::new(),
        };
        tree.rebuild();
        tree
    }

    pub const ROOT: NodeId = 0;

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    /// Visible rows, top to bottom.
    pub fn rows(&self) -> &[NodeId] {
        &self.rows
    }

    /// Row index of a node, if it is visible.
    pub fn row_of(&self, id: NodeId) -> Option<usize> {
        self.rows.iter().position(|&r| r == id)
    }

    /// The node for a path that has been listed.
    pub fn find(&self, path: &Path) -> Option<NodeId> {
        self.nodes.iter().position(|n| n.entry.path == path)
    }

    /// The directory must be listed before its children can be shown.
    pub fn needs_listing(&self, id: NodeId) -> bool {
        let n = &self.nodes[id];
        n.entry.is_dir() && n.children.is_none() && n.error.is_none()
    }

    /// Stores the listing of directory `id`. Children that were listed
    /// before keep their state (expansion, own children).
    pub fn set_children(&mut self, id: NodeId, entries: Vec<Entry>) {
        let depth = self.nodes[id].depth + 1;
        let old: Vec<NodeId> = self.nodes[id].children.clone().unwrap_or_default();
        let mut ids = Vec::with_capacity(entries.len());
        for entry in entries {
            if let Some(&existing) = old
                .iter()
                .find(|&&c| self.nodes[c].entry.path == entry.path)
            {
                self.nodes[existing].entry = entry;
                ids.push(existing);
            } else {
                self.nodes.push(Node {
                    entry,
                    depth,
                    parent: Some(id),
                    children: None,
                    expanded: false,
                    error: None,
                });
                ids.push(self.nodes.len() - 1);
            }
        }
        self.nodes[id].children = Some(ids);
        self.nodes[id].error = None;
        self.rebuild();
    }

    /// Listing of `id` failed.
    pub fn set_error(&mut self, id: NodeId, error: String) {
        self.nodes[id].error = Some(error);
        self.nodes[id].children = Some(Vec::new());
        self.rebuild();
    }

    /// Expands a directory; returns true when it still has to be listed.
    pub fn expand(&mut self, id: NodeId) -> bool {
        if !self.nodes[id].entry.is_dir() {
            return false;
        }
        self.nodes[id].expanded = true;
        self.rebuild();
        self.needs_listing(id)
    }

    pub fn collapse(&mut self, id: NodeId) {
        self.nodes[id].expanded = false;
        self.rebuild();
    }

    /// Marks every listed entry excluded or not by `filter` (after the
    /// exclusions changed); the root never is.
    pub fn refresh_excluded(&mut self, filter: &Filter) {
        for n in self.nodes.iter_mut().skip(1) {
            n.entry.excluded = filter.is_excluded(&n.entry.path, n.entry.is_dir());
        }
    }

    /// Collapses every directory except the root.
    pub fn collapse_all(&mut self) {
        for n in self.nodes.iter_mut().skip(1) {
            n.expanded = false;
        }
        self.rebuild();
    }

    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        let mut stack = vec![Self::ROOT];
        while let Some(id) = stack.pop() {
            rows.push(id);
            let n = &self.nodes[id];
            if n.expanded
                && let Some(children) = &n.children
            {
                stack.extend(children.iter().rev());
            }
        }
        self.rows = rows;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::AutoExclusions;
    use crate::store::Exclusions;

    const PY: AutoExclusions = AutoExclusions {
        dir_names: &[".venv"],
        root_dir_markers: &["pyvenv.cfg"],
    };

    fn layout(root: &Path) {
        for dir in [
            "src/pkg", "tests", ".git", "Zeta", ".venv", ".oxide", ".idea",
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join(".venv/pyvenv.cfg"), "").unwrap();
        for file in ["README.md", "app.py", "b.py", "src/pkg/mod.py", "x.pyc"] {
            std::fs::write(root.join(file), "").unwrap();
        }
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn lists_sorted_and_filtered() {
        let dir = tempfile::tempdir().unwrap();
        layout(dir.path());
        let filter = Filter::new(dir.path(), Exclusions::default(), PY);
        let entries = list_dir(dir.path(), &filter).unwrap();
        assert_eq!(
            names(&entries),
            [
                ".idea",
                ".venv",
                "src",
                "tests",
                "Zeta",
                "app.py",
                "b.py",
                "README.md"
            ]
        );
        assert!(!entries[0].excluded);
        assert!(entries[1].excluded);
        assert!(!entries[2].excluded);
        // Everything below an excluded folder is excluded.
        let inside = list_dir(&dir.path().join(".venv"), &filter).unwrap();
        assert_eq!(names(&inside), ["pyvenv.cfg"]);
        assert!(inside[0].excluded);
    }

    #[test]
    fn a_reference_module_file_excludes_folders_when_the_project_opens() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for d in ["venv/lib", "src"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::create_dir_all(root.join(".idea")).unwrap();
        std::fs::write(
            root.join(".idea/x.iml"),
            r#"<module type="PYTHON_MODULE" version="4">
  <component name="NewModuleRootManager">
    <content url="file://$MODULE_DIR$">
      <excludeFolder url="file://$MODULE_DIR$/venv" />
    </content>
  </component>
</module>"#,
        )
        .unwrap();
        let filter = Filter::for_root(root);
        let entries = list_dir(root, &filter).unwrap();
        assert_eq!(names(&entries), [".idea", "src", "venv"]);
        assert_eq!(
            entries.iter().map(|e| e.excluded).collect::<Vec<_>>(),
            [false, false, true]
        );
        assert!(list_dir(&root.join("venv"), &filter).unwrap()[0].excluded);
    }

    #[test]
    fn expands_lazily_and_flattens_visible_rows() {
        let dir = tempfile::tempdir().unwrap();
        layout(dir.path());
        let filter = Filter::new(dir.path(), Exclusions::default(), PY);
        let mut tree = FileTree::new(dir.path());
        assert!(tree.needs_listing(FileTree::ROOT));
        assert_eq!(tree.rows(), [FileTree::ROOT]);
        tree.set_children(FileTree::ROOT, list_dir(dir.path(), &filter).unwrap());
        assert_eq!(tree.rows().len(), 9);
        let src = tree.find(&dir.path().join("src")).unwrap();
        assert!(tree.expand(src));
        tree.set_children(src, list_dir(&dir.path().join("src"), &filter).unwrap());
        let pkg = tree.find(&dir.path().join("src/pkg")).unwrap();
        assert_eq!(tree.node(pkg).depth, 2);
        assert_eq!(tree.row_of(pkg), Some(tree.row_of(src).unwrap() + 1));
        assert!(tree.expand(pkg));
        tree.set_children(pkg, list_dir(&dir.path().join("src/pkg"), &filter).unwrap());
        assert_eq!(tree.rows().len(), 11);
        // Collapsing keeps the listing; expanding again needs no listing.
        tree.collapse(src);
        assert_eq!(tree.rows().len(), 9);
        assert!(!tree.expand(src));
        assert_eq!(tree.rows().len(), 11);
        // A new listing keeps the state of known children.
        tree.set_children(FileTree::ROOT, list_dir(dir.path(), &filter).unwrap());
        assert!(tree.node(src).expanded);
        assert_eq!(tree.rows().len(), 11);
        tree.collapse_all();
        assert_eq!(tree.rows().len(), 9);

        // Marking a folder excluded updates it and everything listed below.
        let mut marked = Exclusions::default();
        marked.add("src");
        tree.refresh_excluded(&filter.with_exclusions(marked));
        let excluded = |tree: &FileTree, p: &str| {
            tree.node(tree.find(&dir.path().join(p)).unwrap())
                .entry
                .excluded
        };
        assert!(excluded(&tree, "src") && excluded(&tree, "src/pkg/mod.py"));
        assert!(excluded(&tree, ".venv") && !excluded(&tree, "tests"));
        assert!(!tree.node(FileTree::ROOT).entry.excluded);
        tree.refresh_excluded(&filter);
        assert!(!excluded(&tree, "src") && !excluded(&tree, "src/pkg/mod.py"));
    }
}
