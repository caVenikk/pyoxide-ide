//! The Breakpoints dialog (the reference's `BreakpointsDialog`, ⇧⌘F8 /
//! Ctrl+Shift+F8): a non-modal window "Breakpoints". On the left a tree of
//! the product's breakpoint types (Python Line Breakpoint, Python
//! Exception Breakpoint) with their breakpoints, each with a check box
//! (enabled), and a toolbar: + adds a breakpoint of a type that can be
//! added here (an exception breakpoint, with the class chooser), −
//! removes the selected one. On the right the selected breakpoint's
//! properties (`XLightBreakpointPropertiesPanel` with all options):
//! Enabled, Suspend, the Condition check box and field, the logging
//! options, Remove once hit and the dependency (drawn disabled: they come
//! later), the exception type's options (Activation policy), and for a
//! line breakpoint a read-only preview of its source; "Select a
//! breakpoint" while none is. Changes apply at once. Double-click, Enter
//! and F4 open the source; Delete removes; Done closes.

use std::collections::HashSet;
use std::path::PathBuf;

use debugger::{BreakpointId, Debugger, SuspendPolicy};
use editor::decorations::layer;
use editor::{Decoration, Editor, EditorEvent};
use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, FocusHandle, Focusable as _,
    FontWeight, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription,
    Task, WeakEntity, Window, WindowHandle, div, prelude::*, px, size,
};
use serde_json::{Value as Json, json};
use themes::{Metric, UiRole};
use ui::components::{DISABLED_ALPHA, checkbox};
use ui::form::{BarButton, BottomBar, ButtonRole, CHECK_GAP, FIELD_HEIGHT, MenuPopup, radio};
use ui::icons::{Icon, IconName};
use ui::menu::MenuItem;
use ui::theme::ThemeExt as _;

use crate::breakpoints::commands::{
    DialogAdd, DialogAddException, DialogChooseClass, DialogDone, DialogGoToSource, DialogNext,
    DialogPrevious, DialogRemove, DialogSelect, DialogSet, DialogToggleEnabled,
};
use crate::breakpoints::presentation;
use crate::icons;
use crate::view::DebuggerView;

/// The dialog's title (`xbreakpoints.dialog.title`).
pub const TITLE: &str = "Breakpoints";
/// Key contexts: the dialog, its tree.
pub const CONTEXT: &str = "BreakpointsDialog";
pub const TREE_CONTEXT: &str = "BreakpointsTree";
/// The dialog's size and the tree's share of its width (`JBSplitter(0.3)`).
const WIDTH: f32 = 900.0;
const HEIGHT: f32 = 560.0;
const MIN_WIDTH: f32 = 600.0;
const MIN_HEIGHT: f32 = 400.0;
const TREE_SHARE: f32 = 0.3;
const ROW: f32 = 22.0;
const INDENT: f32 = 19.0;
/// The detail panel's insets and the gap between its rows.
const DETAIL_INSET: f32 = 14.0;
const DETAIL_GAP: f32 = 7.0;
/// The source preview's height and the lines it shows around the
/// breakpoint.
const PREVIEW_HEIGHT: f32 = 130.0;
/// The empty detail's text (`xbreakpoint.label.empty`).
const EMPTY: &str = "Select a breakpoint";

/// A row of the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RowKey {
    LineGroup,
    ExceptionGroup,
    Breakpoint(BreakpointId),
}

struct Row {
    key: RowKey,
    depth: usize,
    text: String,
    icon: IconName,
    badge: Option<IconName>,
    checked: bool,
    /// A disabled breakpoint's name is grey (`GRAYED_ATTRIBUTES`).
    grey: bool,
}

/// The Breakpoints dialog.
pub struct BreakpointsDialog {
    host: WeakEntity<DebuggerView>,
    debugger: Debugger,
    selected: Option<RowKey>,
    collapsed: HashSet<RowKey>,
    focus: FocusHandle,
    tree_focus: FocusHandle,
    /// The condition of the selected breakpoint and whether its check box
    /// is on.
    condition: Entity<Editor>,
    condition_on: bool,
    /// The breakpoint the detail shows.
    detail_for: Option<BreakpointId>,
    /// The source preview of the selected line breakpoint, and its file.
    preview: Option<(PathBuf, u32, Entity<Editor>)>,
    _loading: Option<Task<()>>,
    menu: Option<MenuPopup>,
    activate: bool,
    _subscriptions: Vec<Subscription>,
}

impl BreakpointsDialog {
    /// Opens the dialog of the main window `window`.
    pub(crate) fn open(
        view: Entity<DebuggerView>,
        debugger: Debugger,
        select: Option<BreakpointId>,
        activate: bool,
        window: &mut Window,
        cx: &mut Context<DebuggerView>,
    ) -> Option<WindowHandle<Self>> {
        let title_height = ui::dialog_window::title_height(true);
        let options = ui::dialog_window::DialogOptions {
            title: TITLE.into(),
            size: size(px(WIDTH), px(HEIGHT + title_height)),
            min_size: Some(size(px(MIN_WIDTH), px(MIN_HEIGHT))),
            activate,
            themed_title: true,
            modal: false,
            key: Some(CONTEXT.into()),
        };
        let host = view.downgrade();
        ui::dialog_window::open(window, options, cx, move |window, cx| {
            cx.new(|cx| Self::new(host, view, debugger, select, activate, window, cx))
        })
    }

