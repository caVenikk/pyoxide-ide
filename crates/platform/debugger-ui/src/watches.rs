//! The watches and the evaluate field of Threads & Variables (the
//! reference's `XWatchesViewImpl` with `debugger.watches.in.variables`,
//! `WatchesRootNode`, `WatchNodeImpl`):
//!
//! - the evaluate field above the variables (`XDebuggerEmbeddedComboBox`):
//!   Enter evaluates its expression in the selected frame and shows the
//!   value as the first node, `result`; Add to Watches (its button and
//!   ⇧⌘Enter) adds the expression to the watches; the chevron lists the
//!   expressions evaluated before (`XDebuggerHistoryManager`, kept per
//!   project) in the combo box's list; names the frame's code does not
//!   declare are marked and outline the field in red
//!   (`XDebuggerExpressionComboBox`'s error outline);
//! - the watches after the result and before the frame's values, each
//!   evaluated in the frame shown, kept per run configuration type in the
//!   personal project state; New Watch, Remove Watch, Remove All Watches,
//!   Edit and Pause Watch change them, a watch is edited in its row
//!   (`WatchInplaceEditor`); a paused watch is not evaluated at the next
//!   stops (`XPauseWatchAction`);
//! - Set Value in the row of a value (`SetValueInplaceEditor`); the editors
//!   of the rows are combo boxes with a history of their own; Copy Value
//!   (`XCopyValueAction`, the whole value in the clipboard context), Copy
//!   Name, Add to Watches, Evaluate Expression (into the field), Jump To
//!   Source (`XJumpToSourceAction`, the product finds the declaration),
//!   Jump To Type Source (the running program finds the type), View as (the
//!   product's renderers) and the context menu with them
//!   (`XDebugger.Watches.Tree.Popup`, `XDebugger.ValueGroup`).

use std::ops::Range;
use std::sync::Arc;

use debugger::values::{ValueRole, value_text};
use debugger::{
    DebugAdapterProvider, DebugError, EvaluateContext, HistoryKind, Phase, Session, Variable,
};
use editor::{DiagnosticMark, Editor, EditorEvent, MarkMessage, MarkSeverity};
use gpui::{
    AnyElement, App, Bounds, ClipboardItem, Context, Entity, Focusable, InteractiveElement,
    IntoElement, KeystrokeEvent, MouseButton, ParentElement, Pixels, Point, Styled, Subscription,
    Window, anchored, canvas, div, point, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::{Metric, UiRole};
use ui::components::{DISABLED_ALPHA, Tooltip};
use ui::deferred;
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem};
use ui::theme::ThemeExt;

use crate::DebuggerView;
use crate::icons;
use crate::session::commands::*;
use crate::variables::{ICON_ROOM, Row, RowKind, ValueRow, VariablesView};

/// The path segment of the evaluate field's result and its name
/// (`debugger.result.node.name`).
pub(crate) const RESULT: &str = "\u{4}result";
pub(crate) const RESULT_NAME: &str = "result";
/// The path segment of a new watch being typed.
pub(crate) const NEW: &str = "\u{3}new";
/// The key context of the evaluate field and of the editor in a row.
pub const FIELD_CONTEXT: &str = "DebugEvaluate";
pub const EDITOR_CONTEXT: &str = "DebugValueEditor";
/// The evaluate field's hint, with the keys of Evaluate (Enter) and Add to
/// Watches (`debugger.evaluate.expression.or.add.a.watch.hint`).
const HINT_MACOS: &str = "Evaluate expression (\u{23ce}) or add a watch (\u{21e7}\u{2318}\u{23ce})";
const HINT_OTHERS: &str = "Evaluate expression (Enter) or add a watch (Ctrl+Shift+Enter)";
/// The evaluate field (measured on the reference): 32 high over its bottom
/// line, the text's first pixel 12 from the left, the chevron's centre 18
/// from the right and Add to Watches' 51.
const FIELD_HEIGHT: f32 = 33.0;
const FIELD_LEFT: f32 = 11.0;
const FIELD_RIGHT: f32 = 7.0;
const FIELD_BUTTON: f32 = 22.0;
const FIELD_GAP: f32 = 11.0;
/// The ring of a focused combo box (or of one whose expression has an
/// error), drawn inside it.
const RING: f32 = 2.0;
/// The editor of a row: 30 high, centred on its row, 2 after the name's
/// `=` or the icon, reaching this far from the tree's right edge, with the
/// text 8 after the ring and the arrow's part 26 wide after a line.
const EDITOR_HEIGHT: f32 = 30.0;
pub(crate) const EDITOR_RIGHT: f32 = 3.0;
const EDITOR_LEFT: f32 = 2.0;
const EDITOR_TEXT: f32 = 8.0;
const EDITOR_ARROW: f32 = 26.0;
/// The list of a combo box's history: rows 24 high, the text 7.5 after
/// the border, at most ten rows shown (`setMaximumRowCount(10)`).
const HISTORY_ROW: f32 = 24.0;
const HISTORY_TEXT: f32 = 7.5;
const HISTORY_ROWS: usize = 10;
/// How the result is evaluated: as the debug console does, so that a
/// statement runs too.
const RESULT_CONTEXT: EvaluateContext = EvaluateContext::Repl;
/// A paused watch at a stop (`xdebugger.watch.is.paused.message`) and its
/// link (`xdebugger.evaluate.paused.watch`).
const PAUSED: &str = "The watch is paused";
const EVALUATE_PAUSED: &str = "\u{2026} Evaluate";

/// The path segment of the watch at an index.
pub(crate) fn watch_segment(index: usize) -> String {
    format!("\u{3}{index}")
}

/// The name a path segment shows: a watch's expression, `result`, or the
/// segment itself.
pub(crate) fn segment_name(segment: &str, watches: &[String]) -> String {
    if segment == RESULT {
        return RESULT_NAME.to_owned();
    }
    if segment == NEW {
        return String::new();
    }
    match segment.strip_prefix('\u{3}').map(str::parse::<usize>) {
        Some(Ok(i)) => watches.get(i).cloned().unwrap_or_default(),
        _ => segment.to_owned(),
    }
}

/// A watch's or the result's value at the frame shown.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Eval {
    Pending,
    /// A paused watch, not evaluated at this stop.
    Paused,
    Value {
        variable: Box<Variable>,
        /// The size of a container.
        shape: Option<String>,
    },
    /// The adapter's message.
    Error(String),
}

/// The row of the evaluate field's result or of a watch.
#[derive(Debug, Clone, PartialEq)]
pub struct WatchRow {
    /// The name shown: the expression, `result` for the result.
    pub name: String,
    pub expression: String,
    /// The watch's index; `None` for the result and a new watch.
    pub index: Option<usize>,
    pub result: bool,
    pub paused: bool,
    pub value: WatchValue,
}

impl WatchRow {
    pub(crate) fn text(&self) -> String {
        match &self.value {
            WatchValue::None => self.name.clone(),
            WatchValue::Pending => {
                format!("{} = {}", self.name, crate::variables::COLLECTING_DATA)
            }
            WatchValue::Paused => format!("{} {PAUSED}", self.name),
            WatchValue::Value(v) => v.text(),
            WatchValue::Error(e) => format!("{} = {e}", self.name),
        }
    }
}

/// What a watch shows.
#[derive(Debug, Clone, PartialEq)]
pub enum WatchValue {
    /// The program runs: the expression only.
    None,
    Pending,
    /// Paused and not evaluated at this stop.
    Paused,
    Value(Box<ValueRow>),
    /// The error as the product shows it (`{NameError}NameError(…)`).
    Error(String),
}

