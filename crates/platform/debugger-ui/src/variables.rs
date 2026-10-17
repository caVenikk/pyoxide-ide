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

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use debugger::values::{GroupIcon, ValueKind, ValueRole, row_text, type_text, value_text};
use debugger::{
    DebugAdapterProvider, DebugError, Phase, Scope, Session, SessionSnapshot, Variable,
};
use gpui::{
    AnyElement, App, ClickEvent, Context, FocusHandle, Focusable, IntoElement, MouseButton,
    MouseDownEvent, Render, ScrollStrategy, SharedString, Task, UniformListScrollHandle, Window,
    div, prelude::*, px, uniform_list,
};
use serde_json::{Value as Json, json};
use themes::{Metric, UiRole};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use crate::icons;

/// The key context of the tree (its keys: `workbench/keymaps`).
pub const KEY_CONTEXT: &str = "DebugVariables";
/// How long the rows of a frame stay after the program moved on
/// (`XDebugView`'s clear alarm).
const CLEAR_DELAY: Duration = Duration::from_millis(100);
/// Space before a top-level row's chevron, one level, the chevron's and
/// the icon's room.
const ROW_LEFT: f32 = 4.0;
const INDENT: f32 = 16.0;
const CHEVRON: f32 = 16.0;
const CHEVRON_ROOM: f32 = 18.0;
const ICON_ROOM: f32 = 20.0;
/// The separator of path segments.
const SEP: char = '\u{1}';

/// The texts of the reference (`XDebuggerBundle`).
const COLLECTING_DATA: &str = "Collecting data\u{2026}";
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
    Value(ValueRow),
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
    More { text: String },
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
}

impl Row {
    fn expandable(&self) -> bool {
        match &self.kind {
            RowKind::Value(v) => v.variable.reference > 0,
            RowKind::Group { .. } => true,
            _ => false,
        }
    }

    fn expanded(&self) -> bool {
        match &self.kind {
            RowKind::Value(v) => v.expanded,
            RowKind::Group { expanded, .. } => *expanded,
            _ => false,
        }
    }

    /// The row's text: `items = {list: 4} [1, 2, 3, 4]`, a group's title,
    /// a message.
    pub fn text(&self) -> String {
        match &self.kind {
            RowKind::Value(v) => row_text(
                &v.name,
                v.variable.type_name.as_deref(),
                v.shape.as_deref(),
                &v.variable.value,
            ),
            RowKind::Group { title, .. } => title.clone(),
            RowKind::Message { text, .. } | RowKind::More { text } => text.clone(),
        }
    }

    /// The path as names.
    pub fn names(&self) -> Vec<String> {
        split(&self.path)
    }
}

fn join(parent: &str, segment: &str) -> String {
    if parent.is_empty() {
        segment.to_owned()
    } else {
        format!("{parent}{SEP}{segment}")
    }
}

fn split(path: &str) -> Vec<String> {
    if path.is_empty() {
        Vec::new()
    } else {
        path.split(SEP).map(str::to_owned).collect()
    }
}