    fn new(
        host: WeakEntity<DebuggerView>,
        view: Entity<DebuggerView>,
        debugger: Debugger,
        select: Option<BreakpointId>,
        activate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let condition = editor::field::new("", None, false, window, cx);
        let subscriptions = vec![
            // The model changed: the tree and the detail follow.
            cx.observe_in(&view, window, |this, _, window, cx| {
                this.model_changed(window, cx);
            }),
            // The condition applies as it is typed.
            cx.subscribe(&condition, |this, _, event: &EditorEvent, cx| {
                if matches!(event, EditorEvent::Edited(_)) {
                    this.apply_condition(cx);
                }
            }),
            // Closed by its title bar: the host forgets it.
            cx.on_release(|this, cx| {
                // After the update that closed it (the host's, maybe).
                let host = this.host.clone();
                cx.defer(move |cx| {
                    let _ = host.update(cx, |h, cx| h.dialog_closed(cx));
                });
            }),
        ];
        let tree_focus = cx.focus_handle();
        window.focus(&tree_focus);
        let mut dialog = Self {
            host,
            debugger,
            selected: None,
            collapsed: HashSet::new(),
            focus: cx.focus_handle(),
            tree_focus,
            condition,
            condition_on: false,
            detail_for: None,
            preview: None,
            _loading: None,
            menu: None,
            activate,
            _subscriptions: subscriptions,
        };
        let first = select.map(RowKey::Breakpoint).or_else(|| {
            dialog
                .rows()
                .iter()
                .find(|r| matches!(r.key, RowKey::Breakpoint(_)))
                .map(|r| r.key)
        });
        dialog.select(first, window, cx);
        dialog
    }

    // ---- the tree ---------------------------------------------------------------

    /// The rows: the line breakpoints by file and line under their type,
    /// then the exception breakpoints (`BreakpointItemsTreeController`).
    fn rows(&self) -> Vec<Row> {
        let breakpoints = self.debugger.breakpoints();
        let muted = self.debugger.is_muted();
        let provider = self.debugger.provider();
        let mut rows = Vec::new();
        if let Some(kind) = provider.line_breakpoint_type() {
            let mut line: Vec<_> = breakpoints.line().iter().collect();
            line.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
            if !line.is_empty() {
                rows.push(Row {
                    key: RowKey::LineGroup,
                    depth: 0,
                    text: kind.title,
                    icon: icons::BREAKPOINT,
                    badge: None,
                    checked: line.iter().all(|b| b.enabled),
                    grey: false,
                });
                if !self.collapsed.contains(&RowKey::LineGroup) {
                    for b in line {
                        let status = self.debugger.breakpoint_status(b.id);
                        rows.push(Row {
                            key: RowKey::Breakpoint(b.id),
                            depth: 1,
                            text: presentation::short_text(&b.file, b.line),
                            icon: presentation::line_icon(b, muted, status.as_ref()),
                            badge: presentation::badge(b.condition.as_deref()),
                            checked: b.enabled,
                            grey: !b.enabled,
                        });
                    }
                }
            }
        }
        if let Some(kind) = provider.exception_breakpoint_type() {
            let list: Vec<_> = breakpoints
                .exception()
                .iter()
                .filter(|b| b.kind == kind.id)
                .collect();
            if !list.is_empty() {
                rows.push(Row {
                    key: RowKey::ExceptionGroup,
                    depth: 0,
                    text: kind.title.clone(),
                    icon: icons::EXCEPTION,
                    badge: None,
                    checked: list.iter().all(|b| b.enabled),
                    grey: false,
                });
                if !self.collapsed.contains(&RowKey::ExceptionGroup) {
                    for b in list {
                        rows.push(Row {
                            key: RowKey::Breakpoint(b.id),
                            depth: 1,
                            text: kind.display_name(b).to_owned(),
                            icon: presentation::exception_icon(b, muted),
                            badge: presentation::badge(b.condition.as_deref()),
                            checked: b.enabled,
                            grey: !b.enabled,
                        });
                    }
                }
            }
        }
        rows
    }

    /// Selects a row: the detail shows its breakpoint (loaded once the
    /// current update is over: it reads the host, which may be the one
    /// selecting).
    fn select(&mut self, key: Option<RowKey>, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = key;
        let id = match key {
            Some(RowKey::Breakpoint(id)) => Some(id),
            _ => None,
        };
        if id != self.detail_for {
            self.detail_for = id;
            cx.defer_in(window, |this, window, cx| this.load_detail(window, cx));
        }
        cx.notify();
    }

