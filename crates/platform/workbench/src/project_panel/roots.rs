//! Source roots, packages and modules in the project tree (ADR 0013).
//!
//! The product says which kinds of roots there are ([`RootKind`]; Python:
//! Sources Root and Test Sources Root) and which files make a folder a
//! package or a module of its own ([`TreeHooks`]). The tree draws them as
//! the reference IDE's project view does (IntelliJ Community at
//! `pycharm/2026.2.3`, Apache-2.0: `PsiDirectoryNode`,
//! `ProjectViewDirectoryHelper.getLocationString`, `SourceRootIconProvider`
//! and the Python plugin's `PyDirectoryIconProvider`):
//!
//! - a module's content root (the project root, and a folder that is a
//!   module of its own) with the module icon (the product's for a module),
//!   its name in bold and after it, in grey, the kind of root it is
//!   (`sources root`) and, for the project root, its location;
//! - another root with the icon of its kind, without a hint;
//! - an excluded folder with the excluded folder icon;
//! - a package with the package icon.
//!
//! *Mark Directory As* offers the kinds before and after *Excluded* in the
//! product's order, and *Unmark as … Root* for a root (`MarkRootActionBase`,
//! `MarkSourceRootAction`, `UnmarkRootAction`): a folder is not offered the
//! kind it has, and a kind that cannot be nested is not offered inside a
//! source root (`ModuleType.isMarkInnerSupportedFor`). The project root can
//! be marked too. Roots are stored in the project settings in the
//! background, like the exclusions.

use std::path::{Path, PathBuf};

use gpui::{AppContext as _, Context, Window};
use project::fs_ops::FsChange;
use project::{DetectRoots, FileTree, Filter, FolderKind, FolderRules, Node, NodeId, Roots};
use ui::icons::IconName;
use ui::menu::MenuItem;

use super::{
    CancelExclusion, MARK_DIRECTORY_AS, MARK_FILE_AS, MarkExcluded, PanelEvent, ProjectPanel,
};

ui::ui_commands! {
    COMMANDS, scope = "ProjectTree";
    MarkRoot { kind: String, path: Option<String> } => "project.tree.mark-root", "Mark Directory As",
        "Mark Directory As > Sources Root, Test Sources Root, …: marks the selected folder (or `path`, relative to the project, `.` for the project root) as a root of `kind` (the product's kinds: `sources`, `tests`) and stores that in the project settings.",
        r#"{"kind": "sources", "path": "src"}"#;
    UnmarkRoot { path: Option<String> } => "project.tree.unmark-root", "Unmark Root",
        "Mark Directory As > Unmark as … Root: makes the selected root (or `path`) an ordinary folder again.",
        r#"{"path": "src"}"#;
}

/// A kind of root the product offers in *Mark Directory As*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootKind {
    /// The stored kind ([`project::roots::SOURCES`], [`project::roots::TESTS`]).
    pub id: &'static str,
    /// The menu row (`Sources Root`); `Unmark as Sources Root` unmarks it,
    /// and in lower case it is the hint after a content root's name.
    pub name: &'static str,
    /// The icon of such a root in the tree and in the menu.
    pub icon: IconName,
    /// Can be marked on a folder inside a source root.
    pub inner: bool,
    /// Listed after *Excluded* (before it otherwise).
    pub after_excluded: bool,
}

/// What a product adds to the project tree.
#[derive(Debug, Clone, Default)]
pub struct TreeHooks {
    /// The kinds of roots, in the order of *Mark Directory As*.
    pub root_kinds: Vec<RootKind>,
    /// Files that make the folder holding them a module of its own (a
    /// Python project's `pyproject.toml`); package markers are
    /// [`NavigationHooks::package_markers`](crate::NavigationHooks).
    pub module_markers: &'static [&'static str],
    /// The icon of a module's content root that has a module marker
    /// ([`IconName::MODULE`] without one).
    pub module_icon: Option<IconName>,
}

/// The separator between the kind of root and the location after the
/// project root's name (`ProjectViewDirectoryHelper`: a comma, a space and
/// a thin space).
const KIND_LOCATION_SEPARATOR: &str = ", \u{2009}";

/// The product's rules for the tree, set once by the workbench.
#[derive(Debug, Clone, Default)]
pub(super) struct ProductRules {
    pub hooks: TreeHooks,
    pub folders: FolderRules,
    pub detect: Option<DetectRoots>,
}

