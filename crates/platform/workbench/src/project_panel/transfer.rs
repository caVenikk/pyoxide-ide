//! Copying and moving in the project tool window (#37), as the reference's
//! project view does it:
//!
//! - *Cut*, *Copy* and *Paste* (`CopyPasteDelegator`): Cut and Copy keep
//!   the selected files for Paste, cut rows are greyed; Paste into the
//!   selected folder (or the folder of the selected file) opens the Copy
//!   dialog after Copy and the Move dialog after Cut, which ends the cut.
//! - *Copy…* (F5, `CopyHandler`) and *Move…* (F6, `MoveHandler`) open those
//!   dialogs for the selection; dragging rows onto a folder moves them, with
//!   Option (macOS) or Ctrl held it copies (`ProjectViewDropTarget`).
//! - A file of the same name in the target asks *Overwrite* or *Skip* (and
//!   *for all* when there are more, `SkipOverwriteChoice`). A copied file
//!   opens in the editor when *Open copy in editor* is on.
//! - *Copy Path/Reference…* (`CopyReferencePopupGroup`) copies the
//!   absolute path, the file name or the path from the content root;
//!   *Copy Paths* (Cmd+Shift+C / Ctrl+Shift+C, `CopyPathsAction`) the
//!   absolute paths, one per line.

use std::path::{Path, PathBuf};

use gpui::{
    AppContext as _, ClipboardItem, Context, Entity, FocusHandle, Focusable as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div, px,
};
use project::fs_ops::{self, TransferItem, TransferKind};
use themes::{Metric, UiRole};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use super::file_ops::DialogPurpose;
use super::transfer_dialog::{TransferDialog, TransferDialogEvent};
use super::{PanelEvent, ProjectPanel};
use crate::dialog::{Dialog, DialogButton};

ui::ui_commands! {
    COMMANDS, scope = "ProjectTree";
    Cut { paths: Option<Vec<String>> } => "project.tree.cut", "Cut",
        "Keeps the selected files or folders (or `paths`) for Paste, which moves them.",
        r#"{"paths": ["app.py"]}"#;
    Copy { paths: Option<Vec<String>> } => "project.tree.copy", "Copy",
        "Keeps the selected files or folders (or `paths`) for Paste, which copies them.",
        r#"{"paths": ["app.py"]}"#;
    Paste { directory: Option<String> } => "project.tree.paste", "Paste",
        "Pastes what Cut or Copy kept into the selected folder (or the folder of the selected file, or `directory`) with the Move or Copy dialog.",
        r#"{"directory": "src"}"#;
    CopyElement { paths: Option<Vec<String>>, to: Option<String>, name: Option<String> } => "project.tree.copy-element", "Copy…",
        "Copies the selected files or folders (or `paths`): asks in the Copy dialog, or copies into `to` (relative to the project) with `name` for one item.",
        r#"{"paths": ["app.py"], "to": "src", "name": "main.py"}"#;
    Move { paths: Option<Vec<String>>, to: Option<String> } => "project.tree.move", "Move…",
        "Moves the selected files or folders (or `paths`): asks in the Move dialog, or moves into `to` (relative to the project). Open tabs follow.",
        r#"{"paths": ["app.py"], "to": "src"}"#;
    CopyPaths { paths: Option<Vec<String>> } => "project.tree.copy-paths", "Copy Paths",
        "Copies the absolute paths of the selected files or folders (or `paths`) to the clipboard, one per line.",
        r#"{"paths": ["app.py"]}"#;
    CopyPath { kind: String, paths: Option<Vec<String>> } => "project.tree.copy-path", "Copy Path/Reference",
        "Copies a path of the selected files or folders to the clipboard: `kind` is `absolute`, `file-name` or `content-root`.",
        r#"{"kind": "content-root"}"#;
}

/// Files kept by Cut or Copy for Paste.
#[derive(Debug, Clone)]
pub(super) struct FileClipboard {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
}

/// An open Copy or Move dialog and what it is for.
pub(super) struct OpenTransfer {
    pub dialog: Entity<TransferDialog>,
    sources: Vec<PathBuf>,
    /// Paste after Cut: the cut ends when the move is done.
    after_cut: bool,
    return_focus: Option<FocusHandle>,
    _subscription: Subscription,
}

/// A copy or move whose conflicts are being asked about.
pub(super) struct TransferRun {
    kind: TransferKind,
    items: Vec<TransferItem>,
    /// Overwrite (true) or skip, by item; asked in order.
    overwrite: Vec<Option<bool>>,
    /// Overwrite for all / Skip for all.
    all: Option<bool>,
    open_in_editor: bool,
    after_cut: bool,
}

