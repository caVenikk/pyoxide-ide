//! The watches and the evaluate field of Threads & Variables (the
//! reference's `XWatchesViewImpl` with `debugger.watches.in.variables`,
//! `WatchesRootNode`, `WatchNodeImpl`):
//!
//! - the evaluate field above the variables (`XDebuggerEmbeddedComboBox`):
//!   Enter evaluates its expression in the selected frame and shows the
//!   value as the first node, `result`; Add to Watches (its button and
//!   ⇧⌘Enter) adds the expression to the watches; the chevron lists the
//!   expressions evaluated before (`XDebuggerHistoryManager`, kept per
//!   project);
//! - the watches after the result and before the frame's values, each
//!   evaluated in the frame shown, kept per run configuration type in the
//!   personal project state; New Watch, Remove Watch and Edit change them,
//!   a watch is edited in its row (`WatchInplaceEditor`);
//! - Set Value in the row of a value (`SetValueInplaceEditor`), Copy Value
//!   (`XCopyValueAction`, the whole value in the clipboard context), Copy
//!   Name, Add to Watches, Evaluate Expression (into the field), Jump To
//!   Source (`XJumpToSourceAction`, the product finds the declaration) and
//!   the context menu with them (`XDebugger.Watches.Tree.Popup`).

use std::sync::Arc;

use debugger::values::ValueRole;
use debugger::{DebugAdapterProvider, DebugError, EvaluateContext, Phase, Session, Variable};
use editor::{Editor, EditorEvent};
use gpui::{
    AnyElement, App, ClipboardItem, Context, Entity, Focusable, InteractiveElement, IntoElement,
    ParentElement, Pixels, Point, Styled, Subscription, Window, canvas, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::{Metric, UiRole};
use ui::components::{DISABLED_ALPHA, Tooltip};
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::theme::ThemeExt;

use crate::DebuggerView;
use crate::icons;
use crate::session::commands::*;
use crate::variables::{Row, RowKind, ValueRow, VariablesView, parent_of};

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
/// The height of the evaluate field's row.
const FIELD_HEIGHT: f32 = 30.0;
/// How the result is evaluated: as the debug console does, so that a
/// statement runs too.
const RESULT_CONTEXT: EvaluateContext = EvaluateContext::Repl;

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
    Value {
        variable: Variable,
        /// The size of a container.
        shape: Option<String>,
    },
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
    pub value: WatchValue,
}