/// Reads the rules of a project (blocking): imports the reference IDE's
/// exclusions and source folders once, then the exclusions, the roots in
/// effect and the product's folder rules.
pub(super) fn load_rules(root: &Path, rules: &ProductRules) -> Filter {
    // A failed import leaves the project as it was; the rules still read
    // `.idea` until the project has its own settings.
    let _ = project::store::import_idea(root);
    let _ = project::store::import_idea_roots(root, rules.detect);
    let roots = project::store::load_or_detect_roots(root, rules.detect).unwrap_or_default();
    Filter::for_root(root)
        .with_roots(roots)
        .with_folder_rules(rules.folders)
}

/// What a row shows as text.
pub(super) struct RowText {
    /// The name in bold (a module's content root).
    pub bold: bool,
    /// The grey text after the name.
    pub hint: Option<String>,
}

impl ProjectPanel {
    /// Takes the product's tree rules (kinds of roots, package and module
    /// markers, detected source roots).
    pub fn set_product_rules(&mut self, product: &crate::Product) {
        self.rules = ProductRules {
            hooks: product.tree.clone(),
            folders: FolderRules {
                package_markers: product.navigation.package_markers,
                module_markers: product.tree.module_markers,
            },
            detect: product.navigation.source_roots,
        };
    }

    /// The product's kind of the root `path` is, if it is one.
    fn root_kind(&self, path: &Path) -> Option<RootKind> {
        let id = self.filter.as_ref()?.root_kind(path)?;
        self.rules
            .hooks
            .root_kinds
            .iter()
            .find(|k| k.id == id)
            .copied()
    }

    /// A module's content root: the project root, or a folder that is a
    /// module of its own (not excluded).
    fn is_content_root(node: &Node) -> bool {
        node.depth == 0 || (node.entry.folder == FolderKind::Module && !node.entry.excluded)
    }

    /// The icon of a folder row.
    pub(super) fn folder_icon(&self, node: &Node) -> IconName {
        let entry = &node.entry;
        if Self::is_content_root(node) {
            return match (entry.folder, self.rules.hooks.module_icon) {
                (FolderKind::Module, Some(icon)) => icon,
                _ => IconName::MODULE,
            };
        }
        if let Some(kind) = self.root_kind(&entry.path) {
            return kind.icon;
        }
        if entry.excluded {
            // Also inside an excluded folder, as in the reference IDE.
            IconName::EXCLUDED_FOLDER
        } else if entry.folder == FolderKind::Package {
            IconName::PACKAGE
        } else {
            IconName::FOLDER
        }
    }

    /// The bold name and the grey hint of a row: a content root shows the
    /// kind of root it is, the project root also its location.
    pub(super) fn row_text(&self, node: &Node) -> RowText {
        if !Self::is_content_root(node) {
            return RowText {
                bold: false,
                hint: None,
            };
        }
        let mut hint = self
            .root_kind(&node.entry.path)
            .map(|k| k.name.to_lowercase())
            .unwrap_or_default();
        if node.depth == 0
            && let Some(project) = &self.project
        {
            if !hint.is_empty() {
                hint.push_str(KIND_LOCATION_SEPARATOR);
            }
            hint.push_str(&project.display_root());
        }
        RowText {
            bold: true,
            hint: (!hint.is_empty()).then_some(hint),
        }
    }

    /// The source roots in effect (the product's `sources` and `tests`).
    pub fn source_roots(&self) -> Vec<PathBuf> {
        self.filter
            .as_ref()
            .map(Filter::source_roots)
            .unwrap_or_default()
    }

    // ---- Mark Directory As ----------------------------------------------

    /// The path a root command names (`.` or empty for the project root),
    /// or the selected row's, and whether it is a folder.
    fn root_target(&self, path: Option<&str>) -> Option<(PathBuf, bool)> {
        match path.map(str::trim) {
            Some("" | ".") => Some((self.project.as_ref()?.root().to_owned(), true)),
            other => self.target(other),
        }
    }

    /// The folder's stored root path (`.` for the project root).
    fn stored_path(&self, path: &Path) -> Option<String> {
        let root = self.project.as_ref()?.root();
        project::store::relative_or_root(root, path)
    }