/// A copy or move to do: what, where to, and what follows.
struct TransferRequest {
    kind: TransferKind,
    sources: Vec<PathBuf>,
    /// *To directory* as typed (absolute, or relative to the project).
    directory: String,
    /// The new name of a single item.
    name: Option<String>,
    open_in_editor: bool,
    /// Paste after Cut: the cut ends when the move is done.
    after_cut: bool,
}

/// The longest path in the dialog's bold line (the reference's
/// `MAX_PATH_LENGTH` is 70 in a wider dialog).
const LABEL_PATH: usize = 60;

/// Rows being dragged.
#[derive(Debug, Clone)]
pub struct DraggedFiles {
    pub paths: Vec<PathBuf>,
}

/// What follows the pointer while rows are dragged: the first row's icon
/// and name.
pub(super) struct DragPreview {
    name: SharedString,
    icon: IconName,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .px(px(8.0))
            .h(theme.metric(Metric::RowHeight))
            .rounded(theme.metric(Metric::CornerRadius))
            .bg(theme.color(UiRole::SurfaceRaised))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .opacity(0.9)
            .child(Icon::new(self.icon))
            .child(self.name.clone())
    }
}

impl DragPreview {
    pub(super) fn new(name: impl Into<SharedString>, icon: IconName) -> Self {
        Self {
            name: name.into(),
            icon,
        }
    }
}

/// Copy on drop: Option on macOS, Ctrl elsewhere (the platforms' copy
/// modifiers of drag and drop).
fn copy_modifier(modifiers: gpui::Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        modifiers.alt
    } else {
        modifiers.control
    }
}

/// `path` shortened from the middle to `max` characters
/// (`CopyFilesOrDirectoriesDialog.shortenPath`).
fn shorten(path: &Path, max: usize) -> String {
    let text = path.display().to_string();
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text;
    }
    let keep = max.saturating_sub(3);
    let head = keep / 2;
    let tail = keep - head;
    let mut out: String = chars[..head].iter().collect();
    out.push_str("...");
    out.extend(&chars[chars.len() - tail..]);
    out
}

impl ProjectPanel {
    /// The paths a command names (relative to the project), or the
    /// selected row's.
    fn sources(&self, paths: Option<&[String]>) -> Vec<PathBuf> {
        let (Some(root), Some(tree)) = (self.root(), &self.tree) else {
            return Vec::new();
        };
        let paths: Vec<PathBuf> = match paths {
            Some(paths) => paths.iter().filter_map(|p| self.resolve(p)).collect(),
            None => self
                .selected
                .map(|id| vec![tree.node(id).entry.path.clone()])
                .unwrap_or_default(),
        };
        fs_ops::outermost(&paths)
            .into_iter()
            .filter(|p| p != &root && p.starts_with(&root))
            .collect()
    }

    /// The row of `path` is cut (greyed until Paste moves it).
    pub(super) fn is_cut(&self, path: &Path) -> bool {
        self.ops
            .clipboard
            .as_ref()
            .is_some_and(|c| c.cut && c.paths.iter().any(|p| path.starts_with(p)))
    }

    /// The open Copy or Move dialog.
    pub fn transfer_dialog(&self) -> Option<&Entity<TransferDialog>> {
        self.ops.transfer.as_ref().map(|t| &t.dialog)
    }

    /// What Cut or Copy kept, relative to the project, and whether it was
    /// cut.
    pub(super) fn clipboard_json(&self) -> serde_json::Value {
        let Some(c) = &self.ops.clipboard else {
            return serde_json::Value::Null;
        };
        let rel = |p: &PathBuf| {
            self.filter
                .as_ref()
                .and_then(|f| f.relative(p))
                .unwrap_or_default()
        };
        serde_json::json!({
            "paths": c.paths.iter().map(rel).collect::<Vec<_>>(),
            "cut": c.cut,
        })
    }

    pub(super) fn cut(&mut self, a: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.sources(a.paths.as_deref());
        if !paths.is_empty() {
            self.ops.clipboard = Some(FileClipboard { paths, cut: true });
            cx.notify();
        }
    }

    pub(super) fn copy(&mut self, a: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.sources(a.paths.as_deref());
        if !paths.is_empty() {
            self.ops.clipboard = Some(FileClipboard { paths, cut: false });
            cx.notify();
        }
    }

