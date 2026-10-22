//! Values in the editor while the program is suspended (the reference's
//! `ValueLookupManager`, `XValueHint`, `XDebuggerTooltipPopup`,
//! `XDebuggerTreePopup` and `XDebuggerTreeInlayPopup`):
//!
//! - the mouse rests on an expression (700 ms, `DEFAULT_VALUE_TOOLTIP_DELAY`;
//!   the product finds it, [`debugger::DebugAdapterProvider::expression_at`]):
//!   the expression gets `EVALUATED_EXPRESSION_ATTRIBUTES` and its value
//!   shows under the char under the mouse, `{dict: 2} {'count': 4}` after
//!   the value's icon (with the tree's chevron when it has children), then
//!   a separator and Documentation and Problems (which shows the editor's
//!   own popup there). "Evaluating…" shows when the value takes over
//!   200 ms. Nothing to evaluate there: the editor's own popup shows
//!   instead (the editors show none of their own while the program is
//!   suspended, `EditorMouseHoverPopupControl.disablePopups`);
//! - a click on a value with children, or Quick Evaluate Expression: the
//!   value's tree in a popup at the same place (its root expanded and
//!   selected, Set Value under it), which takes the focus;
//! - a click on a value shown after a line: its tree in a popup under it,
//!   with Add as Inline Watch too.
//!
//! A hint goes when another one shows, at a click outside it, when the
//! caret moves, the text changes or the editor scrolls, on Escape, and
//! when the program moves on.

use std::ops::Range;
use std::time::Duration;

use debugger::values::{ValueKind, ValueRole, type_text, value_text};
use debugger::{DebugAdapterProvider, EvaluateContext, Session};
use editor::{DecorationId, Editor};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EntityId, FocusHandle, Focusable as _,
    InteractiveElement, IntoElement, ParentElement as _, Pixels, Point, Styled as _, Task, Window,
    anchored, deferred, div, point, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::{TextRole, UiRole};
use ui::icons::Icon;
use ui::theme::ThemeExt as _;

use crate::DebuggerViewEvent;
use crate::icons;
use crate::variables::{PopupRoot, VariablesView};
use crate::view::DebuggerView;
use crate::watches::Eval;

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Quick Evaluate Expression.
    QuickEvaluate { line: Option<u32>, column: Option<u32> } => "debug.quick-evaluate", "Quick Evaluate Expression",
        "Shows the value of the expression at the caret (or at a line and a column from 1 of the editor) in a popup with its tree, while the program is suspended.",
        r#"{"line": 7, "column": 9}"#;
    /// Expand the value hint under the mouse into its tree.
    ExpandValueHint => "debug.value-hint.expand", "Expand Value",
        "Shows the tree of the value shown under the mouse (a click on the hint).";
    /// The hint's Documentation and Problems.
    ValueHintDocumentation => "debug.value-hint.documentation", "Documentation and Problems",
        "Closes the value shown under the mouse and shows the editor's documentation and problems there instead.";
    /// A click on a value shown after a line.
    OpenInlineValue { line: u32, name: Option<String> } => "debug.inline-value.open", "Open Inline Value",
        "Shows the tree of a value shown after a line (from 1) of the editor in a popup under it, as a click on it does; `name` picks one of several.",
        r#"{"line": 5, "name": "summary"}"#;
    /// Close the value popup.
    CloseValuePopup => "debug.value-popup.close", "Close",
        "Closes the popup with a value's tree, or the value shown under the mouse.";
    /// Add as Inline Watch.
    AddInlineWatch => "debug.value-popup.add-inline-watch", "Add as Inline Watch",
        "Adds the selected value of the popup of a value shown after a line to the watches.";
}

/// The key context of a popup with a value's tree.
pub const CONTEXT: &str = "DebugValuePopup";

/// How long the mouse rests on an expression before its value shows
/// (`XDebuggerSettingsManager.DEFAULT_VALUE_TOOLTIP_DELAY`).
pub(crate) const LOOKUP_DELAY: Duration = Duration::from_millis(700);
/// "Evaluating…" shows when the value takes longer (`XValueHint
/// .evaluateAndShowHint`).
const EVALUATING_DELAY: Duration = Duration::from_millis(200);