    /// A folder excluded by a rule while the folder above is not: the
    /// stored exclusions (cancelled with *Cancel Exclusion*) and the
    /// product's automatic ones (which cannot be cancelled).
    fn is_exclusion_root(&self, path: &Path, filter: &Filter) -> bool {
        filter.is_excluded(path, true)
            && path
                .parent()
                .is_none_or(|parent| !filter.is_excluded(parent, true))
    }

    /// Whether `kind` can be marked on the folder `path`.
    fn can_mark(&self, path: &Path, kind: &RootKind) -> bool {
        let Some(filter) = &self.filter else {
            return false;
        };
        if self.stored_path(path).is_none() || self.is_exclusion_root(path, filter) {
            return false;
        }
        match self.root_kind(path) {
            Some(current) => current.id != kind.id,
            // Inside a source root only kinds that nest (a folder of an
            // excluded folder is no source content).
            None => {
                kind.inner
                    || filter.is_excluded(path, true)
                    || filter.source_root_of(path).is_none()
            }
        }
    }

    /// The rows of *Mark Directory As* (*Mark File As* for a file) for
    /// `id` in the reference's order; `None` when there is nothing to mark.
    pub(super) fn mark_menu(&self, id: NodeId) -> Option<(&'static str, Vec<MenuItem>)> {
        let (tree, filter) = (self.tree.as_ref()?, self.filter.as_ref()?);
        let entry = &tree.node(id).entry;
        let path = &entry.path;
        let rel = self.stored_path(path)?;
        let title = if entry.is_dir() {
            MARK_DIRECTORY_AS
        } else {
            MARK_FILE_AS
        };
        let mut items = Vec::new();
        if id != FileTree::ROOT && filter.is_excluded_root(path) {
            items.push(MenuItem::action(
                "Cancel Exclusion",
                CancelExclusion {
                    path: Some(rel.clone()),
                },
            ));
            return Some((title, items));
        }
        let kind_item = |kind: &RootKind| {
            MenuItem::action(
                kind.name,
                MarkRoot {
                    kind: kind.id.to_owned(),
                    path: Some(rel.clone()),
                },
            )
            .icon(kind.icon)
        };
        let kinds = &self.rules.hooks.root_kinds;
        let offered = |after: bool| {
            kinds
                .iter()
                .filter(move |k| k.after_excluded == after)
                .filter(|k| entry.is_dir() && self.can_mark(path, k))
                .map(kind_item)
                .collect::<Vec<_>>()
        };
        items.extend(offered(false));
        // The project root and what is excluded already cannot be
        // excluded.
        if id != FileTree::ROOT && !entry.excluded {
            items.push(
                MenuItem::action(
                    "Excluded",
                    MarkExcluded {
                        path: Some(rel.clone()),
                    },
                )
                .icon(IconName::EXCLUDED_FOLDER),
            );
        }
        items.extend(offered(true));
        if let Some(kind) = self.root_kind(path) {
            items.push(MenuItem::action(
                format!("Unmark as {}", kind.name),
                UnmarkRoot {
                    path: Some(rel.clone()),
                },
            ));
        }
        (!items.is_empty()).then_some((title, items))
    }

    pub(super) fn mark_root(&mut self, a: &MarkRoot, _: &mut Window, cx: &mut Context<Self>) {
        let Some((path, true)) = self.root_target(a.path.as_deref()) else {
            return;
        };
        let Some(kind) = self
            .rules
            .hooks
            .root_kinds
            .iter()
            .find(|k| k.id == a.kind)
            .copied()
        else {
            return;
        };
        let (Some(rel), Some(filter)) = (self.stored_path(&path), &self.filter) else {
            return;
        };
        if !self.can_mark(&path, &kind) {
            return;
        }
        let mut roots = filter.roots().clone();
        roots.mark(&rel, kind.id);
        self.set_roots(roots, cx);
    }

    pub(super) fn unmark_root(&mut self, a: &UnmarkRoot, _: &mut Window, cx: &mut Context<Self>) {
        let Some((path, _)) = self.root_target(a.path.as_deref()) else {
            return;
        };
        let (Some(rel), Some(filter)) = (self.stored_path(&path), &self.filter) else {
            return;
        };
        let mut roots = filter.roots().clone();
        if roots.unmark(&rel) {
            self.set_roots(roots, cx);
        }
    }

