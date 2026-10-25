//! The file tree of a project: directories are listed when they are first
//! expanded, and the visible rows are kept as a flat list for virtualized
//! drawing.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use crate::filter::Filter;
use crate::roots::FolderKind;

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
    /// What the product sees in a folder (a package, a module); plain for
    /// a file.
    pub folder: FolderKind,
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
        let folder = if is_dir {
            filter.folder_kind(&path)
        } else {
            FolderKind::Plain
        };
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
            folder,
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
                    folder: FolderKind::Plain,
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

    /// The node for a path that has been listed (and is still in its
    /// folder's listing).
    pub fn find(&self, path: &Path) -> Option<NodeId> {
        if path.as_os_str().is_empty() {
            return None;
        }
        (0..self.nodes.len()).find(|&id| self.nodes[id].entry.path == path && self.is_attached(id))
    }

    /// The node is in the tree: every folder above it lists it. Entries
    /// that disappeared from a new listing are kept, detached.
    pub fn is_attached(&self, mut id: NodeId) -> bool {
        while let Some(parent) = self.nodes[id].parent {
            let listed = self.nodes[parent]
                .children
                .as_ref()
                .is_some_and(|c| c.contains(&id));
            if !listed {
                return false;
            }
            id = parent;
        }
        true
    }

    /// The folders in the tree that have been listed, parents first.
    pub fn listed_dirs(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![Self::ROOT];
        while let Some(id) = stack.pop() {
            if let Some(children) = &self.nodes[id].children {
                out.push(id);
                stack.extend(children.iter().rev());
            }
        }
        out
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

    /// What the product sees in folder `id` changed (a marker file was
    /// created or deleted, or the root's kind was read).
    pub fn set_folder_kind(&mut self, id: NodeId, kind: FolderKind) {
        if self.nodes[id].entry.is_dir() {
            self.nodes[id].entry.folder = kind;
        }
    }

    /// Adds an entry that appeared on disk under the listed folder
    /// `parent`, in its sorted place; a folder not listed yet lists it
    /// when it is expanded. Returns the entry's node (an existing one for
    /// a path the tree has).
    pub fn insert(&mut self, parent: NodeId, entry: Entry) -> Option<NodeId> {
        if let Some(id) = self.find(&entry.path) {
            return Some(id);
        }
        let depth = self.nodes[parent].depth + 1;
        self.nodes[parent].children.as_ref()?;
        self.nodes.push(Node {
            entry,
            depth,
            parent: Some(parent),
            children: None,
            expanded: false,
            error: None,
        });
        let id = self.nodes.len() - 1;
        self.attach(parent, id);
        Some(id)
    }

    /// Puts `id` among the children of `parent` in sorted order.
    fn attach(&mut self, parent: NodeId, id: NodeId) {
        let mut children = self.nodes[parent].children.take().unwrap_or_default();
        children.retain(|&c| c != id);
        let at = children
            .iter()
            .position(|&c| order(&self.nodes[id].entry, &self.nodes[c].entry) == Ordering::Less)
            .unwrap_or(children.len());
        children.insert(at, id);
        self.nodes[parent].children = Some(children);
        self.nodes[id].parent = Some(parent);
        self.rebuild();
    }

    /// Takes `id` and everything below it out of the tree (deleted on
    /// disk). Its nodes are kept but never found again.
    pub fn remove(&mut self, id: NodeId) {
        if id == Self::ROOT {
            return;
        }
        if let Some(parent) = self.nodes[id].parent
            && let Some(children) = &mut self.nodes[parent].children
        {
            children.retain(|&c| c != id);
        }
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            let node = &mut self.nodes[n];
            node.entry.path = PathBuf::new();
            node.parent = None;
            if let Some(children) = node.children.take() {
                stack.extend(children);
            }
        }
        self.rebuild();
    }

    /// Renames or moves `id` to `to`, below `new_parent` (a listed
    /// folder): the paths below a folder follow, and it keeps its
    /// expansion and listing.
    pub fn rename(&mut self, id: NodeId, to: &Path, new_parent: NodeId) {
        if id == Self::ROOT {
            return;
        }
        let from = self.nodes[id].entry.path.clone();
        if let Some(parent) = self.nodes[id].parent
            && let Some(children) = &mut self.nodes[parent].children
        {
            children.retain(|&c| c != id);
        }
        let depth_shift = self.nodes[new_parent].depth as isize + 1 - self.nodes[id].depth as isize;
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            let node = &mut self.nodes[n];
            if let Ok(rest) = node.entry.path.strip_prefix(&from) {
                node.entry.path = if rest.as_os_str().is_empty() {
                    to.to_owned()
                } else {
                    to.join(rest)
                };
            }
            node.depth = (node.depth as isize + depth_shift) as usize;
            if let Some(children) = &node.children {
                stack.extend(children.iter().copied());
            }
        }
        self.nodes[id].entry.name = to
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if self.nodes[new_parent].children.is_some() {
            self.attach(new_parent, id);
        } else {
            // The new folder lists its content when it is expanded.
            self.remove(id);
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
    fn a_temporary_save_file_left_behind_is_not_listed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.py"), "").unwrap();
        // From a crashed session, and from a save in flight.
        std::fs::write(dir.path().join(".app.py.pyoxide-save-2147483632-0"), "").unwrap();
        let mine = format!(".app.py.pyoxide-save-{}-1", std::process::id());
        std::fs::write(dir.path().join(mine), "").unwrap();
        // A file that only looks similar is a project file.
        std::fs::write(dir.path().join(".pyoxide-save-notes"), "").unwrap();
        let filter = Filter::new(dir.path(), Exclusions::default(), PY);
        let entries = list_dir(dir.path(), &filter).unwrap();
        assert_eq!(names(&entries), [".pyoxide-save-notes", "app.py"]);
        assert!(filter.is_hidden(".app.py.pyoxide-save-1-0"));
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

    #[test]
    fn entries_gone_from_a_new_listing_are_not_found() {
        let dir = tempfile::tempdir().unwrap();
        layout(dir.path());
        let filter = Filter::new(dir.path(), Exclusions::default(), PY);
        let mut tree = FileTree::new(dir.path());
        tree.set_children(FileTree::ROOT, list_dir(dir.path(), &filter).unwrap());
        let src = tree.find(&dir.path().join("src")).unwrap();
        tree.expand(src);
        tree.set_children(src, list_dir(&dir.path().join("src"), &filter).unwrap());
        assert_eq!(tree.listed_dirs(), vec![FileTree::ROOT, src]);
        let pkg = dir.path().join("src/pkg");
        assert!(tree.find(&pkg).is_some());

        // Gone, and everything that was listed below it.
        std::fs::remove_dir_all(dir.path().join("src")).unwrap();
        tree.set_children(FileTree::ROOT, list_dir(dir.path(), &filter).unwrap());
        assert_eq!(tree.find(&dir.path().join("src")), None);
        assert_eq!(tree.find(&pkg), None);
        assert_eq!(tree.listed_dirs(), vec![FileTree::ROOT]);

        // Back again: a new node, not listed yet.
        std::fs::create_dir_all(&pkg).unwrap();
        tree.set_children(FileTree::ROOT, list_dir(dir.path(), &filter).unwrap());
        let again = tree.find(&dir.path().join("src")).unwrap();
        assert_ne!(again, src);
        assert!(tree.needs_listing(again));
    }

    fn entry(path: &Path, kind: EntryKind) -> Entry {
        Entry {
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            path: path.to_owned(),
            kind,
            symlink: false,
            excluded: false,
            folder: FolderKind::Plain,
        }
    }

    #[test]
    fn folders_get_the_product_kind() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for d in ["pkg", "svc", "plain"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("pkg/__init__.py"), "").unwrap();
        std::fs::write(root.join("svc/pyproject.toml"), "").unwrap();
        std::fs::write(root.join("__init__.py"), "").unwrap();
        let rules = crate::FolderRules {
            package_markers: &["__init__.py"],
            module_markers: &["pyproject.toml"],
        };
        let filter = Filter::new(root, Exclusions::default(), PY).with_folder_rules(rules);
        let entries = list_dir(root, &filter).unwrap();
        let kinds: Vec<(&str, FolderKind)> = entries
            .iter()
            .map(|e| (e.name.as_str(), e.folder))
            .collect();
        assert_eq!(
            kinds,
            [
                ("pkg", FolderKind::Package),
                ("plain", FolderKind::Plain),
                ("svc", FolderKind::Module),
                ("__init__.py", FolderKind::Plain),
            ]
        );
        // Without the product's rules every folder is plain.
        let plain = Filter::new(root, Exclusions::default(), PY);
        let entries = list_dir(root, &plain).unwrap();
        assert!(entries.iter().all(|e| e.folder == FolderKind::Plain));
        let mut tree = FileTree::new(root);
        tree.set_folder_kind(FileTree::ROOT, FolderKind::Package);
        assert_eq!(tree.node(FileTree::ROOT).entry.folder, FolderKind::Package);
    }

    #[test]
    fn inserts_removes_and_renames_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        layout(root);
        let filter = Filter::new(root, Exclusions::default(), PY);
        let mut tree = FileTree::new(root);
        tree.set_children(FileTree::ROOT, list_dir(root, &filter).unwrap());
        let src = tree.find(&root.join("src")).unwrap();
        let names = |tree: &FileTree| -> Vec<String> {
            tree.rows()
                .iter()
                .map(|&id| {
                    let p = &tree.node(id).entry.path;
                    p.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/")
                })
                .collect()
        };
        // A new folder goes before the files, sorted; a folder not listed
        // yet takes nothing.
        let new = tree
            .insert(FileTree::ROOT, entry(&root.join("lib"), EntryKind::Dir))
            .unwrap();
        assert_eq!(tree.node(new).depth, 1);
        assert_eq!(
            names(&tree)[..6],
            ["", ".idea", ".venv", "lib", "src", "tests"]
        );
        assert_eq!(
            tree.insert(src, entry(&root.join("src/x.py"), EntryKind::File)),
            None
        );
        // Renaming an expanded folder keeps it expanded; paths below
        // follow.
        tree.expand(src);
        tree.set_children(src, list_dir(&root.join("src"), &filter).unwrap());
        let pkg = tree.find(&root.join("src/pkg")).unwrap();
        tree.expand(pkg);
        tree.set_children(pkg, list_dir(&root.join("src/pkg"), &filter).unwrap());
        tree.rename(src, &root.join("a_src"), FileTree::ROOT);
        assert_eq!(tree.find(&root.join("src")), None);
        assert!(tree.find(&root.join("a_src/pkg/mod.py")).is_some());
        assert!(tree.node(src).expanded);
        assert_eq!(
            names(&tree)[..7],
            [
                "",
                ".idea",
                ".venv",
                "a_src",
                "a_src/pkg",
                "a_src/pkg/mod.py",
                "lib"
            ]
        );
        // Moved into another folder: depths follow.
        tree.set_children(new, Vec::new());
        tree.rename(pkg, &root.join("lib/pkg"), new);
        assert_eq!(tree.node(pkg).depth, 2);
        let module = tree.find(&root.join("lib/pkg/mod.py")).unwrap();
        assert_eq!(tree.node(module).depth, 3);
        assert_eq!(tree.node(pkg).parent, Some(new));
        // Into a folder that is not listed: it leaves the tree until that
        // folder is listed.
        let tests_dir = tree.find(&root.join("tests")).unwrap();
        tree.rename(pkg, &root.join("tests/pkg"), tests_dir);
        assert_eq!(tree.find(&root.join("tests/pkg")), None);
        assert_eq!(tree.find(&root.join("lib/pkg")), None);
        // Removed with everything below it.
        let tests = tree.find(&root.join("tests")).unwrap();
        tree.remove(tests);
        assert_eq!(tree.find(&root.join("tests")), None);
        assert!(!names(&tree).contains(&"tests".to_owned()));
        tree.remove(src);
        assert_eq!(tree.find(&root.join("a_src")), None);
        assert_eq!(tree.find(Path::new("")), None);
    }
}
