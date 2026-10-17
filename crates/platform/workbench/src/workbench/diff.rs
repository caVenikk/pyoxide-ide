//! Diff tabs (#171): the diff of a file against its committed version in
//! an editor tab (`vcs.show-diff`), opened from the Commit tool window
//! (Enter, double-click, Show Diff), the change marker popup (Show Diff)
//! and the project tool window.
//!
//! The left side is the file's text in `HEAD` (the base text the change
//! markers use); the right side is the file's document: the editor of a
//! [`FilePreview`] that shows the file's tab text and edits it (linked with
//! the tab), or, without a tab, edits and saves its own document, which a
//! tab opened later continues with. A file of the Commit tool window's
//! list opens in the list's preview tab (`Commit: main.py`), with the
//! list's other files a step away (Compare Next and Previous File).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use editor::{Editor, EditorConfig};
use gpui::{
    App, AppContext as _, Context, Entity, EntityId, Focusable as _, InteractiveElement,
    SharedString, Subscription, Task, Window,
};
use ui::icons::IconName;
use vcs::Cancel;
use vcs_ui::OpenDocument;
use vcs_ui::diff::{DiffEvent, DiffRequest, DiffView, ScrollTo, ShowDiff, SideTitle};

use super::{Tab, TabContent, Workbench};
use crate::file_preview::{FilePreview, FilePreviewEvent, PreviewOptions};
use crate::tab_view::{TabView, downcast};

