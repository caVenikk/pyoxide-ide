//! The variables view of a debug session (the reference's
//! `XVariablesView` over an `XDebuggerTree`): the values of the selected
//! frame as a tree, `name = {type: size} value` per row with the kind's
//! icon, children loaded when a node is expanded ("Collecting data…"
//! meanwhile), groups (Special Variables, Return Values) after the values,
//! the rest of a large collection behind an ellipsis row, and the names of
//! values that changed since the last stop in the changed colour.
//!
//! The values themselves are the session's ([`Session::children`] keeps
//! them until the program moves on); this view keeps what the reference's
//! `XDebuggerTreeState` keeps per frame: the expanded nodes, the selected
//! one and the values shown, so that the next stop in the same function
//! expands the same nodes and marks what changed
//! (`XDebuggerTreeRestorer`). When the program moves on, the rows stay for
//! a moment (`XDebugView.requestClear`, 100 ms) so that a step does not
//! flicker.
//!
//! The watches are in the same tree (the reference's
//! `debugger.watches.in.variables`, `XWatchesViewImpl` over a
//! `WatchesRootNode`): the evaluate field's result and the watches come
//! before the frame's values ([`crate::watches`]).

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use debugger::values::{GroupIcon, ValueKind, ValueRole, row_text, type_text, value_text};
use debugger::{
    DebugAdapterProvider, DebugError, Phase, Scope, Session, SessionSnapshot, Variable,
};
use editor::Editor;
use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, Entity, FocusHandle, Focusable, IntoElement,
    MouseButton, MouseDownEvent, Pixels, Point, Render, ScrollStrategy, SharedString, Subscription,
    Task, UniformListScrollHandle, Window, canvas, div, prelude::*, px, uniform_list,
};
use serde_json::{Value as Json, json};
use themes::{Metric, UiRole};
use ui::icons::{Icon, IconName};
use ui::menu::ContextMenu;
use ui::theme::ThemeExt;
use ui::tree_guides::TreeGuides;

use crate::icons;
use crate::watches::{Eval, InlineEdit, WatchRow, WatchValue};

/// The key context of the tree (its keys: `workbench/keymaps`).
pub const KEY_CONTEXT: &str = "DebugVariables";
/// How long the rows of a frame stay after the program moved on
/// (`XDebugView`'s clear alarm).
const CLEAR_DELAY: Duration = Duration::from_millis(100);
/// The rows' inset from the tree's edges (where the selection starts and
/// ends), the space before a top-level row's chevron, one level, the
/// chevron's and the icon's room (measured on the reference's New UI tree:
/// the chevron's centre 19.5 from the edge, the icon at 33, the name's
/// first pixel at 52.5, a level 19).
pub(crate) const ROW_INSET: f32 = 12.0;
const ROW_LEFT: f32 = 0.0;
const INDENT: f32 = 19.0;
const CHEVRON: f32 = 16.0;
const CHEVRON_ROOM: f32 = 21.0;
pub(crate) const ICON_ROOM: f32 = 18.5;
/// Space above the first row.
const TREE_TOP: f32 = 4.0;
/// The separator of path segments.
pub(crate) const SEP: char = '\u{1}';

/// The texts of the reference (`XDebuggerBundle`).
pub(crate) const COLLECTING_DATA: &str = "Collecting data\u{2026}";
const CONNECTING: &str = "Connecting";
const CONNECTED: &str = "Connected";
const DISCONNECTED: &str = "Disconnected";
const FRAME_NOT_AVAILABLE: &str = "Frame is not available";

