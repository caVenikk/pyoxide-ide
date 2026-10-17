//! Line breakpoints in an editor's gutter, as the reference's New UI puts
//! them over the line numbers (`BreakpointPromoterEditorListener`,
//! `XLineBreakpointManager`, `BreakpointGutterIconRenderer`,
//! `XBreakpointVisualRepresentation`):
//!
//! - the mouse over the number of a line a breakpoint can be put on shows
//!   the breakpoint icon translucent in its place, with the hand and the
//!   tooltip "Add Breakpoint";
//! - a click there adds a breakpoint (Alt: a temporary one; Shift: a
//!   non-suspending one that logs, with its properties popup);
//! - on a breakpoint, a click removes it, Alt+click or a middle click
//!   enables or disables it, a right click opens its properties popup; its
//!   tooltip describes it;
//! - a breakpoint's icon replaces the line number, its line gets the
//!   breakpoint background (not while it is disabled or does not suspend)
//!   and a mark on the error stripe;
//! - breakpoints follow the edits of their lines and go with their lines.
//!
//! Which lines take a breakpoint is the product's (`stoppable_lines`),
//! computed off the UI thread for each version of the text.

use std::path::Path;
use std::rc::Rc;

use debugger::{BreakpointId, SuspendPolicy};
use editor::decorations::layer;
use editor::{Decoration, DecorationId, Editor, EditorEvent};
use gpui::{
    AnyElement, AppContext as _, Context, Entity, EntityId, Modifiers, MouseButton,
    ParentElement as _, Styled as _, Subscription, Task, Window, anchored, deferred, div, point,
    prelude::*, px,
};
use themes::UiRole;
use ui::theme::ThemeExt as _;

use crate::breakpoints::presentation;
use crate::icons;
use crate::view::{DebuggerView, line_defaults};

/// The decorations' group in the editor.
pub const GROUP: &str = "breakpoints";

/// The breakpoints of an editor tab.
pub struct Gutter {
    pub(crate) editor: Entity<Editor>,
    /// The decoration of each breakpoint shown.
    pub(crate) marks: Vec<(BreakpointId, DecorationId)>,
    /// The stoppable lines of a version of the text (by line from 1);
    /// `None` inside for a file of another type.
    stoppable: Option<(u64, Option<Rc<Vec<bool>>>)>,
    /// The version whose stoppable lines are being computed.
    computing: Option<(u64, Task<()>)>,
    /// The line (zero-based) whose number the mouse is over.
    hovered: Option<usize>,
    _events: Subscription,
}

impl Gutter {
    pub(crate) fn new(editor: Entity<Editor>, events: Subscription) -> Self {
        Self {
            editor,
            marks: Vec::new(),
            stoppable: None,
            computing: None,
            hovered: None,
            _events: events,
        }
    }
}

/// The tooltip of a line number: a breakpoint's description, or "Add
/// Breakpoint" over the hover icon; shown after the tooltip delay.
pub struct Tooltip {
    pub(crate) editor: Entity<Editor>,
    pub(crate) line: usize,
    pub(crate) lines: Vec<String>,
    pub(crate) shown: bool,
    _timer: Task<()>,
}