    pub(super) fn paste(&mut self, a: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        let Some(clipboard) = self.ops.clipboard.clone() else {
            return;
        };
        let Some(target) = self.target_dir(a.directory.as_deref()) else {
            return;
        };
        let paths: Vec<PathBuf> = clipboard
            .paths
            .iter()
            .filter(|p| std::fs::symlink_metadata(p).is_ok())
            .cloned()
            .collect();
        if paths.is_empty() {
            self.ops.clipboard = None;
            cx.notify();
            return;
        }
        let kind = if clipboard.cut {
            TransferKind::Move
        } else {
            TransferKind::Copy
        };
        self.open_transfer(kind, paths, Some(target), clipboard.cut, window, cx);
    }

    pub(super) fn copy_element(
        &mut self,
        a: &CopyElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sources = self.sources(a.paths.as_deref());
        if sources.is_empty() {
            return;
        }
        match &a.to {
            Some(to) => self.transfer_to(
                TransferRequest {
                    kind: TransferKind::Copy,
                    sources,
                    directory: to.clone(),
                    name: a.name.clone(),
                    open_in_editor: false,
                    after_cut: false,
                },
                window,
                cx,
            ),
            None => self.open_transfer(TransferKind::Copy, sources, None, false, window, cx),
        }
    }

    pub(super) fn move_action(&mut self, a: &Move, window: &mut Window, cx: &mut Context<Self>) {
        let sources = self.sources(a.paths.as_deref());
        if sources.is_empty() {
            return;
        }
        match &a.to {
            Some(to) => self.transfer_to(
                TransferRequest {
                    kind: TransferKind::Move,
                    sources,
                    directory: to.clone(),
                    name: None,
                    open_in_editor: false,
                    after_cut: false,
                },
                window,
                cx,
            ),
            None => self.open_transfer(TransferKind::Move, sources, None, false, window, cx),
        }
    }

