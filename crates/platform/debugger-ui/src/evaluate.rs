//! The Evaluate dialog (the reference's `XDebuggerEvaluationDialog`, Alt+F8 /
//! Evaluate Expression…): a non-modal window "Evaluate" over the suspended
//! frame of the selected session.
//!
//! - Expression mode: the "Expression:" field (`XDebuggerExpressionComboBox`
//!   with the expand button and the history's arrow in it), the hint with
//!   the key of Add to Watches below it, then "Result:" with the value's
//!   tree. Enter evaluates, ⇧⌘Enter adds the expression to the watches.
//! - Code fragment mode (the expand button, Alt+M): a multi-line editor
//!   ("Code fragment:", `evaluateCodeFragment` history) over the result;
//!   ⌘Enter evaluates, the collapse button or Alt+M go back. The mode chosen
//!   last is kept with the project's debugger state.
//! - The result is the tree of the evaluated value (`result`, expanded,
//!   `EvaluatingExpressionRootNode`), "Evaluating…" while the adapter
//!   answers, the adapter's message on an error. The frame's values are
//!   read again after an evaluation (`rebuildViews`), as a statement may
//!   have changed them.
//! - The text of the field or of the editor: the selected value's
//!   expression (when the variables have the focus), else the selected text
//!   of the editor, else the word at its caret. A selection of more than
//!   one line opens the code fragment mode.
//! - The dialog stays open while the program runs and evaluates in the frame
//!   shown after the next stop; it closes with the session.

use std::cell::Cell;
use std::rc::Rc;

use debugger::values::{ValueKind, ValueRole};
use debugger::{Debugger, EvaluateContext, HistoryKind, Phase, Variable};
use editor::{Editor, EditorEvent};
use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeystrokeEvent, MouseButton, ParentElement, Pixels, Render,
    Styled, Subscription, Task, WeakEntity, Window, WindowHandle, anchored, canvas, div, point,
    prelude::*, px, size,
};
use serde_json::{Value as Json, json};
use themes::{Metric, UiRole};
use ui::deferred;
use ui::form::{BarButton, BottomBar, ButtonRole, field_button};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt as _;

use crate::session::commands::{
    CopyValue, VariablesDown, VariablesLeft, VariablesRight, VariablesUp,
};
use crate::variables::{PopupRoot, VariablesView};
use crate::view::DebuggerView;
use crate::watches::{Eval, evaluate};

/// The dialog's title (`xdebugger.evaluate.dialog.title`).
pub const TITLE: &str = "Evaluate";
/// The key context of the dialog (the editors in it add `> Editor`) and of
/// the expression field.
pub const CONTEXT: &str = "EvaluateDialog";
pub const EXPRESSION_CONTEXT: &str = "EvaluateExpression";
/// The dialog's size (`JBUI.DialogSizes.medium()` as measured on the
/// reference), the smallest width (`EvaluationMainPanel`).
const WIDTH: f32 = 500.0;
const HEIGHT: f32 = 322.0;
const MIN_WIDTH: f32 = 450.0;
const MIN_HEIGHT: f32 = 280.0;
/// The insets of the content and the gaps of its parts.
const INSET: f32 = 12.0;
const LABEL_GAP: f32 = 4.0;
const SECTION_GAP: f32 = 8.0;
/// The hint under the field (`ComponentStyle.SMALL`, brighter).
const HINT_SIZE: f32 = 11.0;
/// The history's list: rows 24 high, at most ten shown.
const HISTORY_ROW: f32 = 24.0;
const HISTORY_TEXT: f32 = 7.5;
const HISTORY_ROWS: usize = 10;
/// The message when there is no frame to evaluate in
/// (`xdebugger.evaluate.stack.frame.has.not.evaluator`).
const NO_FRAME: &str = "Cannot evaluate, current stack frame doesn't support evaluation";
/// The row of an evaluation under way
/// (`xdebugger.evaluating.expression.node.message`).
const EVALUATING: &str = "Evaluating\u{2026}";