/// A row of the tree, as drawn and as `debug.state` reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The node's path: the names from the root, [`SEP`] between them.
    pub path: String,
    pub depth: usize,
    pub kind: RowKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RowKind {
    Value(Box<ValueRow>),
    Group {
        title: String,
        icon: IconName,
        expanded: bool,
    },
    /// "Collecting data…", an error or the session's state.
    Message {
        text: String,
        role: UiRole,
        icon: Option<IconName>,
    },
    /// The rest of a large collection, shown on a double-click.
    More {
        text: String,
    },
    /// The evaluate field's result or a watch.
    Watch(Box<WatchRow>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValueRow {
    pub name: String,
    /// The type part: `list: 4`.
    pub type_text: Option<String>,
    /// The size of a container.
    pub shape: Option<String>,
    pub kind: ValueKind,
    pub expanded: bool,
    pub changed: bool,
    /// The scope or value the variable is a child of.
    pub parent: i64,
    pub variable: Variable,
    /// The renderer View as chose and the value's text as it shows it.
    pub renderer: Option<String>,
    pub shown: Option<String>,
}

impl Row {
    /// The value the row shows: a variable's, a watch's or the result's.
    pub(crate) fn value(&self) -> Option<&ValueRow> {
        match &self.kind {
            RowKind::Value(v) => Some(v),
            RowKind::Watch(w) => match &w.value {
                WatchValue::Value(v) => Some(v),
                _ => None,
            },
            _ => None,
        }
    }

    fn expandable(&self) -> bool {
        match &self.kind {
            RowKind::Group { .. } => true,
            _ => self.value().is_some_and(|v| v.variable.reference > 0),
        }
    }

    fn expanded(&self) -> bool {
        match &self.kind {
            RowKind::Group { expanded, .. } => *expanded,
            _ => self.value().is_some_and(|v| v.expanded),
        }
    }

    /// The row's text: `items = {list: 4} [1, 2, 3, 4]`, a group's title,
    /// a message.
    pub fn text(&self) -> String {
        match &self.kind {
            RowKind::Value(v) => v.text(),
            RowKind::Group { title, .. } => title.clone(),
            RowKind::Message { text, .. } | RowKind::More { text } => text.clone(),
            RowKind::Watch(w) => w.text(),
        }
    }
}

impl ValueRow {
    pub(crate) fn text(&self) -> String {
        row_text(
            &self.name,
            self.variable.type_name.as_deref(),
            self.shape.as_deref(),
            self.shown.as_deref().unwrap_or(&self.variable.value),
        )
    }
}

pub(crate) fn join(parent: &str, segment: &str) -> String {
    if parent.is_empty() {
        segment.to_owned()
    } else {
        format!("{parent}{SEP}{segment}")
    }
}

pub(crate) fn split(path: &str) -> Vec<String> {
    if path.is_empty() {
        Vec::new()
    } else {
        path.split(SEP).map(str::to_owned).collect()
    }
}

pub(crate) fn parent_of(path: &str) -> &str {
    path.rfind(SEP).map_or("", |i| &path[..i])
}

/// What the tree showed of a frame (`XDebuggerTreeState`).
#[derive(Debug, Clone, Default)]
struct TreeState {
    /// Value texts by path.
    values: HashMap<String, String>,
    expanded: HashSet<String>,
    selected: Option<String>,
}

/// The frame the tree shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShownFrame {
    pub(crate) id: i64,
    /// The session's generation: the same frame at the next stop is
    /// another frame here.
    generation: u64,
    /// Frames of the same function share their state (the reference's
    /// `XStackFrame.getEqualityObject`).
    key: String,
    /// The frame's file: the evaluate field is coloured as its language.
    pub(crate) file: Option<std::path::PathBuf>,
    /// The frame's line (from 1).
    pub(crate) line: u32,
}

/// The variables of a debug session.
pub struct VariablesView {
    pub(crate) session: Session,
    pub(crate) provider: Option<Arc<dyn DebugAdapterProvider>>,
    pub(crate) focus: FocusHandle,
    scroll: UniformListScrollHandle,
    pub(crate) phase: Phase,
    pub(crate) frame: Option<ShownFrame>,
    /// Grows with every frame shown: late answers for another are dropped.
    pub(crate) epoch: u64,
    scopes: Option<Result<Vec<Scope>, String>>,
    loading: HashSet<i64>,
    errors: HashMap<i64, String>,
    state: TreeState,
    /// The state of the frame's function when it was shown last.
    previous: Option<TreeState>,
    saved: HashMap<String, TreeState>,
    rows: Rc<Vec<Row>>,
    /// The rows are built again before they are drawn or read: what they
    /// show changed.
    pub(crate) dirty: bool,
    /// The rows of the frame shown before, while the new one loads.
    frozen: Option<Rc<Vec<Row>>>,
    _unfreeze: Option<Task<()>>,
    /// The watch expressions of the session's configuration type, and
    /// which of them are paused.
    pub(crate) watches: Vec<String>,
    pub(crate) paused: Vec<bool>,
    /// The watches' values at the frame shown, by expression.
    pub(crate) evaluations: HashMap<String, Eval>,
    /// The evaluate field's result: its expression and value.
    pub(crate) result: Option<(String, Eval)>,
    /// What the result showed before it was evaluated last: its name is in
    /// the changed colour unless the value is the same (the reference's
    /// restorer marks the new result node changed).
    pub(crate) result_before: Option<String>,
    /// The renderer View as chose for a value, by path.
    pub(crate) renderers: HashMap<String, String>,
    /// The open history list of the evaluate field or of a row's editor.
    pub(crate) history: Option<crate::watches::HistoryList>,
    /// The names of the field's expression the frame does not declare
    /// (char ranges), and the frame's source they were looked up in.
    pub(crate) unresolved: Vec<std::ops::Range<usize>>,
    pub(crate) source: Option<(std::path::PathBuf, Arc<str>)>,
    /// The lookups of the names of the field and of a row's editor under
    /// way.
    pub(crate) check: Option<Task<()>>,
    pub(crate) check_editor: Option<Task<()>>,
    /// The evaluate field, made when it is first drawn.
    pub(crate) field: Option<Entity<Editor>>,
    /// Where the evaluate field, the tree and a row's editor were laid out
    /// (the histories and the context menu open there).
    pub(crate) field_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub(crate) tree_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub(crate) editor_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// A value or a watch edited in its row.
    pub(crate) editing: Option<InlineEdit>,
    /// The context menu or the evaluate field's history.
    pub(crate) menu: Option<(Entity<ContextMenu>, Point<Pixels>, Subscription)>,
    /// How the evaluate field gets the colours of a frame's language, and
    /// the file it has them for.
    pub(crate) highlighters: Option<crate::Highlighters>,
    pub(crate) highlighted: Option<std::path::PathBuf>,
    /// The line height of an editor in a row, as last drawn.
    pub(crate) line_height: Pixels,
}

impl VariablesView {
    pub fn new(
        session: Session,
        provider: Option<Arc<dyn DebugAdapterProvider>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let phase = session.phase();
        let (watches, paused) = session
            .debugger()
            .map(|d| {
                (
                    d.watches(session.watches_key()),
                    d.watches_paused(session.watches_key()),
                )
            })
            .unwrap_or_default();
        Self {
            session,
            provider,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            phase,
            frame: None,
            epoch: 0,
            scopes: None,
            loading: HashSet::new(),
            errors: HashMap::new(),
            state: TreeState::default(),
            previous: None,
            saved: HashMap::new(),
            rows: Rc::default(),
            dirty: true,
            frozen: None,
            _unfreeze: None,
            watches,
            paused,
            evaluations: HashMap::new(),
            result: None,
            result_before: None,
            renderers: HashMap::new(),
            history: None,
            unresolved: Vec::new(),
            source: None,
            check: None,
            check_editor: None,
            field: None,
            field_bounds: Rc::default(),
            tree_bounds: Rc::default(),
            editor_bounds: Rc::default(),
            editing: None,
            menu: None,
            highlighters: None,
            highlighted: None,
            line_height: px(16.0),
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The session changed: the tree follows its selected frame. A new
    /// frame keeps the expanded nodes and the selection of its function.
    pub fn follow(&mut self, snapshot: &SessionSnapshot, cx: &mut Context<Self>) {
        self.phase = snapshot.phase;
        self.dirty = true;
        let frame = (snapshot.phase == Phase::Suspended)
            .then(|| {
                let id = snapshot.selected_frame?;
                let f = snapshot.frames.iter().find(|f| f.id == id)?;
                let path = f
                    .source
                    .as_ref()
                    .and_then(|s| s.path.as_ref())
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Some(ShownFrame {
                    id,
                    generation: snapshot.generation,
                    key: format!("{path}:{}", f.name),
                    file: f.source.as_ref().and_then(|s| s.path.clone()),
                    line: f.line,
                })
            })
            .flatten();
        let shown = |f: &ShownFrame| (f.id, f.generation);
        if frame.as_ref().map(shown) == self.frame.as_ref().map(shown) {
            cx.notify();
            return;
        }
        if let Some(old) = self.frame.take() {
            self.saved.insert(old.key, std::mem::take(&mut self.state));
        }
        self.freeze(cx);
        self.epoch += 1;
        self.scopes = None;
        self.loading.clear();
        self.errors.clear();
        // Another frame: the watches are evaluated there, the result goes
        // and an editor closes (`XDebuggerTreeInplaceEditor`).
        self.evaluations.clear();
        self.result = None;
        self.editing = None;
        self.history = None;
        self.previous = frame.as_ref().and_then(|f| self.saved.get(&f.key).cloned());
        self.state = TreeState {
            values: HashMap::new(),
            expanded: self
                .previous
                .as_ref()
                .map(|p| p.expanded.clone())
                .unwrap_or_default(),
            selected: self.previous.as_ref().and_then(|p| p.selected.clone()),
        };
        if let Some(f) = &frame {
            self.load_scopes(f.id, cx);
        }
        self.frame = frame;
        // The field's names are looked up in the new frame's code.
        self.check_field(cx);
        cx.notify();
    }

    /// The values were set or changed: they are loaded again and compared
    /// with what the tree showed (`XDebuggerTree.rebuildAndRestore`).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.previous = Some(self.state.clone());
        self.state.values.clear();
        self.epoch += 1;
        self.loading.clear();
        self.errors.clear();
        // The watches are evaluated again; the result keeps its value
        // (`WatchesRootNode.copyEvaluationResultFromOldRoot`) and is
        // compared with it.
        self.evaluations.clear();
        self.result_before = self
            .previous
            .as_ref()
            .and_then(|p| p.values.get(crate::watches::RESULT).cloned());
        self.freeze(cx);
        self.dirty = true;
        cx.notify();
    }

    /// The rows shown now stay until the new ones are loaded, at most
    /// [`CLEAR_DELAY`].
    fn freeze(&mut self, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        self.frozen = Some(self.rows.clone());
        self._unfreeze = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CLEAR_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                this.frozen = None;
                cx.notify();
            });
        }));
    }

    fn load_scopes(&mut self, frame: i64, cx: &mut Context<Self>) {
        let epoch = self.epoch;
        let scopes = self.session.scopes(frame);
        cx.spawn(async move |this, cx| {
            let result = scopes.await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != epoch {
                    return;
                }
                // Without a state of the function, the first scope is open.
                if let (Ok(scopes), None) = (&result, &this.previous)
                    && let Some(first) = scopes.first().filter(|s| !s.expensive)
                {
                    this.state.expanded.insert(first.name.clone());
                }
                this.scopes = Some(result.map_err(|e| e.to_string()));
                this.dirty = true;
                cx.notify();
            });
        })
        .detach();
    }

    fn load(&mut self, reference: i64, cx: &mut Context<Self>) {
        if !self.loading.insert(reference) {
            return;
        }
        let epoch = self.epoch;
        let children = self.session.children(reference);
        cx.spawn(async move |this, cx| {
            let result = children.await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != epoch {
                    return;
                }
                this.loading.remove(&reference);
                match result {
                    Ok(_) | Err(DebugError::NotSuspended) | Err(DebugError::Ended) => {}
                    Err(e) => {
                        this.errors.insert(reference, e.to_string());
                    }
                }
                this.dirty = true;
                cx.notify();
            });
        })
        .detach();
    }

    // ---- rows ------------------------------------------------------------------

    /// Builds the rows from the session's loaded values, when what they
    /// show changed, and loads what an expanded node needs.
    pub(crate) fn rebuild(&mut self, cx: &mut Context<Self>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let mut b = Builder {
            view: self,
            rows: Vec::new(),
            wanted: Vec::new(),
            evaluate: Vec::new(),
            values: HashMap::new(),
            complete: true,
        };
        b.root();
        let Builder {
            rows,
            wanted,
            evaluate,
            values,
            complete,
            ..
        } = b;
        self.state.values = values;
        if complete {
            self.frozen = None;
        }
        self.rows = Rc::new(rows);
        for reference in wanted {
            self.load(reference, cx);
        }
        for expression in evaluate {
            self.evaluate_watch(expression, cx);
        }
    }

    /// The rows as drawn: the previous frame's while the new one loads.
    fn shown_rows(&self) -> Rc<Vec<Row>> {
        self.frozen.clone().unwrap_or_else(|| self.rows.clone())
    }

    /// The rows built now.
    pub(crate) fn rows(&mut self, cx: &mut Context<Self>) -> Rc<Vec<Row>> {
        self.rebuild(cx);
        self.rows.clone()
    }

    fn selected_index(&self, rows: &[Row]) -> Option<usize> {
        let selected = self.state.selected.as_deref()?;
        rows.iter().position(|r| r.path == selected)
    }

    /// The selected row's path.
    pub(crate) fn selected_path(&self) -> Option<&str> {
        self.state.selected.as_deref()
    }

    pub(crate) fn deselect(&mut self, cx: &mut Context<Self>) {
        self.state.selected = None;
        cx.notify();
    }

    /// The selected row, as built now.
    pub(crate) fn selected_row(&mut self, cx: &mut Context<Self>) -> Option<Row> {
        let rows = self.rows(cx);
        let i = self.selected_index(&rows)?;
        Some(rows[i].clone())
    }

    /// A row's path as names: a watch by its expression, the result as
    /// `result`.
    pub fn names(&self, path: &str) -> Vec<String> {
        split(path)
            .into_iter()
            .map(|segment| crate::watches::segment_name(&segment, &self.watches))
            .collect()
    }

    /// The path of the row with these names.
    fn path_of(&self, names: &[String]) -> Option<String> {
        self.rows
            .iter()
            .find(|r| self.names(&r.path) == names)
            .map(|r| r.path.clone())
    }

    // ---- actions -----------------------------------------------------------------

    pub fn select(&mut self, path: &str, cx: &mut Context<Self>) {
        self.state.selected = Some(path.to_owned());
        if let Some(i) = self.rows.iter().position(|r| r.path == path) {
            self.scroll.scroll_to_item(i, ScrollStrategy::Top);
        }
        cx.notify();
    }

    /// Selects the node at a path of names (`["items", "0"]`).
    pub fn select_names(&mut self, names: &[String], cx: &mut Context<Self>) -> bool {
        self.rebuild(cx);
        match self.path_of(names) {
            Some(path) => {
                self.select(&path, cx);
                true
            }
            None => false,
        }
    }

    pub fn set_expanded(&mut self, path: &str, expanded: bool, cx: &mut Context<Self>) {
        if expanded {
            self.state.expanded.insert(path.to_owned());
        } else {
            self.state.expanded.remove(path);
        }
        self.dirty = true;
        cx.notify();
    }

    /// Expands or collapses the node at a path of names; `None` toggles.
    pub fn expand_names(
        &mut self,
        names: &[String],
        expanded: Option<bool>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.rebuild(cx);
        let Some(path) = self.path_of(names) else {
            return false;
        };
        let Some(row) = self.rows.iter().find(|r| r.path == path) else {
            return false;
        };
        if matches!(row.kind, RowKind::More { .. }) {
            self.set_expanded(&path, true, cx);
            return true;
        }
        if !row.expandable() {
            return false;
        }
        let expanded = expanded.unwrap_or(!row.expanded());
        self.set_expanded(&path, expanded, cx);
        true
    }

    /// Up and Down: the row above or below the selected one.
    pub fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.rebuild(cx);
        let rows = self.rows.clone();
        if rows.is_empty() {
            return;
        }
        let next = match self.selected_index(&rows) {
            Some(i) => (i as isize + delta).clamp(0, rows.len() as isize - 1) as usize,
            None => 0,
        };
        self.select(&rows[next].path, cx);
    }

    /// Left: collapses the selected node, or selects its parent; Right:
    /// expands it, or selects its first child.
    pub fn left_right(&mut self, right: bool, cx: &mut Context<Self>) {
        self.rebuild(cx);
        let rows = self.rows.clone();
        let Some(i) = self.selected_index(&rows) else {
            self.move_selection(0, cx);
            return;
        };
        let row = &rows[i];
        if right {
            let more = matches!(row.kind, RowKind::More { .. });
            if more || (row.expandable() && !row.expanded()) {
                self.set_expanded(&row.path, true, cx);
            } else if row.expanded()
                && let Some(next) = rows.get(i + 1)
            {
                self.select(&next.path, cx);
            }
        } else if row.expandable() && row.expanded() {
            self.set_expanded(&row.path, false, cx);
        } else {
            let parent = parent_of(&row.path);
            if rows.iter().any(|r| r.path == parent) {
                self.select(parent, cx);
            }
        }
    }

    /// The selected value's row: a variable's, a watch's or the result's.
    pub fn selected_value(&self) -> Option<ValueRow> {
        let selected = self.state.selected.as_deref()?;
        self.rows
            .iter()
            .find(|r| r.path == selected)
            .and_then(|r| r.value().cloned())
    }

    /// The selected row's path as names.
    pub fn selected_names(&self) -> Option<Vec<String>> {
        self.state.selected.as_deref().map(|p| self.names(p))
    }

    /// The tree as `debug.state` reports it.
    pub fn state_json(&self, cx: &App) -> Json {
        let rows = self.shown_rows();
        let selected = self.state.selected.as_deref();
        json!({
            "frame": self.frame.as_ref().map(|f| f.id),
            "selected": selected.map(|p| self.names(p)),
            "watches": self.watches,
            "paused": self.paused,
            "evaluate": self.field_json(cx),
            "editing": self.editing_json(cx),
            "menu": self.menu.as_ref().map(|(m, _, _)| m.read(cx).state_json()),
            "history": self.history_json(),
            "rows": rows.iter().map(|r| {
                let mut row = json!({
                    "path": self.names(&r.path),
                    "depth": r.depth,
                    "text": r.text(),
                    "selected": selected == Some(r.path.as_str()),
                });
                let value = |row: &mut Json, v: &ValueRow| {
                    row["kind"] = json!(v.kind.as_str());
                    row["name"] = json!(v.name);
                    row["type"] = json!(v.type_text);
                    row["value"] = json!(v.shown.clone().unwrap_or_else(|| value_text(&v.variable.value)));
                    row["expandable"] = json!(v.variable.reference > 0);
                    row["expanded"] = json!(v.expanded);
                    row["changed"] = json!(v.changed);
                    if let Some(renderer) = &v.renderer {
                        row["renderer"] = json!(renderer);
                    }
                };
                match &r.kind {
                    RowKind::Value(v) => value(&mut row, v),
                    RowKind::Watch(w) => {
                        if let WatchValue::Value(v) = &w.value {
                            value(&mut row, v);
                        }
                        row["kind"] = json!(if w.result { "result" } else { "watch" });
                        row["name"] = json!(w.name);
                        row["expression"] = json!(w.expression);
                        row["watch"] = json!(w.index);
                        row["state"] = json!(w.value.as_str());
                        row["paused"] = json!(w.paused);
                        if let WatchValue::Error(e) = &w.value {
                            row["error"] = json!(e);
                        }
                    }
                    RowKind::Group { expanded, .. } => {
                        row["kind"] = json!("group");
                        row["expandable"] = json!(true);
                        row["expanded"] = json!(expanded);
                    }
                    RowKind::Message { role, .. } => {
                        row["kind"] = json!(match role {
                            UiRole::DebuggerErrorMessage => "error",
                            UiRole::DebuggerCollectingData => "loading",
                            _ => "info",
                        });
                    }
                    RowKind::More { .. } => {
                        row["kind"] = json!("more");
                    }
                }
                row
            }).collect::<Vec<_>>(),
        })
    }

    // ---- drawing -----------------------------------------------------------------

    /// Where the chevrons are, for the indent guides.
    fn guides() -> TreeGuides {
        TreeGuides {
            first: px(ROW_INSET + ROW_LEFT),
            indent: px(INDENT),
            control: px(CHEVRON),
        }
    }

    fn render_row(&self, row: &Row, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let row_h = theme.metric(Metric::RowHeight);
        let selected = self.state.selected.as_deref() == Some(row.path.as_str());
        let left = px(ROW_LEFT + INDENT * row.depth as f32);
        let selection = if focused {
            theme.color(UiRole::Selection)
        } else {
            theme.color(UiRole::SelectionInactive)
        };
        let text = theme.color(UiRole::Text);
        let path = row.path.clone();
        let chevron = row.expandable().then(|| {
            let expanded = row.expanded();
            let path = row.path.clone();
            div()
                .id(SharedString::from(format!(
                    "variables-chevron-{}",
                    row.path
                )))
                .absolute()
                .left(left)
                .top((row_h - px(CHEVRON)) / 2.0)
                .size(px(CHEVRON))
                .child(ui::components::tree_arrow(expanded, cx))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.set_expanded(&path, !expanded, cx);
                }))
        });
        let content_left = left + px(CHEVRON_ROOM);
        let icon = |name: IconName, room: f32| {
            div()
                .flex_none()
                .w(px(room))
                .child(Icon::new(name).size(px(16.0)))
        };
        let name_div = |name: &str, role: UiRole| {
            div()
                .flex_none()
                .text_color(theme.color(role))
                .child(name.to_owned())
        };
        let eq = || div().flex_none().text_color(text).child("\u{a0}=\u{a0}");
        // A value's type and value, after its name.
        let value_tail = |v: &ValueRow| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .when_some(v.type_text.clone(), |d, t| {
                    d.child(
                        div()
                            .text_color(theme.color(UiRole::DebuggerType))
                            .child(format!("{{{t}}}\u{a0}")),
                    )
                })
                .child(
                    div().text_color(text).child(
                        v.shown
                            .clone()
                            .unwrap_or_else(|| value_text(&v.variable.value)),
                    ),
                )
        };
        let kind_icon = |v: &ValueRow| match v.kind {
            ValueKind::Primitive => icons::PRIMITIVE,
            ValueKind::Array => icons::ARRAY,
            ValueKind::Object => icons::OBJECT,
        };
        let name_role = |v: &ValueRow| {
            if v.changed {
                UiRole::DebuggerChangedValue
            } else {
                UiRole::DebuggerValueName
            }
        };
        // The editor of the row, when it is edited in place: after the
        // icon (a watch) or after `name = ` (Set Value).
        let edit = self.editing.as_ref().filter(|e| e.path == row.path);
        let editor = edit.map(|e| self.render_inline_editor(e, cx));
        let content: AnyElement = match (&row.kind, editor) {
            (RowKind::Value(v), Some(editor)) => div()
                .flex()
                .flex_row()
                .items_center()
                .w_full()
                .child(icon(kind_icon(v), ICON_ROOM))
                .child(name_div(&v.name, name_role(v)))
                .child(eq())
                .child(editor)
                .into_any_element(),
            (RowKind::Watch(w), Some(editor)) => {
                let i = match &w.value {
                    _ if w.result => icons::EVALUATION_RESULT,
                    _ if w.paused => icons::PAUSE,
                    WatchValue::Value(v) => kind_icon(v),
                    _ => icons::WATCH,
                };
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .w_full()
                    .child(icon(i, 16.0))
                    .child(editor)
                    .into_any_element()
            }
            (RowKind::Value(v), None) => div()
                .flex()
                .flex_row()
                .items_center()
                .child(icon(kind_icon(v), ICON_ROOM))
                .child(name_div(&v.name, name_role(v)))
                .child(eq())
                .child(value_tail(v))
                .into_any_element(),
            (RowKind::Watch(w), None) => self.render_watch_content(w, cx),
            (RowKind::Group { title, icon: i, .. }, _) => div()
                .flex()
                .flex_row()
                .items_center()
                .child(icon(*i, ICON_ROOM))
                .child(div().text_color(text).child(title.clone()))
                .into_any_element(),
            (
                RowKind::Message {
                    text: message,
                    role,
                    icon: i,
                },
                _,
            ) => div()
                .flex()
                .flex_row()
                .items_center()
                .when_some(*i, |d, i| d.child(icon(i, ICON_ROOM)))
                .child(div().text_color(theme.color(*role)).child(message.clone()))
                .into_any_element(),
            (RowKind::More { text: message }, _) => div()
                .text_color(theme.color(UiRole::TextMuted))
                .child(message.clone())
                .into_any_element(),
        };
        let more = matches!(row.kind, RowKind::More { .. });
        let expandable = row.expandable();
        let expanded = row.expanded();
        let menu_path = row.path.clone();
        let edited = edit.is_some();
        // A new watch's row is not selected while it is typed
        // (`WatchesRootNode.editWatch`).
        let new_watch = edit.is_some_and(|e| e.new_watch_at().is_some());
        let guides = Self::guides().row(
            row.depth,
            expandable && expanded,
            row_h,
            theme.color(UiRole::TreeIndentGuide),
        );
        div()
            .id(SharedString::from(format!("variables-row-{}", row.path)))
            .debug_selector({
                let names = self.names(&row.path).join("/");
                move || format!("variables-row-{names}")
            })
            .relative()
            .h(row_h)
            .w_full()
            .px(px(ROW_INSET))
            .children(guides)
            .child(
                div()
                    .relative()
                    .size_full()
                    .rounded(theme.metric(Metric::CornerRadius))
                    .when(selected && !new_watch, |d| d.bg(selection))
                    .child(
                        div()
                            .absolute()
                            .left(content_left)
                            .top_0()
                            .h(row_h)
                            // The editor reaches nearly to the tree's edge.
                            .when(edited, |d| {
                                d.right(px(crate::watches::EDITOR_RIGHT - ROW_INSET))
                            })
                            .flex()
                            .items_center()
                            .whitespace_nowrap()
                            .child(content),
                    )
                    .children(chevron),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    if edited {
                        return;
                    }
                    window.focus(&this.focus);
                    this.select(&path, cx);
                    if e.click_count == 2 && (more || expandable) {
                        let path = this.state.selected.clone().unwrap_or_default();
                        this.set_expanded(&path, more || !expanded, cx);
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    if edited {
                        return;
                    }
                    window.focus(&this.focus);
                    this.select(&menu_path, cx);
                    this.open_menu(e.position, window, cx);
                }),
            )
            .into_any_element()
    }
}