    pub(crate) fn select_breakpoint(
        &mut self,
        id: BreakpointId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.collapsed.clear();
        self.select(Some(RowKey::Breakpoint(id)), window, cx);
    }

    /// The condition field and the preview of the selected breakpoint.
    fn load_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let breakpoints = self.debugger.breakpoints();
        let (condition, line) = match self.detail_for {
            Some(id) => match breakpoints.get(id) {
                Some(b) => (b.condition.clone(), Some((b.file.clone(), b.line))),
                None => (
                    breakpoints
                        .exception()
                        .iter()
                        .find(|b| b.id == id)
                        .and_then(|b| b.condition.clone()),
                    None,
                ),
            },
            None => (None, None),
        };
        self.condition_on = condition.is_some();
        let text = condition.unwrap_or_default();
        self.condition
            .update(cx, |e, cx| e.set_text_selected(&text, cx));
        match line {
            Some((file, line)) => self.load_preview(file, line, window, cx),
            None => {
                self.preview = None;
                self._loading = None;
            }
        }
    }

    /// The source preview at a line: the open tab's text, or the file's
    /// read off the UI thread.
    fn load_preview(
        &mut self,
        file: PathBuf,
        line: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((path, at, editor)) = &mut self.preview
            && *path == file
        {
            *at = line;
            let editor = editor.clone();
            self.decorate_preview(&editor, line, cx);
            editor.update(cx, |e, cx| e.go_to(line as usize, None, cx));
            return;
        }
        let open = self
            .host
            .upgrade()
            .and_then(|h| h.read(cx).editor_of(&file, cx))
            .map(|e| e.read(cx).text());
        match open {
            Some(text) => self.show_preview(file, line, text, window, cx),
            None => {
                let path = file.clone();
                let read = cx.background_spawn(async move { std::fs::read_to_string(&path) });
                self._loading = Some(cx.spawn_in(window, async move |this, cx| {
                    let text = read.await.unwrap_or_default();
                    let _ = this.update_in(cx, |this, window, cx| {
                        if this.detail_line() == Some((file.clone(), line)) {
                            this.show_preview(file, line, text, window, cx);
                        }
                    });
                }));
            }
        }
    }

    fn detail_line(&self) -> Option<(PathBuf, u32)> {
        let b = self.debugger.breakpoints();
        let b = b.get(self.detail_for?)?;
        Some((b.file.clone(), b.line))
    }

    fn show_preview(
        &mut self,
        file: PathBuf,
        line: u32,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let highlighter = self
            .host
            .upgrade()
            .and_then(|h| h.read(cx).highlighters.clone())
            .and_then(|f| f(&file));
        let editor = cx.new(|cx| {
            let mut e = Editor::new(text::Buffer::new(&text), window, cx);
            e.buffer_mut().set_read_only(true);
            e.set_viewer(true);
            e.set_path(Some(file.clone()));
            if let Some(h) = highlighter {
                e.set_highlighter(Some(h), cx);
            }
            e.go_to(line as usize, None, cx);
            e
        });
        self.decorate_preview(&editor, line, cx);
        self.preview = Some((file, line, editor));
        cx.notify();
    }

    /// The breakpoints of the preview's file in its gutter, and the
    /// selected one's line in the breakpoint colour (`showInEditor` with
    /// `BREAKPOINT_ATTRIBUTES`).
    fn decorate_preview(&self, editor: &Entity<Editor>, line: u32, cx: &mut Context<Self>) {
        let Some(file) = editor.read(cx).path().cloned() else {
            return;
        };
        let breakpoints = self.debugger.breakpoints();
        let muted = self.debugger.is_muted();
        let len = editor.read(cx).buffer().len_lines();
        let mut items: Vec<_> = breakpoints
            .in_file(&file)
            .into_iter()
            .filter(|b| (b.line as usize) <= len)
            .map(|b| {
                let status = self.debugger.breakpoint_status(b.id);
                let l = b.line as usize - 1;
                (
                    l..l + 1,
                    Decoration::default()
                        .gutter_icon(presentation::line_icon(b, muted, status.as_ref()), None)
                        .badge(presentation::badge(b.condition.as_deref())),
                )
            })
            .collect();
        let l = (line as usize).clamp(1, len.max(1)) - 1;
        items.push((
            l..l + 1,
            Decoration::default().background(UiRole::Breakpoint, layer::BREAKPOINT),
        ));
        editor.update(cx, |e, cx| {
            e.set_decorations(super::gutter::GROUP, items, cx);
        });
    }

    /// The model changed (here or elsewhere): a removed breakpoint leaves
    /// the selection, the preview shows the icons again.
    fn model_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows();
        if let Some(key) = self.selected
            && !rows.iter().any(|r| r.key == key)
        {
            let next = rows.iter().find(|r| matches!(r.key, RowKey::Breakpoint(_)));
            self.select(next.map(|r| r.key), window, cx);
        }
        if let Some((file, line, editor)) = self.preview.clone()
            && let Some((now_file, now_line)) = self.detail_line()
        {
            if now_file == file {
                if now_line != line {
                    self.load_preview(now_file, now_line, window, cx);
                } else {
                    self.decorate_preview(&editor, line, cx);
                }
            } else {
                self.load_preview(now_file, now_line, window, cx);
            }
        }
        cx.notify();
    }

    fn move_selection(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows();
        if rows.is_empty() {
            return;
        }
        let at = self
            .selected
            .and_then(|k| rows.iter().position(|r| r.key == k));
        let next = match at {
            Some(i) => (i as isize + by).clamp(0, rows.len() as isize - 1) as usize,
            None => 0,
        };
        self.select(Some(rows[next].key), window, cx);
    }

    /// The check box of a row: a breakpoint's enabled state, or all of a
    /// group's.
    fn toggle_row(&mut self, key: RowKey, cx: &mut Context<Self>) {
        let rows = self.rows();
        let Some(row) = rows.iter().find(|r| r.key == key) else {
            return;
        };
        let on = !row.checked;
        let ids: Vec<BreakpointId> = match key {
            RowKey::Breakpoint(id) => vec![id],
            RowKey::LineGroup => self
                .debugger
                .breakpoints()
                .line()
                .iter()
                .map(|b| b.id)
                .collect(),
            RowKey::ExceptionGroup => self
                .debugger
                .breakpoints()
                .exception()
                .iter()
                .map(|b| b.id)
                .collect(),
        };
        for id in ids {
            self.set_enabled(id, on);
        }
        self.host_changed(cx);
    }

    fn set_enabled(&self, id: BreakpointId, on: bool) {
        if !self.debugger.update_breakpoint(id, |b| b.enabled = on) {
            self.debugger
                .update_exception_breakpoint(id, |b| b.enabled = on);
        }
    }

    /// The gutters show a change made here at once.
    fn host_changed(&self, cx: &mut Context<Self>) {
        if let Some(host) = self.host.upgrade() {
            host.update(cx, |h, cx| {
                h.refresh_all(cx);
                cx.notify();
            });
        }
        cx.notify();
    }

    fn apply_condition(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.detail_for else {
            return;
        };
        let text = self.condition.read(cx).text();
        let condition = Some(text.trim().to_owned()).filter(|t| !t.is_empty() && self.condition_on);
        let changed = |old: &Option<String>| *old != condition;
        let line = self
            .debugger
            .breakpoints()
            .get(id)
            .map(|b| b.condition.clone());
        match line {
            Some(old) if changed(&old) => {
                self.debugger
                    .update_breakpoint(id, |b| b.condition = condition.clone());
            }
            Some(_) => {}
            None => {
                self.debugger
                    .update_exception_breakpoint(id, |b| b.condition = condition.clone());
            }
        }
        self.host_changed(cx);
    }

    // ---- actions ------------------------------------------------------------------

    fn on_select(&mut self, a: &DialogSelect, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows();
        let key = if let Some(i) = a.index {
            rows.get(i).map(|r| r.key)
        } else if let Some(class) = &a.class {
            self.debugger
                .breakpoints()
                .exception()
                .iter()
                .find(|b| {
                    b.class == *class
                        || self
                            .debugger
                            .provider()
                            .exception_breakpoint_type()
                            .is_some_and(|t| t.display_name(b) == class)
                })
                .map(|b| RowKey::Breakpoint(b.id))
        } else if let (Some(file), Some(line)) = (&a.file, a.line) {
            debugger::storage::resolve_path(file, self.debugger.root())
                .and_then(|p| self.debugger.breakpoints().at(&p, line).map(|b| b.id))
                .map(RowKey::Breakpoint)
        } else {
            None
        };
        if key.is_some() {
            self.select(key, window, cx);
        }
    }

    fn on_previous(&mut self, _: &DialogPrevious, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, window, cx);
    }

    fn on_next(&mut self, _: &DialogNext, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, window, cx);
    }

    fn on_toggle_enabled(
        &mut self,
        _: &DialogToggleEnabled,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(key) = self.selected {
            self.toggle_row(key, cx);
        }
    }

    fn on_remove(&mut self, _: &DialogRemove, window: &mut Window, cx: &mut Context<Self>) {
        let Some(RowKey::Breakpoint(id)) = self.selected else {
            return;
        };
        let rows = self.rows();
        let at = rows.iter().position(|r| r.key == RowKey::Breakpoint(id));
        if !self.debugger.remove_breakpoint(id) {
            return;
        }
        // The next breakpoint row takes the selection.
        let rows_after = self.rows();
        let next = at
            .and_then(|i| {
                rows_after
                    .iter()
                    .skip(i.saturating_sub(1))
                    .find(|r| matches!(r.key, RowKey::Breakpoint(_)))
            })
            .map(|r| r.key);
        self.select(next, window, cx);
        self.host_changed(cx);
    }

    fn on_go_to_source(
        &mut self,
        _: &DialogGoToSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(RowKey::Breakpoint(id)) = self.selected else {
            if let Some(key) = self.selected {
                self.toggle_collapsed(key, cx);
            }
            return;
        };
        let Some(b) = self.debugger.breakpoints().get(id).cloned() else {
            return;
        };
        if let Some(host) = self.host.upgrade() {
            host.update(cx, |h, cx| h.navigate(b.file.clone(), b.line, cx));
        }
        self.close(window, cx);
    }

    fn toggle_collapsed(&mut self, key: RowKey, cx: &mut Context<Self>) {
        if !self.collapsed.remove(&key) {
            self.collapsed.insert(key);
        }
        cx.notify();
    }

    /// + : the types that can be added here (`AddXBreakpointAction`).
    fn on_add(&mut self, _: &DialogAdd, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kind) = self.debugger.provider().exception_breakpoint_type() else {
            return;
        };
        let items = vec![MenuItem::action(kind.title, DialogChooseClass).icon(icons::EXCEPTION)];
        self.open_menu(items, None, window, cx);
    }

    /// The class chooser of a new exception breakpoint
    /// (`PyClassTreeChooserDialog` "Select Exception Class"): the classes
    /// the product offers, with speed search.
    fn on_choose_class(
        &mut self,
        _: &DialogChooseClass,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.choose_class(window, cx);
    }

    pub(crate) fn choose_class(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let provider = self.debugger.provider().clone();
        let Some(kind) = provider.exception_breakpoint_type() else {
            return;
        };
        let items = provider
            .exception_classes()
            .into_iter()
            .map(|class| {
                MenuItem::action(class.clone(), DialogAddException { class })
                    .icon(IconName::NODE_CLASS)
            })
            .collect();
        self.open_menu(items, Some(kind.chooser_title.into()), window, cx);
    }

    fn on_add_exception(
        &mut self,
        a: &DialogAddException,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_menu(cx);
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let id = host.update(cx, |h, cx| h.add_exception_breakpoint(&a.class, cx));
        if let Some(id) = id {
            self.collapsed.remove(&RowKey::ExceptionGroup);
            self.select(Some(RowKey::Breakpoint(id)), window, cx);
        }
    }

    fn open_menu(
        &mut self,
        items: Vec<MenuItem>,
        title: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_menu(cx);
        // Under the + button.
        let position = gpui::point(
            px(DETAIL_GAP),
            px(ui::dialog_window::title_height(true) + 30.0),
        );
        let focus = self.tree_focus.clone();
        self.menu = MenuPopup::open(
            items,
            title,
            position,
            &focus,
            self.activate,
            window,
            cx,
            |this: &mut Self, cx| this.close_menu(cx),
        );
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(menu) = self.menu.take() {
            menu.close(cx);
        }
    }

    fn on_set(&mut self, a: &DialogSet, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.detail_for else {
            return;
        };
        if let Some(enabled) = a.enabled {
            self.set_enabled(id, enabled);
        }
        if let Some(suspend) = a.suspend.as_deref().and_then(SuspendPolicy::parse)
            && !self.debugger.update_breakpoint(id, |b| b.suspend = suspend)
        {
            self.debugger
                .update_exception_breakpoint(id, |b| b.suspend = suspend);
        }
        if let Some(on) = a.condition_enabled {
            self.condition_on = on;
            if on {
                window.focus(&self.condition.focus_handle(cx));
            }
            self.apply_condition(cx);
        }
        if let Some(text) = &a.condition {
            self.condition_on = true;
            self.condition
                .update(cx, |e, cx| e.set_text_selected(text, cx));
            self.apply_condition(cx);
        }
        if let (Some(key), Some(value)) = (&a.option, a.value) {
            let value = if value { "true" } else { "false" };
            self.debugger.update_exception_breakpoint(id, |b| {
                b.properties.insert(key.clone(), value.to_owned());
            });
        }
        self.host_changed(cx);
    }

    fn on_done(&mut self, _: &DialogDone, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }

    fn close(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(cx);
        if let Some(host) = self.host.upgrade() {
            host.update(cx, |h, cx| h.close_dialog(cx));
        }
    }

    pub(crate) fn state_json(&self, cx: &App) -> Json {
        let rows: Vec<Json> = self
            .rows()
            .iter()
            .map(|r| {
                json!({
                    "text": r.text,
                    "depth": r.depth,
                    "checked": r.checked,
                    "icon": r.icon.0,
                    "badge": r.badge.map(|b| b.0),
                    "selected": self.selected == Some(r.key),
                })
            })
            .collect();
        let detail = self.detail_for.map(|id| {
            let breakpoints = self.debugger.breakpoints();
            match breakpoints.get(id) {
                Some(b) => json!({
                    "title": presentation::short_text(&b.file, b.line),
                    "enabled": b.enabled,
                    "suspend": b.suspend.as_str(),
                    "condition": self.condition.read(cx).text(),
                    "condition_enabled": self.condition_on,
                    "preview": self.preview.as_ref().map(|(_, line, e)| json!({
                        "line": line,
                        "caret_line": crate::view::caret_line(e, cx),
                        "viewer": e.read(cx).is_viewer(),
                    })),
                }),
                None => {
                    let e = breakpoints.exception().iter().find(|b| b.id == id);
                    let kind = self.debugger.provider().exception_breakpoint_type();
                    json!({
                        "title": e.zip(kind.as_ref()).map(|(e, k)| k.display_name(e).to_owned()),
                        "enabled": e.map(|e| e.enabled),
                        "suspend": e.map(|e| e.suspend.as_str()),
                        "condition": self.condition.read(cx).text(),
                        "condition_enabled": self.condition_on,
                        "options": kind.map(|k| k.options.iter().map(|o| json!({
                            "label": o.label,
                            "checked": e.is_some_and(|e| e.flag(&o.key)),
                        })).collect::<Vec<_>>()),
                    })
                }
            }
        });
        json!({
            "title": TITLE,
            "rows": rows,
            "detail": detail,
            "empty": detail.is_none().then_some(EMPTY),
            "menu": self.menu.is_some(),
        })
    }

    // ---- drawing ------------------------------------------------------------------

    fn render_tree(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let radius = theme.metric(Metric::CornerRadius);
        let hover = theme.color(UiRole::Hover);
        let selection = theme.color(UiRole::Selection);
        let border = theme.color(UiRole::Border);
        let grey = theme.color(UiRole::TextDisabled);
        let tool = |id: &'static str,
                    icon: IconName,
                    tooltip: &'static str,
                    enabled: bool,
                    action: Box<dyn gpui::Action>| {
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(22.0))
                .rounded(radius)
                .when(enabled, |d| d.hover(move |s| s.bg(hover)))
                .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
                .child(Icon::new(icon))
                .tooltip(ui::components::Tooltip::text(tooltip, None))
                .when(enabled, |d| {
                    d.on_click(move |_, window, cx| {
                        window.dispatch_action(action.boxed_clone(), cx)
                    })
                })
        };
        let removable = match self.selected {
            Some(RowKey::Breakpoint(id)) => {
                self.debugger.breakpoints().get(id).is_some()
                    || self
                        .debugger
                        .breakpoints()
                        .exception()
                        .iter()
                        .any(|b| b.id == id && !b.default)
            }
            _ => false,
        };
        let can_add = self
            .debugger
            .provider()
            .exception_breakpoint_type()
            .is_some();
        let toolbar = div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .px(px(6.0))
            .h(px(30.0))
            .child(tool(
                "breakpoints-add",
                IconName::ADD,
                "Add",
                can_add,
                Box::new(DialogAdd),
            ))
            .child(tool(
                "breakpoints-remove",
                IconName::REMOVE,
                "Remove",
                removable,
                Box::new(DialogRemove),
            ));
        let tree_focused = self.tree_focus.is_focused(window);
        let rows: Vec<AnyElement> = self
            .rows()
            .into_iter()
            .enumerate()
            .map(|(i, row)| {
                let selected = self.selected == Some(row.key);
                let key = row.key;
                let group = matches!(key, RowKey::LineGroup | RowKey::ExceptionGroup);
                let expanded = !self.collapsed.contains(&key);
                div()
                    .id(("breakpoints-row", i))
                    .flex()
                    .flex_none()
                    .flex_row()
                    .items_center()
                    .h(px(ROW))
                    .mx(px(6.0))
                    .pl(px(4.0 + INDENT * row.depth as f32))
                    .rounded(radius)
                    .when(selected, |d| d.bg(selection))
                    .when(selected && !tree_focused, |d| d.opacity(0.9))
                    .child(
                        div()
                            .id(("breakpoints-chevron", i))
                            .flex_none()
                            .w(px(16.0))
                            .when(group, |d| {
                                d.child(Icon::new(if expanded {
                                    IconName::CHEVRON_DOWN
                                } else {
                                    IconName::CHEVRON_RIGHT
                                }))
                                .on_click(cx.listener(
                                    move |this, _: &ClickEvent, _, cx| {
                                        cx.stop_propagation();
                                        this.toggle_collapsed(key, cx);
                                    },
                                ))
                            }),
                    )
                    .child(
                        div()
                            .id(("breakpoints-check", i))
                            .flex_none()
                            .ml(px(3.0))
                            .child(checkbox(("breakpoints-checkbox", i), row.checked, cx))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.toggle_row(key, cx);
                            })),
                    )
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .ml(px(5.0))
                            .size(px(16.0))
                            .child(Icon::new(row.icon))
                            .children(row.badge.map(|b| {
                                div()
                                    .absolute()
                                    .left(px(editor::decorations::BADGE_OFFSET.0))
                                    .top(px(editor::decorations::BADGE_OFFSET.1))
                                    .child(Icon::new(b).size(px(editor::decorations::BADGE_SIZE)))
                            })),
                    )
                    .child(
                        div()
                            .ml(px(4.0))
                            .whitespace_nowrap()
                            .when(row.grey, |d| d.text_color(grey))
                            .child(row.text),
                    )
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        window.focus(&this.tree_focus);
                        this.select(Some(key), window, cx);
                        if e.click_count() >= 2 {
                            window.dispatch_action(Box::new(DialogGoToSource), cx);
                        }
                    }))
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(window.viewport_size().width * TREE_SHARE)
            .h_full()
            .border_r_1()
            .border_color(border)
            .child(toolbar)
            .child(
                div()
                    .id("breakpoints-tree")
                    .key_context(TREE_CONTEXT)
                    .track_focus(&self.tree_focus)
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .children(rows),
            )
            .into_any_element()
    }

    fn render_detail(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let Some(id) = self.detail_for else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(muted)
                .child(EMPTY)
                .into_any_element();
        };
        let breakpoints = self.debugger.breakpoints();
        let line = breakpoints.get(id).cloned();
        let exception = breakpoints.exception().iter().find(|b| b.id == id).cloned();
        let kind = self.debugger.provider().exception_breakpoint_type();
        let (title, enabled, suspend, thread) = match (&line, &exception) {
            (Some(b), _) => (
                presentation::short_text(&b.file, b.line),
                b.enabled,
                b.suspend,
                self.debugger
                    .provider()
                    .line_breakpoint_type()
                    .is_some_and(|t| t.suspend_thread),
            ),
            (None, Some(e)) => (
                kind.as_ref()
                    .map(|k| k.display_name(e).to_owned())
                    .unwrap_or_else(|| e.class.clone()),
                e.enabled,
                e.suspend,
                false,
            ),
            (None, None) => return div().into_any_element(),
        };
        let disabled_text = theme.color(UiRole::TextDisabled);
        let default = self.debugger.default_suspend();
        let set = |a: DialogSet| {
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                window.dispatch_action(Box::new(a.clone()), cx)
            }
        };
        let empty_set = DialogSet {
            enabled: None,
            suspend: None,
            condition: None,
            condition_enabled: None,
            option: None,
            value: None,
        };
        let check = |id: &'static str, checked: bool, label: SharedString, enabled: bool| {
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(CHECK_GAP))
                .when(!enabled, |d| d.text_color(disabled_text))
                .child(
                    div()
                        .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
                        .child(checkbox((id, 0usize), checked, cx)),
                )
                .child(label)
        };
        let suspends = suspend != SuspendPolicy::None;
        let policy = if suspends { suspend } else { default };
        let radio_choice = |id: &'static str, p: SuspendPolicy, label: &'static str| {
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(CHECK_GAP))
                .child(radio((id, 0usize), policy == p, cx))
                .child(
                    div()
                        .when(p == default, |d| d.font_weight(FontWeight::BOLD))
                        .when(!suspends, |d| d.text_color(disabled_text))
                        .child(label),
                )
                .when(suspends, |d| {
                    d.on_click(set(DialogSet {
                        suspend: Some(p.as_str().to_owned()),
                        ..empty_set.clone()
                    }))
                })
        };
        let disabled_field = || {
            ui::components::input_frame(false, false, cx)
                .h(px(FIELD_HEIGHT))
                .px(px(8.0))
                .opacity(DISABLED_ALPHA)
                .child(div().flex_1())
                .child(Icon::new(IconName::EXPAND))
                .child(Icon::new(IconName::CHEVRON_DOWN))
        };
        let field = editor::field::frame(&self.condition, false, false, window, cx)
            .when(!self.condition_on, |d| d.opacity(DISABLED_ALPHA))
            .child(ui::form::field_button(
                "breakpoints-expand",
                IconName::EXPAND,
                "Expand",
                None,
            ))
            .child(ui::form::field_button(
                "breakpoints-history",
                IconName::CHEVRON_DOWN,
                "History",
                None,
            ));
        let suspend_row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(14.0))
            .child(
                check(
                    "breakpoints-suspend",
                    suspends,
                    if thread {
                        "Suspend:"
                    } else {
                        "Suspend execution"
                    }
                    .into(),
                    true,
                )
                .on_click(set(DialogSet {
                    suspend: Some(if suspends {
                        "none".to_owned()
                    } else {
                        default.as_str().to_owned()
                    }),
                    ..empty_set.clone()
                })),
            )
            .when(thread, |d| {
                d.child(radio_choice("breakpoints-all", SuspendPolicy::All, "All"))
                    .child(radio_choice(
                        "breakpoints-thread",
                        SuspendPolicy::Thread,
                        "Thread",
                    ))
            });
        let condition = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                check(
                    "breakpoints-condition",
                    self.condition_on,
                    "Condition:".into(),
                    true,
                )
                .on_click(set(DialogSet {
                    condition_enabled: Some(!self.condition_on),
                    ..empty_set.clone()
                })),
            )
            .child(div().pl(px(22.0)).child(field));
        // The logging options, Remove once hit and the dependency come
        // later: drawn disabled with the breakpoint's values.
        let (log_message, log_stack, log_expression, temporary) = line
            .as_ref()
            .map(|b| {
                (
                    b.log_message,
                    b.log_stack,
                    b.log_expression.is_some(),
                    b.temporary,
                )
            })
            .unwrap_or_default();
        let actions = div()
            .flex()
            .flex_col()
            .gap(px(DETAIL_GAP))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(14.0))
                    .child("Log:")
                    .child(check(
                        "breakpoints-log-message",
                        log_message,
                        "\"Breakpoint hit\" message".into(),
                        false,
                    ))
                    .child(check(
                        "breakpoints-log-stack",
                        log_stack,
                        "Stack trace".into(),
                        false,
                    )),
            )
            .child(check(
                "breakpoints-log-expression",
                log_expression,
                "Evaluate and log:".into(),
                false,
            ))
            .child(div().pl(px(22.0)).child(disabled_field()))
            .when(line.is_some(), |d| {
                d.child(check(
                    "breakpoints-temporary",
                    temporary,
                    "Remove once hit".into(),
                    false,
                ))
            })
            .child(div().child("Disable until hitting the following breakpoint:"))
            .child(
                div()
                    .pl(px(14.0))
                    .opacity(DISABLED_ALPHA)
                    .child(ui::form::combo(
                        "breakpoints-master",
                        vec![div().child("<None>").into_any_element()],
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(14.0))
                    .pl(px(14.0))
                    .text_color(disabled_text)
                    .child("After hit:")
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(CHECK_GAP))
                            .opacity(DISABLED_ALPHA)
                            .child(radio("breakpoints-after-disable", true, cx))
                            .child("Disable again"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(CHECK_GAP))
                            .opacity(DISABLED_ALPHA)
                            .child(radio("breakpoints-after-leave", false, cx))
                            .child("Leave enabled"),
                    ),
            );
        // The exception type's own options (`PyExceptionBreakpointPropertiesPanel`).
        let options = exception.as_ref().zip(kind.as_ref()).map(|(e, k)| {
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(ui::form::group_header(k.options_title.clone(), cx))
                .children(k.options.iter().enumerate().map(|(i, o)| {
                    let checked = e.flag(&o.key);
                    div()
                        .id(("breakpoints-option", i))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(CHECK_GAP))
                        .pl(px(6.0))
                        .child(checkbox(("breakpoints-option-check", i), checked, cx))
                        .child(o.label.clone())
                        .on_click(set(DialogSet {
                            option: Some(o.key.clone()),
                            value: Some(!checked),
                            ..empty_set.clone()
                        }))
                }))
        });
        let preview = self.preview.as_ref().map(|(_, _, editor)| {
            div()
                .flex_none()
                .h(px(PREVIEW_HEIGHT))
                .mt(px(4.0))
                .border_t_1()
                .border_color(theme.color(UiRole::Border))
                .child(editor.clone())
        });
        div()
            .id("breakpoints-detail")
            .flex_1()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .child(
                div()
                    .id("breakpoints-properties")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(DETAIL_GAP))
                    .p(px(DETAIL_INSET))
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .whitespace_nowrap()
                            .child(title),
                    )
                    .child(
                        check("breakpoints-enabled", enabled, "Enabled".into(), true).on_click(
                            set(DialogSet {
                                enabled: Some(!enabled),
                                ..empty_set.clone()
                            }),
                        ),
                    )
                    .child(suspend_row)
                    .child(condition)
                    .child(actions)
                    .children(options),
            )
            .children(preview)
            .into_any_element()
    }
}

