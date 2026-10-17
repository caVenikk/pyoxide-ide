//! Change markers of the editor tabs and file status colours (#169): every
//! tab gets a [`LineStatus`] whose base text is the file's text in `HEAD`
//! (read when the repository's statuses come and again when `HEAD` moves),
//! the marker popup opens from a marker or from Next / Previous Change, and
//! the statuses colour the names in the project tree, the tabs, Recent
//! Files and Go to File.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use editor::{Editor, EditorEvent};
use gpui::{
    AnyElement, AppContext as _, ClipboardItem, Context, Div, Entity, EntityId, InteractiveElement,
    ParentElement as _, Pixels, Point, Styled as _, Subscription, Task, Window, div, px,
};
use vcs::{Cancel, FileStatus, Range, Snapshot};
use vcs_ui::markers::{next_range, previous_range, range_at_line};
use vcs_ui::{
    CloseMarkerPopup, CopyChangeLines, FileColors, LineStatus, LineStatusEvent, MarkerPopup,
    NextChange, OpenDocument, PopupAction, PreviousChange, RollbackLines, ShowCurrentChange,
    ShowDiffLines,
};

use crate::workbench::Workbench;

/// The change markers of a tab.
struct Tracked {
    path: PathBuf,
    status: Entity<LineStatus>,
    /// What the base text was read for: `HEAD`'s commit and the path in it.
    base_of: Option<BaseOf>,
    _loading: Option<Task<()>>,
    _events: Subscription,
}

/// The marker popup while it shows.
struct OpenPopup {
    /// Counts the popups, so that a late close of one leaves the next.
    serial: u64,
    editor: Entity<Editor>,
    status: Entity<LineStatus>,
    view: Entity<MarkerPopup>,
    /// The caret when it opened: a caret move closes it.
    caret: usize,
    /// Where it was drawn last.
    drawn_at: Cell<Option<Point<Pixels>>>,
    _events: [Subscription; 2],
}

/// The change markers of the window's tabs and the status colours.
#[derive(Default)]
pub(in crate::workbench) struct ChangeMarkers {
    /// Shared with the project tree, Recent Files and Go to File.
    pub(in crate::workbench) colors: FileColors,
    trackers: HashMap<EntityId, Tracked>,
    popup: Option<OpenPopup>,
    serial: u64,
}

/// What a file's base text is read for: `HEAD`'s commit and the path the
/// file has there (the old path of a renamed file).
type BaseOf = (String, PathBuf);

/// The base of a file. Unversioned, ignored and added files have none.
fn base_of(snapshot: &Snapshot, path: &Path) -> Option<BaseOf> {
    if !path.starts_with(&snapshot.info.root) {
        return None;
    }
    let oid = snapshot.info.head.oid()?;
    match snapshot.status.status(path) {
        FileStatus::Unversioned | FileStatus::Ignored | FileStatus::Added => None,
        _ => Some((
            oid.to_owned(),
            snapshot.status.old_path(path).unwrap_or(path).to_owned(),
        )),
    }
}

impl Workbench {
    /// An editor tab was opened: its change markers start; their base text
    /// comes with the repository's statuses.
    pub(in crate::workbench) fn vcs_tab_opened(
        &mut self,
        path: &Path,
        editor: &Entity<Editor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let status = cx.new(|cx| LineStatus::new(editor, cx));
        let events = cx.subscribe_in(&status, window, |this, status, event, window, cx| {
            this.vcs_line_status_event(status, event, window, cx)
        });
        let unsaved = editor.read(cx).is_modified();
        let m = &mut self.version_control.markers;
        m.colors.set_document(
            path,
            Some(OpenDocument {
                unsaved,
                changed: None,
            }),
        );
        m.trackers.insert(
            editor.entity_id(),
            Tracked {
                path: path.to_owned(),
                status,
                base_of: None,
                _loading: None,
                _events: events,
            },
        );
        // Once the tab is in the list.
        cx.defer_in(window, |this, _, cx| this.vcs_load_bases(cx));
    }