    pub(super) fn copy_paths(&mut self, a: &CopyPaths, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.sources_or_root(a.paths.as_deref());
        if paths.is_empty() {
            return;
        }
        let text = paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    pub(super) fn copy_path(&mut self, a: &CopyPath, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.sources_or_root(a.paths.as_deref());
        let Some(root) = self.root() else { return };
        let text: Vec<String> = paths
            .iter()
            .map(|p| match a.kind.as_str() {
                "file-name" => p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                "content-root" => p
                    .strip_prefix(&root)
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default(),
                _ => p.display().to_string(),
            })
            .collect();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text.join("\n")));
        }
    }

    /// The paths a command names, or the selected row's (the project root
    /// too).
    fn sources_or_root(&self, paths: Option<&[String]>) -> Vec<PathBuf> {
        match paths {
            Some(paths) => paths.iter().filter_map(|p| self.resolve(p)).collect(),
            None => self
                .selected_path()
                .map(|(p, _)| vec![p.to_owned()])
                .unwrap_or_default(),
        }
    }

    /// A row can be dragged (all but the project root) and takes rows
    /// dropped on it: into its folder, or the folder of its file.
    pub(super) fn drag_and_drop(
        &self,
        row: gpui::Stateful<gpui::Div>,
        id: project::NodeId,
        icon: IconName,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        use gpui::InteractiveElement as _;
        use gpui::StatefulInteractiveElement as _;
        let Some(tree) = &self.tree else { return row };
        let node = tree.node(id);
        let entry = &node.entry;
        let target = if entry.is_dir() {
            entry.path.clone()
        } else {
            entry.path.parent().map(Path::to_owned).unwrap_or_default()
        };
        let drop_bg = cx.theme().color(UiRole::Hover);
        let row = row
            .drag_over::<DraggedFiles>(move |style, _, _, _| style.bg(drop_bg))
            .on_drop(
                cx.listener(move |this, dragged: &DraggedFiles, window, cx| {
                    this.drop_files(dragged, target.clone(), window, cx)
                }),
            );
        if node.depth == 0 {
            return row;
        }
        let name: SharedString = entry.name.clone().into();
        row.on_drag(
            DraggedFiles {
                paths: vec![entry.path.clone()],
            },
            move |_, _, _, cx| cx.new(|_| DragPreview::new(name.clone(), icon)),
        )
    }

    /// Rows dropped on the folder `target` (the folder of a file row).
    pub(super) fn drop_files(
        &mut self,
        dragged: &DraggedFiles,
        target: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sources: Vec<PathBuf> = fs_ops::outermost(&dragged.paths)
            .into_iter()
            // Dropped on itself or where it is already: nothing to do.
            .filter(|p| p != &target && p.parent() != Some(target.as_path()))
            .collect();
        if sources.is_empty() || sources.iter().any(|p| target.starts_with(p)) {
            return;
        }
        let kind = if copy_modifier(window.modifiers()) {
            TransferKind::Copy
        } else {
            TransferKind::Move
        };
        self.open_transfer(kind, sources, Some(target), false, window, cx);
    }

    // ---- the dialog -----------------------------------------------------------

    /// Opens the Copy or Move dialog for `sources`, with `target` (or the
    /// folder that holds them) in *To directory*.
    fn open_transfer(
        &mut self,
        kind: TransferKind,
        sources: Vec<PathBuf>,
        target: Option<PathBuf>,
        after_cut: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(first) = sources.first().cloned() else {
            return;
        };
        let target = target
            .or_else(|| first.parent().map(Path::to_owned))
            .unwrap_or_default();
        let is_dir = |p: &Path| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir());
        let label = match (kind, sources.as_slice()) {
            (TransferKind::Copy, [one]) => {
                let what = if is_dir(one) { "directory" } else { "file" };
                format!("Copy {what} {}", shorten(one, LABEL_PATH))
            }
            (TransferKind::Move, [one]) => {
                let what = if is_dir(one) { "directory" } else { "file" };
                format!("Move {what} {}", shorten(one, LABEL_PATH))
            }
            (kind, many) => {
                let dirs = many.iter().filter(|p| is_dir(p)).count();
                let what = if dirs == 0 {
                    "files"
                } else if dirs == many.len() {
                    "directories"
                } else {
                    "files and directories"
                };
                match kind {
                    TransferKind::Copy => format!("Copy specified {what}"),
                    // `move.specified.elements` for a mix.
                    TransferKind::Move if dirs != 0 && dirs != many.len() => {
                        "Move specified elements".to_owned()
                    }
                    TransferKind::Move => format!("Move specified {what}"),
                }
            }
        };
        let name = (kind == TransferKind::Copy && sources.len() == 1).then(|| {
            (
                first
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                !is_dir(&first),
            )
        });
        // Open copy in editor is on for one file; Open in editor is off.
        let open = kind == TransferKind::Copy && sources.len() == 1 && !is_dir(&first);
        let directory = target.display().to_string();
        let dialog =
            cx.new(|cx| TransferDialog::new(kind, label, name, &directory, open, window, cx));
        self.context_menu = None;
        let return_focus = window.focused(cx).or_else(|| Some(self.focus.clone()));
        let subscription = cx.subscribe_in(&dialog, window, |this, _, event, window, cx| {
            this.transfer_event(event, window, cx)
        });
        window.focus(&dialog.focus_handle(cx));
        self.ops.transfer = Some(OpenTransfer {
            dialog,
            sources,
            after_cut,
            return_focus,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn close_transfer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = self.ops.transfer.take() {
            match open.return_focus {
                Some(focus) => window.focus(&focus),
                None => window.focus(&self.focus),
            }
        }
        cx.notify();
    }

    fn transfer_event(
        &mut self,
        event: &TransferDialogEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TransferDialogEvent::Cancelled => self.close_transfer(window, cx),
            TransferDialogEvent::Confirmed {
                name,
                directory,
                open_in_editor,
            } => {
                let Some(open) = &self.ops.transfer else {
                    return;
                };
                let kind = open.dialog.read(cx).kind();
                let (sources, after_cut) = (open.sources.clone(), open.after_cut);
                self.transfer_to(
                    TransferRequest {
                        kind,
                        sources,
                        directory: directory.clone(),
                        name: name.clone(),
                        open_in_editor: *open_in_editor,
                        after_cut,
                    },
                    window,
                    cx,
                );
            }
        }
    }

    /// Checks the target and the names in the background; then asks about
    /// conflicts and does the work. A problem with the input shows in the
    /// open dialog (or an Error dialog without one).
    fn transfer_to(
        &mut self,
        request: TransferRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.root() else { return };
        let TransferRequest {
            kind,
            sources,
            directory,
            name,
            open_in_editor,
            after_cut,
        } = request;
        let generation = self.generation;
        self.loading += 1;
        let task = cx.background_spawn(async move {
            let target = fs_ops::target_dir(&directory, &root)?;
            fs_ops::plan_transfer(kind, &sources, &target, name.as_deref(), &root)
        });
        cx.spawn_in(window, async move |this, cx| {
            let planned = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.loading -= 1;
                if this.generation != generation {
                    return;
                }
                match planned {
                    Err(e) => match &this.ops.transfer {
                        Some(open) => open
                            .dialog
                            .update(cx, |d, cx| d.set_message(Some(e.to_string().into()), cx)),
                        None => this.show_error("Error", e.to_string(), window, cx),
                    },
                    Ok(items) => {
                        this.close_transfer(window, cx);
                        let overwrite = items
                            .iter()
                            .map(|i| (!i.conflict).then_some(false))
                            .collect();
                        this.ops.run = Some(TransferRun {
                            kind,
                            items,
                            overwrite,
                            all: None,
                            open_in_editor,
                            after_cut,
                        });
                        this.continue_transfer(window, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Asks about the next conflict, or does the work when every one is
    /// decided.
    fn continue_transfer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(run) = &mut self.ops.run else { return };
        let next = run.overwrite.iter().position(Option::is_none);
        let Some(next) = next else {
            let run = self.ops.run.take().expect("a run");
            self.run_transfer(run, window, cx);
            return;
        };
        if let Some(all) = run.all {
            run.overwrite[next] = Some(all);
            self.continue_transfer(window, cx);
            return;
        }
        let item = &run.items[next];
        let many = run.items.iter().filter(|i| i.conflict).count() > 1;
        let title = match run.kind {
            TransferKind::Copy => "Copy Files/Directories",
            TransferKind::Move => "Move",
        };
        let name = item
            .to
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let folder = item
            .to
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let message = format!("File '{name}' already exists in directory '{folder}'");
        let mut buttons = Vec::new();
        if many {
            buttons.push(DialogButton {
                id: "skip-all",
                label: "Skip for all".into(),
            });
            buttons.push(DialogButton {
                id: "overwrite-all",
                label: "Overwrite for all".into(),
            });
        }
        buttons.push(DialogButton {
            id: "skip",
            label: "Skip".into(),
        });
        buttons.push(DialogButton {
            id: "overwrite",
            label: "Overwrite".into(),
        });
        let dialog = cx.new(|cx| {
            Dialog::new(title, message, buttons, cx).with_icon(IconName::QUESTION_DIALOG)
        });
        self.open_dialog(dialog, DialogPurpose::Conflict(next), window, cx);
    }

    /// The person answered the question about the conflict of item
    /// `index` (Escape skips).
    pub(super) fn conflict_answered(
        &mut self,
        index: usize,
        button: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(run) = &mut self.ops.run else { return };
        let overwrite = matches!(button, Some("overwrite" | "overwrite-all"));
        if matches!(button, Some("overwrite-all" | "skip-all")) {
            run.all = Some(overwrite);
        }
        if let Some(slot) = run.overwrite.get_mut(index) {
            *slot = Some(overwrite);
        }
        self.continue_transfer(window, cx);
    }

    /// Copies or moves the planned items in the background.
    fn run_transfer(&mut self, run: TransferRun, window: &mut Window, cx: &mut Context<Self>) {
        let generation = self.generation;
        self.loading += 1;
        let TransferRun {
            kind,
            items,
            overwrite,
            open_in_editor,
            after_cut,
            ..
        } = run;
        let task = cx.background_spawn(async move {
            let mut changes = Vec::new();
            let mut done = Vec::new();
            for (item, overwrite) in items.iter().zip(overwrite) {
                let overwrite = overwrite.unwrap_or(false);
                if item.conflict && !overwrite {
                    continue;
                }
                let result = match kind {
                    TransferKind::Copy => fs_ops::copy_item(item, overwrite),
                    TransferKind::Move => fs_ops::move_item(item, overwrite),
                };
                match result {
                    Ok(c) => {
                        changes.extend(c);
                        done.push((item.to.clone(), item.is_dir));
                    }
                    Err(e) => {
                        let what = match kind {
                            TransferKind::Copy => "copy",
                            TransferKind::Move => "move",
                        };
                        let error = format!("Cannot {what} '{}': {e}", item.from.display());
                        return (changes, done, Some(error));
                    }
                }
            }
            (changes, done, None)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (changes, done, error) = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.loading -= 1;
                if this.generation != generation {
                    return;
                }
                this.apply_changes(&changes, cx);
                if kind == TransferKind::Move && after_cut && error.is_none() {
                    this.ops.clipboard = None;
                }
                // The first copied or moved item is selected; with Open in
                // editor, the first file opens.
                if let Some((first, _)) = done.first() {
                    this.reveal(first, cx);
                }
                if open_in_editor && let Some((file, _)) = done.iter().find(|(_, dir)| !dir) {
                    cx.emit(PanelEvent::OpenFile(file.clone()));
                }
                if let Some(error) = error {
                    this.show_error("Error", error, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_shortened_in_the_middle() {
        assert_eq!(shorten(Path::new("/a/b.py"), 70), "/a/b.py");
        let long = format!("/{}/end.py", "x".repeat(100));
        let short = shorten(Path::new(&long), 20);
        assert_eq!(short.chars().count(), 20);
        assert!(short.starts_with("/xxxxxx") && short.ends_with("end.py"));
        assert!(short.contains("..."));
    }
}