impl Render for BreakpointsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tree = self.render_tree(window, cx);
        let detail = self.render_detail(window, cx);
        let theme = cx.theme();
        let bar = BottomBar::new()
            .help(ui::form::help_button("breakpoints-help", None, cx))
            .button(BarButton::new(
                "breakpoints-done",
                "Done",
                ButtonRole::Ok,
                Box::new(DialogDone),
            ))
            .render(cx);
        div()
            .id("breakpoints-dialog")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_select))
            .on_action(cx.listener(Self::on_previous))
            .on_action(cx.listener(Self::on_next))
            .on_action(cx.listener(Self::on_toggle_enabled))
            .on_action(cx.listener(Self::on_remove))
            .on_action(cx.listener(Self::on_go_to_source))
            .on_action(cx.listener(Self::on_add))
            .on_action(cx.listener(Self::on_choose_class))
            .on_action(cx.listener(Self::on_add_exception))
            .on_action(cx.listener(Self::on_set))
            .on_action(cx.listener(Self::on_done))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.color(UiRole::Surface))
            .text_color(theme.color(UiRole::Text))
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .children(ui::dialog_window::title_bar(TITLE, true, cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(tree)
                    .child(detail),
            )
            .child(bar)
            .child(ui::pointer::frame())
    }
}

impl gpui::Focusable for BreakpointsDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.tree_focus.clone()
    }
}