/// The text of a value being evaluated (`XDebuggerBundle`).
const EVALUATING: &str = "Evaluating\u{2026}";

/// The tooltip (`XDebuggerTooltipPopup`, measured on the reference): the
/// content's insets, the room after the text (`emptyRight(20)` and the
/// label's own), the separator's height, the toolbar button's insets
/// around its 16 icon, the popup's corner radius.
const TIP_TOP: f32 = 10.0;
/// The tooltip's height inside its border (measured: insets of 10 around a
/// row of 22; the separator and the button reach into the insets).
const TIP_HEIGHT: f32 = 42.0;
const TIP_LEFT: f32 = 11.0;
const TIP_TEXT_RIGHT: f32 = 24.0;
const TIP_SEPARATOR: f32 = 24.0;
const TIP_BUTTON_LEFT: f32 = 8.0;
const TIP_BUTTON_RIGHT: f32 = 9.5;
const TIP_ICON_GAP: f32 = 4.5;
const RADIUS: f32 = 8.0;
/// The tree popup (`XDebuggerTreePopup.updateDebugPopupBounds`): the
/// margins added to the tree's size, the toolbar's height (its
/// placeholder of 27), an action's insets (`createCustomToolbarComponent`).
const TREE_MARGIN: f32 = 30.0;
const TOOLBAR_HEIGHT: f32 = 27.0;
const ACTION_LEFT: f32 = 10.0;
const ACTION_RIGHT: f32 = 4.0;
const SHORTCUT_LEFT: f32 = 4.0;
const SHORTCUT_RIGHT: f32 = 12.0;

/// Where a hint shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Anchor {
    /// Under the char at this offset (the mouse's).
    Char(usize),
    /// Under a value shown after a line, from where its inlay starts.
    AfterText(DecorationId),
}

/// What a hint shows.
pub(crate) enum Content {
    Evaluating {
        visible: bool,
    },
    /// The value, collapsed.
    Value(PopupRoot),
    /// The value's tree.
    Tree(Entity<VariablesView>),
}

/// The hint shown.
pub(crate) struct Shown {
    pub editor: Entity<Editor>,
    /// The chars of the expression; empty for a value after a line.
    pub range: Range<usize>,
    pub expression: String,
    pub anchor: Anchor,
    pub content: Content,
    /// From a value after a line (Add as Inline Watch).
    pub inline: bool,
    /// Where the click put the caret: the popup stays when the editor
    /// reports it.
    caret: Option<usize>,
    _evaluation: Option<Task<()>>,
}

/// The value hints of a window.
#[derive(Default)]
pub(crate) struct ValueHints {
    /// The editor and char offset under the mouse.
    under: Option<(EntityId, usize)>,
    delay: Option<Task<()>>,
    request: Option<Task<()>>,
    /// Counts the hints asked for; answers for an older one are dropped.
    generation: u64,
    pub shown: Option<Shown>,
    /// The focus of a closed popup's tree and the editor's: the editor
    /// gets the focus back when the tree had it.
    refocus: Option<(FocusHandle, FocusHandle)>,
}

impl DebuggerView {
    /// The tree of the open popup.
    pub(crate) fn value_popup_tree(&self) -> Option<Entity<VariablesView>> {
        match &self.hints.shown.as_ref()?.content {
            Content::Tree(tree) => Some(tree.clone()),
            _ => None,
        }
    }

    /// The suspended session and frame values are evaluated in.
    fn suspended_frame(&self, cx: &App) -> Option<(Session, i64)> {
        let point = self.execution.as_ref()?;
        let tab = self.tool_window.read(cx).selected_tab()?;
        (tab.session.id() == point.session).then(|| (tab.session.clone(), point.frame))
    }

    fn provider(&self) -> Option<std::sync::Arc<dyn DebugAdapterProvider>> {
        self.debugger.as_ref().map(|d| d.provider().clone())
    }