impl DebuggerView {
    pub(crate) fn editor_event(
        &mut self,
        editor: &Entity<Editor>,
        event: &EditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            EditorEvent::Edited(_) | EditorEvent::Reloaded => self.follow_edits(editor, cx),
            EditorEvent::GutterHovered { line } => {
                if let Some(g) = self.gutters.get_mut(&editor.entity_id()) {
                    g.hovered = *line;
                }
                self.update_hover(editor.entity_id(), cx);
            }
            EditorEvent::GutterClicked {
                line,
                button,
                modifiers,
                ..
            } => self.gutter_clicked(editor, *line, *button, *modifiers, window, cx),
            _ => {}
        }
    }

    /// The editor moved the decorations through an edit: the breakpoints
    /// take their lines, and the ones whose lines are gone are removed.
    fn follow_edits(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let id = editor.entity_id();
        let (Some(g), Some(debugger)) = (self.gutters.get(&id), &self.debugger) else {
            return;
        };
        let e = editor.read(cx);
        let Some(path) = e.path().cloned() else {
            return;
        };
        let breakpoints = debugger.breakpoints();
        let moves: Vec<(BreakpointId, Option<u32>)> = g
            .marks
            .iter()
            .filter_map(|(bp, deco)| {
                let now = e.decoration_lines(*deco).map(|l| l.start as u32 + 1);
                let was = breakpoints.get(*bp)?.line;
                (now != Some(was)).then_some((*bp, now))
            })
            .collect();
        if !moves.is_empty() {
            debugger.set_lines(&path, &moves);
            self.refresh_editor(id, cx);
        }
        self.update_hover(id, cx);
    }

    /// A press on a line number.
    fn gutter_clicked(
        &mut self,
        editor: &Entity<Editor>,
        line: usize,
        button: MouseButton,
        modifiers: Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(debugger) = self.debugger.clone() else {
            return;
        };
        let Some(path) = editor.read(cx).path().cloned() else {
            return;
        };
        self.tooltip = None;
        // Control+click is the context click on macOS.
        let button =
            if cfg!(target_os = "macos") && modifiers.control && button == MouseButton::Left {
                MouseButton::Right
            } else {
                button
            };
        let existing = debugger.breakpoints().at(&path, line as u32 + 1).cloned();
        match (button, existing) {
            (MouseButton::Left, Some(b)) => {
                if modifiers.alt {
                    self.toggle_enabled_at(&path, b.line, cx);
                } else if !modifiers.modified() {
                    // A click with other modifiers does not remove
                    // (`RemoveBreakpointGutterIconAction`).
                    debugger.remove_breakpoint(b.id);
                    self.refresh_path(&path, cx);
                }
            }
            (MouseButton::Left, None) => {
                let logging = modifiers.shift && !modifiers.alt;
                self.toggle_from_gutter(editor, line, modifiers.alt, logging, window, cx);
            }
            (MouseButton::Middle, Some(b)) => self.toggle_enabled_at(&path, b.line, cx),
            (MouseButton::Right, Some(b)) => self.open_popup(editor, b.id, window, cx),
            _ => {}
        }
        self.update_hover(editor.entity_id(), cx);
        cx.notify();
    }

    // ---- stoppable lines --------------------------------------------------------

    /// Whether a line (from 1) of an editor takes a breakpoint, from the
    /// stoppable lines of its current text (computed now when they are not
    /// known for it).
    pub(crate) fn can_put(&mut self, id: EntityId, line: u32, cx: &mut Context<Self>) -> bool {
        let Some(provider) = self.debugger.as_ref().map(|d| d.provider().clone()) else {
            return false;
        };
        let Some(g) = self.gutters.get_mut(&id) else {
            return false;
        };
        let e = g.editor.read(cx);
        let version = e.buffer().version();
        let lines = match &g.stoppable {
            Some((v, lines)) if *v == version => lines.clone(),
            _ => {
                let Some(path) = e.path().cloned() else {
                    return false;
                };
                let lines = provider.stoppable_lines(&path, &e.text()).map(Rc::new);
                g.stoppable = Some((version, lines.clone()));
                lines
            }
        };
        lines.is_some_and(|l| line >= 1 && l.get(line as usize - 1).copied().unwrap_or(false))
    }

    /// The stoppable lines of the editor's current text, when known; else
    /// they are computed off the UI thread and the hover is updated then.
    fn stoppable_now(
        &mut self,
        id: EntityId,
        cx: &mut Context<Self>,
    ) -> Option<Option<Rc<Vec<bool>>>> {
        let provider = self.debugger.as_ref()?.provider().clone();
        let g = self.gutters.get_mut(&id)?;
        let e = g.editor.read(cx);
        let version = e.buffer().version();
        if let Some((v, lines)) = &g.stoppable
            && *v == version
        {
            return Some(lines.clone());
        }
        if g.computing.as_ref().is_some_and(|(v, _)| *v == version) {
            return None;
        }
        let path = e.path().cloned()?;
        let rope = e.buffer().rope().clone();
        let compute =
            cx.background_spawn(async move { provider.stoppable_lines(&path, &rope.to_string()) });
        let task = cx.spawn(async move |this, cx| {
            let lines = compute.await.map(Rc::new);
            let _ = this.update(cx, |this, cx| {
                if let Some(g) = this.gutters.get_mut(&id) {
                    g.stoppable = Some((version, lines));
                    g.computing = None;
                }
                this.update_hover(id, cx);
            });
        });
        g.computing = Some((version, task));
        None
    }

    /// The hover icon and the tooltip of the line number under the mouse.
    pub(crate) fn update_hover(&mut self, id: EntityId, cx: &mut Context<Self>) {
        let Some(g) = self.gutters.get(&id) else {
            return;
        };
        let editor = g.editor.clone();
        let hovered = g.hovered;
        let path = editor.read(cx).path().cloned();
        let (Some(line), Some(path), Some(debugger)) = (hovered, path, self.debugger.clone())
        else {
            editor.update(cx, |e, cx| e.set_gutter_hover_icon(None, cx));
            self.set_tooltip(&editor, None, cx);
            return;
        };
        if let Some(b) = debugger.breakpoints().at(&path, line as u32 + 1) {
            editor.update(cx, |e, cx| e.set_gutter_hover_icon(None, cx));
            let status = debugger.breakpoint_status(b.id);
            let (default_suspend, thread) = line_defaults(&debugger);
            let lines = presentation::tooltip(b, status.as_ref(), default_suspend, thread);
            self.set_tooltip(&editor, Some((line, lines)), cx);
            return;
        }
        let stoppable = self
            .stoppable_now(id, cx)
            .flatten()
            .is_some_and(|l| l.get(line).copied().unwrap_or(false));
        let icon = stoppable.then_some((line, icons::BREAKPOINT));
        editor.update(cx, |e, cx| e.set_gutter_hover_icon(icon, cx));
        let tip = stoppable.then(|| (line, vec![presentation::ADD_HINT.to_owned()]));
        self.set_tooltip(&editor, tip, cx);
    }

    /// Shows a tooltip for a line after the delay, or hides it.
    fn set_tooltip(
        &mut self,
        editor: &Entity<Editor>,
        tip: Option<(usize, Vec<String>)>,
        cx: &mut Context<Self>,
    ) {
        // The mouse is over another line number, or no longer over one.
        let Some((line, lines)) = tip else {
            if self.tooltip.take().is_some() {
                cx.notify();
            }
            return;
        };
        if let Some(t) = &mut self.tooltip
            && t.editor == *editor
            && t.line == line
        {
            if t.lines != lines {
                t.lines = lines;
                cx.notify();
            }
            return;
        }
        let delay = self.tooltip_delay;
        let shown = delay.is_zero();
        let timer = if shown {
            Task::ready(())
        } else {
            let wait = cx.background_executor().timer(delay);
            cx.spawn(async move |this, cx| {
                wait.await;
                let _ = this.update(cx, |this, cx| {
                    if let Some(t) = &mut this.tooltip {
                        t.shown = true;
                        cx.notify();
                    }
                });
            })
        };
        self.tooltip = Some(Tooltip {
            editor: editor.clone(),
            line,
            lines,
            shown,
            _timer: timer,
        });
        cx.notify();
    }

    // ---- decorations --------------------------------------------------------------

    pub(crate) fn refresh_all(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<EntityId> = self.gutters.keys().copied().collect();
        for id in ids {
            self.refresh_editor(id, cx);
        }
    }

    /// The gutters of the editors of a file show its breakpoints again.
    pub(crate) fn refresh_path(&mut self, path: &Path, cx: &mut Context<Self>) {
        let ids: Vec<EntityId> = self
            .gutters
            .iter()
            .filter(|(_, g)| g.editor.read(cx).path().is_some_and(|p| p == path))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.refresh_editor(id, cx);
        }
        cx.notify();
    }

    /// An editor's decorations: one per breakpoint of its file, with the
    /// icon, the background, the stripe mark and the tooltip.
    pub(crate) fn refresh_editor(&mut self, id: EntityId, cx: &mut Context<Self>) {
        let Some(g) = self.gutters.get(&id) else {
            return;
        };
        let editor = g.editor.clone();
        let (path, len) = {
            let e = editor.read(cx);
            (e.path().cloned(), e.buffer().len_lines())
        };
        let mut shown: Vec<BreakpointId> = Vec::new();
        let mut items = Vec::new();
        let mut clicks = false;
        if let (Some(debugger), Some(path)) = (&self.debugger, &path) {
            let breakpoints = debugger.breakpoints();
            let muted = debugger.is_muted();
            let (default_suspend, thread) = line_defaults(debugger);
            let in_file = breakpoints.in_file(path);
            clicks = !in_file.is_empty() || debugger.provider().line_breakpoint_file(path);
            for b in in_file {
                let line = b.line as usize - 1;
                if line >= len {
                    continue;
                }
                let status = debugger.breakpoint_status(b.id);
                let tooltip =
                    presentation::tooltip(b, status.as_ref(), default_suspend, thread).join("\n");
                let mut decoration = Decoration::default()
                    .gutter_icon(
                        presentation::line_icon(b, muted, status.as_ref()),
                        Some(tooltip.into()),
                    )
                    .badge(presentation::badge(b.condition.as_deref()))
                    .stripe(UiRole::DiagnosticErrorStripe, false);
                if b.enabled && b.suspend != SuspendPolicy::None {
                    decoration = decoration.background(UiRole::Breakpoint, layer::BREAKPOINT);
                }
                items.push((line..line + 1, decoration));
                shown.push(b.id);
            }
        }
        let ids = editor.update(cx, |e, cx| {
            e.set_line_number_clicks(clicks);
            e.set_decorations(GROUP, items, cx)
        });
        if let Some(g) = self.gutters.get_mut(&id) {
            g.marks = shown.into_iter().zip(ids).collect();
        }
        self.update_hover(id, cx);
    }

    /// The tooltip over the active editor, once its delay is over.
    pub(crate) fn render_tooltip(
        &self,
        active: Option<&Entity<Editor>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let t = self.tooltip.as_ref().filter(|t| t.shown)?;
        if active != Some(&t.editor) {
            return None;
        }
        let cell = t.editor.read(cx).line_number_bounds(t.line)?;
        let theme = cx.theme();
        // Right of the mouse over the icon, its first line level with the
        // icon (the reference's tooltip at the mouse).
        let position = point(cell.right() - px(4.0), cell.top() - px(4.0));
        Some(
            deferred(
                anchored()
                    .position(position)
                    .snap_to_window_with_margin(px(4.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .px(px(10.0))
                            .py(px(6.0))
                            .rounded(theme.metric(themes::Metric::CornerRadius) * 2.0)
                            .border_1()
                            .border_color(theme.color(UiRole::PopupBorder))
                            .bg(theme.color(UiRole::SurfaceRaised))
                            .shadow(ui::components::popup_shadow())
                            .font_family(theme.ui_family())
                            .text_size(theme.ui_size())
                            .text_color(theme.color(UiRole::Text))
                            .whitespace_nowrap()
                            .children(t.lines.iter().map(|l| div().child(l.clone()))),
                    ),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}