ui::ui_commands! {
    COMMANDS, scope = "EvaluateDialog";
    /// Evaluate.
    Evaluate { expression: Option<String> } => "evaluate-dialog.evaluate", "Evaluate",
        "Evaluates the dialog's expression (or code fragment) in the selected frame and shows its value as a tree; with an expression it is put into the field first.",
        r#"{"expression": "len(items)"}"#;
    /// Switch between Expression and Code Fragment mode.
    SwitchMode => "evaluate-dialog.switch-mode", "Switch Evaluation Mode",
        "Switches the Evaluate dialog between expression mode (one line) and code fragment mode (several statements); the text stays.";
    /// Add to Watches.
    AddToWatches => "evaluate-dialog.add-to-watches", "Add to Watches",
        "Adds the dialog's expression to the watches (expression mode only); the dialog stays open.";
    /// Move the focus to the result's tree.
    FocusResult => "evaluate-dialog.focus-result", "Focus Result",
        "Gives the focus to the result tree of the Evaluate dialog.";
    /// The expressions evaluated before.
    ShowHistory => "evaluate-dialog.history", "Show History",
        "Opens the list of the expressions (or code fragments) evaluated before under the field; choosing one puts it into the field.";
    /// Choose a row of the open history.
    ChooseHistory { index: usize } => "evaluate-dialog.choose-history", "Choose from History",
        "Chooses the row of the open history list at a zero-based index.",
        r#"{"index": 0}"#;
    /// Close.
    Close => "evaluate-dialog.close", "Close",
        "Closes the Evaluate dialog, keeping the text in the history.";
}

/// Registers the commands.
pub fn register(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
}

/// What the dialog evaluates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Expression,
    Fragment,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Mode::Expression => "expression",
            Mode::Fragment => "fragment",
        }
    }

    fn history(self) -> HistoryKind {
        match self {
            Mode::Expression => HistoryKind::Evaluate,
            Mode::Fragment => HistoryKind::CodeFragment,
        }
    }
}

/// The result panel.
enum Outcome {
    None,
    Pending,
    Error(String),
    Value(Entity<VariablesView>),
}

/// The open history list.
struct History {
    items: Vec<String>,
    highlighted: Option<usize>,
    _keys: Subscription,
}

/// The Evaluate dialog.
pub struct EvaluateDialog {
    host: WeakEntity<DebuggerView>,
    debugger: Debugger,
    mode: Mode,
    expression: Entity<Editor>,
    fragment: Entity<Editor>,
    outcome: Outcome,
    /// What the result was evaluated from.
    evaluated: Option<String>,
    /// Grows with every evaluation: late answers of an earlier one are
    /// dropped.
    epoch: u64,
    _evaluation: Option<Task<()>>,
    history: Option<History>,
    field_bounds: Rc<Cell<Bounds<Pixels>>>,
    activate: bool,
    /// The result tree had the focus when it was drawn last.
    result_focused: bool,
    _subscriptions: Vec<Subscription>,
}

impl EvaluateDialog {
    /// Opens the dialog of the main window `window` (or brings the open
    /// one to the front), with `text` in its field.
    pub(crate) fn open(
        view: Entity<DebuggerView>,
        debugger: Debugger,
        text: String,
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
        let handle = ui::dialog_window::open(window, options, cx, {
            let text = text.clone();
            move |window, cx| {
                cx.new(|cx| Self::new(host, view, debugger, &text, activate, window, cx))
            }
        })?;
        // An open dialog takes the new text.
        let _ = handle.update(cx, |dialog, window, cx| {
            dialog.set_text(&text, window, cx);
        });
        Some(handle)
    }