/// Builds the rows of a view.
struct Builder<'a> {
    view: &'a VariablesView,
    rows: Vec<Row>,
    /// References to load.
    wanted: Vec<i64>,
    /// Watch expressions to evaluate.
    evaluate: Vec<String>,
    values: HashMap<String, String>,
    /// Every node of the root is loaded.
    complete: bool,
}

impl Builder<'_> {
    /// The evaluate field's result and the watches, before the frame's
    /// values (`WatchesRootNode.getChildren`); while the program runs the
    /// watches show their expressions only. A paused watch is not
    /// evaluated (`WatchNodeImpl.XWatchValue.computePresentation`).
    fn watches(&mut self) {
        let view = self.view;
        let frame = view.frame.is_some() && view.phase == Phase::Suspended;
        if let Some((expression, eval)) = &view.result {
            let eval = frame.then_some(eval);
            self.watch(crate::watches::RESULT, expression, None, true, false, eval);
        }
        let new = view.editing.as_ref().and_then(InlineEdit::new_watch_at);
        for (i, expression) in view.watches.iter().enumerate() {
            if new == Some(i) {
                self.watch(crate::watches::NEW, "", None, false, false, None);
            }
            let paused = view.paused.get(i).copied().unwrap_or(false);
            let eval = match view.evaluations.get(expression) {
                Some(eval) => Some(eval),
                None if frame && paused => Some(&Eval::Paused),
                None if frame => {
                    if !self.evaluate.contains(expression) {
                        self.evaluate.push(expression.clone());
                    }
                    Some(&Eval::Pending)
                }
                None => None,
            };
            let eval = eval.filter(|_| frame);
            self.watch(
                &crate::watches::watch_segment(i),
                expression,
                Some(i),
                false,
                paused,
                eval,
            );
        }
        if new.is_some_and(|i| i >= view.watches.len()) {
            self.watch(crate::watches::NEW, "", None, false, false, None);
        }
    }

    fn watch(
        &mut self,
        segment: &str,
        expression: &str,
        index: Option<usize>,
        result: bool,
        paused: bool,
        eval: Option<&Eval>,
    ) {
        let view = self.view;
        let path = segment.to_owned();
        let name = if result {
            crate::watches::RESULT_NAME.to_owned()
        } else {
            expression.to_owned()
        };
        let value = match eval {
            None => WatchValue::None,
            Some(Eval::Pending) => {
                self.complete = false;
                WatchValue::Pending
            }
            Some(Eval::Paused) => WatchValue::Paused,
            Some(Eval::Error(message)) => WatchValue::Error(match &view.provider {
                Some(p) => p.evaluation_error(message).shown(),
                None => message.clone(),
            }),
            Some(Eval::Value { variable, shape }) => {
                let shown = value_text(&variable.value);
                // The result is new each time it is evaluated: changed
                // unless it shows what it showed before.
                let changed = if result {
                    view.result_before.as_deref() != Some(shown.as_str())
                } else {
                    !paused
                        && view
                            .previous
                            .as_ref()
                            .and_then(|p| p.values.get(&path))
                            .is_some_and(|old| *old != shown)
                };
                self.values.insert(path.clone(), shown);
                let kind = match view
                    .provider
                    .as_ref()
                    .map(|p| p.present_value(variable).role)
                {
                    Some(ValueRole::Value(kind)) => kind,
                    _ if variable.reference > 0 => ValueKind::Object,
                    _ => ValueKind::Primitive,
                };
                let expanded = variable.reference > 0 && view.state.expanded.contains(&path);
                let (renderer, rendered) = self.rendered(&path, variable);
                WatchValue::Value(Box::new(ValueRow {
                    name: name.clone(),
                    type_text: type_text(variable.type_name.as_deref(), shape.as_deref()),
                    shape: shape.clone(),
                    kind,
                    expanded,
                    changed,
                    parent: 0,
                    variable: (**variable).clone(),
                    renderer,
                    shown: rendered,
                }))
            }
        };
        let children = match &value {
            WatchValue::Value(v) if v.expanded => Some(v.variable.reference),
            _ => None,
        };
        self.rows.push(Row {
            path: path.clone(),
            depth: 0,
            kind: RowKind::Watch(Box::new(WatchRow {
                name,
                expression: expression.to_owned(),
                index,
                result,
                paused,
                value,
            })),
        });
        if let Some(reference) = children {
            self.children(reference, &path, 1, false);
        }
        // Set Value or an edited watch failed: the message under its row.
        if let Some(error) = view
            .editing
            .as_ref()
            .filter(|e| e.path == path)
            .and_then(|e| e.error.clone())
        {
            self.message(
                &path,
                1,
                &error,
                UiRole::DebuggerErrorMessage,
                Some(IconName::ERROR),
            );
        }
    }

    /// The renderer View as chose for the value at `path` (or the type's
    /// default when it has a choice) and the value's text as it shows it.
    fn rendered(&self, path: &str, variable: &Variable) -> (Option<String>, Option<String>) {
        let Some(provider) = &self.view.provider else {
            return (None, None);
        };
        let (renderers, default) =
            provider.value_renderers(variable.type_name.as_deref().unwrap_or(""));
        if renderers.is_empty() {
            return (None, None);
        }
        let renderer = self
            .view
            .renderers
            .get(path)
            .filter(|r| renderers.contains(&r.as_str()))
            .cloned()
            .unwrap_or_else(|| default.to_owned());
        let shown = provider.render_value(&renderer, &variable.value);
        (Some(renderer), shown)
    }

    fn message(
        &mut self,
        path: &str,
        depth: usize,
        text: &str,
        role: UiRole,
        icon: Option<IconName>,
    ) {
        self.rows.push(Row {
            path: join(path, &format!("\u{2}{text}")),
            depth,
            kind: RowKind::Message {
                text: text.to_owned(),
                role,
                icon,
            },
        });
    }

    fn root(&mut self) {
        let view = self.view;
        self.watches();
        let state_message = match view.phase {
            Phase::Starting => Some(CONNECTING),
            Phase::Running => Some(CONNECTED),
            Phase::Stopping | Phase::Ended => Some(DISCONNECTED),
            Phase::Suspended => None,
        };
        if let Some(text) = state_message {
            self.message("", 0, text, UiRole::Text, Some(IconName::INFO));
            return;
        }
        if view.frame.is_none() {
            self.message(
                "",
                0,
                FRAME_NOT_AVAILABLE,
                UiRole::Text,
                Some(IconName::INFO),
            );
            return;
        }
        let scopes = match &view.scopes {
            None => {
                self.complete = false;
                self.message("", 0, COLLECTING_DATA, UiRole::DebuggerCollectingData, None);
                return;
            }
            Some(Err(error)) => {
                let error = error.clone();
                self.message(
                    "",
                    0,
                    &error,
                    UiRole::DebuggerErrorMessage,
                    Some(IconName::ERROR),
                );
                return;
            }
            Some(Ok(scopes)) => scopes.clone(),
        };
        let first_only = view.provider.as_ref().is_some_and(|p| p.first_scope_only());
        if first_only {
            if let Some(scope) = scopes.first() {
                self.children(scope.reference, "", 0, true);
            }
            return;
        }
        for (i, scope) in scopes.iter().enumerate() {
            let path = join("", &scope.name);
            let expanded = view.state.expanded.contains(&path);
            self.rows.push(Row {
                path: path.clone(),
                depth: 0,
                kind: RowKind::Group {
                    title: scope.name.clone(),
                    icon: IconName::FOLDER,
                    expanded,
                },
            });
            if expanded {
                self.children(scope.reference, &path, 1, i == 0);
            }
        }
    }

    /// The rows of a scope's or a value's children.
    fn children(&mut self, reference: i64, path: &str, depth: usize, root: bool) {
        let view = self.view;
        let Some(children) = view.session.loaded_children(reference) else {
            if let Some(error) = view.errors.get(&reference) {
                let error = error.clone();
                self.message(
                    path,
                    depth,
                    &error,
                    UiRole::DebuggerErrorMessage,
                    Some(IconName::ERROR),
                );
            } else {
                if root {
                    self.complete = false;
                }
                if !view.loading.contains(&reference) {
                    self.wanted.push(reference);
                }
                self.message(
                    path,
                    depth,
                    COLLECTING_DATA,
                    UiRole::DebuggerCollectingData,
                    None,
                );
            }
            return;
        };
        let mut groups: Vec<(Variable, String, GroupIcon)> = Vec::new();
        let mut returns: Vec<(usize, String)> = Vec::new();
        let mut return_group = None;
        for (i, variable) in children.items.iter().enumerate() {
            let node = match &view.provider {
                Some(p) => p.present_value(variable),
                None => debugger::values::ValueNode::plain(variable),
            };
            match node.role {
                ValueRole::Value(kind) => {
                    self.value(
                        variable,
                        children.shape(i),
                        &node.name,
                        kind,
                        reference,
                        path,
                        depth,
                    );
                }
                ValueRole::Group { title, icon } => groups.push((variable.clone(), title, icon)),
                ValueRole::ReturnValue { group } => {
                    return_group = Some(group);
                    returns.push((i, node.name));
                }
                ValueRole::More { remaining } => {
                    let more_path = join(path, &format!("\u{2026}{}", variable.value));
                    if view.state.expanded.contains(&more_path) {
                        // The rest of the collection, in place.
                        self.children(variable.reference, path, depth, false);
                    } else {
                        self.rows.push(Row {
                            path: more_path,
                            depth,
                            kind: RowKind::More {
                                text: match remaining {
                                    Some(n) => format!(
                                        "... ({n} more {}. Double-click to see)",
                                        if n == 1 { "item" } else { "items" }
                                    ),
                                    None => "... (Double-click to see more items)".to_owned(),
                                },
                            },
                        });
                    }
                }
            }
        }
        // Return Values first, then the adapter's groups
        // (`PyStackFrame.addChildren`).
        if let Some(title) = return_group {
            let group_path = join(path, &title);
            let expanded = view.state.expanded.contains(&group_path);
            self.rows.push(Row {
                path: group_path.clone(),
                depth,
                kind: RowKind::Group {
                    title,
                    icon: icons::RETURN_VALUES,
                    expanded,
                },
            });
            if expanded {
                for (i, name) in returns {
                    let variable = &children.items[i];
                    let kind = match &view.provider {
                        Some(p) => match p.present_value(variable).role {
                            ValueRole::Value(kind) => kind,
                            _ => ValueKind::Object,
                        },
                        None => ValueKind::Primitive,
                    };
                    let kind = if variable.reference > 0 {
                        kind
                    } else {
                        ValueKind::Primitive
                    };
                    self.value(
                        variable,
                        children.shape(i),
                        &name,
                        kind,
                        reference,
                        &group_path,
                        depth + 1,
                    );
                }
            }
        }
        for (variable, title, icon) in groups {
            let group_path = join(path, &title);
            let expanded = view.state.expanded.contains(&group_path);
            self.rows.push(Row {
                path: group_path.clone(),
                depth,
                kind: RowKind::Group {
                    title,
                    icon: match icon {
                        GroupIcon::Special => icons::SPECIAL_VARIABLES,
                        GroupIcon::ReturnValues => icons::RETURN_VALUES,
                        GroupIcon::Plain => IconName::FOLDER,
                    },
                    expanded,
                },
            });
            if expanded {
                self.children(variable.reference, &group_path, depth + 1, false);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn value(
        &mut self,
        variable: &Variable,
        shape: Option<&str>,
        name: &str,
        kind: ValueKind,
        parent: i64,
        parent_path: &str,
        depth: usize,
    ) {
        let view = self.view;
        let path = join(parent_path, name);
        let shown = value_text(&variable.value);
        let changed = view
            .previous
            .as_ref()
            .is_some_and(|p| match p.values.get(&path) {
                Some(old) => *old != shown,
                None => parent_path.is_empty() || p.expanded.contains(parent_path),
            });
        self.values.insert(path.clone(), shown);
        let expanded = variable.reference > 0 && view.state.expanded.contains(&path);
        let (renderer, rendered) = self.rendered(&path, variable);
        self.rows.push(Row {
            path: path.clone(),
            depth,
            kind: RowKind::Value(Box::new(ValueRow {
                name: name.to_owned(),
                type_text: type_text(variable.type_name.as_deref(), shape),
                shape: shape.map(str::to_owned),
                kind,
                expanded,
                changed,
                parent,
                variable: variable.clone(),
                renderer,
                shown: rendered,
            })),
        });
        // Set Value failed: the message under the row.
        if let Some(error) = view
            .editing
            .as_ref()
            .filter(|e| e.path == path)
            .and_then(|e| e.error.clone())
        {
            self.message(
                &path,
                depth + 1,
                &error,
                UiRole::DebuggerErrorMessage,
                Some(IconName::ERROR),
            );
        }
        if expanded {
            self.children(variable.reference, &path, depth + 1, false);
        }
    }
}

impl Focusable for VariablesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for VariablesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.rebuild(cx);
        self.line_height = editor::field_line_height_for(false, window, cx);
        let rows = self.shown_rows();
        let focused = self.focus.contains_focused(window, cx);
        let count = rows.len();
        let list = uniform_list(
            "variables-rows",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .filter_map(|ix| rows.get(ix).map(|r| this.render_row(r, focused, cx)))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.scroll.clone())
        .size_full();
        let field = self.render_field(window, cx);
        let bounds = self.tree_bounds.clone();
        let tree = div()
            .id("debug-variables")
            .debug_selector(|| "debug-variables".into())
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .relative()
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .pt(px(TREE_TOP))
            .pb(px(2.0))
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .child(list)
            // A double-click below the rows adds a watch
            // (`XWatchesViewImpl.installEditListeners`).
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus);
                    if e.click_count == 2 {
                        this.new_watch(None, window, cx);
                    }
                }),
            );
        let history = self.render_history(window, cx);
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(field)
            .child(tree)
            .children(history)
            .children(
                self.menu
                    .as_ref()
                    .map(|(menu, position, _)| ui::menu::overlay(menu, *position, window)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_join_and_split_names() {
        let p = join(&join("", "items"), "0");
        assert_eq!(split(&p), vec!["items".to_owned(), "0".to_owned()]);
        assert_eq!(parent_of(&p), "items");
        assert_eq!(parent_of("items"), "");
        assert!(split("").is_empty());
    }
}