    /// Applies new roots to the rules, then writes the project settings in
    /// the background.
    fn set_roots(&mut self, roots: Roots, cx: &mut Context<Self>) {
        let Some(filter) = &self.filter else { return };
        self.filter = Some(filter.with_roots(roots));
        self.publish_roots();
        self.roots_dirty = true;
        self.save_settings(cx);
        cx.emit(PanelEvent::RulesChanged);
        cx.notify();
    }

    /// Makes the roots in effect known to code that has only a path
    /// ([`project::roots::publish`]).
    pub(super) fn publish_roots(&self) {
        if let (Some(project), Some(filter)) = (&self.project, &self.filter) {
            project::roots::publish(project.root(), filter.roots());
        }
    }

    /// Roots at or below folders the application renamed, moved or
    /// deleted follow them.
    pub(super) fn follow_roots(&mut self, changes: &[FsChange], cx: &mut Context<Self>) {
        let Some(filter) = &self.filter else { return };
        let mut roots = filter.roots().clone();
        let mut changed = false;
        for change in changes {
            match change {
                FsChange::Removed { path, .. } => {
                    if let Some(rel) = filter.relative(path) {
                        changed |= roots.removed(&rel);
                    }
                }
                FsChange::Renamed { from, to, .. } => {
                    if let (Some(old), Some(new)) = (filter.relative(from), filter.relative(to)) {
                        changed |= roots.moved(&old, &new);
                    }
                }
                FsChange::Created { .. } => {}
            }
        }
        if changed {
            self.set_roots(roots, cx);
        }
    }

    /// Folders whose kind may have changed with these changes: created
    /// and renamed folders, and the folders of created, deleted or renamed
    /// marker files.
    pub(super) fn folder_kind_targets(&self, changes: &[FsChange]) -> Vec<PathBuf> {
        let rules = self.rules.folders;
        let is_marker = |path: &Path| {
            path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                rules.package_markers.contains(&n) || rules.module_markers.contains(&n)
            })
        };
        let mut out = Vec::new();
        let mut add = |path: &Path, is_dir: bool| {
            if is_dir {
                out.push(path.to_owned());
            } else if is_marker(path)
                && let Some(parent) = path.parent()
            {
                out.push(parent.to_owned());
            }
        };
        for change in changes {
            match change {
                FsChange::Created { path, is_dir } => add(path, *is_dir),
                FsChange::Removed { path, is_dir } => {
                    if !*is_dir {
                        add(path, false);
                    }
                }
                FsChange::Renamed { from, to, is_dir } => {
                    add(to, *is_dir);
                    if !*is_dir {
                        add(from, false);
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Looks again at what the product sees in these folders (in the
    /// background) and shows it.
    pub(super) fn refresh_folder_kinds(&mut self, dirs: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(filter) = self.filter.clone() else {
            return;
        };
        if dirs.is_empty() {
            return;
        }
        let generation = self.generation;
        self.loading += 1;
        let task = cx.background_spawn(async move {
            dirs.into_iter()
                .map(|dir| {
                    let kind = filter.folder_kind(&dir);
                    (dir, kind)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let kinds = task.await;
            let _ = this.update(cx, |this, cx| {
                this.loading -= 1;
                if this.generation != generation {
                    return;
                }
                if let Some(tree) = &mut this.tree {
                    for (dir, kind) in kinds {
                        if let Some(id) = tree.find(&dir) {
                            tree.set_folder_kind(id, kind);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The roots by kind (for `workbench.state`).
    pub(super) fn roots_json(&self) -> serde_json::Value {
        let Some(filter) = &self.filter else {
            return serde_json::Value::Null;
        };
        let roots = filter.roots();
        roots
            .kinds()
            .map(|kind| {
                let paths: Vec<&str> = roots.paths(kind).collect();
                (kind.to_owned(), serde_json::json!(paths))
            })
            .collect::<serde_json::Map<_, _>>()
            .into()
    }

    /// A row's root kind, folder kind, bold name and hint (for
    /// `workbench.state`).
    pub(super) fn row_roots_json(&self, node: &Node) -> serde_json::Value {
        let text = self.row_text(node);
        serde_json::json!({
            "root_kind": self.filter.as_ref().and_then(|f| f.root_kind(&node.entry.path)),
            "folder": match node.entry.folder {
                FolderKind::Plain => None,
                FolderKind::Package => Some("package"),
                FolderKind::Module => Some("module"),
            },
            "bold": text.bold,
            "hint": text.hint,
        })
    }
}
