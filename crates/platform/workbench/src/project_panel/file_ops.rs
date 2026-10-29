//! File operations of the project tool window (#37), as the reference
//! IDE's project view does them:
//!
//! - *New* (`NewElementAction`, Cmd+N / Alt+Insert): a popup with *File*
//!   and *Directory*, also the first row of the context menu. *File*
//!   (`CreateFileAction`) and *Directory* (`CreateDirectoryOrPackageAction`)
//!   ask for a name in a light popup; `a/b/c.py` creates the folders on the
//!   way. The folder is the selected one, or the folder of the selected
//!   file. A new file is selected in the tree and opened in the editor, a
//!   new folder is selected with the focus in the tree
//!   (`IdeViewForProjectViewPane.selectElement`).
//! - *Rename…* (`RenameElementAction`, Shift+F6): the Rename dialog; a
//!   plain rename of the file or folder.
//! - *Delete…* (`DeleteAction`, Delete / Cmd+Backspace): asks, then moves
//!   to the system trash where it can (`DeleteHandler`).
//!
//! The work on disk runs in the background (`project::fs_ops`); the tree
//! changes in place and [`PanelEvent::FilesChanged`] tells the workbench
//! (editor tabs, file index, language services).

use std::path::{Path, PathBuf};

use gpui::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, Focusable as _, IntoElement,
    ParentElement as _, Pixels, Point, Styled as _, Subscription, Window, anchored, div, point, px,
};
use project::fs_ops::{self, FsChange, NewKind};
use project::{Entry, EntryKind, FileTree, NodeId};
use themes::Metric;
use ui::deferred;
use ui::icons::IconName;
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::theme::ThemeExt;

use super::name_popup::{Message, NamePopup, NamePopupEvent, NamePopupKind};
use super::{ICON_X, PanelEvent, ProjectPanel, TEXT_X, TREE_LEFT};
use crate::dialog::{Dialog, DialogButton, DialogEvent};

ui::ui_commands! {
    COMMANDS, scope = "ProjectTree";
    NewElement => "project.tree.new", "New…",
        "Opens the New popup for the selected folder (or the folder of the selected file): File, Directory.";
    NewFile { name: Option<String>, directory: Option<String> } => "project.tree.new-file", "Create New File",
        "Creates an empty file in the selected folder (or the folder of the selected file, or `directory`, relative to the project): asks for its name in the New File popup, or takes `name` (`a/b.py` creates the folders on the way). The file is selected and opens in the editor.",
        r#"{"name": "src/app.py"}"#;
    NewDirectory { name: Option<String>, directory: Option<String> } => "project.tree.new-directory", "Create New Directory",
        "Creates a folder in the selected folder (or the folder of the selected file, or `directory`): asks for its name in the New Directory popup, or takes `name` (`a/b` creates both).",
        r#"{"name": "docs"}"#;
    Rename { path: Option<String>, name: Option<String> } => "project.tree.rename", "Rename…",
        "Renames the selected file or folder (or `path`, relative to the project): asks for the new name in the Rename dialog, or takes `name`. Open tabs follow.",
        r#"{"path": "app.py", "name": "main.py"}"#;
    ReloadFromDisk { paths: Option<Vec<String>> } => "project.tree.reload-from-disk", "Reload from Disk",
        "Looks at the selected files and folders (or `paths`, relative to the project) on disk again: open files changed outside are reloaded, the folders listed again.",
        r#"{"paths": ["src"]}"#;
    Delete { paths: Option<Vec<String>> } => "project.tree.delete", "Delete",
        "Asks, then deletes the selected file or folder (or `paths`, relative to the project), to the system trash where it can. Tabs of deleted files close.",
        r#"{"paths": ["app.py"]}"#;
}

/// What an open name popup is for.
pub(super) enum Purpose {
    New {
        dir: PathBuf,
        kind: NewKind,
    },
    /// A product's item of New (its index).
    Template {
        dir: PathBuf,
        item: usize,
    },
    Rename {
        path: PathBuf,
    },
}

struct OpenPopup {
    popup: Entity<NamePopup>,
    purpose: Purpose,
    return_focus: Option<FocusHandle>,
    _subscription: Subscription,
}

/// Why an operation did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum OpError {
    /// The typed name: shown in the open popup, as the reference checks it.
    Name(String),
    /// The file system refused: a dialog with the reference's title
    /// ("Cannot Create File", "Rename", "Error").
    Io {
        title: &'static str,
        message: String,
    },
}