impl WatchValue {
    pub fn as_str(&self) -> &'static str {
        match self {
            WatchValue::None => "none",
            WatchValue::Pending => "pending",
            WatchValue::Paused => "paused",
            WatchValue::Value(_) => "value",
            WatchValue::Error(_) => "error",
        }
    }
}

/// What a row's editor changes.
#[derive(Debug, Clone)]
pub(crate) enum EditTarget {
    /// Set Value of a child of `parent` (a scope or a value).
    SetValue {
        parent: i64,
        variable: Box<Variable>,
    },
    /// A watch's expression (an empty one removes it).
    Watch { index: usize },
    /// The result becomes a watch at the top.
    Result,
    /// A new watch at an index.
    NewWatch { at: usize },
}

impl EditTarget {
    /// The history of the editor (`setValue`, `watch`).
    fn history(&self) -> HistoryKind {
        match self {
            EditTarget::SetValue { .. } => HistoryKind::SetValue,
            _ => HistoryKind::Watch,
        }
    }
}

/// A value or a watch edited in its row (`XDebuggerTreeInplaceEditor`).
pub(crate) struct InlineEdit {
    /// The edited row.
    pub path: String,
    pub target: EditTarget,
    pub editor: Entity<Editor>,
    /// Why the last Apply failed.
    pub error: Option<String>,
    /// An Apply waits for the debugger.
    pub applying: bool,
    /// The names of its expression the frame does not declare.
    pub unresolved: Vec<Range<usize>>,
    _blur: Subscription,
    _edited: Subscription,
}

impl InlineEdit {
    /// Where a new watch is typed.
    pub(crate) fn new_watch_at(&self) -> Option<usize> {
        match self.target {
            EditTarget::NewWatch { at } => Some(at),
            _ => None,
        }
    }
}

/// The open list of a combo box's history: the evaluate field's or the
/// editor of a row (`ComboPopup`). The field keeps the focus; Up, Down,
/// Enter and Escape act on the list while it is open.
pub(crate) struct HistoryList {
    /// The evaluate field's list, else the row editor's.
    pub field: bool,
    pub items: Vec<String>,
    pub highlighted: Option<usize>,
    /// The combo box the list opens under.
    pub bounds: Bounds<Pixels>,
    _keys: Subscription,
}