impl WatchRow {
    pub(crate) fn text(&self) -> String {
        match &self.value {
            WatchValue::None => self.name.clone(),
            WatchValue::Pending => {
                format!("{} = {}", self.name, crate::variables::COLLECTING_DATA)
            }
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
    Value(ValueRow),
    Error(String),
}

impl WatchValue {
    pub fn as_str(&self) -> &'static str {
        match self {
            WatchValue::None => "none",
            WatchValue::Pending => "pending",
            WatchValue::Value(_) => "value",
            WatchValue::Error(_) => "error",
        }
    }
}

/// What a row's editor changes.
#[derive(Debug, Clone)]
pub(crate) enum EditTarget {
    /// Set Value of a child of `parent` (a scope or a value).
    SetValue { parent: i64, variable: Variable },
    /// A watch's expression (an empty one removes it).
    Watch { index: usize },
    /// The result becomes a watch at the top.
    Result,
    /// A new watch at an index.
    NewWatch { at: usize },
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
    _blur: Subscription,
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
async fn evaluate(
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
    Eval::Value { variable, shape }
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
        let watches = self
            .session
            .debugger()
            .map(|d| d.watches(self.session.watches_key()))
            .unwrap_or_default();
        if watches != self.watches {
            self.watches = watches;
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

    /// Add to Watches: the selected value's expression, at the end
    /// (`DebuggerUIUtil.addToWatches`).
    pub(crate) fn add_to_watches(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.selected_row(cx) else {
            return;
        };
        let expression = match &row.kind {
            RowKind::Watch(w) => Some(w.expression.clone()),
            _ => row.value().map(|v| {
                v.variable
                    .evaluate_name
                    .clone()
                    .unwrap_or_else(|| v.variable.name.clone())
            }),
        };
        if let Some(expression) = expression.filter(|e| !e.trim().is_empty()) {
            self.add_watch(&expression, None, cx);
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
        cx.subscribe(&field, |_, _, event: &EditorEvent, cx| {
            if matches!(event, EditorEvent::Edited(_)) {
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

    /// Puts a text into the field (selected) and gives it the focus.
    pub(crate) fn set_field_text(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let field = self.ensure_field(window, cx);
        field.update(cx, |e, cx| e.set_text_selected(text, cx));
        window.focus(&field.focus_handle(cx));
        self.close_menu(cx);
        cx.notify();
    }

    /// Enter in the field: the expression's value becomes the result node,
    /// selected (`XWatchesViewImpl.addExpressionResultNode`).
    pub(crate) fn evaluate_field(
        &mut self,
        expression: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(text) = &expression {
            let field = self.ensure_field(window, cx);
            field.update(cx, |e, cx| e.set_text_selected(text, cx));
        }
        let expression = self.field_text(cx).trim().to_owned();
        if expression.is_empty() {
            return;
        }
        if let Some(debugger) = self.session.debugger() {
            debugger.remember_expression(&expression);
        }
        let Some(frame) = self.frame_id() else {
            return;
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
            let field = self.ensure_field(window, cx);
            field.update(cx, |e, cx| e.set_text_selected(text, cx));
        }
        let expression = self.field_text(cx).trim().to_owned();
        if expression.is_empty() {
            return;
        }
        if let Some(debugger) = self.session.debugger() {
            debugger.remember_expression(&expression);
        }
        self.add_watch(&expression, None, cx);
    }

    /// Evaluate Expression of a value: its expression into the field.
    pub(crate) fn evaluate_value(
        &mut self,
        expression: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let expression = expression.or_else(|| {
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
        });
        self.set_field_text(&expression.unwrap_or_default(), window, cx);
    }

    /// The expressions evaluated before, below the field.
    pub(crate) fn show_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let history = self
            .session
            .debugger()
            .map(|d| d.expression_history())
            .unwrap_or_default();
        if history.is_empty() {
            return;
        }
        let field = self.ensure_field(window, cx);
        window.focus(&field.focus_handle(cx));
        let items = history
            .into_iter()
            .map(|e| MenuItem::action(e.clone(), SetFieldExpression { text: e }))
            .collect();
        let bounds = self.field_bounds.get();
        let menu = ContextMenu::popup(
            items,
            PopupOptions {
                min_width: Some(f32::from(bounds.size.width)),
                ..PopupOptions::default()
            },
            window,
            cx,
        );
        self.show_menu(menu, bounds.bottom_left(), window, cx);
    }

    pub(crate) fn field_json(&self, cx: &App) -> Json {
        json!({
            "text": self.field_text(cx),
            "history": self.session.debugger().map(|d| d.expression_history()),
        })
    }

    /// The evaluate field's row: the editor with its hint, Add to Watches
    /// and the history's chevron (`XDebuggerEmbeddedComboBox`).
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
        let watch_keys = ui::keys::shortcut_in(&WatchField { expression: None }, &focus, window);
        let theme = cx.theme();
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
                .size(px(22.0))
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
            .pl(px(8.0))
            .pr(px(6.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .border_b_1()
            .border_color(theme.color(UiRole::Border))
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

    // ---- editing in a row ----------------------------------------------------------

    fn start_edit(
        &mut self,
        path: String,
        target: EditTarget,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = editor::field::new(text, None, false, window, cx);
        editor.update(cx, |e, cx| e.set_text_selected(text, cx));
        let focus = editor.focus_handle(cx);
        let blur = cx.on_blur(&focus, window, |this, window, cx| {
            // A click elsewhere closes the editor without a change.
            if this.editing.as_ref().is_some_and(|e| !e.applying) {
                this.cancel_edit(window, cx);
            }
        });
        self.close_menu(cx);
        self.editing = Some(InlineEdit {
            path: path.clone(),
            target,
            editor,
            error: None,
            applying: false,
            _blur: blur,
        });
        self.dirty = true;
        self.select(&path, cx);
        window.focus(&focus);
        cx.notify();
    }

    /// Enter in the row's editor.
    pub(crate) fn commit_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = &mut self.editing else {
            return;
        };
        if edit.applying {
            return;
        }
        let text = edit.editor.read(cx).text();
        let target = edit.target.clone();
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
            variable: v.variable.clone(),
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

    pub(crate) fn render_inline_editor(&self, edit: &InlineEdit, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let row_h = theme.metric(Metric::RowHeight);
        // The editor's own background over the row's selection.
        ui::components::input_frame(true, edit.error.is_some(), cx)
            .id("variables-editor")
            .debug_selector(|| "variables-editor".into())
            .key_context(EDITOR_CONTEXT)
            .flex_1()
            .h(row_h - px(2.0))
            .px(px(4.0))
            .bg(theme.color(UiRole::EditorBackground))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(self.line_height)
                    .overflow_hidden()
                    .child(edit.editor.clone()),
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
        })
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
        let RowKind::Value(v) = &row.kind else {
            return None;
        };
        let top = self
            .session
            .loaded_scopes(frame)
            .is_some_and(|scopes| scopes.iter().any(|s| s.reference == v.parent));
        if !top {
            return None;
        }
        let snapshot = self.session.snapshot();
        let f = snapshot.frames.iter().find(|f| f.id == frame)?;
        let path = f.source.as_ref()?.path.clone()?;
        Some((path, f.line, v.variable.name.clone()))
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
        self.menu = Some((menu, position, dismissed));
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    /// The context menu of the selected row (`XDebugger.Watches.Tree.Popup`
    /// with the subset of `XDebugger.ValueGroup` Pyoxide has).
    pub(crate) fn open_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row = self.selected_row(cx);
        let items = self.menu_items(row.as_ref(), cx);
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

    fn menu_items(&self, row: Option<&Row>, _cx: &App) -> Vec<MenuItem> {
        let watch = row.and_then(|r| match &r.kind {
            RowKind::Watch(w) => Some(w.as_ref()),
            _ => None,
        });
        let value = row.and_then(Row::value);
        let suspended = self.frame_id().is_some();
        let mut items = vec![MenuItem::action(
            "New Watch\u{2026}",
            NewWatch { expression: None },
        )];
        if watch.is_some() {
            items.push(MenuItem::action(
                "Remove Watch",
                RemoveWatch {
                    expression: None,
                    index: None,
                },
            ));
            items.push(MenuItem::action(
                "Edit\u{2026}",
                EditWatch { expression: None },
            ));
        }
        items.push(MenuItem::separator());
        if watch.is_none() {
            let settable = matches!(row.map(|r| &r.kind), Some(RowKind::Value(_)))
                && suspended
                && self.session.can_set_values();
            items.push(
                MenuItem::action("Set Value\u{2026}", SetValue { value: None }).enabled(settable),
            );
        }
        items.push(MenuItem::action("Copy Value", CopyValue).enabled(value.is_some()));
        items.push(
            MenuItem::action("Copy Name", CopyName).enabled(value.is_some() || watch.is_some()),
        );
        items.push(MenuItem::separator());
        items.push(
            MenuItem::action(
                "Evaluate Expression\u{2026}",
                EvaluateValue { expression: None },
            )
            .enabled(suspended),
        );
        items.push(
            MenuItem::action("Add to Watches", AddToWatches)
                .enabled(value.is_some() || watch.is_some()),
        );
        items.push(MenuItem::separator());
        let top_value = matches!(row.map(|r| &r.kind), Some(RowKind::Value(_)))
            && row.is_some_and(|r| {
                let parent = parent_of(&r.path);
                parent.is_empty() || !parent.contains(crate::variables::SEP)
            });
        items.push(MenuItem::action("Jump To Source", JumpToValueSource).enabled(top_value));
        items
    }
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
    let el = on!(el, VariablesMenu, |t, _a, window, cx| t
        .menu_at_selection(window, cx));
    let v = view;
    el.on_action(move |_: &JumpToValueSource, _, cx| {
        v.update(cx, |v, cx| v.jump_to_value_source(cx))
    })
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
}