    fn new(
        host: WeakEntity<DebuggerView>,
        view: Entity<DebuggerView>,
        debugger: Debugger,
        text: &str,
        activate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mode = if text.contains('\n') || debugger.evaluate_fragment_mode() {
            Mode::Fragment
        } else {
            Mode::Expression
        };
        let expression = editor::field::new("", None, false, window, cx);
        let fragment = cx.new(|cx| Editor::new(text::Buffer::new(""), window, cx));
        let subscriptions = vec![
            cx.subscribe(&expression, |_, _, event: &EditorEvent, cx| {
                if matches!(event, EditorEvent::Edited(_)) {
                    cx.notify();
                }
            }),
            cx.subscribe(&fragment, |_, _, event: &EditorEvent, cx| {
                if matches!(event, EditorEvent::Edited(_)) {
                    cx.notify();
                }
            }),
            // The session ended: the dialog closes with it.
            cx.observe_in(&view, window, |this, _, window, cx| {
                this.session_changed(window, cx);
            }),
            // Closed by its title bar: the host forgets it.
            cx.on_release(|this, cx| {
                let host = this.host.clone();
                cx.defer(move |cx| {
                    let _ = host.update(cx, |h, cx| h.evaluate_dialog_closed(cx));
                });
            }),
        ];
        let mut dialog = Self {
            host,
            debugger,
            mode,
            expression,
            fragment,
            outcome: Outcome::None,
            evaluated: None,
            epoch: 0,
            _evaluation: None,
            history: None,
            field_bounds: Rc::new(Cell::new(Bounds::default())),
            activate,
            result_focused: false,
            _subscriptions: subscriptions,
        };
        // After the update of the host that opens it.
        let this = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = this.update(cx, |this, cx| this.highlight(cx));
        });
        dialog.set_text(text, window, cx);
        dialog
    }

    /// The frame's language colours for the editors
    /// (`XDebuggerEditorBase.setSourcePosition`).
    fn highlight(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let host = host.read(cx);
        let file = host.execution.as_ref().map(|p| p.path.clone());
        let (Some(file), Some(highlighters)) = (file, host.highlighters.clone()) else {
            return;
        };
        let expression = highlighters(&file);
        let fragment = highlighters(&file);
        self.expression
            .update(cx, |e, cx| e.set_highlighter(expression, cx));
        self.fragment
            .update(cx, |e, cx| e.set_highlighter(fragment, cx));
    }

    fn editor(&self) -> &Entity<Editor> {
        match self.mode {
            Mode::Expression => &self.expression,
            Mode::Fragment => &self.fragment,
        }
    }

    fn text(&self, cx: &App) -> String {
        self.editor().read(cx).text()
    }

    /// Puts a text into the dialog's editor, selected in the expression
    /// mode so that typing replaces it, and gives the editor the focus.
    pub(crate) fn set_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if text.is_empty() {
            window.focus(&self.editor().focus_handle(cx));
            return;
        }
        if text.contains('\n') && self.mode == Mode::Expression {
            self.mode = Mode::Fragment;
        }
        let mode = self.mode;
        self.editor().update(cx, |e, cx| match mode {
            Mode::Expression => e.set_text_selected(text, cx),
            Mode::Fragment => {
                e.set_text_selected(text, cx);
                let end = e.buffer().len_chars();
                e.set_selections(text::SelectionSet::caret(end), cx);
            }
        });
        window.focus(&self.editor().focus_handle(cx));
        cx.notify();
    }

    /// The dialog's selected frame: its session and id.
    fn frame(&self, cx: &App) -> Option<(debugger::Session, i64)> {
        self.host.upgrade()?.read(cx).suspended_frame(cx)
    }

    // ---- the commands --------------------------------------------------------------

    fn on_evaluate(&mut self, a: &Evaluate, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = &a.expression {
            self.set_text(text, window, cx);
        }
        self.evaluate(window, cx);
    }

    /// Evaluate: the editor's text in the selected frame
    /// (`XDebuggerEvaluationDialog.evaluate`).
    fn evaluate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.history = None;
        let expression = self.text(cx);
        if expression.trim().is_empty() {
            return;
        }
        self.debugger.remember(self.mode.history(), &expression);
        window.focus(&self.editor().focus_handle(cx));
        self.epoch += 1;
        self.evaluated = Some(expression.clone());
        let Some((session, frame)) = self.frame(cx) else {
            self.outcome = Outcome::Error(NO_FRAME.to_owned());
            self._evaluation = None;
            cx.notify();
            return;
        };
        self.outcome = Outcome::Pending;
        let epoch = self.epoch;
        let task = evaluate(
            session.clone(),
            Some(self.debugger.provider().clone()),
            frame,
            expression,
            EvaluateContext::Repl,
        );
        self._evaluation = Some(cx.spawn_in(window, async move |this, cx| {
            let eval = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.epoch == epoch {
                    this.evaluation_done(session, frame, eval, window, cx);
                }
            });
        }));
        cx.notify();
    }

    fn evaluation_done(
        &mut self,
        session: debugger::Session,
        frame: i64,
        eval: Eval,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match eval {
            Eval::Value { variable, shape } => {
                let tree = self.result_tree(session, frame, *variable, shape, cx);
                self.outcome = Outcome::Value(tree);
            }
            Eval::Error(message) => self.outcome = Outcome::Error(message),
            Eval::Pending | Eval::Paused => self.outcome = Outcome::None,
        }
        // The values of the frame may have changed (`rebuildViews`).
        if let Some(host) = self.host.upgrade() {
            host.update(cx, |h, cx| {
                h.with_variables(cx, |t, cx| t.reload(cx));
            });
        }
        cx.notify();
    }

    /// The result's tree: `result` at the root, expanded and selected.
    fn result_tree(
        &self,
        session: debugger::Session,
        frame: i64,
        mut variable: Variable,
        shape: Option<String>,
        cx: &mut Context<Self>,
    ) -> Entity<VariablesView> {
        let provider = self.debugger.provider().clone();
        let kind = match provider.present_value(&variable).role {
            ValueRole::Value(kind) => kind,
            _ if variable.reference > 0 => ValueKind::Object,
            _ => ValueKind::Primitive,
        };
        if variable.evaluate_name.is_none() {
            variable.evaluate_name = Some(variable.name.clone());
        }
        let root = PopupRoot {
            name: crate::watches::RESULT_NAME.to_owned(),
            variable,
            shape,
            kind,
            parent: 0,
        };
        let (location, highlighters) = match self.host.upgrade() {
            Some(host) => {
                let host = host.read(cx);
                (
                    host.execution.as_ref().map(|p| (p.path.clone(), p.line)),
                    host.highlighters.clone(),
                )
            }
            None => (None, None),
        };
        cx.new(|cx| {
            let mut view =
                VariablesView::for_popup(session, Some(provider), frame, location, root, cx);
            view.highlighters = highlighters;
            view
        })
    }

    fn on_switch_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx);
        self.history = None;
        self.mode = match self.mode {
            Mode::Expression => Mode::Fragment,
            Mode::Fragment => Mode::Expression,
        };
        // Only the user's choice is remembered.
        self.debugger
            .set_evaluate_fragment_mode(self.mode == Mode::Fragment);
        // The text goes over; a code fragment of several lines does not
        // fit the field, which keeps the lines apart with spaces.
        let text = match self.mode {
            Mode::Fragment => text,
            Mode::Expression => text.lines().collect::<Vec<_>>().join(" "),
        };
        let mode = self.mode;
        self.editor().update(cx, |e, cx| {
            e.set_text_selected(&text, cx);
            if mode == Mode::Fragment {
                let end = e.buffer().len_chars();
                e.set_selections(text::SelectionSet::caret(end), cx);
            }
        });
        window.focus(&self.editor().focus_handle(cx));
        cx.notify();
    }

    fn on_add_to_watches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode != Mode::Expression {
            return;
        }
        let expression = self.text(cx).trim().to_owned();
        if expression.is_empty() {
            return;
        }
        self.debugger.remember(HistoryKind::Evaluate, &expression);
        if let Some(host) = self.host.upgrade() {
            host.update(cx, |h, cx| {
                h.with_variables(cx, |t, cx| t.add_watch_expression(&expression, cx));
            });
        }
        window.focus(&self.editor().focus_handle(cx));
        cx.notify();
    }

    fn on_focus_result(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Outcome::Value(tree) = &self.outcome {
            window.focus(&tree.focus_handle(cx));
        }
    }

    fn on_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.history.is_some() {
            self.history = None;
            cx.notify();
            return;
        }
        let text = self.text(cx);
        self.debugger.remember(self.mode.history(), &text);
        ui::dialog_window::close(window.window_handle(), cx);
    }

    /// The host changed: the dialog closes once the session it evaluates
    /// in has ended (`sessionStopped`).
    fn session_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let ended = host
            .read(cx)
            .tool_window
            .read(cx)
            .selected_tab()
            .is_none_or(|t| t.session.phase() == Phase::Ended);
        if ended {
            ui::dialog_window::close(window.window_handle(), cx);
        }
    }

    /// The focus of the result tree, once there is one.
    pub fn result_focus(&self, cx: &App) -> Option<FocusHandle> {
        match &self.outcome {
            Outcome::Value(tree) => Some(tree.read(cx).focus.clone()),
            _ => None,
        }
    }

    fn with_tree(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut VariablesView, &mut Context<VariablesView>),
    ) {
        if let Outcome::Value(tree) = &self.outcome {
            tree.update(cx, f);
        }
    }

    // ---- the history ------------------------------------------------------------------

    fn on_show_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.history.take().is_some() {
            cx.notify();
            return;
        }
        let items = self.debugger.history(self.mode.history());
        window.focus(&self.editor().focus_handle(cx));
        let view = cx.entity().downgrade();
        let handle = window.window_handle();
        // The keys of the list, before the editor's own bindings.
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
        self.history = Some(History {
            items,
            highlighted: None,
            _keys: keys,
        });
        cx.notify();
    }

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
                None => self.history = None,
            },
            "escape" => self.history = None,
            _ => return false,
        }
        cx.notify();
        true
    }

    fn choose_history(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(list) = self.history.take() else {
            return;
        };
        if let Some(text) = list.items.get(index).cloned() {
            self.editor().update(cx, |e, cx| {
                e.set_text_selected(&text, cx);
                let end = e.buffer().len_chars();
                e.set_selections(text::SelectionSet::caret(end), cx);
            });
            window.focus(&self.editor().focus_handle(cx));
        }
        cx.notify();
    }

    // ---- state ------------------------------------------------------------------------

    pub(crate) fn state_json(&self, cx: &App) -> Json {
        let (state, message, tree) = match &self.outcome {
            Outcome::None => ("none", None, Json::Null),
            Outcome::Pending => ("pending", None, Json::Null),
            Outcome::Error(m) => ("error", Some(m.clone()), Json::Null),
            Outcome::Value(tree) => ("value", None, tree.read(cx).state_json(cx)),
        };
        json!({
            "mode": self.mode.as_str(),
            "expression": self.expression.read(cx).text(),
            "fragment": self.fragment.read(cx).text(),
            "evaluated": self.evaluated,
            "evaluations": self.epoch,
            "result": {"state": state, "message": message, "tree": tree},
            "history": self.history.as_ref().map(|h| json!({
                "items": h.items,
                "highlighted": h.highlighted,
            })),
            "frame": self.frame(cx).is_some(),
            "result_focused": self.result_focused,
            "activate": self.activate,
        })
    }

    // ---- drawing ----------------------------------------------------------------------

    fn label(&self, text: &'static str, cx: &App) -> AnyElement {
        div()
            .flex_none()
            .pb(px(LABEL_GAP))
            .text_color(cx.theme().color(UiRole::Text))
            .child(text)
            .into_any_element()
    }

    /// The "Expression:" field with the buttons in it.
    fn render_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let bounds = self.field_bounds.clone();
        let frame = editor::field::frame(&self.expression, false, false, window, cx)
            .child(field_button(
                "evaluate-expand",
                IconName::EXPAND,
                "Expand",
                Some(Box::new(SwitchMode)),
            ))
            .child(field_button(
                "evaluate-history",
                IconName::CHEVRON_DOWN,
                "Show History",
                Some(Box::new(ShowHistory)),
            ));
        div()
            .id("evaluate-expression")
            .debug_selector(|| "evaluate-expression".into())
            .key_context(EXPRESSION_CONTEXT)
            .relative()
            .flex_none()
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .child(frame)
            .into_any_element()
    }

    /// The code fragment's editor in its frame, with the collapse button.
    fn render_fragment(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let focused = self.fragment.focus_handle(cx).is_focused(window);
        let theme = cx.theme();
        let button = div()
            .absolute()
            .top(px(4.0))
            .right(px(6.0))
            .child(field_button(
                "evaluate-collapse",
                IconName::COLLAPSE,
                "Collapse",
                Some(Box::new(SwitchMode)),
            ));
        ui::components::input_frame(focused, false, cx)
            .id("evaluate-fragment")
            .debug_selector(|| "evaluate-fragment".into())
            .flex_1()
            .min_h(px(0.0))
            .items_start()
            .px(px(6.0))
            .py(px(2.0))
            .bg(theme.color(UiRole::Surface))
            .child(
                div()
                    .size_full()
                    .overflow_hidden()
                    .child(self.fragment.clone()),
            )
            .child(button)
            .into_any_element()
    }

    fn render_result(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let body = match &self.outcome {
            Outcome::None => div().into_any_element(),
            Outcome::Pending => div()
                .id("evaluate-pending")
                .pl(px(4.0))
                .text_color(theme.color(UiRole::TextDisabled))
                .child(EVALUATING)
                .into_any_element(),
            Outcome::Error(message) => div()
                .id("evaluate-error")
                .debug_selector(|| "evaluate-error".into())
                .flex()
                .flex_row()
                .items_start()
                .gap(px(4.0))
                .pl(px(4.0))
                .text_color(theme.color(UiRole::Danger))
                .child(Icon::new(IconName::ERROR).size(px(16.0)))
                .child(
                    div()
                        .min_w(px(0.0))
                        .child(self.debugger.provider().evaluation_error(message).shown()),
                )
                .into_any_element(),
            Outcome::Value(tree) => div()
                .id("evaluate-tree")
                .debug_selector(|| "evaluate-tree".into())
                .size_full()
                .child(tree.clone())
                .into_any_element(),
        };
        self.result_focused = match &self.outcome {
            Outcome::Value(tree) => tree.read(cx).focus.contains_focused(window, cx),
            _ => false,
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .child(self.label("Result:", cx))
            .child(div().flex_1().min_h(px(0.0)).overflow_hidden().child(body))
            .into_any_element()
    }

    /// The history's list under the field.
    fn render_history(&self, window: &Window, cx: &Context<Self>) -> Option<AnyElement> {
        let list = self.history.as_ref()?;
        let theme = cx.theme();
        let b = self.field_bounds.get();
        let radius = theme.metric(Metric::CornerRadius);
        let selection = theme.color(UiRole::Selection);
        let rows = list.items.iter().enumerate().map(|(i, item)| {
            div()
                .id(("evaluate-history-row", i))
                .debug_selector(move || format!("evaluate-history-row-{i}"))
                .flex_none()
                .h(px(HISTORY_ROW))
                .px(px(HISTORY_TEXT))
                .flex()
                .items_center()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .when(list.highlighted == Some(i), |d| d.bg(selection))
                .child(item.lines().next().unwrap_or_default().to_owned())
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
            .id("evaluate-history-list")
            .debug_selector(|| "evaluate-history-list".into())
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
        let viewport = window.viewport_size();
        Some(
            deferred(
                anchored().position(point(px(0.0), px(0.0))).child(
                    div()
                        .id("evaluate-history-backdrop")
                        .w(viewport.width)
                        .h(viewport.height)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.history = None;
                                cx.notify();
                            }),
                        )
                        .child(
                            anchored()
                                .position(point(b.left(), b.bottom()))
                                .snap_to_window_with_margin(px(4.0))
                                .child(popup),
                        ),
                ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}

impl Render for EvaluateDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fragment = self.mode == Mode::Fragment;
        let hint_focus = self.editor().focus_handle(cx);
        let watch_keys = ui::keys::shortcut_in_view(&AddToWatches, &hint_focus, window, cx);
        let content = if fragment {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .gap(px(SECTION_GAP))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h(px(0.0))
                        .child(self.label("Code fragment:", cx))
                        .child(self.render_fragment(window, cx)),
                )
                .child(self.render_result(window, cx))
        } else {
            let hint = watch_keys.map(|keys| {
                div()
                    .flex_none()
                    .flex()
                    .justify_end()
                    .pt(px(2.0))
                    .pb(px(6.0))
                    .text_size(px(HINT_SIZE))
                    .text_color(cx.theme().color(UiRole::TextDisabled))
                    .child(format!("Use {keys} to add to Watches"))
            });
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .child(self.label("Expression:", cx))
                .child(self.render_field(window, cx))
                .children(hint)
                .child(self.render_result(window, cx))
        };
        let history = self.render_history(window, cx);
        let theme = cx.theme();
        let bar = BottomBar::new()
            .help(ui::form::help_button("evaluate-help", None, cx))
            .button(BarButton::new(
                "evaluate-close",
                "Close",
                ButtonRole::Cancel,
                Box::new(Close),
            ))
            .button(BarButton::new(
                "evaluate-evaluate",
                "Evaluate",
                ButtonRole::Ok,
                Box::new(Evaluate { expression: None }),
            ))
            .render(cx);
        div()
            .id("evaluate-dialog")
            .debug_selector(|| "evaluate-dialog".into())
            .key_context(CONTEXT)
            .on_action(
                cx.listener(|this, a: &Evaluate, window, cx| this.on_evaluate(a, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SwitchMode, window, cx| this.on_switch_mode(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &AddToWatches, window, cx| {
                    this.on_add_to_watches(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &FocusResult, window, cx| this.on_focus_result(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ShowHistory, window, cx| this.on_show_history(window, cx)),
            )
            .on_action(cx.listener(|this, a: &ChooseHistory, window, cx| {
                this.choose_history(a.index, window, cx)
            }))
            .on_action(cx.listener(|this, _: &Close, window, cx| this.on_close(window, cx)))
            .on_action(cx.listener(|this, _: &VariablesUp, _, cx| {
                this.with_tree(cx, |t, cx| t.move_selection(-1, cx))
            }))
            .on_action(cx.listener(|this, _: &VariablesDown, _, cx| {
                this.with_tree(cx, |t, cx| t.move_selection(1, cx))
            }))
            .on_action(cx.listener(|this, _: &VariablesLeft, _, cx| {
                this.with_tree(cx, |t, cx| t.left_right(false, cx))
            }))
            .on_action(cx.listener(|this, _: &VariablesRight, _, cx| {
                this.with_tree(cx, |t, cx| t.left_right(true, cx))
            }))
            .on_action(cx.listener(|this, _: &CopyValue, _, cx| {
                this.with_tree(cx, |t, cx| t.copy_value(cx))
            }))
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
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .px(px(INSET))
                    .pt(px(INSET - 2.0))
                    .pb(px(INSET))
                    .child(content),
            )
            .child(bar)
            .children(history)
            .child(ui::pointer::frame())
    }
}

impl Focusable for EvaluateDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor().focus_handle(cx)
    }
}

impl DebuggerView {
    /// Evaluate Expression (Alt+F8): the dialog over the suspended frame,
    /// with `expression`, else the selected value's (when the variables
    /// have the focus), else the editor's selection or the word at its
    /// caret (`XDebuggerEvaluateActionHandler`). Without a suspended frame
    /// the command does nothing (the reference's action is disabled).
    pub(crate) fn evaluate_expression_action(
        &mut self,
        expression: Option<String>,
        editor: Option<&Entity<Editor>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.suspended_frame(cx).is_none() {
            return;
        }
        let Some(debugger) = self.debugger.clone() else {
            return;
        };
        let text = match expression {
            Some(text) => text,
            None => self.expression_to_evaluate(editor, window, cx),
        };
        let view = cx.entity();
        self.evaluate_dialog =
            EvaluateDialog::open(view, debugger, text, self.activate, window, cx);
        cx.notify();
    }

    /// The text an Evaluate dialog opens with.
    fn expression_to_evaluate(
        &mut self,
        editor: Option<&Entity<Editor>>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> String {
        if let Some(variables) = self.tool_window.read(cx).selected_variables()
            && variables.read(cx).focus.contains_focused(window, cx)
            && let Some(expression) = variables.update(cx, |v, cx| v.selected_expression(cx))
        {
            return expression;
        }
        let Some(editor) = editor else {
            return String::new();
        };
        let e = editor.read(cx);
        let buffer = e.buffer();
        let selection = e.selections().primary().range();
        if !selection.is_empty() {
            return buffer.slice(selection);
        }
        let word = text::movement::word_range(buffer, e.selections().primary().head);
        let text = buffer.slice(word);
        if text.chars().all(|c| c.is_alphanumeric() || c == '_') {
            text
        } else {
            String::new()
        }
    }

    pub(crate) fn close_evaluate_dialog(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.evaluate_dialog.take() {
            ui::dialog_window::close(dialog.into(), cx);
        }
    }

    /// The Evaluate dialog was closed (by its title bar too).
    pub(crate) fn evaluate_dialog_closed(&mut self, cx: &mut Context<Self>) {
        if self
            .evaluate_dialog
            .as_ref()
            .is_some_and(|d| d.read(cx).is_err())
        {
            self.evaluate_dialog = None;
            cx.notify();
        }
    }
}