/// The text of an evaluation's error: the last line of a traceback.
fn error_text(error: &DebugError) -> String {
    let text = error.to_string();
    text.lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// Evaluates an expression in a frame, with the size of a container.
pub(crate) async fn evaluate(
    session: Session,
    provider: Option<Arc<dyn DebugAdapterProvider>>,
    frame: i64,
    expression: String,
    context: EvaluateContext,
) -> Eval {
    let value = match session.evaluate_in(&expression, Some(frame), context).await {
        Ok(value) => value,
        Err(error) => return Eval::Error(error_text(&error)),
    };
    let variable = Variable {
        name: expression.clone(),
        value: value.value,
        type_name: value.type_name,
        presentation: value.presentation,
        evaluate_name: Some(expression.clone()),
        reference: value.reference,
        named: value.named,
        indexed: value.indexed,
    };
    let sized = variable.reference > 0
        && provider
            .as_ref()
            .is_none_or(|p| matches!(p.present_value(&variable).role, ValueRole::Value(_)));
    let shape = if sized {
        session
            .sizes(frame, vec![expression])
            .await
            .ok()
            .and_then(|s| s.into_iter().next().flatten())
    } else {
        None
    };
    Eval::Value {
        variable: Box::new(variable),
        shape,
    }
}

impl VariablesView {
    /// The suspended frame shown.
    fn frame_id(&self) -> Option<i64> {
        self.frame
            .as_ref()
            .filter(|_| self.phase == Phase::Suspended)
            .map(|f| f.id)
    }

    // ---- watches ---------------------------------------------------------------

    /// The watches of the configuration type changed (here or elsewhere).
    pub fn sync_watches(&mut self, cx: &mut Context<Self>) {
        let (watches, paused) = self
            .session
            .debugger()
            .map(|d| {
                let key = self.session.watches_key();
                (d.watches(key), d.watches_paused(key))
            })
            .unwrap_or_default();
        if watches != self.watches || paused != self.paused {
            self.watches = watches;
            self.paused = paused;
            self.dirty = true;
            cx.notify();
        }
    }

    /// Evaluates a watch in the frame shown.
    pub(crate) fn evaluate_watch(&mut self, expression: String, cx: &mut Context<Self>) {
        let Some(frame) = self.frame_id() else {
            return;
        };
        self.evaluations.insert(expression.clone(), Eval::Pending);
        let epoch = self.epoch;
        let task = evaluate(
            self.session.clone(),
            self.provider.clone(),
            frame,
            expression.clone(),
            EvaluateContext::Watch,
        );
        cx.spawn(async move |this, cx| {
            let eval = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch == epoch {
                    this.evaluations.insert(expression, eval);
                    this.dirty = true;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The watch selected (its index), or the result (`None` inside).
    fn selected_watch(&mut self, cx: &mut Context<Self>) -> Option<Option<usize>> {
        match self.selected_row(cx)?.kind {
            RowKind::Watch(w) if w.result => Some(None),
            RowKind::Watch(w) => w.index.map(Some),
            _ => None,
        }
    }

    /// Adds a watch at the end (Add as Inline Watch of a value popup).
    pub(crate) fn add_watch_expression(&mut self, expression: &str, cx: &mut Context<Self>) {
        self.add_watch(expression, None, cx);
    }

    /// Adds a watch and selects it.
    fn add_watch(&mut self, expression: &str, at: Option<usize>, cx: &mut Context<Self>) {
        let Some(debugger) = self.session.debugger() else {
            return;
        };
        let key = self.session.watches_key().to_owned();
        let index = at.unwrap_or(self.watches.len()).min(self.watches.len());
        if debugger.add_watch(&key, expression, Some(index)) {
            self.sync_watches(cx);
            self.select(&watch_segment(index), cx);
        }
    }

    /// New Watch: `expression` after the selected watch (at the end
    /// without one), or an empty watch edited in place there
    /// (`WatchesRootNode.addNewWatch`).
    pub(crate) fn new_watch(
        &mut self,
        expression: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing.is_some() {
            return;
        }
        let at = match self.selected_watch(cx) {
            Some(Some(i)) => i + 1,
            _ => self.watches.len(),
        };
        match expression {
            Some(expression) => self.add_watch(&expression, Some(at), cx),
            None => {
                self.start_edit(NEW.to_owned(), EditTarget::NewWatch { at }, "", window, cx);
            }
        }
    }

    /// Remove Watch: the selected watch (or the result), or the one with
    /// an expression or at an index; the next one is selected
    /// (`XWatchesViewImpl.removeWatches`).
    pub(crate) fn remove_watch(
        &mut self,
        expression: Option<String>,
        index: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let target = match (index, expression) {
            (Some(i), _) => Some(Some(i)),
            (None, Some(e)) => self.watches.iter().position(|w| *w == e).map(Some),
            (None, None) => self.selected_watch(cx),
        };
        match target {
            Some(None) => {
                self.result = None;
                self.dirty = true;
                cx.notify();
            }
            Some(Some(i)) => {
                let Some(debugger) = self.session.debugger() else {
                    return;
                };
                if debugger.remove_watch(self.session.watches_key(), i) {
                    self.sync_watches(cx);
                    if !self.watches.is_empty() {
                        let next = i.min(self.watches.len() - 1);
                        self.select(&watch_segment(next), cx);
                    }
                }
            }
            None => {}
        }
    }

    /// Remove All Watches: the watches and the result
    /// (`WatchesRootNode.removeAllChildren`).
    pub(crate) fn remove_all_watches(&mut self, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            return;
        }
        if let Some(debugger) = self.session.debugger() {
            debugger.remove_all_watches(self.session.watches_key());
        }
        self.result = None;
        self.dirty = true;
        self.sync_watches(cx);
        cx.notify();
    }

    /// Pause Watch, or Resume Watch of a paused one: the selected watch or
    /// the one at `index`. A paused watch keeps the value it shows; a
    /// resumed one is evaluated again (`XPauseWatchAction`).
    pub(crate) fn pause_watch(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        let index = match index {
            Some(i) => Some(i),
            None => self.selected_watch(cx).flatten(),
        };
        let Some(i) = index.filter(|i| *i < self.watches.len()) else {
            return;
        };
        // A watch being evaluated cannot be paused (the evaluation cannot
        // be cancelled).
        let expression = self.watches[i].clone();
        if self.evaluations.get(&expression) == Some(&Eval::Pending) {
            return;
        }
        let paused = self.paused.get(i).copied().unwrap_or(false);
        let Some(debugger) = self.session.debugger() else {
            return;
        };
        if debugger.set_watch_paused(self.session.watches_key(), i, !paused) {
            if paused {
                self.evaluations.remove(&expression);
            }
            self.dirty = true;
            self.sync_watches(cx);
        }
    }

    /// "… Evaluate" of a paused watch: evaluated once, it stays paused.
    pub(crate) fn evaluate_paused_watch(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(expression) = self.watches.get(index).cloned() {
            self.evaluate_watch(expression, cx);
            self.dirty = true;
            cx.notify();
        }
    }

    /// Edit: the selected watch in place, or its expression set to
    /// `expression` (the result becomes a watch).
    pub(crate) fn edit_watch(
        &mut self,
        expression: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.selected_row(cx) else {
            return;
        };
        let RowKind::Watch(w) = &row.kind else {
            return;
        };
        let target = match (w.result, w.index) {
            (true, _) => EditTarget::Result,
            (false, Some(index)) => EditTarget::Watch { index },
            _ => return,
        };
        match expression {
            Some(expression) => self.apply_watch(target, &expression, cx),
            None => {
                let text = w.expression.clone();
                self.start_edit(row.path.clone(), target, &text, window, cx);
            }
        }
    }

    fn apply_watch(&mut self, target: EditTarget, expression: &str, cx: &mut Context<Self>) {
        let Some(debugger) = self.session.debugger() else {
            return;
        };
        let key = self.session.watches_key().to_owned();
        match target {
            EditTarget::Watch { index } => {
                if debugger.edit_watch(&key, index, expression) {
                    self.sync_watches(cx);
                    if index < self.watches.len() {
                        self.select(&watch_segment(index), cx);
                    }
                }
            }
            EditTarget::NewWatch { at } => {
                if !expression.trim().is_empty() {
                    self.add_watch(expression, Some(at), cx);
                }
            }
            EditTarget::Result => {
                self.result = None;
                self.dirty = true;
                if !expression.trim().is_empty() {
                    self.add_watch(expression, Some(0), cx);
                }
            }
            EditTarget::SetValue { .. } => {}
        }
    }

    /// The expression of the selected row: a watch's own, else the
    /// value's.
    fn selected_expression(&mut self, cx: &mut Context<Self>) -> Option<String> {
        let row = self.selected_row(cx)?;
        match &row.kind {
            RowKind::Watch(w) => Some(w.expression.clone()),
            _ => row.value().map(|v| {
                v.variable
                    .evaluate_name
                    .clone()
                    .unwrap_or_else(|| v.variable.name.clone())
            }),
        }
        .filter(|e| !e.trim().is_empty())
    }

    /// Add to Watches: the selected value's expression, at the end
    /// (`DebuggerUIUtil.addToWatches`).
    pub(crate) fn add_to_watches(&mut self, cx: &mut Context<Self>) {
        if let Some(expression) = self.selected_expression(cx) {
            self.add_watch(&expression, None, cx);
        }
    }

    /// View as: the selected value shown by a renderer of its type.
    pub(crate) fn view_as(&mut self, renderer: &str, cx: &mut Context<Self>) {
        let Some(row) = self.selected_row(cx) else {
            return;
        };
        let Some(v) = row.value() else {
            return;
        };
        let Some(provider) = &self.provider else {
            return;
        };
        let (renderers, _) =
            provider.value_renderers(v.variable.type_name.as_deref().unwrap_or(""));
        if renderers.contains(&renderer) {
            self.renderers.insert(row.path.clone(), renderer.to_owned());
            self.dirty = true;
            cx.notify();
        }
    }

    // ---- the evaluate field ------------------------------------------------------

    fn ensure_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<Editor> {
        if let Some(field) = &self.field {
            return field.clone();
        }
        let hint = if cfg!(target_os = "macos") {
            HINT_MACOS
        } else {
            HINT_OTHERS
        };
        let field = editor::field::new("", Some(hint.into()), false, window, cx);
        field.update(cx, |e, _| e.set_placeholder_unfocused(true));
        cx.subscribe(&field, |this, _, event: &EditorEvent, cx| {
            if matches!(event, EditorEvent::Edited(_)) {
                this.check_field(cx);
                cx.notify();
            }
        })
        .detach();
        self.field = Some(field.clone());
        field
    }

    fn field_text(&self, cx: &App) -> String {
        self.field
            .as_ref()
            .map(|f| f.read(cx).text())
            .unwrap_or_default()
    }

    /// Puts a text into the field, the caret at its end.
    fn put_field_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let field = self.ensure_field(window, cx);
        field.update(cx, |e, cx| put_text(e, text, cx));
        self.check_field(cx);
    }

    /// Puts a text into the field and gives it the focus.
    pub(crate) fn set_field_text(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.put_field_text(text, window, cx);
        if let Some(field) = &self.field {
            window.focus(&field.focus_handle(cx));
        }
        self.close_menu(cx);
        self.history = None;
        cx.notify();
    }

    /// Looks up the names of the field's expression in the frame's code,
    /// off the UI thread; the field marks those it does not declare
    /// (`Unresolved reference`) and is outlined in red while there are
    /// any.
    pub(crate) fn check_field(&mut self, cx: &mut Context<Self>) {
        if let Some(field) = self.field.clone() {
            self.check_names(field, true, cx);
        }
    }

    /// [`Self::check_field`] for the editor of a row.
    fn check_row_editor(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = self.editing.as_ref().map(|e| e.editor.clone()) {
            self.check_names(editor, false, cx);
        }
    }

    fn check_names(&mut self, editor: Entity<Editor>, field: bool, cx: &mut Context<Self>) {
        let expression = editor.read(cx).text();
        let frame = self
            .frame
            .as_ref()
            .filter(|_| self.phase == Phase::Suspended)
            .and_then(|f| Some((f.file.clone()?, f.line)));
        let (Some(provider), Some((file, line))) = (self.provider.clone(), frame) else {
            self.set_unresolved(&editor, field, Vec::new(), cx);
            return;
        };
        if expression.trim().is_empty() {
            self.set_unresolved(&editor, field, Vec::new(), cx);
            return;
        }
        let cached = self
            .source
            .as_ref()
            .filter(|(path, _)| *path == file)
            .map(|(_, text)| text.clone());
        let task = cx.background_spawn(async move {
            let text: Arc<str> = match cached {
                Some(text) => text,
                None => std::fs::read_to_string(&file).ok()?.into(),
            };
            let ranges = provider.unresolved_names(&text, line, &expression);
            Some((file, text, expression, ranges))
        });
        let check = cx.spawn(async move |this, cx| {
            let Some((file, text, expression, ranges)) = task.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if editor.read(cx).text() == expression {
                    this.source = Some((file, text));
                    this.set_unresolved(&editor, field, ranges, cx);
                }
            });
        });
        if field {
            self.check = Some(check);
        } else {
            self.check_editor = Some(check);
        }
    }

    fn set_unresolved(
        &mut self,
        editor: &Entity<Editor>,
        field: bool,
        ranges: Vec<Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        let shown = if field {
            &self.unresolved
        } else {
            match &self.editing {
                Some(edit) if edit.editor == *editor => &edit.unresolved,
                _ => return,
            }
        };
        if ranges == *shown {
            return;
        }
        let text: Vec<char> = editor.read(cx).text().chars().collect();
        let marks = ranges
            .iter()
            .map(|r| {
                let name: String = text
                    .get(r.clone())
                    .map(|c| c.iter().collect())
                    .unwrap_or_default();
                DiagnosticMark {
                    range: r.clone(),
                    severity: MarkSeverity::Error,
                    unnecessary: false,
                    deprecated: false,
                    messages: vec![MarkMessage {
                        severity: MarkSeverity::Error,
                        text: format!("Unresolved reference '{name}'").into(),
                        source: None,
                        code: None,
                        href: None,
                    }],
                }
            })
            .collect();
        editor.update(cx, |e, cx| e.set_diagnostics(marks, cx));
        if field {
            self.unresolved = ranges;
        } else if let Some(edit) = &mut self.editing {
            edit.unresolved = ranges;
        }
        cx.notify();
    }

    /// Enter in the field: the expression's value becomes the result node,
    /// selected (`XWatchesViewImpl.addExpressionResultNode`). With the
    /// history open, Enter only closes it.
    pub(crate) fn evaluate_field(
        &mut self,
        expression: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if expression.is_none() && self.history.as_ref().is_some_and(|h| h.field) {
            self.close_history(cx);
            return;
        }
        if let Some(text) = &expression {
            self.put_field_text(text, window, cx);
        }
        let expression = self.field_text(cx).trim().to_owned();
        if expression.is_empty() {
            return;
        }
        if let Some(debugger) = self.session.debugger() {
            debugger.remember(HistoryKind::Evaluate, &expression);
        }
        let Some(frame) = self.frame_id() else {
            return;
        };
        // What the result showed: the new one is compared with it.
        self.result_before = match &self.result {
            Some((_, Eval::Value { variable, .. })) => Some(value_text(&variable.value)),
            _ => None,
        };
        self.result = Some((expression.clone(), Eval::Pending));
        self.dirty = true;
        self.select(RESULT, cx);
        let epoch = self.epoch;
        let task = evaluate(
            self.session.clone(),
            self.provider.clone(),
            frame,
            expression.clone(),
            RESULT_CONTEXT,
        );
        cx.spawn(async move |this, cx| {
            let eval = task.await;
            let _ = this.update(cx, |this, cx| {
                let current = this
                    .result
                    .as_ref()
                    .is_some_and(|(e, v)| *e == expression && *v == Eval::Pending);
                if this.epoch == epoch && current {
                    this.result = Some((expression, eval));
                    this.dirty = true;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// Add to Watches of the field: its expression at the end of the
    /// watches.
    pub(crate) fn watch_field(
        &mut self,
        expression: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(text) = &expression {
            self.put_field_text(text, window, cx);
        }
        let expression = self.field_text(cx).trim().to_owned();
        if expression.is_empty() {
            return;
        }
        if let Some(debugger) = self.session.debugger() {
            debugger.remember(HistoryKind::Evaluate, &expression);
        }
        self.close_history(cx);
        self.add_watch(&expression, None, cx);
    }

    /// Evaluate Expression of a value: its expression into the field.
    pub(crate) fn evaluate_value(
        &mut self,
        expression: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let expression = expression.or_else(|| self.selected_expression(cx));
        self.set_field_text(&expression.unwrap_or_default(), window, cx);
    }

    pub(crate) fn field_json(&self, cx: &App) -> Json {
        json!({
            "text": self.field_text(cx),
            "history": self.session.debugger().map(|d| d.history(HistoryKind::Evaluate)),
            "unresolved": self.unresolved.iter().map(|r| [r.start, r.end]).collect::<Vec<_>>(),
        })
    }

    /// The evaluate field's row: the editor with its hint, Add to Watches
    /// and the history's chevron (`XDebuggerEmbeddedComboBox`), the ring
    /// while it has the focus (or its list is open), red while its
    /// expression has a name the frame does not declare.
    pub(crate) fn render_field(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let field = self.ensure_field(window, cx);
        // The expression in the colours of the frame's language
        // (`XDebuggerExpressionComboBox.setSourcePosition`).
        let file = self.frame.as_ref().and_then(|f| f.file.clone());
        if let (Some(file), Some(highlighters)) = (file, &self.highlighters)
            && self.highlighted.as_ref() != Some(&file)
        {
            let highlighter = highlighters(&file);
            field.update(cx, |e, cx| e.set_highlighter(highlighter, cx));
            self.highlighted = Some(file);
        }
        let empty = field.read(cx).text().trim().is_empty();
        let focus = field.focus_handle(cx);
        let focused = focus.is_focused(window) || self.history.as_ref().is_some_and(|h| h.field);
        let watch_keys = ui::keys::shortcut_in(&WatchField { expression: None }, &focus, window);
        let theme = cx.theme();
        let ring = if !self.unresolved.is_empty() {
            Some(theme.color(UiRole::Danger))
        } else if focused {
            Some(theme.color(UiRole::Focus))
        } else {
            None
        };
        let line = editor::field_line_height_for(false, window, cx);
        let bounds = self.field_bounds.clone();
        let button = |id: &'static str, icon: IconName, enabled: bool| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .size(px(FIELD_BUTTON))
                .rounded(theme.metric(Metric::ButtonRadius))
                .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
                .when(enabled, |d| {
                    let hover = theme.color(UiRole::TabHover);
                    d.hover(move |s| s.bg(hover))
                })
                .child(Icon::new(icon).size(px(16.0)))
        };
        div()
            .id("debug-evaluate")
            .debug_selector(|| "debug-evaluate".into())
            .key_context(FIELD_CONTEXT)
            .relative()
            .flex_none()
            .h(px(FIELD_HEIGHT))
            .pl(px(FIELD_LEFT))
            .pr(px(FIELD_RIGHT))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(FIELD_GAP))
            .border_b_1()
            .border_color(theme.color(UiRole::Border))
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            // The ring, square, inside the field above its bottom line.
            .when_some(ring, |d, ring| {
                d.child(
                    div()
                        .debug_selector(|| "debug-evaluate-ring".into())
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .border(px(RING))
                        .border_color(ring),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(line)
                    .overflow_hidden()
                    .child(field),
            )
            .child(
                button("debug-evaluate-watch", icons::ADD_TO_WATCHES, !empty)
                    .tooltip(Tooltip::text("Add to Watches", watch_keys.map(Into::into)))
                    .when(!empty, |d| {
                        d.on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(WatchField { expression: None }), cx)
                        })
                    }),
            )
            // The combo box's arrow: enabled with an empty history too.
            .child(
                button("debug-evaluate-history", IconName::CHEVRON_DOWN, true)
                    .tooltip(Tooltip::text("Show History", None))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(ShowEvaluateHistory), cx)
                    }),
            )
            .into_any_element()
    }

    // ---- the histories -------------------------------------------------------------

    /// The evaluate field's history below it; open, it closes.
    pub(crate) fn show_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.history.as_ref().is_some_and(|h| h.field) {
            self.close_history(cx);
            return;
        }
        let items = self
            .session
            .debugger()
            .map(|d| d.history(HistoryKind::Evaluate))
            .unwrap_or_default();
        let field = self.ensure_field(window, cx);
        window.focus(&field.focus_handle(cx));
        self.close_menu(cx);
        let bounds = self.field_bounds.get();
        self.open_history(true, items, bounds, window, cx);
    }

    /// The history of the editor of a row below it; open, it closes.
    pub(crate) fn show_editor_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.history.as_ref().is_some_and(|h| !h.field) {
            self.close_history(cx);
            return;
        }
        let Some(edit) = &self.editing else {
            return;
        };
        let kind = edit.target.history();
        window.focus(&edit.editor.focus_handle(cx));
        let items = self
            .session
            .debugger()
            .map(|d| d.history(kind))
            .unwrap_or_default();
        let bounds = self.editor_bounds.get();
        self.open_history(false, items, bounds, window, cx);
    }

    fn open_history(
        &mut self,
        field: bool,
        items: Vec<String>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The keys of the list, before the field's own bindings.
        let view = cx.entity().downgrade();
        let handle = window.window_handle();
        let keys = cx.intercept_keystrokes(move |event: &KeystrokeEvent, window, cx| {
            if window.window_handle() != handle {
                return;
            }
            let k = &event.keystroke;
            if k.modifiers.modified() {
                return;
            }
            let key = k.key.clone();
            if !matches!(key.as_str(), "up" | "down" | "enter" | "escape") {
                return;
            }
            let handled = view
                .update(cx, |this, cx| this.history_key(&key, window, cx))
                .unwrap_or(false);
            if handled {
                cx.stop_propagation();
            }
        });
        self.history = Some(HistoryList {
            field,
            items,
            highlighted: None,
            bounds,
            _keys: keys,
        });
        cx.notify();
    }

    pub(crate) fn close_history(&mut self, cx: &mut Context<Self>) {
        if self.history.take().is_some() {
            cx.notify();
        }
    }

    /// Up, Down, Enter and Escape while a history is open.
    fn history_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(list) = &mut self.history else {
            return false;
        };
        let n = list.items.len();
        match key {
            "down" if n > 0 => {
                list.highlighted = Some(list.highlighted.map_or(0, |i| (i + 1).min(n - 1)));
            }
            "up" if n > 0 => {
                list.highlighted = Some(list.highlighted.map_or(0, |i| i.saturating_sub(1)));
            }
            "enter" => match list.highlighted {
                Some(i) => self.choose_history(i, window, cx),
                None => self.close_history(cx),
            },
            "escape" => self.close_history(cx),
            _ => return false,
        }
        cx.notify();
        true
    }

    /// An expression of the open history goes into its field.
    pub(crate) fn choose_history(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(list) = self.history.take() else {
            return;
        };
        let Some(text) = list.items.get(index).cloned() else {
            return;
        };
        if list.field {
            self.set_field_text(&text, window, cx);
        } else if let Some(edit) = &self.editing {
            let editor = edit.editor.clone();
            editor.update(cx, |e, cx| put_text(e, &text, cx));
            window.focus(&editor.focus_handle(cx));
            self.check_row_editor(cx);
        }
        cx.notify();
    }

    pub(crate) fn history_json(&self) -> Json {
        match &self.history {
            None => Json::Null,
            Some(list) => json!({
                "of": if list.field { "field" } else { "editor" },
                "items": list.items,
                "highlighted": list.highlighted,
            }),
        }
    }

    /// The open history: the combo box's list right under it, as wide as
    /// it, no row highlighted until the pointer or a key moves there.
    pub(crate) fn render_history(&self, window: &Window, cx: &Context<Self>) -> Option<AnyElement> {
        let list = self.history.as_ref()?;
        let theme = cx.theme();
        let b = list.bounds;
        // The field's list covers its bottom line.
        let top = if list.field {
            b.bottom() - px(1.0)
        } else {
            b.bottom()
        };
        let radius = theme.metric(Metric::CornerRadius);
        let selection = theme.color(UiRole::Selection);
        let rows = list.items.iter().enumerate().map(|(i, item)| {
            div()
                .id(("debug-history-row", i))
                .debug_selector(move || format!("debug-history-row-{i}"))
                .flex_none()
                .h(px(HISTORY_ROW))
                .px(px(HISTORY_TEXT))
                .flex()
                .items_center()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .when(list.highlighted == Some(i), |d| d.bg(selection))
                .child(item.clone())
                .on_mouse_move(cx.listener(move |this, _, _, cx| {
                    if let Some(list) = &mut this.history
                        && list.highlighted != Some(i)
                    {
                        list.highlighted = Some(i);
                        cx.notify();
                    }
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.choose_history(i, window, cx);
                    }),
                )
        });
        let popup = div()
            .id("debug-history")
            .debug_selector(|| "debug-history".into())
            .occlude()
            .w(b.size.width)
            .max_h(px(HISTORY_ROW * HISTORY_ROWS as f32 + 2.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .rounded_b(radius)
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(ui::components::popup_shadow())
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .text_color(theme.color(UiRole::Text))
            .children(rows);
        let size = window.viewport_size();
        Some(
            deferred(
                anchored().position(point(px(0.0), px(0.0))).child(
                    div()
                        .id("debug-history-backdrop")
                        .w(size.width)
                        .h(size.height)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.close_history(cx)),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(|this, _, _, cx| this.close_history(cx)),
                        )
                        .child(
                            anchored()
                                .position(point(b.left(), top))
                                .snap_to_window_with_margin(px(4.0))
                                .child(popup),
                        ),
                ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    // ---- editing in a row ----------------------------------------------------------

    fn start_edit(
        &mut self,
        path: String,
        target: EditTarget,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The text with the caret at its end (what the reference shows once
        // the editor's document is ready).
        let editor = editor::field::new(text, None, false, window, cx);
        editor.update(cx, |e, cx| put_text(e, text, cx));
        // The expression in the colours of the frame's language.
        if let (Some(file), Some(highlighters)) = (
            self.frame.as_ref().and_then(|f| f.file.clone()),
            &self.highlighters,
        ) {
            let highlighter = highlighters(&file);
            editor.update(cx, |e, cx| e.set_highlighter(highlighter, cx));
        }
        let edited = cx.subscribe(&editor, |this, _, event: &EditorEvent, cx| {
            if matches!(event, EditorEvent::Edited(_)) {
                this.check_row_editor(cx);
            }
        });
        let focus = editor.focus_handle(cx);
        let blur = cx.on_blur(&focus, window, |this, window, cx| {
            // A click elsewhere closes the editor without a change; its
            // history keeps it open.
            if this.history.as_ref().is_some_and(|h| !h.field) {
                return;
            }
            if this.editing.as_ref().is_some_and(|e| !e.applying) {
                this.cancel_edit(window, cx);
            }
        });
        self.close_menu(cx);
        self.history = None;
        let new = matches!(target, EditTarget::NewWatch { .. });
        self.editing = Some(InlineEdit {
            path: path.clone(),
            target,
            editor,
            error: None,
            applying: false,
            unresolved: Vec::new(),
            _blur: blur,
            _edited: edited,
        });
        self.dirty = true;
        if new {
            // The new watch's row is not selected (`editWatch`).
            self.deselect(cx);
        } else {
            self.select(&path, cx);
        }
        window.focus(&focus);
        self.check_row_editor(cx);
        cx.notify();
    }

    /// Enter in the row's editor (with its history open, the highlighted
    /// expression goes into it first, `doPopupOKAction`).
    pub(crate) fn commit_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self
            .history
            .as_ref()
            .filter(|h| !h.field)
            .and_then(|h| h.highlighted)
        {
            self.choose_history(i, window, cx);
        }
        self.history = None;
        let Some(edit) = &mut self.editing else {
            return;
        };
        if edit.applying {
            return;
        }
        let text = edit.editor.read(cx).text();
        let target = edit.target.clone();
        if let Some(debugger) = self.session.debugger() {
            debugger.remember(target.history(), &text);
        }
        match target {
            EditTarget::SetValue { parent, variable } => {
                edit.applying = true;
                let pending = self.session.set_value(parent, &variable, text.trim());
                cx.spawn_in(window, async move |this, cx| {
                    let result = pending.await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        let Some(edit) = &mut this.editing else {
                            return;
                        };
                        edit.applying = false;
                        match result {
                            Ok(_) => {
                                this.editing = None;
                                window.focus(&this.focus);
                                this.reload(cx);
                            }
                            Err(error) => {
                                edit.error = Some(error_text(&error));
                                this.dirty = true;
                                window.focus(&edit.editor.focus_handle(cx));
                                cx.notify();
                            }
                        }
                    });
                })
                .detach();
            }
            target => {
                self.editing = None;
                self.dirty = true;
                window.focus(&self.focus);
                self.apply_watch(target, &text, cx);
                cx.notify();
            }
        }
    }

    /// Escape in the row's editor, or the focus went elsewhere: a new
    /// watch goes, the row is selected (`WatchInplaceEditor.cancelEditing`).
    pub(crate) fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.editing.take() else {
            return;
        };
        self.history = None;
        let focused = edit.editor.focus_handle(cx).is_focused(window);
        self.dirty = true;
        if focused {
            window.focus(&self.focus);
        }
        if edit.new_watch_at().is_some() && self.selected_path() == Some(NEW) {
            self.deselect(cx);
        }
        cx.notify();
    }

    /// Set Value (F2): the selected value edited in its row; on a watch,
    /// its expression (Edit shares the key, `XEditWatchAction`). With
    /// `value`, the value is set at once.
    pub(crate) fn set_value(
        &mut self,
        value: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing.is_some() {
            return;
        }
        let Some(row) = self.selected_row(cx) else {
            return;
        };
        let v = match &row.kind {
            RowKind::Watch(_) => return self.edit_watch(value, window, cx),
            RowKind::Value(v) => v.clone(),
            _ => return,
        };
        if self.frame_id().is_none() || !self.session.can_set_values() {
            return;
        }
        let target = EditTarget::SetValue {
            parent: v.parent,
            variable: Box::new(v.variable.clone()),
        };
        self.start_edit(row.path.clone(), target, &v.variable.value, window, cx);
        if let Some(value) = value {
            if let Some(edit) = &self.editing {
                edit.editor
                    .update(cx, |e, cx| e.set_text_selected(&value, cx));
            }
            self.commit_edit(window, cx);
        }
    }

    /// The editor of a row (`XDebuggerExpressionComboBox` in the tree): a
    /// combo box 30 high centred on the row, its ring, the expression,
    /// and its history's arrow after a line.
    pub(crate) fn render_inline_editor(&self, edit: &InlineEdit, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let radius = theme.metric(Metric::CornerRadius) * 1.5;
        let ring = if edit.error.is_some() || !edit.unresolved.is_empty() {
            theme.color(UiRole::Danger)
        } else {
            theme.color(UiRole::Focus)
        };
        let bounds = self.editor_bounds.clone();
        div()
            .id("variables-editor")
            .debug_selector(|| "variables-editor".into())
            .key_context(EDITOR_CONTEXT)
            .relative()
            .flex_1()
            .min_w(px(0.0))
            .h(px(EDITOR_HEIGHT))
            .ml(px(EDITOR_LEFT))
            .flex()
            .flex_row()
            .items_center()
            .rounded(radius)
            .border(px(RING))
            .border_color(ring)
            .bg(theme.color(UiRole::Input))
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .pl(px(EDITOR_TEXT))
                    .h(self.line_height)
                    .overflow_hidden()
                    .child(edit.editor.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(1.0))
                    .h_full()
                    .bg(theme.color(UiRole::BorderInput)),
            )
            .child(
                div()
                    .id("variables-editor-history")
                    .debug_selector(|| "variables-editor-history".into())
                    .flex_none()
                    .w(px(EDITOR_ARROW - RING))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Icon::new(IconName::CHEVRON_DOWN).size(px(16.0)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.show_editor_history(window, cx);
                        }),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn editing_json(&self, cx: &App) -> Json {
        let Some(edit) = &self.editing else {
            return Json::Null;
        };
        json!({
            "path": self.names(&edit.path),
            "target": match edit.target {
                EditTarget::SetValue { .. } => "value",
                EditTarget::Watch { .. } => "watch",
                EditTarget::Result => "result",
                EditTarget::NewWatch { .. } => "new-watch",
            },
            "text": edit.editor.read(cx).text(),
            "error": edit.error,
            "unresolved": edit.unresolved.iter().map(|r| [r.start, r.end]).collect::<Vec<_>>(),
        })
    }

    // ---- the rows of the watches -------------------------------------------------------

    /// A watch's or the result's row: its icon (the result's, a paused
    /// watch's), its name (the result's in the changed colour when it
    /// shows something new), and its value, error, "Collecting data…" or
    /// paused message.
    pub(crate) fn render_watch_content(&self, w: &WatchRow, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let text = theme.color(UiRole::Text);
        let icon = |name: IconName| {
            div()
                .flex_none()
                .w(px(ICON_ROOM))
                .child(Icon::new(name).size(px(16.0)))
        };
        let name_div = |role: UiRole| {
            div()
                .flex_none()
                .text_color(theme.color(role))
                .child(w.name.clone())
        };
        let eq = || div().flex_none().text_color(text).child("\u{a0}=\u{a0}");
        let row = || div().flex().flex_row().items_center();
        match &w.value {
            WatchValue::Value(v) => {
                let kind_icon = match v.kind {
                    debugger::values::ValueKind::Primitive => icons::PRIMITIVE,
                    debugger::values::ValueKind::Array => icons::ARRAY,
                    debugger::values::ValueKind::Object => icons::OBJECT,
                };
                let i = if w.result {
                    icons::EVALUATION_RESULT
                } else if w.paused {
                    icons::PAUSE
                } else {
                    kind_icon
                };
                let role = if v.changed {
                    UiRole::DebuggerChangedValue
                } else {
                    UiRole::DebuggerValueName
                };
                row()
                    .child(icon(i))
                    .child(name_div(role))
                    .child(eq())
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
                    .into_any_element()
            }
            WatchValue::Pending => row()
                .child(icon(if w.result {
                    icons::EVALUATION_RESULT
                } else {
                    icons::WATCH
                }))
                .child(name_div(UiRole::DebuggerValueName))
                .child(eq())
                .child(
                    div()
                        .text_color(theme.color(UiRole::DebuggerCollectingData))
                        .child(crate::variables::COLLECTING_DATA),
                )
                .into_any_element(),
            WatchValue::Paused => {
                let index = w.index;
                row()
                    .child(icon(icons::PAUSE))
                    .child(name_div(UiRole::DebuggerValueName))
                    .child(
                        div()
                            .flex_none()
                            .text_color(theme.color(UiRole::TextMuted))
                            .child(format!("\u{a0}{PAUSED}")),
                    )
                    .child(
                        ui::link::link(
                            ("variables-evaluate-paused", index.unwrap_or(0)),
                            EVALUATE_PAUSED,
                            cx,
                        )
                        .flex_none()
                        .ml(px(3.5))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                if let Some(i) = index {
                                    this.evaluate_paused_watch(i, cx);
                                }
                            }),
                        ),
                    )
                    .into_any_element()
            }
            WatchValue::Error(message) => row()
                .child(icon(IconName::ERROR))
                .child(name_div(UiRole::DebuggerValueName))
                .child(eq())
                .child(
                    div()
                        .text_color(theme.color(UiRole::DebuggerErrorMessage))
                        .child(value_text(message)),
                )
                .into_any_element(),
            WatchValue::None => row()
                .child(icon(if w.result {
                    icons::EVALUATION_RESULT
                } else if w.paused {
                    icons::PAUSE
                } else {
                    icons::WATCH
                }))
                .child(name_div(UiRole::DebuggerValueName))
                .into_any_element(),
        }
    }

    // ---- copy -------------------------------------------------------------------------

    /// Copy Value: the whole value when the adapter evaluates for the
    /// clipboard, else the text the row shows (`XCopyValueAction`). A
    /// watch's or the result's own expression is not evaluated again (it
    /// may change what it shows).
    pub(crate) fn copy_value(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.selected_row(cx) else {
            return;
        };
        let Some(v) = row.value().cloned() else {
            return;
        };
        let shown = v.variable.value.clone();
        let again = matches!(row.kind, RowKind::Value(_));
        let whole = match (&v.variable.evaluate_name, self.frame_id()) {
            (Some(name), Some(frame)) if again && self.session.evaluates_for_clipboard() => Some(
                self.session
                    .evaluate_in(name, Some(frame), EvaluateContext::Clipboard),
            ),
            _ => None,
        };
        match whole {
            Some(pending) => {
                cx.spawn(async move |_, cx| {
                    let text = pending.await.map(|e| e.value).unwrap_or(shown);
                    let _ = cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(text)));
                })
                .detach();
            }
            None => cx.write_to_clipboard(ClipboardItem::new_string(shown)),
        }
    }

    /// Copy Name.
    pub(crate) fn copy_name(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.selected_row(cx) else {
            return;
        };
        let name = match &row.kind {
            RowKind::Watch(w) => Some(w.name.clone()),
            _ => row.value().map(|v| v.name.clone()),
        };
        if let Some(name) = name {
            cx.write_to_clipboard(ClipboardItem::new_string(name));
        }
    }

    // ---- Jump To Source ----------------------------------------------------------------

    /// The selected value of the frame (a child of one of its scopes), the
    /// frame's file and line: where its declaration is looked for.
    pub(crate) fn jump_target(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(std::path::PathBuf, u32, String)> {
        let frame = self.frame_id()?;
        let row = self.selected_row(cx)?;
        // A value of the frame (a child of one of its scopes) by its name;
        // a watch by its expression (the product finds a plain name).
        let name = match &row.kind {
            RowKind::Value(v) => {
                let top = self
                    .session
                    .loaded_scopes(frame)
                    .is_some_and(|scopes| scopes.iter().any(|s| s.reference == v.parent));
                if !top {
                    return None;
                }
                v.variable.name.clone()
            }
            RowKind::Watch(w) if !w.result => w.expression.trim().to_owned(),
            _ => return None,
        };
        let snapshot = self.session.snapshot();
        let f = snapshot.frames.iter().find(|f| f.id == frame)?;
        let path = f.source.as_ref()?.path.clone()?;
        Some((path, f.line, name))
    }

    /// The frame and the expression of the selected value whose type Jump
    /// To Type Source looks for.
    pub(crate) fn type_source_target(&mut self, cx: &mut Context<Self>) -> Option<(i64, String)> {
        let frame = self.frame_id()?;
        let row = self.selected_row(cx)?;
        row.value()?.variable.type_name.as_ref()?;
        Some((frame, self.selected_expression(cx)?))
    }

    // ---- the context menu -------------------------------------------------------------

    fn show_menu(
        &mut self,
        menu: Entity<ContextMenu>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dismissed = cx.subscribe_in(&menu, window, |this, _, _: &gpui::DismissEvent, _, cx| {
            this.menu = None;
            cx.notify();
        });
        self.history = None;
        self.menu = Some((menu, position, dismissed));
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    /// The context menu of the selected row (`XDebugger.Watches.Tree.Popup`
    /// with `XDebugger.ValueGroup`).
    pub(crate) fn open_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row = self.selected_row(cx);
        let rows = !self.rows(cx).is_empty();
        let items = self.menu_items(row.as_ref(), rows);
        let menu = ContextMenu::build(items, window, cx);
        self.show_menu(menu, position, window, cx);
    }

    /// The context menu under the selected row (the command).
    pub(crate) fn menu_at_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The menu shows the keys of the tree.
        window.focus(&self.focus);
        let rows = self.rows(cx);
        let index = self
            .selected_path()
            .and_then(|p| rows.iter().position(|r| r.path == p))
            .unwrap_or(0);
        let bounds = self.tree_bounds.get();
        let row_h = cx.theme().metric(Metric::RowHeight);
        let position = bounds.origin + gpui::point(px(40.0), row_h * (index as f32 + 1.0));
        self.open_menu(position, window, cx);
    }

    /// The items, as the reference shows them for a value and a watch:
    /// the watches' actions, a line, the value's group (Customize Data View
    /// and Show Referring Objects where the product's values have them,
    /// Inspect and Compare Value with Clipboard, which Pyoxide does not
    /// offer yet, shown disabled), View as for a type with renderers.
    fn menu_items(&self, row: Option<&Row>, rows: bool) -> Vec<MenuItem> {
        let watch = row.and_then(|r| match &r.kind {
            RowKind::Watch(w) if w.index.is_some() || w.result => Some(w.as_ref()),
            _ => None,
        });
        let value = row.and_then(Row::value);
        let suspended = self.frame_id().is_some();
        let extras = self
            .provider
            .as_ref()
            .map(|p| p.extra_value_actions())
            .unwrap_or_default();
        let mut items = vec![
            MenuItem::action("New Watch\u{2026}", NewWatch { expression: None })
                .icon(IconName::ADD),
        ];
        if watch.is_some() {
            items.push(
                MenuItem::action(
                    "Remove Watch",
                    RemoveWatch {
                        expression: None,
                        index: None,
                    },
                )
                .icon(IconName::REMOVE),
            );
        }
        items.push(MenuItem::action("Remove All Watches", RemoveAllWatches).enabled(rows));
        if let Some(w) = watch {
            // Edit has the key of Set Value (`use-shortcut-of`).
            items.push(MenuItem::action("Edit\u{2026}", SetValue { value: None }));
            if !w.result {
                let computed = !matches!(w.value, WatchValue::Pending);
                let (label, icon) = if w.paused {
                    ("Resume Watch", IconName::RUN)
                } else {
                    ("Pause Watch", icons::PAUSE)
                };
                items.push(
                    MenuItem::action(label, PauseWatch { index: None })
                        .icon(icon)
                        .enabled(computed),
                );
            }
        }
        items.push(MenuItem::separator());
        if value.is_some() && extras.contains(&debugger::ExtraValueAction::CustomizeDataView) {
            items.push(MenuItem::unavailable("Customize Data View"));
        }
        if row.is_some_and(|r| r.value().is_some()) {
            items.push(MenuItem::unavailable("Inspect\u{2026}"));
        }
        if watch.is_none() {
            let settable = matches!(row.map(|r| &r.kind), Some(RowKind::Value(_)))
                && suspended
                && self.session.can_set_values();
            items.push(
                MenuItem::action("Set Value\u{2026}", SetValue { value: None }).enabled(settable),
            );
        }
        items.push(MenuItem::action("Copy Value", CopyValue).enabled(value.is_some()));
        items.push(MenuItem::unavailable("Compare Value with Clipboard"));
        items.push(
            MenuItem::action("Copy Name", CopyName).enabled(value.is_some() || watch.is_some()),
        );
        items.push(MenuItem::separator());
        items.push(
            MenuItem::action(
                "Evaluate Expression\u{2026}",
                EvaluateValue { expression: None },
            )
            .icon(icons::EVALUATE_EXPRESSION)
            .enabled(suspended),
        );
        items.push(
            MenuItem::action("Add to Watches", AddToWatches)
                .icon(icons::ADD_TO_WATCHES)
                .enabled(value.is_some() || watch.is_some()),
        );
        if value.is_some() && extras.contains(&debugger::ExtraValueAction::ShowReferringObjects) {
            items.push(MenuItem::unavailable("Show Referring Objects\u{2026}"));
        }
        items.push(MenuItem::separator());
        // Enabled for every value as in the reference
        // (`XValue.canNavigateToSource`); the product finds a frame's
        // values and the watches that are a name.
        items.push(
            MenuItem::action("Jump To Source", JumpToValueSource)
                .enabled(suspended && (value.is_some() || watch.is_some())),
        );
        let typed = value.is_some_and(|v| v.variable.type_name.is_some());
        items.push(
            MenuItem::action("Jump To Type Source", JumpToTypeSource).enabled(typed && suspended),
        );
        // View as: the renderers of the value's type, the one shown checked
        // (`ViewAsGroup`, hidden without renderers).
        if let (Some(v), Some(provider)) = (value, &self.provider) {
            let (renderers, _) =
                provider.value_renderers(v.variable.type_name.as_deref().unwrap_or(""));
            if !renderers.is_empty() {
                let current = v.renderer.clone().unwrap_or_default();
                let choices = renderers
                    .iter()
                    .map(|r| {
                        let item = MenuItem::action(
                            *r,
                            ViewAs {
                                renderer: (*r).to_owned(),
                            },
                        );
                        if *r == current {
                            item.icon(IconName::CHECKED)
                        } else {
                            item
                        }
                    })
                    .collect();
                items.push(MenuItem::separator());
                items.push(MenuItem::submenu("View as", choices));
            }
        }
        items
    }
}