    /// A tab was closed: its markers stop.
    pub(in crate::workbench) fn vcs_tab_closed(
        &mut self,
        editor: &Entity<Editor>,
        cx: &mut Context<Self>,
    ) {
        let m = &mut self.version_control.markers;
        if let Some(t) = m.trackers.remove(&editor.entity_id())
            && !self.tabs.iter().any(|tab| tab.path == t.path)
        {
            m.colors.set_document(&t.path, None);
        }
        if m.popup.as_ref().is_some_and(|p| p.editor == *editor) {
            self.vcs_close_popup(cx);
        }
        self.vcs_colors_changed(cx);
    }

    /// The statuses changed: names are coloured again and base texts are
    /// read where `HEAD` or a file's status changed what they are.
    pub(in crate::workbench) fn vcs_statuses_changed(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.version_control.snapshot.clone();
        self.version_control.markers.colors.set_snapshot(snapshot);
        self.vcs_load_bases(cx);
        self.vcs_colors_changed(cx);
    }

    /// Another project: no statuses and no base texts until its
    /// repository is found.
    pub(in crate::workbench) fn vcs_markers_reset(&mut self, cx: &mut Context<Self>) {
        self.vcs_close_popup(cx);
        let m = &mut self.version_control.markers;
        m.colors.set_snapshot(None);
        for t in m.trackers.values_mut() {
            t.base_of = None;
            t._loading = None;
            t.status.update(cx, |s, cx| s.set_base(None, cx));
        }
        self.vcs_colors_changed(cx);
    }

    /// Reads the base texts that are not known yet, off the UI thread.
    /// Without a repository the bases are left as they are.
    fn vcs_load_bases(&mut self, cx: &mut Context<Self>) {
        let (Some(snapshot), Some(repo)) = (self.version_control.snapshot.clone(), self.git_repo())
        else {
            return;
        };
        let wanted: Vec<(EntityId, PathBuf, bool, Option<BaseOf>)> = self
            .tabs
            .iter()
            .map(|tab| {
                (
                    tab.editor.entity_id(),
                    tab.path.clone(),
                    tab.editor.read(cx).is_modified(),
                    base_of(&snapshot, &tab.path),
                )
            })
            .collect();
        let m = &mut self.version_control.markers;
        for (id, path, unsaved, want) in wanted {
            let Some(t) = m.trackers.get_mut(&id) else {
                continue;
            };
            // The file was renamed or moved, or saved.
            if t.path != path {
                m.colors.set_document(&t.path, None);
                t.path = path.clone();
            }
            let doc = OpenDocument {
                unsaved,
                changed: t.status.read(cx).changed(),
            };
            m.colors.set_document(&path, Some(doc));
            if t.base_of == want {
                continue;
            }
            t.base_of = want.clone();
            let Some((_, base_path)) = want else {
                t._loading = None;
                t.status.update(cx, |s, cx| s.set_base(None, cx));
                continue;
            };
            let repo = repo.clone();
            let read = cx.background_spawn(async move {
                use vcs::Backend as _;
                repo.base_text(&base_path, &Cancel::new())
            });
            let status = t.status.downgrade();
            t._loading = Some(cx.spawn(async move |_, cx| {
                let base = read.await.ok().flatten();
                let _ = status.update(cx, |s, cx| s.set_base(base, cx));
            }));
        }
    }

    /// Sets the text an open file's change markers compare with, as the
    /// repository's statuses do (for a file without a repository, tests
    /// and tools); `None` removes the markers.
    pub fn vcs_set_base_text(&mut self, path: &Path, base: Option<&str>, cx: &mut Context<Self>) {
        let Some(editor) = self.editor_for(path) else {
            return;
        };
        if let Some(t) = self
            .version_control
            .markers
            .trackers
            .get_mut(&editor.entity_id())
        {
            t.base_of = base.map(|_| (String::new(), path.to_owned()));
            t._loading = None;
            let base: Option<Arc<str>> = base.map(Arc::from);
            t.status.update(cx, |s, cx| s.set_base(base, cx));
        }
    }