/// What an open message dialog is for.
pub(super) enum DialogPurpose {
    /// Paths to delete, the outermost ones.
    Delete(Vec<PathBuf>),
    /// A copy or move asks about the conflict of its item.
    Conflict(usize),
    /// An error to read.
    Error,
}

struct OpenDialog {
    dialog: Entity<Dialog>,
    purpose: DialogPurpose,
    return_focus: Option<FocusHandle>,
    _subscription: Subscription,
}

/// The file operations' state of a project tool window.
pub(super) struct FileOps {
    popup: Option<OpenPopup>,
    dialog: Option<OpenDialog>,
    /// Deleted files go to the system trash (the reference's "Move deleted
    /// files to the trash/bin", on by default).
    to_trash: bool,
    /// Changes with every check of a typed name, so a late answer is
    /// dropped.
    pub(super) check: u64,
    /// The product's rows of New.
    pub(super) new_items: Vec<super::NewItem>,
    /// What Cut or Copy kept for Paste.
    pub(super) clipboard: Option<super::transfer::FileClipboard>,
    /// The open Copy or Move dialog.
    pub(super) transfer: Option<super::transfer::OpenTransfer>,
    /// A copy or move asking about its conflicts.
    pub(super) run: Option<super::transfer::TransferRun>,
    /// The row dragged rows are over (drawn as the drop target).
    pub(super) drop_target: Option<NodeId>,
}

impl Default for FileOps {
    fn default() -> Self {
        Self {
            popup: None,
            dialog: None,
            to_trash: true,
            check: 0,
            new_items: Vec::new(),
            clipboard: None,
            transfer: None,
            run: None,
            drop_target: None,
        }
    }
}

impl FileOps {
    /// Closes the popup and the dialog (another project opens).
    pub(super) fn close(&mut self) {
        self.popup = None;
        self.dialog = None;
        self.clipboard = None;
        self.transfer = None;
        self.run = None;
        self.drop_target = None;
    }
}

impl ProjectPanel {
    /// Deleted files go to the system trash (on by default); off, they are
    /// deleted for good (tests).
    pub fn set_delete_to_trash(&mut self, on: bool) {
        self.ops.to_trash = on;
    }

    /// The open New File, New Directory or Rename popup.
    pub fn name_popup(&self) -> Option<&Entity<NamePopup>> {
        self.ops.popup.as_ref().map(|p| &p.popup)
    }

    /// The open Delete (or error) dialog.
    pub fn dialog(&self) -> Option<&Entity<Dialog>> {
        self.ops.dialog.as_ref().map(|d| &d.dialog)
    }