/// The title of a diff of the Commit tool window's list
/// (`ChangesViewDiffPreviewProcessor`).
fn commit_title(path: &Path) -> String {
    format!("Commit: {}", file_name(path))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The right side's title (`merge.version.title.current`).
const CURRENT_VERSION: &str = "Current version";

impl TabView for DiffView {
    fn tab_title(&self, _: &App) -> SharedString {
        self.request()
            .map(|r| r.title.clone())
            .unwrap_or_else(|| "Diff".into())
    }

    fn tab_icon(&self, _: &App) -> IconName {
        IconName::DIFF
    }

    fn tab_tooltip(&self, _: &App) -> Option<SharedString> {
        self.request().map(|r| r.path.display().to_string().into())
    }

    fn is_preview(&self, _: &App) -> bool {
        self.request().is_some_and(|r| r.preview)
    }

    fn tab_state(&self, cx: &App) -> serde_json::Value {
        let mut state = self.state_json(cx);
        state["kind"] = "diff".into();
        state
    }
}

/// The diff tabs of the window.
#[derive(Default)]
pub(super) struct DiffTabs {
    tabs: HashMap<EntityId, DiffTab>,
}

/// A diff tab's state.
struct DiffTab {
    view: Entity<DiffView>,
    /// The right side's document.
    document: Entity<FilePreview>,
    /// The files Compare Next and Previous File step through, and the one
    /// shown.
    files: Vec<PathBuf>,
    index: usize,
    /// The list is the Commit tool window's.
    commit: bool,
    /// The file being loaded: its base text and its document.
    loading: Option<Loading>,
    _subscriptions: Vec<Subscription>,
}

struct Loading {
    path: PathBuf,
    scroll: ScrollTo,
    base: Option<Option<Arc<str>>>,
    document: bool,
    _task: Option<Task<()>>,
}

impl Workbench {
    /// The diff tab of `view`.
    fn diff_tab(&self, id: EntityId) -> Option<&DiffTab> {
        self.diff_tabs.tabs.get(&id)
    }

    /// The index of a tab's view.
    fn view_tab(&self, id: EntityId) -> Option<usize> {
        self.tabs
            .iter()
            .position(|t| t.view().is_some_and(|v| v.entity_id() == id))
    }

    /// The diff of the active tab.
    pub fn active_diff(&self) -> Option<Entity<DiffView>> {
        let view = self.tabs.get(self.active?)?.view()?;
        downcast::<DiffView>(view).cloned()
    }

    // ---- opening ----------------------------------------------------------------

    /// `vcs.show-diff`: the file at `path`, or the selection of the focused
    /// Commit tool window or project tool window, or the active file.
    pub(super) fn vcs_show_diff(
        &mut self,
        action: &ShowDiff,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = match &action.path {
            Some(path) => Some(self.diff_path(path, cx)),
            None => self.diff_context_file(window, cx),
        };
        if let Some(path) = path {
            self.show_diff_of(path, ScrollTo::FirstChange, window, cx);
        }
    }

    /// A path of a command: relative to the repository's root (or to the
    /// project without one), or absolute.
    fn diff_path(&self, path: &str, cx: &App) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            return p.to_owned();
        }
        match &self.version_control.snapshot {
            Some(s) => {
                let mut out = s.info.root.clone();
                out.extend(path.split('/').filter(|s| !s.is_empty()));
                out
            }
            None => self.resolve(path, cx),
        }
    }

    fn diff_context_file(&self, window: &Window, cx: &App) -> Option<PathBuf> {
        let commit = self.commit_ui.view.read(cx);
        if commit.contains_focus(window, cx) {
            return commit.selected_files().into_iter().next();
        }
        let panel = self.panel.read(cx);
        if panel.focus_handle(cx).contains_focused(window, cx)
            && let Some((path, false)) = panel.selected_path()
        {
            return Some(path.to_owned());
        }
        self.active_file(cx)
    }

    /// Opens the diff of `path`. A file of the Commit tool window's list
    /// opens in the list's preview tab with the list (the selected files
    /// when there are several, else the files of its group); another file
    /// in a tab of its own.
    pub(super) fn show_diff_of(
        &mut self,
        path: PathBuf,
        scroll: ScrollTo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let commit = self.commit_ui.view.read(cx);
        let tree = commit.tree();
        let (files, index, commit_list) = match tree.group_of(&path) {
            Some(group) => {
                let selected = commit.selected_files();
                let files = if selected.len() > 1 && selected.contains(&path) {
                    selected
                } else {
                    tree.files(group)
                };
                let index = files.iter().position(|p| *p == path).unwrap_or(0);
                (files, index, true)
            }
            None => (vec![path.clone()], 0, false),
        };
        // The list's preview tab, or the file's own diff tab.
        let existing = self.diff_tabs.tabs.iter().find_map(|(id, t)| {
            let same = if commit_list {
                t.commit
            } else {
                !t.commit && t.files.get(t.index) == Some(&path)
            };
            same.then_some(*id)
        });
        let id = match existing {
            Some(id) => id,
            None => self.new_diff_tab(commit_list, window, cx),
        };
        if let Some(t) = self.diff_tabs.tabs.get_mut(&id) {
            t.files = files;
            t.index = index;
        }
        if let Some(i) = self.view_tab(id) {
            self.activate(i, window, cx);
        }
        self.load_diff(id, scroll, cx);
    }

    /// A new diff tab after the others, its document linked with the
    /// tabs of the files it shows.
    fn new_diff_tab(
        &mut self,
        commit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> EntityId {
        let options = PreviewOptions {
            editable: true,
            soft_wrap_button: false,
            empty_text: "".into(),
            name: "diff",
        };
        let host = cx.entity().downgrade();
        let document = cx.new(|cx| {
            let mut p = FilePreview::new(options, self.file_types.clone(), window, cx);
            p.set_host(host);
            p
        });
        let editor = document.read(cx).editor().clone();
        let view = cx.new(|cx| DiffView::new(editor, window, cx));
        let id = view.entity_id();
        let subscriptions = vec![
            cx.subscribe_in(&view, window, move |this, _, event, window, cx| {
                this.diff_event(id, event, window, cx)
            }),
            cx.subscribe(&document, move |this, _, event: &FilePreviewEvent, cx| {
                this.diff_document_event(id, event, cx)
            }),
            cx.observe(&view, |_, _, cx| cx.notify()),
            // The title's check box follows the Commit tool window.
            cx.observe(&self.commit_ui.view, move |this, _, cx| {
                this.diff_sync_included(id, cx)
            }),
        ];
        self.diff_tabs.tabs.insert(
            id,
            DiffTab {
                view: view.clone(),
                document,
                files: Vec::new(),
                index: 0,
                commit,
                loading: None,
                _subscriptions: subscriptions,
            },
        );
        self.tabs.push(Tab {
            content: TabContent::View(Box::new(view)),
            _subscriptions: Vec::new(),
        });
        id
    }

    /// Reads the shown file's base text and document; the diff shows them
    /// once both are there.
    fn load_diff(&mut self, id: EntityId, scroll: ScrollTo, cx: &mut Context<Self>) {
        let Some(t) = self.diff_tabs.tabs.get(&id) else {
            return;
        };
        let Some(path) = t.files.get(t.index).cloned() else {
            return;
        };
        let document = t.document.clone();
        // The base: the one the file's tab compares with, else read.
        let tab_base = self
            .vcs_line_status(&path)
            .and_then(|s| s.read(cx).base().cloned());
        let snapshot = self.version_control.snapshot.clone();
        let base_of = snapshot
            .as_deref()
            .and_then(|s| super::version_control::base_of(s, &path));
        let (base, task) = match (tab_base, base_of, self.git_repo()) {
            (Some(base), _, _) => (Some(Some(base)), None),
            (None, Some((_, base_path)), Some(repo)) => {
                let read = cx.background_spawn(async move {
                    use vcs::Backend as _;
                    repo.base_text(&base_path, &Cancel::new()).ok().flatten()
                });
                let task = cx.spawn(async move |this, cx| {
                    let base = read.await;
                    let _ = this.update(cx, |this, cx| this.diff_base_read(id, base, cx));
                });
                (None, Some(task))
            }
            // Added, unversioned or not in a repository: nothing on the left.
            _ => (Some(None), None),
        };
        let t = self.diff_tabs.tabs.get_mut(&id).expect("the tab");
        t.loading = Some(Loading {
            path: path.clone(),
            scroll,
            base,
            document: false,
            _task: task,
        });
        // The document: the file's tab text, or the file read (a deleted
        // file has none).
        let shown = if path.is_file() {
            let tab = self.editor_for(&path);
            document.update(cx, |d, cx| d.show_file_of_tab(path.clone(), tab, cx))
        } else {
            document.update(cx, |d, cx| d.clear(cx));
            true
        };
        if shown {
            self.diff_document_ready(id, &path, cx);
        }
    }

    fn diff_base_read(&mut self, id: EntityId, base: Option<Arc<str>>, cx: &mut Context<Self>) {
        let Some(l) = self
            .diff_tabs
            .tabs
            .get_mut(&id)
            .and_then(|t| t.loading.as_mut())
        else {
            return;
        };
        l.base = Some(base);
        self.diff_loaded(id, cx);
    }

    fn diff_document_event(
        &mut self,
        id: EntityId,
        event: &FilePreviewEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            FilePreviewEvent::Loaded(path) => self.diff_document_ready(id, path, cx),
            FilePreviewEvent::Message(text) => {
                self.message = Some(text.clone().into());
                cx.notify();
            }
            FilePreviewEvent::Edited(_) => self.note_input(cx),
        }
    }

    fn diff_document_ready(&mut self, id: EntityId, path: &Path, cx: &mut Context<Self>) {
        let Some(l) = self
            .diff_tabs
            .tabs
            .get_mut(&id)
            .and_then(|t| t.loading.as_mut())
            .filter(|l| l.path == path)
        else {
            return;
        };
        l.document = true;
        self.diff_loaded(id, cx);
    }

    /// Both sides are read: the diff shows them.
    fn diff_loaded(&mut self, id: EntityId, cx: &mut Context<Self>) {
        let Some(t) = self.diff_tabs.tabs.get_mut(&id) else {
            return;
        };
        let ready = t
            .loading
            .as_ref()
            .is_some_and(|l| l.document && l.base.is_some());
        if !ready {
            return;
        }
        let loading = t.loading.take().expect("loading");
        let base = loading.base.flatten();
        let request = self.diff_request(id, &loading.path, base.is_some(), cx);
        let file_type = self.file_types.for_path(&loading.path).cloned();
        let highlighter = file_type
            .as_ref()
            .and_then(|t| t.highlighter.as_ref())
            .map(|factory| factory());
        let config = file_type
            .as_ref()
            .map(|t| t.editor_config())
            .unwrap_or_else(EditorConfig::default);
        let Some(t) = self.diff_tabs.tabs.get(&id) else {
            return;
        };
        let view = t.view.clone();
        view.update(cx, |v, cx| {
            v.set_request(request, base, highlighter, config, loading.scroll, cx)
        });
        cx.notify();
    }

    /// The titles of the shown file's diff.
    fn diff_request(&self, id: EntityId, path: &Path, has_base: bool, cx: &App) -> DiffRequest {
        let t = &self.diff_tabs.tabs[&id];
        let snapshot = self.version_control.snapshot.as_deref();
        let rel = |p: &Path| -> String {
            self.project(cx)
                .and_then(|pr| pr.relative(p))
                .map(|r| r.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string())
        };
        let base_path = snapshot
            .and_then(|s| s.status.old_path(path))
            .unwrap_or(path)
            .to_owned();
        let left = if has_base {
            SideTitle {
                label: snapshot
                    .and_then(|s| s.info.head.oid())
                    .map(vcs_ui::widget::short_hash)
                    .unwrap_or_default()
                    .into(),
                path: Some(rel(&base_path).into()),
                tooltip: Some(base_path.display().to_string().into()),
                read_only: true,
            }
        } else {
            SideTitle::default()
        };
        let renamed = base_path != path;
        let right = SideTitle {
            label: CURRENT_VERSION.into(),
            path: renamed.then(|| rel(path).into()),
            tooltip: renamed.then(|| path.display().to_string().into()),
            read_only: !path.is_file(),
        };
        let tree = self.commit_ui.view.read(cx).tree();
        let included = tree.group_of(path).map(|_| tree.is_included(path));
        DiffRequest {
            path: path.to_owned(),
            title: if t.commit {
                commit_title(path)
            } else {
                file_name(path)
            }
            .into(),
            left,
            right,
            included,
            files: (t.index, t.files.len()),
            preview: t.commit,
        }
    }

    // ---- what the diff asks for -------------------------------------------------

    fn diff_event(
        &mut self,
        id: EntityId,
        event: &DiffEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(t) = self.diff_tab(id) else {
            return;
        };
        let Some(path) = t.files.get(t.index).cloned() else {
            return;
        };
        match event {
            DiffEvent::JumpToSource { line, column } => {
                let point = text::Point::new(*line as usize, *column as usize);
                self.open_file_at(path, point, None, window, cx);
            }
            DiffEvent::NextFile => self.diff_step(id, true, ScrollTo::FirstChange, window, cx),
            DiffEvent::PreviousFile { last_change } => {
                let scroll = if *last_change {
                    ScrollTo::LastChange
                } else {
                    ScrollTo::FirstChange
                };
                self.diff_step(id, false, scroll, window, cx)
            }
            DiffEvent::SetIncluded(included) => {
                let rel = self
                    .version_control
                    .snapshot
                    .as_ref()
                    .map(|s| git::commands::relative_path(&s.info.root, &path))
                    .unwrap_or_else(|| path.display().to_string());
                window.dispatch_action(
                    Box::new(vcs_ui::commit::SetIncluded {
                        paths: Some(vec![rel]),
                        included: *included,
                    }),
                    cx,
                );
            }
            DiffEvent::Changed => self.diff_document_status(id, cx),
        }
    }

    /// Compare Next (Previous) File.
    fn diff_step(
        &mut self,
        id: EntityId,
        next: bool,
        scroll: ScrollTo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(t) = self.diff_tabs.tabs.get_mut(&id) else {
            return;
        };
        let index = if next {
            t.index + 1
        } else {
            match t.index.checked_sub(1) {
                Some(i) => i,
                None => return,
            }
        };
        if index >= t.files.len() {
            return;
        }
        let old = t.files.get(t.index).cloned();
        t.index = index;
        let view = t.view.clone();
        // The Commit tool window selects the file shown.
        let select = t.files.get(index).cloned().filter(|_| t.commit);
        if let Some(old) = old {
            self.diff_forget_document(&old, cx);
        }
        if let Some(path) = select {
            self.commit_ui
                .view
                .update(cx, |v, cx| v.select_path(&path, cx));
        }
        window.focus(&view.focus_handle(cx));
        self.load_diff(id, scroll, cx);
    }

    /// The title's check box follows the file's inclusion.
    fn diff_sync_included(&mut self, id: EntityId, cx: &mut Context<Self>) {
        let Some(t) = self.diff_tab(id) else {
            return;
        };
        let Some(path) = t.files.get(t.index) else {
            return;
        };
        let tree = self.commit_ui.view.read(cx).tree();
        let included = tree.group_of(path).map(|_| tree.is_included(path));
        let view = t.view.clone();
        view.update(cx, |v, cx| v.set_included(included, cx));
    }

    /// The colour of a file the diff edits without a tab: modified at once
    /// when its text differs from the base (as a tab's editor makes it).
    fn diff_document_status(&mut self, id: EntityId, cx: &mut Context<Self>) {
        let Some(t) = self.diff_tab(id) else {
            return;
        };
        let Some(path) = t.files.get(t.index).cloned() else {
            return;
        };
        if self.file_tab(&path).is_some() {
            return;
        }
        let view = t.view.read(cx);
        let document = OpenDocument {
            unsaved: view
                .editor(vcs_ui::diff::Side::Right)
                .read(cx)
                .is_modified(),
            changed: Some(!view.changes().is_empty()),
        };
        if self
            .version_control
            .markers
            .colors
            .set_document(&path, Some(document))
        {
            self.panel.update(cx, |_, cx| cx.notify());
            cx.notify();
        }
    }

    /// The diff shows another file or closed: without a tab the file has
    /// no open document any more.
    fn diff_forget_document(&mut self, path: &Path, cx: &mut Context<Self>) {
        if self.file_tab(path).is_none()
            && self.version_control.markers.colors.set_document(path, None)
        {
            self.panel.update(cx, |_, cx| cx.notify());
        }
    }

    // ---- the workbench's hooks ----------------------------------------------------

    /// A view tab was closed.
    pub(super) fn view_tab_closed(&mut self, id: EntityId, cx: &mut Context<Self>) {
        if let Some(t) = self.diff_tabs.tabs.remove(&id) {
            // Its own document is written first.
            t.document.update(cx, |d, cx| d.save(cx));
            if let Some(path) = t.files.get(t.index) {
                self.diff_forget_document(path, cx);
            }
        }
    }

    /// Every tab was closed (another project).
    pub(super) fn diff_tabs_cleared(&mut self, cx: &mut Context<Self>) {
        for (_, t) in self.diff_tabs.tabs.drain() {
            t.document.update(cx, |d, cx| d.save(cx));
        }
    }

    /// A file's tab opened: a diff of the file edits the tab's text from
    /// now on.
    pub(super) fn diff_tab_opened(
        &mut self,
        path: &Path,
        editor: &Entity<Editor>,
        cx: &mut Context<Self>,
    ) {
        for t in self.diff_tabs.tabs.values() {
            t.document
                .update(cx, |d, cx| d.tab_opened(path, editor, cx));
        }
    }

    /// A file's tab closed: a diff of the file keeps its text as its own
    /// document (saved by the tab first).
    pub(super) fn diff_file_tab_closed(&mut self, path: &Path, cx: &mut Context<Self>) {
        let ids: Vec<EntityId> = self
            .diff_tabs
            .tabs
            .iter()
            .filter(|(_, t)| t.files.get(t.index).is_some_and(|p| p == path))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            let t = &self.diff_tabs.tabs[&id];
            let document = t.document.clone();
            document.update(cx, |d, cx| d.tab_closed(path, cx));
        }
    }

    /// The text a diff edits of `path` without a tab, for a tab that opens
    /// the file (the tab continues with it and the diff follows the tab).
    pub(super) fn diff_document(&self, path: &Path, cx: &App) -> Option<text::Buffer> {
        self.diff_tabs
            .tabs
            .values()
            .find_map(|t| t.document.read(cx).document(path, cx))
    }

    /// Writes the diffs' own documents (Save All).
    pub(super) fn diff_save_documents(&mut self, cx: &mut Context<Self>) {
        for t in self.diff_tabs.tabs.values() {
            t.document.update(cx, |d, cx| d.save(cx));
        }
    }

    /// A file was renamed or moved: the diffs' lists follow.
    pub(super) fn diff_files_moved(&mut self, from: &Path, to: &Path, _: &mut Context<Self>) {
        for t in self.diff_tabs.tabs.values_mut() {
            for p in &mut t.files {
                if let Ok(rest) = p.strip_prefix(from) {
                    *p = if rest.as_os_str().is_empty() {
                        to.to_owned()
                    } else {
                        to.join(rest)
                    };
                }
            }
        }
    }

    /// The diff commands reach the active diff when the focus is elsewhere.
    pub(super) fn diff_actions<E: InteractiveElement>(&self, root: E, cx: &mut Context<Self>) -> E {
        let root = root.on_action(cx.listener(Self::vcs_show_diff));
        match self.active_diff() {
            Some(view) => vcs_ui::diff::view::forward_actions(root, view),
            None => root,
        }
    }

    /// The diff tab of the active tab as JSON.
    pub(super) fn diff_state_json(&self, cx: &App) -> serde_json::Value {
        match self.active_diff() {
            Some(view) => view.read(cx).state_json(cx),
            None => serde_json::Value::Null,
        }
    }
}