    /// The change markers of an open file.
    pub fn vcs_line_status(&self, path: &Path) -> Option<Entity<LineStatus>> {
        let editor = self.editor_for(path)?;
        self.version_control
            .markers
            .trackers
            .get(&editor.entity_id())
            .map(|t| t.status.clone())
    }

    /// The views that colour names are drawn again.
    fn vcs_colors_changed(&mut self, cx: &mut Context<Self>) {
        self.panel.update(cx, |_, cx| cx.notify());
        self.recent_files.update(cx, |_, cx| cx.notify());
        self.search_everywhere.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    fn vcs_line_status_event(
        &mut self,
        status: &Entity<LineStatus>,
        event: &LineStatusEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            LineStatusEvent::Changed => {
                let s = status.read(cx);
                let doc = OpenDocument {
                    unsaved: s.unsaved(cx),
                    changed: s.changed(),
                };
                let path = s.editor().and_then(|e| e.read(cx).path().cloned());
                if let Some(path) = path
                    && self
                        .version_control
                        .markers
                        .colors
                        .set_document(&path, Some(doc))
                {
                    self.vcs_colors_changed(cx);
                }
                cx.notify();
            }
            LineStatusEvent::MarkerClicked { index, line } => {
                self.vcs_show_popup(status.clone(), *index, *line, window, cx)
            }
        }
    }

    // ---- the popup ------------------------------------------------------------

    /// Shows the popup of a range under document line `line`.
    fn vcs_show_popup(
        &mut self,
        status: Entity<LineStatus>,
        index: usize,
        line: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let s = status.read(cx);
        let (Some(range), Some(base), Some(editor)) = (
            s.ranges().get(index).copied(),
            s.base().cloned(),
            s.editor(),
        ) else {
            return;
        };
        let enabled = [
            previous_range(s.ranges(), range.line1).is_some(),
            next_range(s.ranges(), range.line1).is_some(),
            true,
            // Comes with the diff viewer.
            false,
            range.vcs_line1 < range.vcs_line2,
        ];
        let e = editor.read(cx);
        let buffer = e.buffer();
        let doc = (range.line1 as usize..range.line2 as usize)
            .filter(|&l| l < buffer.len_lines())
            .map(|l| buffer.line_text(l).into_owned())
            .collect::<Vec<_>>()
            .join("\n");
        let caret = e.selections().primary().head;
        let highlighter = e
            .path()
            .and_then(|p| self.file_types.for_path(p))
            .and_then(|t| t.highlighter.as_ref())
            .map(|factory| factory());
        let text = vcs_ui::popup_text(&base, &doc, &range, highlighter);
        let doc_start = buffer.line_start((range.line1 as usize).min(buffer.len_lines()));
        let highlights = vcs_ui::doc_highlights(&text, &doc, doc_start, &range, cx);
        self.vcs_close_popup(cx);
        editor.update(cx, |e, cx| e.set_diff_highlights(highlights, cx));
        let view =
            cx.new(|cx| MarkerPopup::new(editor.clone(), range, line, text, enabled, window, cx));
        let actions = cx.subscribe_in(
            &view,
            window,
            |this, _, action: &PopupAction, window, cx| this.vcs_popup_action(*action, window, cx),
        );
        // A change of the text, of the caret or of the selection closes it
        // (`LineStatusMarkerPopupService`).
        let closes = cx.subscribe(&editor, |this, editor, event: &EditorEvent, cx| {
            let m = &mut this.version_control.markers;
            let Some(p) = &m.popup else {
                return;
            };
            let close = match event {
                EditorEvent::Edited(_) | EditorEvent::Reloaded => true,
                EditorEvent::SelectionsChanged => {
                    let primary = *editor.read(cx).selections().primary();
                    primary.head != p.caret || !primary.is_empty()
                }
                _ => false,
            };
            if close {
                this.vcs_close_popup(cx);
            }
        });
        let m = &mut self.version_control.markers;
        m.serial += 1;
        m.popup = Some(OpenPopup {
            serial: m.serial,
            editor,
            status,
            view,
            caret,
            drawn_at: Cell::new(None),
            _events: [actions, closes],
        });
        cx.notify();
    }