    /// The mouse moved over an editor's text (`offset` `None`: off the
    /// text): after the lookup delay the value of the expression there
    /// shows (`ValueLookupManager.mouseMoved`).
    pub(crate) fn value_hovered(
        &mut self,
        editor: &Entity<Editor>,
        offset: Option<usize>,
        modifiers: gpui::Modifiers,
        cx: &mut Context<Self>,
    ) {
        let id = editor.entity_id();
        let target = offset.filter(|_| modifiers == gpui::Modifiers::default());
        let target = target.map(|o| (id, o));
        if self.hints.under == target {
            return;
        }
        self.hints.under = target;
        self.hints.delay = None;
        let Some((_, offset)) = target else {
            return;
        };
        if self.execution.is_none() {
            return;
        }
        if let Some(shown) = &self.hints.shown {
            // The popup of a value after a line stays; over the
            // expression shown, the hint stays.
            if shown.inline || (shown.editor.entity_id() == id && shown.range.contains(&offset)) {
                return;
            }
        }
        let editor = editor.clone();
        self.hints.delay = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LOOKUP_DELAY).await;
            let _ = this.update(cx, |this, cx| this.lookup(&editor, offset, false, cx));
        }));
    }

    /// The expression at `offset` of an editor, off the UI thread; then
    /// its value: in a hint (`tree` false) or in a tree popup.
    fn lookup(
        &mut self,
        editor: &Entity<Editor>,
        offset: usize,
        tree: bool,
        cx: &mut Context<Self>,
    ) {
        self.hints.delay = None;
        let (Some(provider), Some(_)) = (self.provider(), self.suspended_frame(cx)) else {
            return;
        };
        let e = editor.read(cx);
        let Some(path) = e.path().cloned() else {
            return;
        };
        let text = e.buffer().text();
        self.hints.generation += 1;
        let generation = self.hints.generation;
        let editor = editor.clone();
        let find = cx.background_spawn(async move {
            let byte = byte_of(&text, offset)?;
            let range = provider.expression_at(&path, &text, byte)?;
            let expression = text.get(range.clone())?.to_owned();
            let chars = char_of(&text, range.start)..char_of(&text, range.end);
            Some((chars, expression))
        });
        self.hints.request = Some(cx.spawn(async move |this, cx| {
            let found = find.await;
            let _ = this.update(cx, |this, cx| {
                if this.hints.generation != generation {
                    return;
                }
                this.hints.request = None;
                match found {
                    Some((range, expression)) => {
                        this.show_hint(&editor, range, expression, offset, tree, cx)
                    }
                    None if !tree => {
                        // Nothing to evaluate: the editor's own popup.
                        this.close_value_hint(cx);
                        cx.emit(DebuggerViewEvent::ShowInfoTooltip {
                            editor: editor.clone(),
                            offset,
                        });
                    }
                    None => {}
                }
            });
        }));
    }

    /// Evaluates an expression of an editor and shows its value.
    fn show_hint(
        &mut self,
        editor: &Entity<Editor>,
        range: Range<usize>,
        expression: String,
        offset: usize,
        tree: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(shown) = &self.hints.shown
            && shown.editor == *editor
            && shown.range == range
            && !tree
        {
            return;
        }
        let Some((session, frame)) = self.suspended_frame(cx) else {
            return;
        };
        self.close_value_hint(cx);
        let provider = self.provider();
        let generation = self.hints.generation;
        let task = crate::watches::evaluate(
            session.clone(),
            provider.clone(),
            frame,
            expression.clone(),
            EvaluateContext::Hover,
        );
        let evaluation = cx.spawn(async move |this, cx| {
            let evaluating = cx.background_executor().timer(EVALUATING_DELAY);
            let result = futures::future::select(Box::pin(task), Box::pin(evaluating)).await;
            let eval = match result {
                futures::future::Either::Left((eval, _)) => eval,
                futures::future::Either::Right((_, task)) => {
                    let _ = this.update(cx, |this, cx| {
                        if let Some(Shown {
                            content: Content::Evaluating { visible },
                            ..
                        }) = &mut this.hints.shown
                        {
                            *visible = true;
                            cx.notify();
                        }
                    });
                    task.await
                }
            };
            let _ = this.update(cx, |this, cx| {
                if this.hints.generation != generation || this.hints.shown.is_none() {
                    return;
                }
                this.evaluated(eval, offset, tree, cx);
            });
        });
        let role = if self.on_execution_line(editor, range.start, cx) {
            TextRole::DebuggerEvaluatedExpressionExecutionLine
        } else {
            TextRole::DebuggerEvaluatedExpression
        };
        editor.update(cx, |e, cx| {
            e.set_range_background(Some((range.clone(), role)), cx)
        });
        self.hints.shown = Some(Shown {
            editor: editor.clone(),
            range,
            expression,
            anchor: Anchor::Char(offset),
            content: Content::Evaluating { visible: false },
            inline: false,
            caret: None,
            _evaluation: Some(evaluation),
        });
        cx.notify();
    }

    /// The value came: a hint, or its tree; an error closes the hint and
    /// shows the editor's own popup.
    fn evaluated(&mut self, eval: Eval, offset: usize, tree: bool, cx: &mut Context<Self>) {
        let provider = self.provider();
        let Some(shown) = &mut self.hints.shown else {
            return;
        };
        let (variable, shape) = match eval {
            Eval::Value { variable, shape } => (*variable, shape),
            _ => {
                let editor = shown.editor.clone();
                self.close_value_hint(cx);
                if !tree {
                    cx.emit(DebuggerViewEvent::ShowInfoTooltip { editor, offset });
                }
                return;
            }
        };
        let kind = match provider.map(|p| p.present_value(&variable).role) {
            Some(ValueRole::Value(kind)) => kind,
            _ if variable.reference > 0 => ValueKind::Object,
            _ => ValueKind::Primitive,
        };
        let root = PopupRoot {
            name: shown.expression.clone(),
            variable,
            shape,
            kind,
            parent: 0,
        };
        // A value without children is never a tree (`XValueHint`).
        if tree && root.variable.reference > 0 {
            self.hints.refocus = None;
            let view = self.popup_tree(root, cx);
            if let Some(shown) = &mut self.hints.shown {
                shown.content = Content::Tree(view);
            }
        } else if let Some(shown) = &mut self.hints.shown {
            shown.content = Content::Value(root);
        }
        cx.notify();
    }

    /// A tree of `root` for a popup, in the suspended frame.
    fn popup_tree(&self, root: PopupRoot, cx: &mut Context<Self>) -> Entity<VariablesView> {
        let provider = self.provider();
        let (session, frame) = self
            .suspended_frame(cx)
            .expect("a popup's tree needs a suspended frame");
        let location = self.execution.as_ref().map(|p| (p.path.clone(), p.line));
        let highlighters = self.highlighters.clone();
        cx.new(|cx| {
            let mut view = VariablesView::for_popup(session, provider, frame, location, root, cx);
            view.highlighters = highlighters;
            view
        })
    }

    fn on_execution_line(&self, editor: &Entity<Editor>, offset: usize, cx: &App) -> bool {
        let Some(point) = self.execution.as_ref().filter(|p| p.top) else {
            return false;
        };
        let e = editor.read(cx);
        e.path().is_some_and(|p| *p == point.path)
            && e.buffer().char_to_line(offset.min(e.buffer().len_chars())) as u32 + 1 == point.line
    }

    /// Closes the hint or the popup and takes the expression's background
    /// away.
    pub(crate) fn close_value_hint(&mut self, cx: &mut Context<Self>) {
        self.hints.generation += 1;
        self.hints.request = None;
        let Some(shown) = self.hints.shown.take() else {
            return;
        };
        if let Content::Tree(tree) = &shown.content {
            self.hints.refocus = Some((tree.focus_handle(cx), shown.editor.focus_handle(cx)));
        }
        shown
            .editor
            .update(cx, |e, cx| e.set_range_background(None, cx));
        cx.notify();
    }

    /// Something happened in an editor that closes its hint: the caret
    /// moved, the text changed, the view scrolled.
    pub(crate) fn value_hint_editor_changed(
        &mut self,
        editor: &Entity<Editor>,
        caret_moved: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(shown) = &self.hints.shown else {
            return;
        };
        if shown.editor != *editor {
            return;
        }
        if caret_moved && shown.caret.is_some() {
            let caret = editor.read(cx).selections().primary().head;
            if shown.caret == Some(caret) {
                return;
            }
        }
        self.close_value_hint(cx);
    }

    // ---- commands -----------------------------------------------------------

    /// Quick Evaluate Expression: the tree of the selection's text, or of
    /// the expression at the caret (or at a line and a column).
    pub(crate) fn quick_evaluate(
        &mut self,
        a: &QuickEvaluate,
        editor: Option<&Entity<Editor>>,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = editor.cloned() else {
            return;
        };
        if self.suspended_frame(cx).is_none() {
            return;
        }
        let e = editor.read(cx);
        let buffer = e.buffer();
        let offset = match a.line {
            Some(line) => {
                let line = (line.max(1) as usize - 1).min(buffer.len_lines().saturating_sub(1));
                let column = a.column.unwrap_or(1).max(1) as usize - 1;
                (buffer.line_start(line) + column).min(buffer.line_end(line))
            }
            None => e.selections().primary().head,
        };
        let selection = e.selections().primary().range();
        if a.line.is_none() && !selection.is_empty() {
            let expression = buffer.slice(selection.clone());
            self.hints.generation += 1;
            let start = selection.start;
            self.show_hint(&editor, selection, expression, start, true, cx);
            return;
        }
        self.lookup(&editor, offset, true, cx);
    }

    /// A click on the hint: its tree (`AbstractValueHint
    /// .createExpandableHintComponent`).
    pub(crate) fn expand_value_hint(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = match &self.hints.shown {
            Some(Shown {
                content: Content::Value(root),
                ..
            }) if root.variable.reference > 0 => root.clone(),
            _ => return,
        };
        let view = self.popup_tree(root, cx);
        window.focus(&view.focus_handle(cx));
        if let Some(shown) = &mut self.hints.shown {
            shown.content = Content::Tree(view);
        }
        cx.notify();
    }

    /// Documentation and Problems: the editor's own popup instead.
    pub(crate) fn value_hint_documentation(&mut self, cx: &mut Context<Self>) {
        let Some(shown) = &self.hints.shown else {
            return;
        };
        let editor = shown.editor.clone();
        let offset = match shown.anchor {
            Anchor::Char(offset) => offset,
            Anchor::AfterText(_) => return,
        };
        self.close_value_hint(cx);
        cx.emit(DebuggerViewEvent::ShowInfoTooltip { editor, offset });
    }

    /// A click on a value after a line: the caret goes to the line's end
    /// (where the click is) and the value's tree shows under it
    /// (`InlineDebugRenderer.onClick`).
    pub(crate) fn inline_value_clicked(
        &mut self,
        editor: &Entity<Editor>,
        id: DecorationId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(value) = self.inline_value_of(editor.entity_id(), id).cloned() else {
            return;
        };
        if self.suspended_frame(cx).is_none() {
            return;
        }
        if let Some(shown) = &self.hints.shown
            && shown.anchor == Anchor::AfterText(id)
        {
            return;
        }
        self.close_value_hint(cx);
        self.hints.refocus = None;
        let caret = editor.update(cx, |e, cx| {
            let line = (value.line as usize - 1).min(e.buffer().len_lines().saturating_sub(1));
            let end = e.buffer().line_end(line);
            e.set_selections(text::SelectionSet::caret(end), cx);
            end
        });
        let root = PopupRoot {
            name: value.name.clone(),
            variable: value.variable.clone(),
            shape: value.shape.clone(),
            kind: value.kind,
            parent: value.parent,
        };
        let view = self.popup_tree(root, cx);
        window.focus(&view.focus_handle(cx));
        self.hints.shown = Some(Shown {
            editor: editor.clone(),
            range: 0..0,
            expression: value.name,
            anchor: Anchor::AfterText(id),
            content: Content::Tree(view),
            inline: true,
            caret: Some(caret),
            _evaluation: None,
        });
        cx.notify();
    }

    /// `debug.inline-value.open`: the value after a line of the active
    /// editor, as a click on it.
    pub(crate) fn open_inline_value(
        &mut self,
        a: &OpenInlineValue,
        editor: Option<&Entity<Editor>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = editor.cloned() else {
            return;
        };
        let Some(id) = self.inline_value_id(editor.entity_id(), a.line, a.name.as_deref()) else {
            return;
        };
        self.inline_value_clicked(&editor, id, window, cx);
    }

    /// Add as Inline Watch: the selected value's expression goes to the
    /// watches (there are no watches after the lines).
    pub(crate) fn add_inline_watch(&mut self, cx: &mut Context<Self>) {
        let Some(tree) = self.value_popup_tree() else {
            return;
        };
        if !self.hints.shown.as_ref().is_some_and(|s| s.inline) {
            return;
        }
        let expression = tree.read(cx).selected_value().map(|v| {
            v.variable
                .evaluate_name
                .clone()
                .unwrap_or_else(|| v.name.clone())
        });
        self.close_value_hint(cx);
        if let (Some(expression), Some(variables)) =
            (expression, self.tool_window.read(cx).selected_variables())
        {
            variables.update(cx, |v, cx| v.add_watch_expression(&expression, cx));
        }
    }

    // ---- drawing ------------------------------------------------------------

    /// The hint or the popup over the active editor.
    pub(crate) fn render_value_hint(
        &mut self,
        active: Option<&Entity<Editor>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if let Some((tree, editor)) = self.hints.refocus.take()
            && window.focused(cx).is_none_or(|f| f == tree)
        {
            window.focus(&editor);
        }
        let shown = self.hints.shown.as_ref()?;
        if active != Some(&shown.editor) {
            return None;
        }
        let origin = self.hint_origin(shown, cx)?;
        let theme = cx.theme();
        let frame = div()
            .id("debug-value-hint")
            .debug_selector(|| "debug-value-hint".into())
            .occlude()
            .bg(theme.color(UiRole::SurfaceRaised))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .rounded(px(RADIUS))
            .shadow_md()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_value_hint(cx)));
        let content = match &shown.content {
            Content::Evaluating { visible: false } => return None,
            Content::Evaluating { visible: true } => frame
                .child(
                    div()
                        .px(px(TIP_LEFT))
                        .py(px(TIP_TOP))
                        .text_color(theme.color(UiRole::Text))
                        .child(EVALUATING),
                )
                .into_any_element(),
            Content::Value(root) => self.render_value_tooltip(frame, root, cx),
            Content::Tree(tree) => {
                let tree = tree.clone();
                self.render_tree_popup(frame, &tree, shown.inline, window, cx)
            }
        };
        Some(
            deferred(
                anchored()
                    .position(origin)
                    .snap_to_window_with_margin(px(4.0))
                    .child(content),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// Where the hint's top left corner goes: under the char under the
    /// mouse, or under a value after a line from where its inlay starts.
    fn hint_origin(&self, shown: &Shown, cx: &App) -> Option<Point<Pixels>> {
        let e = shown.editor.read(cx);
        match shown.anchor {
            Anchor::Char(offset) => {
                let b = e.bounds_for_range(offset..offset)?;
                Some(point(b.left(), b.bottom()))
            }
            Anchor::AfterText(id) => {
                let b = e.after_text_bounds(id)?;
                let char_width = e.bounds_for_range(0..0)?.size.width;
                Some(point(b.left() - char_width * 1.5, b.bottom()))
            }
        }
    }

    /// The collapsed value (`XValueHint` over `XDebuggerTooltipPopup`):
    /// its chevron when it has children, its icon, `{type} value`, a
    /// separator and Documentation and Problems.
    fn render_value_tooltip(
        &self,
        frame: gpui::Stateful<gpui::Div>,
        root: &PopupRoot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let expandable = root.variable.reference > 0;
        let icon = match root.kind {
            ValueKind::Primitive => icons::PRIMITIVE,
            ValueKind::Array => icons::ARRAY,
            ValueKind::Object => icons::OBJECT,
        };
        let type_part = type_text(root.variable.type_name.as_deref(), root.shape.as_deref());
        let value = value_text(&root.variable.value);
        let text = div()
            .id("debug-value-hint-text")
            .flex()
            .flex_row()
            .items_center()
            .pl(px(TIP_LEFT))
            .pr(px(TIP_TEXT_RIGHT))
            .whitespace_nowrap()
            .when(expandable, |d| {
                d.cursor_pointer()
                    .child(
                        div()
                            .flex_none()
                            .size(px(16.0))
                            .child(ui::components::tree_arrow(false, cx)),
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.expand_value_hint(window, cx)))
            })
            .child(
                div()
                    .flex_none()
                    .size(px(16.0))
                    .child(Icon::new(icon).size(px(16.0))),
            )
            .child(div().w(px(TIP_ICON_GAP)))
            .when_some(type_part, |d, t| {
                d.child(
                    div()
                        .text_color(theme.color(UiRole::DebuggerType))
                        .child(format!("{{{t}}}\u{a0}")),
                )
            })
            .child(div().text_color(theme.color(UiRole::Text)).child(value));
        let button = div()
            .id("debug-value-hint-documentation")
            .debug_selector(|| "debug-value-hint-documentation".into())
            .flex_none()
            .ml(px(TIP_BUTTON_LEFT - 3.0))
            .mr(px(TIP_BUTTON_RIGHT - 3.0))
            .p(px(3.0))
            .rounded(px(4.0))
            .cursor_pointer()
            .hover(|s| s.bg(theme.color(UiRole::Hover)))
            .child(Icon::new(icons::DOCUMENTATION_AND_PROBLEMS).size(px(16.0)))
            .tooltip(ui::components::Tooltip::text(
                "Documentation and Problems",
                Some("\u{2318}F1".into()),
            ))
            .on_click(cx.listener(|this, _, _, cx| this.value_hint_documentation(cx)));
        frame
            .flex()
            .flex_row()
            .items_center()
            .h(px(TIP_HEIGHT + 2.0))
            .child(text)
            .child(
                div()
                    .flex_none()
                    .w(px(1.0))
                    .h(px(TIP_SEPARATOR))
                    .bg(theme.color(UiRole::PopupBorder)),
            )
            .child(button)
            .into_any_element()
    }

    /// The tree popup (`XDebuggerTreePopup`): the tree, sized to its rows
    /// plus the margins (at most half the window), and the toolbar's
    /// links: Set Value (F2), and for a value after a line Add as Inline
    /// Watch.
    fn render_tree_popup(
        &self,
        frame: gpui::Stateful<gpui::Div>,
        tree: &Entity<VariablesView>,
        inline: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (rows, tree_width, editing) = tree.update(cx, |t, cx| {
            (
                t.row_count(cx),
                t.preferred_width(window, cx),
                t.editing.is_some(),
            )
        });
        let theme = cx.theme();
        let row_h = theme.metric(themes::Metric::RowHeight);
        let link = theme.color(UiRole::TextLink);
        let shortcut = theme.color(UiRole::Shortcut);
        let action = |id: &'static str, label: &'static str, keys: Option<&'static str>| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .flex()
                .flex_row()
                .items_center()
                .flex_none()
                .pl(px(ACTION_LEFT))
                .child(
                    div()
                        .cursor_pointer()
                        .text_color(link)
                        .pr(px(ACTION_RIGHT))
                        .child(label),
                )
                .when_some(keys, |d, k| {
                    d.child(
                        div()
                            .pl(px(SHORTCUT_LEFT))
                            .pr(px(SHORTCUT_RIGHT))
                            .text_color(shortcut)
                            .child(k),
                    )
                })
        };
        let actions: Vec<AnyElement> = if editing {
            vec![
                action("debug-value-popup-set", "Set", Some("\u{23ce}"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.with_variables(cx, |t, cx| t.commit_edit(window, cx))
                    }))
                    .into_any_element(),
                action("debug-value-popup-cancel", "Cancel", Some("\u{238b}"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.with_variables(cx, |t, cx| t.cancel_edit(window, cx))
                    }))
                    .into_any_element(),
            ]
        } else {
            let mut actions = vec![
                action("debug-value-popup-set-value", "Set value", Some("F2"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.with_variables(cx, |t, cx| t.set_value(None, window, cx))
                    }))
                    .into_any_element(),
            ];
            if inline {
                actions.push(
                    action(
                        "debug-value-popup-inline-watch",
                        "Add as inline watch",
                        None,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.add_inline_watch(cx)))
                    .into_any_element(),
                );
            }
            actions
        };
        let toolbar_width: Pixels = px(if inline { 330.0 } else { 120.0 });
        let viewport = window.viewport_size();
        let width = (tree_width.max(toolbar_width) + px(TREE_MARGIN)).min(viewport.width / 2.0);
        let tree_height =
            px(crate::variables::POPUP_TREE_TOP) + row_h * rows as f32 + px(TREE_MARGIN);
        let height = tree_height.min(viewport.height / 2.0 - px(TOOLBAR_HEIGHT));
        frame
            .key_context(CONTEXT)
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(div().w(width).h(height).child(tree.clone()))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(TOOLBAR_HEIGHT))
                    .w(width)
                    .children(actions),
            )
            .into_any_element()
    }

    /// The hint as `debug.state` shows it.
    pub(crate) fn value_hint_json(&self, cx: &App) -> Json {
        let Some(shown) = &self.hints.shown else {
            return Json::Null;
        };
        let mut out = json!({
            "expression": shown.expression,
            "range": [shown.range.start, shown.range.end],
            "inline": shown.inline,
        });
        match &shown.content {
            Content::Evaluating { visible } => {
                out["kind"] = json!("evaluating");
                out["visible"] = json!(visible);
            }
            Content::Value(root) => {
                out["kind"] = json!("value");
                out["expandable"] = json!(root.variable.reference > 0);
                out["text"] = json!(match type_text(
                    root.variable.type_name.as_deref(),
                    root.shape.as_deref()
                ) {
                    Some(t) => format!("{{{t}}} {}", value_text(&root.variable.value)),
                    None => value_text(&root.variable.value),
                });
            }
            Content::Tree(tree) => {
                out["kind"] = json!("tree");
                out["tree"] = tree.read(cx).state_json(cx);
                out["focused"] = Json::Null;
            }
        }
        out
    }
}