    /// The popup and dialog over the window, while open. The main window
    /// draws them (also while the tool window is hidden), outside the
    /// tree's key context, so the tree's keys do not reach the tree.
    pub fn overlays(&self, window: &Window) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if let Some(open) = &self.ops.popup {
            out.push(super::name_popup::overlay(&open.popup));
        }
        if let Some(open) = &self.ops.transfer {
            out.push(
                deferred(
                    anchored()
                        .position(point(px(0.0), px(0.0)))
                        .child(open.dialog.clone()),
                )
                .into_any_element(),
            );
        }
        if let Some(open) = &self.ops.dialog {
            let size = window.viewport_size();
            out.push(
                deferred(
                    anchored().position(point(px(0.0), px(0.0))).child(
                        div()
                            .relative()
                            .w(size.width)
                            .h(size.height)
                            .child(open.dialog.clone()),
                    ),
                )
                .into_any_element(),
            );
        }
        out
    }

    pub(super) fn root(&self) -> Option<PathBuf> {
        self.project.as_ref().map(|p| p.root().to_owned())
    }

    /// A path a command names, relative to the project (or absolute
    /// inside it).
    pub(super) fn resolve(&self, path: &str) -> Option<PathBuf> {
        let root = self.root()?;
        let p = Path::new(path);
        let p = if p.is_absolute() {
            p.to_owned()
        } else {
            native_path::join(&root, p)
        };
        p.starts_with(&root).then_some(p)
    }

    /// The folder new files go to: `directory`, else the selected folder,
    /// else the folder of the selected file (`IdeView.getDirectories`).
    pub(super) fn target_dir(&self, directory: Option<&str>) -> Option<PathBuf> {
        if let Some(d) = directory {
            return self.resolve(d);
        }
        let (path, is_dir) = self.selected_path()?;
        if is_dir {
            Some(path.to_owned())
        } else {
            path.parent().map(Path::to_owned)
        }
    }

    /// The rows of *New* in the context menu and the New popup: File,
    /// Directory, then the product's.
    fn new_items(&self, dir: Option<String>) -> Vec<MenuItem> {
        let product = self.product_new_items(dir.clone());
        let mut items = vec![
            MenuItem::action(
                "File",
                NewFile {
                    name: None,
                    directory: dir.clone(),
                },
            )
            .icon(IconName::FILE_TEXT),
            MenuItem::action(
                "Directory",
                NewDirectory {
                    name: None,
                    directory: dir,
                },
            )
            .icon(IconName::FOLDER),
        ];
        items.extend(product);
        items
    }

    /// *New* (a submenu) and a separator, the first rows of the context
    /// menu of `id`.
    pub(super) fn new_menu_items(&self, id: NodeId) -> Vec<MenuItem> {
        let Some(tree) = &self.tree else {
            return Vec::new();
        };
        let entry = &tree.node(id).entry;
        let dir = if entry.is_dir() {
            entry.path.clone()
        } else {
            entry.path.parent().map(Path::to_owned).unwrap_or_default()
        };
        let rel = self
            .root()
            .and_then(|r| dir.strip_prefix(&r).ok().map(|p| p.to_owned()))
            .map(|p| p.to_string_lossy().replace('\\', "/"));
        let paths = self
            .filter
            .as_ref()
            .and_then(|f| f.relative(&entry.path))
            .map(|r| vec![r]);
        let not_root = paths.is_some();
        let copy_path = |label: &'static str, kind: &str| {
            MenuItem::action(
                label,
                super::transfer::CopyPath {
                    kind: kind.to_owned(),
                    paths: paths.clone(),
                },
            )
        };
        let copy_path_items = vec![
            copy_path("Absolute Path", "absolute"),
            copy_path("File Name", "file-name"),
            copy_path("Path from Content Root", "content-root").enabled(not_root),
        ];
        vec![
            MenuItem::submenu("New", self.new_items(rel.clone())),
            MenuItem::separator(),
            MenuItem::action(
                "Cut",
                super::transfer::Cut {
                    paths: paths.clone(),
                },
            )
            .icon(IconName::CUT)
            .enabled(not_root),
            MenuItem::action(
                "Copy",
                super::transfer::Copy {
                    paths: paths.clone(),
                },
            )
            .icon(IconName::COPY)
            .enabled(not_root),
            MenuItem::submenu("Copy Path/Reference…", copy_path_items),
            MenuItem::action("Paste", super::transfer::Paste { directory: rel })
                .icon(IconName::PASTE)
                .enabled(self.ops.clipboard.is_some()),
            MenuItem::separator(),
        ]
    }

    /// *Rename…*, *Refactor* (*Rename…*, *Move…*, *Copy…*) and *Delete…*
    /// with their separators, after Find in Files in the context menu of
    /// `id`; the project root has none of them to choose.
    pub(super) fn edit_menu_items(&self, id: NodeId) -> Vec<MenuItem> {
        let (Some(tree), Some(filter)) = (&self.tree, &self.filter) else {
            return Vec::new();
        };
        let entry = &tree.node(id).entry;
        let reload = |paths: Option<Vec<String>>| {
            MenuItem::action("Reload from Disk", ReloadFromDisk { paths }).icon(IconName::REFRESH)
        };
        let open = open_menu_items(&entry.path, entry.is_dir());
        let Some(rel) = filter.relative(&entry.path) else {
            let mut items = vec![
                MenuItem::separator(),
                MenuItem::unavailable("Rename…"),
                MenuItem::submenu("Refactor", Vec::new()).enabled(false),
                MenuItem::separator(),
                MenuItem::unavailable("Delete…"),
            ];
            items.extend(open);
            items.extend([
                MenuItem::separator(),
                // The project root: everything.
                reload(Some(vec![String::new()])),
            ]);
            return items;
        };
        let rename = || {
            MenuItem::action(
                "Rename…",
                Rename {
                    path: Some(rel.clone()),
                    name: None,
                },
            )
        };
        let refactor = vec![
            rename(),
            MenuItem::separator(),
            MenuItem::action(
                "Move…",
                super::transfer::Move {
                    paths: Some(vec![rel.clone()]),
                    to: None,
                },
            ),
            MenuItem::action(
                "Copy…",
                super::transfer::CopyElement {
                    paths: Some(vec![rel.clone()]),
                    to: None,
                    name: None,
                },
            ),
        ];
        let mut items = vec![
            MenuItem::separator(),
            rename(),
            MenuItem::submenu("Refactor", refactor),
            MenuItem::separator(),
            MenuItem::action(
                "Delete…",
                Delete {
                    paths: Some(vec![rel.clone()]),
                },
            ),
        ];
        items.extend(open);
        items.extend([MenuItem::separator(), reload(Some(vec![rel]))]);
        items
    }

    /// Below the selected row, where its name starts (window coordinates).
    fn below_selected(&self, cx: &gpui::App) -> Option<Point<Pixels>> {
        let tree = self.tree.as_ref()?;
        let id = self.selected?;
        let row = tree.row_of(id)?;
        let theme = cx.theme();
        let row_height = theme.metric(Metric::RowHeight);
        let indent = theme.metric(Metric::TreeIndent) * tree.node(id).depth as f32;
        let x = px(TREE_LEFT + ICON_X + TEXT_X) + indent;
        let y = row_height * (row + 1) as f32 - self.scroll_top();
        Some(self.expansion.view().origin + point(x, y))
    }

    // ---- commands ---------------------------------------------------------

    pub(super) fn new_element(
        &mut self,
        _: &NewElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dir) = self.target_dir(None) else {
            return;
        };
        let rel = self
            .root()
            .and_then(|r| dir.strip_prefix(&r).ok().map(|p| p.to_owned()))
            .map(|p| p.to_string_lossy().replace('\\', "/"));
        let Some(position) = self.below_selected(cx) else {
            return;
        };
        window.focus(&self.focus);
        let menu = ContextMenu::popup(
            self.new_items(rel),
            PopupOptions {
                title: Some("New".into()),
                speed_search: true,
                ..PopupOptions::default()
            },
            window,
            cx,
        );
        let dismissed = cx.subscribe_in(&menu, window, |this, _, _: &gpui::DismissEvent, _, cx| {
            this.context_menu = None;
            cx.notify();
        });
        self.context_menu = Some((menu, position, dismissed));
        cx.notify();
    }

    pub(super) fn new_file(&mut self, a: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        self.start_new(
            NewKind::File,
            a.name.clone(),
            a.directory.as_deref(),
            window,
            cx,
        );
    }

    pub(super) fn new_directory(
        &mut self,
        a: &NewDirectory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_new(
            NewKind::Directory,
            a.name.clone(),
            a.directory.as_deref(),
            window,
            cx,
        );
    }

    fn start_new(
        &mut self,
        kind: NewKind,
        name: Option<String>,
        directory: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.filter.is_none() {
            return;
        }
        let Some(dir) = self.target_dir(directory) else {
            return;
        };
        if let Some(name) = name {
            self.create(dir, kind, name, window, cx);
            return;
        }
        let title = match kind {
            NewKind::File => "New File",
            NewKind::Directory => "New Directory",
        };
        let popup = cx.new(|cx| NamePopup::new_item(title, window, cx));
        self.open_popup(popup, Purpose::New { dir, kind }, window, cx);
    }

    pub(super) fn rename_action(
        &mut self,
        a: &Rename,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, is_dir)) = self.target(a.path.as_deref()) else {
            return;
        };
        let Some(root) = self.root() else { return };
        if path == root || !path.starts_with(&root) {
            return;
        }
        if let Some(name) = &a.name {
            self.rename_to(path, name.clone(), window, cx);
            return;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // A file's name is selected without its extension
        // (`NameSuggestionsField.selectNameWithoutExtension`).
        let chars = name.chars().count();
        let selected = match name.rfind('.') {
            Some(dot) if !is_dir && dot > 0 => 0..name[..dot].chars().count(),
            _ => 0..chars,
        };
        let kind = if is_dir { "directory" } else { "file" };
        let label = format!("Rename {kind} '{name}' to:");
        let popup = cx.new(|cx| NamePopup::rename(label, &name, selected, window, cx));
        self.open_popup(popup, Purpose::Rename { path }, window, cx);
    }

    pub(super) fn delete_action(
        &mut self,
        a: &Delete,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(root), Some(tree)) = (self.root(), &self.tree) else {
            return;
        };
        let paths: Vec<PathBuf> = match &a.paths {
            Some(paths) => paths.iter().filter_map(|p| self.resolve(p)).collect(),
            None => self
                .selected
                .map(|id| vec![tree.node(id).entry.path.clone()])
                .unwrap_or_default(),
        };
        let paths: Vec<PathBuf> = fs_ops::outermost(&paths)
            .into_iter()
            .filter(|p| p != &root && p.starts_with(&root))
            .collect();
        if paths.is_empty() {
            return;
        }
        let items: Vec<(PathBuf, bool)> = paths
            .iter()
            .map(|p| {
                let dir = match tree.find(p) {
                    Some(id) => tree.node(id).entry.is_dir() && !tree.node(id).entry.symlink,
                    None => std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()),
                };
                (p.clone(), dir)
            })
            .collect();
        let message = fs_ops::delete_prompt(&items);
        let dialog = cx.new(|cx| {
            Dialog::new(
                "Delete",
                message,
                vec![
                    DialogButton {
                        id: "cancel",
                        label: "Cancel".into(),
                    },
                    DialogButton {
                        id: "delete",
                        label: "Delete".into(),
                    },
                ],
                cx,
            )
            .with_icon(IconName::QUESTION_DIALOG)
        });
        self.open_dialog(dialog, DialogPurpose::Delete(paths), window, cx);
    }

    /// Reload from Disk (`SynchronizeCurrentFileAction`): the workbench
    /// looks at the open files under the paths again; the tree lists their
    /// folders again.
    pub(super) fn reload_from_disk(
        &mut self,
        a: &ReloadFromDisk,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(root), Some(tree)) = (self.root(), &self.tree) else {
            return;
        };
        let paths: Vec<PathBuf> = match &a.paths {
            Some(paths) => paths.iter().filter_map(|p| self.resolve(p)).collect(),
            None => self
                .selected
                .map(|id| vec![tree.node(id).entry.path.clone()])
                .unwrap_or_else(|| vec![root.clone()]),
        };
        if paths.is_empty() {
            return;
        }
        self.refresh_paths(&paths, cx);
        cx.emit(PanelEvent::ReloadFromDisk(paths));
    }

    /// Lists again the listed folders that are `paths` or inside them, and
    /// the folders that hold the paths.
    pub fn refresh_paths(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        let Some(tree) = &self.tree else { return };
        let dirs: Vec<NodeId> = tree
            .listed_dirs()
            .into_iter()
            .filter(|&id| {
                let dir = &tree.node(id).entry.path;
                paths
                    .iter()
                    .any(|p| dir.starts_with(p) || p.parent() == Some(dir.as_path()))
            })
            .collect();
        for id in dirs {
            self.list(id, cx);
        }
    }

    // ---- popups and dialogs -------------------------------------------------

    pub(super) fn open_popup(
        &mut self,
        popup: Entity<NamePopup>,
        purpose: Purpose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.context_menu = None;
        let return_focus = window.focused(cx).or_else(|| Some(self.focus.clone()));
        let subscription = cx.subscribe_in(&popup, window, |this, popup, event, window, cx| {
            this.popup_event(popup.clone(), event, window, cx)
        });
        window.focus(&popup.focus_handle(cx));
        self.ops.popup = Some(OpenPopup {
            popup,
            purpose,
            return_focus,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(super) fn close_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = self.ops.popup.take() {
            match open.return_focus {
                Some(focus) => window.focus(&focus),
                None => window.focus(&self.focus),
            }
        }
        cx.notify();
    }

    fn popup_event(
        &mut self,
        popup: Entity<NamePopup>,
        event: &NamePopupEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = &self.ops.popup else { return };
        if open.popup != popup {
            return;
        }
        match event {
            NamePopupEvent::Cancelled => self.close_popup(window, cx),
            NamePopupEvent::Changed(text) => match &open.purpose {
                Purpose::New {
                    dir,
                    kind: NewKind::Directory,
                } => {
                    let dir = dir.clone();
                    self.check_new_directory(dir, text.clone(), popup, cx)
                }
                Purpose::New { .. } => {}
                Purpose::Template { dir, item } => {
                    if self.is_package(*item) {
                        let dir = dir.clone();
                        self.check_new_package(dir, text.clone(), popup, cx)
                    }
                }
                Purpose::Rename { .. } => {
                    let name = text.trim();
                    let message = (!fs_ops::valid_file_name(name)).then(|| Message {
                        text: fs_ops::NameError::NotValid(name.to_owned())
                            .to_string()
                            .into(),
                        warning: false,
                    });
                    popup.update(cx, |p, cx| {
                        p.set_can_confirm(message.is_none(), cx);
                        p.set_message(message, cx);
                    });
                }
            },
            NamePopupEvent::Confirmed(text) => match &open.purpose {
                Purpose::New { dir, kind } => {
                    let (dir, kind) = (dir.clone(), *kind);
                    self.create(dir, kind, text.clone(), window, cx)
                }
                Purpose::Template { dir, item } => {
                    let (dir, item) = (dir.clone(), *item);
                    let kind = popup.read(cx).selected_kind();
                    self.create_from_template(dir, item, kind, text.clone(), window, cx)
                }
                Purpose::Rename { path } => {
                    let path = path.clone();
                    self.rename_to(path, text.clone(), window, cx)
                }
            },
        }
    }

    /// The New Directory popup checks the name as it is typed and shows an
    /// error or a warning (`CreateDirectoryOrPackageAction`).
    fn check_new_directory(
        &mut self,
        dir: PathBuf,
        text: String,
        popup: Entity<NamePopup>,
        cx: &mut Context<Self>,
    ) {
        let (Some(root), Some(filter)) = (self.root(), self.filter.clone()) else {
            return;
        };
        self.ops.check += 1;
        let check = self.ops.check;
        let task = cx.background_spawn(async move {
            if text.is_empty() {
                return None;
            }
            match fs_ops::check_new(&dir, &text, NewKind::Directory, &root, &filter) {
                Err(e) => Some(Message {
                    text: e.to_string().into(),
                    warning: false,
                }),
                Ok(_) => fs_ops::new_directory_warning(&text, &filter).map(|w| Message {
                    text: w.to_string().into(),
                    warning: true,
                }),
            }
        });
        cx.spawn(async move |this, cx| {
            let message = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.ops.check == check {
                    popup.update(cx, |p, cx| p.set_message(message, cx));
                }
            });
        })
        .detach();
    }

    pub(super) fn open_dialog(
        &mut self,
        dialog: Entity<Dialog>,
        purpose: DialogPurpose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.context_menu = None;
        let return_focus = window.focused(cx).or_else(|| Some(self.focus.clone()));
        let subscription = cx.subscribe_in(&dialog, window, |this, _, event, window, cx| {
            let DialogEvent::Closed { button, .. } = event;
            this.dialog_closed(*button, window, cx);
        });
        window.focus(&dialog.focus_handle(cx));
        self.ops.dialog = Some(OpenDialog {
            dialog,
            purpose,
            return_focus,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn dialog_closed(
        &mut self,
        button: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.ops.dialog.take() else {
            return;
        };
        match open.return_focus {
            Some(focus) => window.focus(&focus),
            None => window.focus(&self.focus),
        }
        match (open.purpose, button) {
            (DialogPurpose::Delete(paths), Some("delete")) => self.delete(paths, window, cx),
            (DialogPurpose::Conflict(index), button) => {
                self.conflict_answered(index, button, window, cx)
            }
            _ => {}
        }
        cx.notify();
    }

    /// An error in a dialog titled `title` (the reference's
    /// `Messages.showErrorDialog`), with OK.
    pub(super) fn show_error(
        &mut self,
        title: &'static str,
        message: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dialog = cx.new(|cx| {
            Dialog::new(
                title,
                message,
                vec![DialogButton {
                    id: "ok",
                    label: "OK".into(),
                }],
                cx,
            )
            .with_icon(IconName::WARNING_DIALOG)
        });
        self.open_dialog(dialog, DialogPurpose::Error, window, cx);
    }

    // ---- the work -----------------------------------------------------------

    /// Checks `name` again and creates the file or folder in the
    /// background; an error goes into the open popup (or a dialog).
    fn create(
        &mut self,
        dir: PathBuf,
        kind: NewKind,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(root), Some(filter)) = (self.root(), self.filter.clone()) else {
            return;
        };
        let generation = self.generation;
        self.loading += 1;
        let task = cx.background_spawn(async move {
            let target = fs_ops::check_new(&dir, &name, kind, &root, &filter)
                .map_err(|e| OpError::Name(e.to_string()))?;
            let changes = fs_ops::create(&target).map_err(|e| {
                // `CreateFileAction.getErrorTitle`; a folder's error is
                // `CommonBundle.getErrorTitle`.
                let (title, what) = match kind {
                    NewKind::File => ("Cannot Create File", "file"),
                    NewKind::Directory => ("Error", "directory"),
                };
                OpError::Io {
                    title,
                    message: format!("Cannot create {what} '{}': {e}", target.path.display()),
                }
            })?;
            Ok::<_, OpError>((target.path, changes))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.loading -= 1;
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok((path, changes)) => {
                        this.close_popup(window, cx);
                        this.apply_changes(&changes, cx);
                        this.reveal(&path, cx);
                        match kind {
                            // Selected in the tree, opened in the editor
                            // (which takes the focus).
                            NewKind::File => cx.emit(PanelEvent::OpenFile(path)),
                            NewKind::Directory => window.focus(&this.focus),
                        }
                    }
                    Err(message) => this.operation_failed(message, window, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn rename_to(
        &mut self,
        path: PathBuf,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.root() else { return };
        let generation = self.generation;
        self.loading += 1;
        let task = cx.background_spawn(async move {
            fs_ops::check_rename(&path, &name, &root).map_err(|e| OpError::Name(e.to_string()))?;
            // `RenameUtil.showErrorMessage`: titled Rename.
            fs_ops::rename(&path, &name, &root).map_err(|e| OpError::Io {
                title: "Rename",
                message: e.to_string(),
            })
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.loading -= 1;
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(change) => {
                        this.close_popup(window, cx);
                        if let Some(change) = change {
                            this.apply_changes(&[change], cx);
                        }
                    }
                    Err(message) => this.operation_failed(message, window, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn delete(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root() else { return };
        let generation = self.generation;
        let to_trash = self.ops.to_trash;
        self.loading += 1;
        let task = cx.background_spawn(async move {
            let mut changes = Vec::new();
            for path in paths {
                let is_dir = std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir());
                match fs_ops::delete(&path, &root, to_trash) {
                    Ok(_) => changes.push(FsChange::Removed { path, is_dir }),
                    Err(e) => {
                        let error = format!("Cannot delete '{}': {e}", path.display());
                        return (changes, Some(error));
                    }
                }
            }
            (changes, None)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (changes, error) = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.loading -= 1;
                if this.generation != generation {
                    return;
                }
                this.apply_changes(&changes, cx);
                if let Some(error) = error {
                    this.show_error("Error", error, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// An operation failed: a name error shows in the open popup, a file
    /// system error in a dialog (the Rename dialog closes first, as the
    /// reference runs the refactoring after closing it; the New popups stay
    /// behind the error).
    pub(super) fn operation_failed(
        &mut self,
        error: OpError,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match (error, self.ops.popup.as_ref()) {
            (OpError::Name(message), Some(open)) => open.popup.update(cx, |p, cx| {
                if p.kind() == NamePopupKind::Rename {
                    p.set_can_confirm(true, cx);
                }
                p.set_message(
                    Some(Message {
                        text: message.into(),
                        warning: false,
                    }),
                    cx,
                )
            }),
            (OpError::Name(message), None) => self.show_error("Error", message, window, cx),
            (OpError::Io { title, message }, popup) => {
                if popup.is_some_and(|p| p.popup.read(cx).kind() == NamePopupKind::Rename) {
                    self.close_popup(window, cx);
                }
                self.show_error(title, message, window, cx)
            }
        }
    }

    /// Shows changes made on disk in the tree, keeps the stored exclusions
    /// and roots on renamed and deleted folders, and tells the workbench.
    pub fn apply_changes(&mut self, changes: &[FsChange], cx: &mut Context<Self>) {
        if changes.is_empty() {
            return;
        }
        let (Some(tree), Some(filter)) = (&mut self.tree, self.filter.clone()) else {
            return;
        };
        let selected_row = self.selected.and_then(|s| tree.row_of(s));
        let mut exclusions = filter.exclusions().clone();
        let mut exclusions_changed = false;
        for change in changes {
            match change {
                FsChange::Created { path, is_dir } => {
                    insert(tree, &filter, path, *is_dir);
                }
                FsChange::Removed { path, .. } => {
                    if let Some(id) = tree.find(path) {
                        tree.remove(id);
                    }
                    if let Some(rel) = filter.relative(path) {
                        let below = format!("{rel}/");
                        let before = exclusions.paths.len();
                        let gone: Vec<String> = exclusions
                            .paths
                            .iter()
                            .filter(|p| **p == rel || p.starts_with(&below))
                            .cloned()
                            .collect();
                        for p in gone {
                            exclusions.remove(&p);
                        }
                        exclusions_changed |= exclusions.paths.len() != before;
                    }
                }
                FsChange::Renamed { from, to, is_dir } => {
                    let parent = to.parent().and_then(|p| tree.find(p));
                    match (tree.find(from), parent) {
                        (Some(id), Some(parent)) => tree.rename(id, to, parent),
                        (Some(id), None) => tree.remove(id),
                        (None, Some(_)) => insert(tree, &filter, to, *is_dir),
                        (None, None) => {}
                    }
                    if let (Some(old), Some(new)) = (filter.relative(from), filter.relative(to)) {
                        let below = format!("{old}/");
                        let moved: Vec<String> = exclusions
                            .paths
                            .iter()
                            .filter(|p| **p == old || p.starts_with(&below))
                            .cloned()
                            .collect();
                        for p in moved {
                            exclusions.remove(&p);
                            exclusions.add(&format!("{new}{}", &p[old.len()..]));
                            exclusions_changed = true;
                        }
                    }
                }
            }
        }
        // A deleted row's place goes to the row after it (or the last).
        if self.selected.is_some_and(|s| !alive(tree, s)) {
            let rows = tree.rows();
            self.selected = selected_row
                .map(|r| rows[r.min(rows.len() - 1)])
                .or(Some(FileTree::ROOT));
        }
        if exclusions_changed {
            self.set_exclusions(exclusions, cx);
        } else {
            tree.refresh_excluded(&filter);
        }
        // Roots follow their folders; packages and modules show as such.
        self.follow_roots(changes, cx);
        let dirs = self.folder_kind_targets(changes);
        self.refresh_folder_kinds(dirs, cx);
        cx.emit(PanelEvent::FilesChanged(changes.to_vec()));
        cx.notify();
    }
}

/// The node is still in the tree (not removed).
fn alive(tree: &FileTree, id: NodeId) -> bool {
    let path = &tree.node(id).entry.path;
    !path.as_os_str().is_empty()
}

/// Adds a created file or folder to its listed parent, unless the rules
/// hide it.
fn insert(tree: &mut FileTree, filter: &project::Filter, path: &Path, is_dir: bool) {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return;
    };
    let name = name.to_string_lossy().into_owned();
    if filter.is_hidden(&name) {
        return;
    }
    let Some(parent) = tree.find(parent) else {
        return;
    };
    tree.insert(
        parent,
        Entry {
            name,
            path: path.to_owned(),
            kind: if is_dir {
                EntryKind::Dir
            } else {
                EntryKind::File
            },
            symlink: false,
            excluded: filter.is_excluded(path, is_dir),
            // Looked at in the background (`refresh_folder_kinds`).
            folder: project::FolderKind::Plain,
        },
    );
}

/// *Open in Right Split* (a file) and *Open In* (*Finder*, *Terminal*),
/// the group after the run rows of the reference's Project view menu
/// (`OpenInRightSplit`, `RevealGroup`), with the separator before it.
fn open_menu_items(path: &std::path::Path, is_dir: bool) -> Vec<MenuItem> {
    use crate::commands::{OpenInRightSplit, OpenInTerminal, RevealFile};
    let path = Some(path.display().to_string());
    let mut items = vec![MenuItem::separator()];
    if !is_dir {
        items.push(
            MenuItem::action(
                "Open in Right Split",
                OpenInRightSplit { path: path.clone() },
            )
            .icon(IconName::SPLIT_VERTICALLY),
        );
    }
    items.push(MenuItem::submenu(
        "Open In",
        vec![
            MenuItem::action(
                crate::workbench::tab_menu::FILE_MANAGER,
                RevealFile { path: path.clone() },
            ),
            MenuItem::action("Terminal", OpenInTerminal { path }),
        ],
    ));
    items
}