    /// Closes the marker popup; the editor's highlights of its change go.
    pub(in crate::workbench) fn vcs_close_popup(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.version_control.markers.popup.take() {
            p.editor.update(cx, |e, cx| e.set_diff_highlights(None, cx));
            cx.notify();
        }
    }

    fn vcs_popup_action(
        &mut self,
        action: PopupAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(p) = &self.version_control.markers.popup else {
            return;
        };
        let status = p.status.clone();
        let range = p.view.read(cx).range;
        match action {
            PopupAction::Previous | PopupAction::Next => {
                let ranges = status.read(cx).ranges();
                let target = if action == PopupAction::Next {
                    next_range(ranges, range.line1)
                } else {
                    previous_range(ranges, range.line1)
                };
                if let Some(i) = target {
                    self.vcs_go_to_change(&status, i, window, cx);
                }
            }
            PopupAction::Rollback => {
                if let Some(i) = status.read(cx).find(&range) {
                    status.update(cx, |s, cx| s.rollback(&[i], cx));
                }
            }
            PopupAction::ShowDiff => {}
            PopupAction::Copy => Self::vcs_copy(&status, &range, cx),
        }
    }

    /// Moves the caret to a range and shows its popup under the caret's
    /// line (`scrollAndShow`).
    fn vcs_go_to_change(
        &mut self,
        status: &Entity<LineStatus>,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(line) = status.update(cx, |s, cx| s.go_to(index, cx)) {
            self.vcs_show_popup(status.clone(), index, line, window, cx);
        }
    }