/// The byte offset of char `offset` of `text`.
fn byte_of(text: &str, offset: usize) -> Option<usize> {
    text.char_indices().nth(offset).map(|(b, _)| b)
}

/// The char offset of byte `byte` of `text`.
fn char_of(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].chars().count()
}

/// The commands of the value hints, at the window's root.
pub(crate) fn forward_actions<E: InteractiveElement>(
    el: E,
    view: Entity<DebuggerView>,
    active: Option<Entity<Editor>>,
) -> E {
    let v = view.clone();
    let editor = active.clone();
    let el = el.on_action(move |a: &QuickEvaluate, _, cx| {
        v.update(cx, |v, cx| v.quick_evaluate(a, editor.as_ref(), cx))
    });
    let v = view.clone();
    let el = el.on_action(move |_: &ExpandValueHint, window, cx| {
        v.update(cx, |v, cx| v.expand_value_hint(window, cx))
    });
    let v = view.clone();
    let el = el.on_action(move |_: &ValueHintDocumentation, _, cx| {
        v.update(cx, |v, cx| v.value_hint_documentation(cx))
    });
    let v = view.clone();
    let editor = active;
    let el = el.on_action(move |a: &OpenInlineValue, window, cx| {
        v.update(cx, |v, cx| {
            v.open_inline_value(a, editor.as_ref(), window, cx)
        })
    });
    let v = view.clone();
    let el = el
        .on_action(move |_: &CloseValuePopup, _, cx| v.update(cx, |v, cx| v.close_value_hint(cx)));
    let v = view;
    el.on_action(move |_: &AddInlineWatch, _, cx| v.update(cx, |v, cx| v.add_inline_watch(cx)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_convert_between_chars_and_bytes() {
        let text = "é = 1\nsummary";
        assert_eq!(byte_of(text, 0), Some(0));
        assert_eq!(byte_of(text, 1), Some(2));
        assert_eq!(char_of(text, 2), 1);
        assert_eq!(char_of(text, text.len()), text.chars().count());
        assert_eq!(byte_of(text, 100), None);
    }
}