/// Puts a text into a field, the caret at its end.
fn put_text(e: &mut Editor, text: &str, cx: &mut Context<Editor>) {
    e.set_text_selected(text, cx);
    let end = text.chars().count();
    e.set_selections(
        text::SelectionSet::single(text::Selection::new(end, end)),
        cx,
    );
}

/// The commands of the evaluate field, the watches and the values, at the
/// window's root (the selected tab's variables view handles them).
pub(crate) fn forward_watch_actions<E: InteractiveElement>(el: E, view: Entity<DebuggerView>) -> E {
    macro_rules! on {
        ($el:expr, $action:ty, |$t:ident, $a:ident, $window:ident, $cx:ident| $body:expr) => {{
            let v = view.clone();
            $el.on_action(move |$a: &$action, $window, cx| {
                v.update(cx, |v, cx| v.with_variables(cx, |$t, $cx| $body))
            })
        }};
    }
    let el = on!(el, EvaluateField, |t, a, window, cx| t.evaluate_field(
        a.expression.clone(),
        window,
        cx
    ));
    let el = on!(el, WatchField, |t, a, window, cx| t.watch_field(
        a.expression.clone(),
        window,
        cx
    ));
    let el = on!(el, ShowEvaluateHistory, |t, _a, window, cx| t
        .show_history(window, cx));
    let el = on!(el, ShowEditorHistory, |t, _a, window, cx| t
        .show_editor_history(window, cx));
    let el = on!(el, ChooseHistory, |t, a, window, cx| t
        .choose_history(a.index, window, cx));
    let el = on!(el, SetFieldExpression, |t, a, window, cx| t
        .set_field_text(&a.text, window, cx));
    let el = on!(el, EvaluateValue, |t, a, window, cx| t.evaluate_value(
        a.expression.clone(),
        window,
        cx
    ));
    let el = on!(el, NewWatch, |t, a, window, cx| t.new_watch(
        a.expression.clone(),
        window,
        cx
    ));
    let el = on!(el, RemoveWatch, |t, a, _window, cx| t.remove_watch(
        a.expression.clone(),
        a.index,
        cx
    ));
    let el = on!(el, RemoveAllWatches, |t, _a, _window, cx| t
        .remove_all_watches(cx));
    let el = on!(el, PauseWatch, |t, a, _window, cx| t
        .pause_watch(a.index, cx));
    let el = on!(el, EvaluatePausedWatch, |t, a, _window, cx| t
        .evaluate_paused_watch(a.index, cx));
    let el = on!(el, EditWatch, |t, a, window, cx| t.edit_watch(
        a.expression.clone(),
        window,
        cx
    ));
    let el = on!(el, AddToWatches, |t, _a, _window, cx| t.add_to_watches(cx));
    let el = on!(el, SetValue, |t, a, window, cx| t.set_value(
        a.value.clone(),
        window,
        cx
    ));
    let el = on!(el, CommitEdit, |t, _a, window, cx| t
        .commit_edit(window, cx));
    let el = on!(el, CancelEdit, |t, _a, window, cx| t
        .cancel_edit(window, cx));
    let el = on!(el, CopyValue, |t, _a, _window, cx| t.copy_value(cx));
    let el = on!(el, CopyName, |t, _a, _window, cx| t.copy_name(cx));
    let el = on!(el, ViewAs, |t, a, _window, cx| t.view_as(&a.renderer, cx));
    let el = on!(el, VariablesMenu, |t, _a, window, cx| t
        .menu_at_selection(window, cx));
    let v = view.clone();
    let el = el.on_action(move |_: &JumpToValueSource, _, cx| {
        v.update(cx, |v, cx| v.jump_to_value_source(cx))
    });
    let v = view;
    el.on_action(move |_: &JumpToTypeSource, _, cx| v.update(cx, |v, cx| v.jump_to_type_source(cx)))
}