fn parent_of(path: &str) -> &str {
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
struct ShownFrame {
    id: i64,
    /// Frames of the same function share their state (the reference's
    /// `XStackFrame.getEqualityObject`).
    key: String,
}

/// The variables of a debug session.
pub struct VariablesView {
    session: Session,
    provider: Option<Arc<dyn DebugAdapterProvider>>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    phase: Phase,
    frame: Option<ShownFrame>,
    /// Grows with every frame shown: late answers for another are dropped.
    epoch: u64,
    scopes: Option<Result<Vec<Scope>, String>>,
    loading: HashSet<i64>,
    errors: HashMap<i64, String>,
    state: TreeState,
    /// The state of the frame's function when it was shown last.
    previous: Option<TreeState>,
    saved: HashMap<String, TreeState>,
    rows: Rc<Vec<Row>>,
    /// The rows of the frame shown before, while the new one loads.
    frozen: Option<Rc<Vec<Row>>>,
    _unfreeze: Option<Task<()>>,
}

impl VariablesView {
    pub fn new(
        session: Session,
        provider: Option<Arc<dyn DebugAdapterProvider>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let phase = session.phase();
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
            frozen: None,
            _unfreeze: None,
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The session changed: the tree follows its selected frame. A new
    /// frame keeps the expanded nodes and the selection of its function.
    pub fn follow(&mut self, snapshot: &SessionSnapshot, cx: &mut Context<Self>) {
        self.phase = snapshot.phase;
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
                    key: format!("{path}:{}", f.name),
                })
            })
            .flatten();
        if frame.as_ref().map(|f| f.id) == self.frame.as_ref().map(|f| f.id) {
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
        self.freeze(cx);
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
                cx.notify();
            });
        })
        .detach();
    }

    // ---- rows ------------------------------------------------------------------

    /// Builds the rows from the session's loaded values and loads what an
    /// expanded node needs.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let mut b = Builder {
            view: self,
            rows: Vec::new(),
            wanted: Vec::new(),
            values: HashMap::new(),
            complete: true,
        };
        b.root();
        let Builder {
            rows,
            wanted,
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
    }

    /// The rows as drawn: the previous frame's while the new one loads.
    fn shown_rows(&self) -> Rc<Vec<Row>> {
        self.frozen.clone().unwrap_or_else(|| self.rows.clone())
    }

    fn selected_index(&self, rows: &[Row]) -> Option<usize> {
        let selected = self.state.selected.as_deref()?;
        rows.iter().position(|r| r.path == selected)
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
        let path = names.join(&SEP.to_string());
        if self.rows.iter().any(|r| r.path == path) {
            self.select(&path, cx);
            true
        } else {
            false
        }
    }

    pub fn set_expanded(&mut self, path: &str, expanded: bool, cx: &mut Context<Self>) {
        if expanded {
            self.state.expanded.insert(path.to_owned());
        } else {
            self.state.expanded.remove(path);
        }
        cx.notify();
    }

    /// Expands or collapses the node at a path of names; `None` toggles.
    pub fn expand_names(
        &mut self,
        names: &[String],
        expanded: Option<bool>,
        cx: &mut Context<Self>,
    ) -> bool {
        let path = names.join(&SEP.to_string());
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
        let rows = self.rows.clone();
        let Some(i) = self.selected_index(&rows) else {
            self.move_selection(0, cx);
            return;
        };
        let row = &rows[i];
        if right {
            if matches!(row.kind, RowKind::More { .. }) {
                self.set_expanded(&row.path, true, cx);
            } else if row.expandable() && !row.expanded() {
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

    /// The selected value's row.
    pub fn selected_value(&self) -> Option<ValueRow> {
        let selected = self.state.selected.as_deref()?;
        self.rows.iter().find_map(|r| match &r.kind {
            RowKind::Value(v) if r.path == selected => Some(v.clone()),
            _ => None,
        })
    }

    /// The selected row's path as names.
    pub fn selected_names(&self) -> Option<Vec<String>> {
        self.state.selected.as_deref().map(split)
    }

    /// The tree as `debug.state` reports it.
    pub fn state_json(&self) -> Json {
        let rows = self.shown_rows();
        let selected = self.state.selected.as_deref();
        json!({
            "frame": self.frame.as_ref().map(|f| f.id),
            "selected": selected.map(split),
            "rows": rows.iter().map(|r| {
                let mut row = json!({
                    "path": r.names(),
                    "depth": r.depth,
                    "text": r.text(),
                    "selected": selected == Some(r.path.as_str()),
                });
                match &r.kind {
                    RowKind::Value(v) => {
                        row["kind"] = json!(v.kind.as_str());
                        row["name"] = json!(v.name);
                        row["type"] = json!(v.type_text);
                        row["value"] = json!(value_text(&v.variable.value));
                        row["expandable"] = json!(v.variable.reference > 0);
                        row["expanded"] = json!(v.expanded);
                        row["changed"] = json!(v.changed);
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
                .id(SharedString::from(format!("variables-chevron-{}", row.path)))
                .absolute()
                .left(left)
                .top((row_h - px(CHEVRON)) / 2.0)
                .child(Icon::new(if expanded {
                    IconName::CHEVRON_DOWN
                } else {
                    IconName::CHEVRON_RIGHT
                }))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.set_expanded(&path, !expanded, cx);
                }))
        });
        let content_left = left + px(CHEVRON_ROOM);
        let icon = |name: IconName| {
            div()
                .flex_none()
                .w(px(ICON_ROOM))
                .child(Icon::new(name).size(px(16.0)))
        };
        let content: AnyElement = match &row.kind {
            RowKind::Value(v) => {
                let name_role = if v.changed {
                    UiRole::DebuggerChangedValue
                } else {
                    UiRole::DebuggerValueName
                };
                let kind_icon = match v.kind {
                    ValueKind::Primitive => icons::PRIMITIVE,
                    ValueKind::Array => icons::ARRAY,
                    ValueKind::Object => icons::OBJECT,
                };
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .child(icon(kind_icon))
                    .child(div().text_color(theme.color(name_role)).child(v.name.clone()))
                    .child(div().text_color(text).child("\u{a0}=\u{a0}"))
                    .when_some(v.type_text.clone(), |d, t| {
                        d.child(
                            div()
                                .text_color(theme.color(UiRole::DebuggerType))
                                .child(format!("{{{t}}}\u{a0}")),
                        )
                    })
                    .child(div().text_color(text).child(value_text(&v.variable.value)))
                    .into_any_element()
            }
            RowKind::Group { title, icon: i, .. } => div()
                .flex()
                .flex_row()
                .items_center()
                .child(icon(*i))
                .child(div().text_color(text).child(title.clone()))
                .into_any_element(),
            RowKind::Message {
                text: message,
                role,
                icon: i,
            } => div()
                .flex()
                .flex_row()
                .items_center()
                .when_some(*i, |d, i| d.child(icon(i)))
                .child(div().text_color(theme.color(*role)).child(message.clone()))
                .into_any_element(),
            RowKind::More { text: message } => div()
                .text_color(theme.color(UiRole::TextMuted))
                .child(message.clone())
                .into_any_element(),
        };
        let more = matches!(row.kind, RowKind::More { .. });
        let expandable = row.expandable();
        let expanded = row.expanded();
        div()
            .id(SharedString::from(format!("variables-row-{}", row.path)))
            .debug_selector({
                let names = row.names().join("/");
                move || format!("variables-row-{names}")
            })
            .relative()
            .h(row_h)
            .w_full()
            .px(px(4.0))
            .child(
                div()
                    .size_full()
                    .rounded(theme.metric(Metric::CornerRadius))
                    .when(selected, |d| d.bg(selection))
                    .child(
                        div()
                            .absolute()
                            .left(content_left)
                            .top_0()
                            .h(row_h)
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
                    window.focus(&this.focus);
                    this.select(&path, cx);
                    if e.click_count == 2 && (more || expandable) {
                        let path = this.state.selected.clone().unwrap_or_default();
                        this.set_expanded(&path, more || !expanded, cx);
                    }
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
    values: HashMap<String, String>,
    /// Every node of the root is loaded.
    complete: bool,
}

impl Builder<'_> {
    fn message(&mut self, path: &str, depth: usize, text: &str, role: UiRole, icon: Option<IconName>) {
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
            self.message("", 0, FRAME_NOT_AVAILABLE, UiRole::Text, Some(IconName::INFO));
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
                self.message("", 0, &error, UiRole::DebuggerErrorMessage, Some(IconName::ERROR));
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
                self.message(path, depth, &error, UiRole::DebuggerErrorMessage, Some(IconName::ERROR));
            } else {
                if root {
                    self.complete = false;
                }
                if !view.loading.contains(&reference) {
                    self.wanted.push(reference);
                }
                self.message(path, depth, COLLECTING_DATA, UiRole::DebuggerCollectingData, None);
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
                    self.value(variable, children.shape(i), &node.name, kind, reference, path, depth);
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
        let changed = view.previous.as_ref().is_some_and(|p| match p.values.get(&path) {
            Some(old) => *old != shown,
            None => parent_path.is_empty() || p.expanded.contains(parent_path),
        });
        self.values.insert(path.clone(), shown);
        let expanded = variable.reference > 0 && view.state.expanded.contains(&path);
        self.rows.push(Row {
            path: path.clone(),
            depth,
            kind: RowKind::Value(ValueRow {
                name: name.to_owned(),
                type_text: type_text(variable.type_name.as_deref(), shape),
                shape: shape.map(str::to_owned),
                kind,
                expanded,
                changed,
                parent,
                variable: variable.clone(),
            }),
        });
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
        div()
            .id("debug-variables")
            .debug_selector(|| "debug-variables".into())
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .py(px(2.0))
            .child(list)
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