    fn vcs_copy(status: &Entity<LineStatus>, range: &Range, cx: &mut Context<Self>) {
        if range.vcs_line1 == range.vcs_line2 {
            return;
        }
        if let Some(text) = status.read(cx).base_lines(range) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// The popup at its place over the active editor.
    pub(in crate::workbench) fn vcs_render_popup(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let p = self.version_control.markers.popup.as_ref()?;
        if self.active_editor() != Some(&p.editor) {
            return None;
        }
        let serial = p.serial;
        let this = cx.entity().downgrade();
        let close = move |_: &mut Window, cx: &mut gpui::App| {
            let _ = this.update(cx, |w, cx| {
                if w.version_control
                    .markers
                    .popup
                    .as_ref()
                    .is_some_and(|p| p.serial == serial)
                {
                    w.vcs_close_popup(cx);
                }
            });
        };
        let (element, at) = match vcs_ui::render_popup(&p.view, close, window, cx) {
            Some((element, at)) => (Some(element), Some(at)),
            None => (None, None),
        };
        // The editor scrolls to the line in its next frame: the popup
        // follows in the frame after it.
        if p.drawn_at.replace(at) != at {
            window.request_animation_frame();
        }
        element
    }

    /// The marker popup's key context around the active editor while it
    /// shows.
    pub(in crate::workbench) fn vcs_editor_container(
        &self,
        container: Div,
        editor: &Entity<Editor>,
    ) -> Div {
        match &self.version_control.markers.popup {
            Some(p) if p.editor == *editor => div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .key_context(vcs_ui::marker_popup::CONTEXT)
                .child(container),
            _ => container,
        }
    }

    /// The marker popup as JSON (`workbench.state`).
    pub(in crate::workbench) fn vcs_popup_json(&self, cx: &gpui::App) -> serde_json::Value {
        match &self.version_control.markers.popup {
            Some(p) => p.view.read(cx).state_json(cx),
            None => serde_json::Value::Null,
        }
    }

    // ---- commands -------------------------------------------------------------

    /// Adds the handlers of the `vcs.*` commands to the window's root.
    pub(in crate::workbench) fn vcs_actions<E: InteractiveElement>(
        &self,
        root: E,
        cx: &mut Context<Self>,
    ) -> E {
        root.on_action(cx.listener(Self::vcs_next_change))
            .on_action(cx.listener(Self::vcs_previous_change))
            .on_action(cx.listener(Self::vcs_show_current_change))
            .on_action(cx.listener(Self::vcs_rollback_lines))
            .on_action(cx.listener(Self::vcs_show_diff_lines))
            .on_action(cx.listener(Self::vcs_copy_change_lines))
            .on_action(cx.listener(Self::vcs_close_marker_popup))
    }

    /// The active editor and its markers.
    fn vcs_active(&self) -> Option<(Entity<Editor>, Entity<LineStatus>)> {
        let editor = self.active_editor()?.clone();
        let status = self
            .version_control
            .markers
            .trackers
            .get(&editor.entity_id())?
            .status
            .clone();
        Some((editor, status))
    }

    /// The range of the popup over `editor`, when it shows.
    fn vcs_popup_range(&self, editor: &Entity<Editor>, cx: &gpui::App) -> Option<Range> {
        match &self.version_control.markers.popup {
            Some(p) if p.editor == *editor => Some(p.view.read(cx).range),
            _ => None,
        }
    }

    fn vcs_navigate(&mut self, next: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((editor, status)) = self.vcs_active() else {
            return;
        };
        // From the popup's range, else from the caret's line.
        let line = match self.vcs_popup_range(&editor, cx) {
            Some(r) => Some(r.line1),
            None => status.read(cx).caret_line(cx),
        };
        let Some(line) = line else {
            return;
        };
        let ranges = status.read(cx).ranges();
        let target = if next {
            next_range(ranges, line)
        } else {
            previous_range(ranges, line)
        };
        if let Some(i) = target {
            self.vcs_go_to_change(&status, i, window, cx);
        }
    }

    fn vcs_next_change(&mut self, _: &NextChange, window: &mut Window, cx: &mut Context<Self>) {
        self.vcs_navigate(true, window, cx);
    }

    fn vcs_previous_change(
        &mut self,
        _: &PreviousChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.vcs_navigate(false, window, cx);
    }

    fn vcs_show_current_change(
        &mut self,
        _: &ShowCurrentChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, status)) = self.vcs_active() else {
            return;
        };
        let s = status.read(cx);
        let Some(i) = s.caret_line(cx).and_then(|l| range_at_line(s.ranges(), l)) else {
            return;
        };
        self.vcs_go_to_change(&status, i, window, cx);
    }

    fn vcs_rollback_lines(&mut self, _: &RollbackLines, _: &mut Window, cx: &mut Context<Self>) {
        let Some((editor, status)) = self.vcs_active() else {
            return;
        };
        let indices = match self.vcs_popup_range(&editor, cx) {
            Some(r) => status.read(cx).find(&r).into_iter().collect(),
            None => status.read(cx).selected(cx),
        };
        if !indices.is_empty() {
            status.update(cx, |s, cx| s.rollback(&indices, cx));
        }
    }

    /// Show Diff for Lines: the diff viewer is not there yet; the popup's
    /// key takes it, so that it does not duplicate the line.
    fn vcs_show_diff_lines(&mut self, _: &ShowDiffLines, _: &mut Window, _: &mut Context<Self>) {}

    fn vcs_copy_change_lines(
        &mut self,
        _: &CopyChangeLines,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((editor, status)) = self.vcs_active() else {
            return;
        };
        let range = self.vcs_popup_range(&editor, cx).or_else(|| {
            let s = status.read(cx);
            let i = range_at_line(s.ranges(), s.caret_line(cx)?)?;
            s.ranges().get(i).copied()
        });
        if let Some(range) = range {
            Self::vcs_copy(&status, &range, cx);
        }
    }

    fn vcs_close_marker_popup(
        &mut self,
        _: &CloseMarkerPopup,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.vcs_close_popup(cx);
    }
}