impl DebuggerView {
    /// Jump To Source of the selected value: its declaration in the frame's
    /// file, found by the product off the UI thread.
    pub(crate) fn jump_to_value_source(&mut self, cx: &mut Context<Self>) {
        let Some(provider) = self.debugger.as_ref().map(|d| d.provider().clone()) else {
            return;
        };
        let mut target = None;
        self.with_variables(cx, |t, cx| target = t.jump_target(cx));
        let Some((path, line, name)) = target else {
            return;
        };
        let read = cx.background_spawn({
            let path = path.clone();
            async move {
                let text = std::fs::read_to_string(&path).ok()?;
                provider.value_declaration(&text, line, &name)
            }
        });
        cx.spawn(async move |this, cx| {
            if let Some((line, column)) = read.await {
                let _ = this.update(cx, |_, cx| {
                    cx.emit(crate::DebuggerViewEvent::OpenAt { path, line, column })
                });
            }
        })
        .detach();
    }

    /// Jump To Type Source of the selected value: the running program says
    /// where its type is declared (the product's expression, evaluated in
    /// the frame); nothing opens for a type without a source (a builtin).
    pub(crate) fn jump_to_type_source(&mut self, cx: &mut Context<Self>) {
        let Some(provider) = self.debugger.as_ref().map(|d| d.provider().clone()) else {
            return;
        };
        let mut target = None;
        let mut session = None;
        self.with_variables(cx, |t, cx| {
            target = t.type_source_target(cx);
            session = Some(t.session().clone());
        });
        let (Some((frame, expression)), Some(session)) = (target, session) else {
            return;
        };
        let Some(asked) = provider.type_source_expression(&expression) else {
            return;
        };
        let pending = session.evaluate_in(&asked, Some(frame), EvaluateContext::Watch);
        cx.spawn(async move |this, cx| {
            let Ok(value) = pending.await else {
                return;
            };
            let Some((path, line)) = provider.parse_type_source(&value.value) else {
                return;
            };
            let _ = this.update(cx, |_, cx| {
                cx.emit(crate::DebuggerViewEvent::OpenAt {
                    path,
                    line,
                    column: 1,
                })
            });
        })
        .detach();
    }
}
